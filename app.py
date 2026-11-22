"""PhaseMiX Flask control panel server for multi-output Windows WASAPI audio routing."""

from __future__ import annotations

import logging
import math
import os
import subprocess
import threading
import time
import webbrowser
from urllib.parse import urlparse

from flask import Flask, abort, jsonify, request, send_from_directory
from flask_cors import CORS

logging.getLogger("werkzeug").setLevel(logging.WARNING)

from audio_engine import (
    AudioRouterEngine,
    get_windows_endpoint_volumes,
    set_windows_endpoint_volume,
)

BASE_DIR = os.path.dirname(os.path.abspath(__file__))
FRONTEND_DIST_DIR = os.path.join(BASE_DIR, "frontend", "dist")
CONFIG_PATH = os.path.join(BASE_DIR, "config.json")

ALLOWED_LOCAL_ORIGINS = {
    "http://127.0.0.1:5055",
    "http://localhost:5055",
    "http://127.0.0.1:5173",
    "http://localhost:5173",
}

app = Flask(__name__, static_folder=None)
CORS(app, origins=list(ALLOWED_LOCAL_ORIGINS))
engine = AudioRouterEngine(CONFIG_PATH)


@app.before_request
def _security_guard():
    # Reject DNS-rebinding Host headers and cross-origin POST mutations from external websites
    host = (request.host or "").split(":")[0].lower()
    if host not in ("127.0.0.1", "localhost", ""):
        abort(403)
    if request.method in ("POST", "PUT", "PATCH", "DELETE"):
        origin = request.headers.get("Origin")
        if origin:
            parsed = urlparse(origin)
            origin_host = (parsed.hostname or "").lower()
            if origin_host not in ("127.0.0.1", "localhost"):
                abort(403)


@app.after_request
def _security_headers(resp):
    resp.headers["X-Content-Type-Options"] = "nosniff"
    resp.headers["X-Frame-Options"] = "DENY"
    resp.headers["Referrer-Policy"] = "no-referrer"
    if request.path.startswith("/api/"):
        resp.headers["Cache-Control"] = "no-store"
    return resp

_win_vol_cache: dict = {"ts": 0.0, "data": []}
_win_vol_lock = threading.Lock()


def _get_cached_win_vols(force: bool = False) -> list[dict]:
    now = time.monotonic()
    with _win_vol_lock:
        if force or (now - _win_vol_cache["ts"]) > 4.0:
            _win_vol_cache["data"] = get_windows_endpoint_volumes()
            _win_vol_cache["ts"] = now
        return list(_win_vol_cache["data"])


def _match_win_vol(volumes: list[dict], friendly_name: str) -> dict | None:
    if not friendly_name:
        return None
    needle = friendly_name.replace(" [Loopback]", "").strip().lower()
    for item in volumes:
        if item["name"].lower() == needle:
            return item
    for item in volumes:
        if needle in item["name"].lower() or item["name"].lower() in needle:
            return item
    return None


def _build_status_payload(force_vol_refresh: bool = False) -> dict:
    vols = _get_cached_win_vols(force=force_vol_refresh)
    return {
        "running": engine.running,
        "mixer": dict(engine.mixer),
        "meters": dict(engine.meters),
        "telemetry": dict(engine.telemetry),
        "selected": dict(engine.selected_ids),
        "selected_names": dict(engine.selected_names),
        "win_vols": {
            "input": _match_win_vol(vols, engine.selected_names.get("input_name", "")),
            "intex": _match_win_vol(vols, engine.selected_names.get("intex_name", "")),
            "boat": _match_win_vol(vols, engine.selected_names.get("boat_name", "")),
            "laptop": _match_win_vol(vols, engine.selected_names.get("laptop_name", "")),
        },
    }


@app.route("/")
def index():
    if os.path.exists(os.path.join(FRONTEND_DIST_DIR, "index.html")):
        return send_from_directory(FRONTEND_DIST_DIR, "index.html")
    return jsonify({"ok": False, "error": "Frontend not built. Run npm run build in frontend/"}), 503


@app.route("/assets/<path:filename>")
def frontend_assets(filename: str):
    return send_from_directory(os.path.join(FRONTEND_DIST_DIR, "assets"), filename)


@app.route("/<path:filename>")
def frontend_static(filename: str):
    dist_file = os.path.join(FRONTEND_DIST_DIR, filename)
    if os.path.isfile(dist_file):
        return send_from_directory(FRONTEND_DIST_DIR, filename)
    abort(404)


@app.route("/api/devices", methods=["GET", "POST"])
def devices():
    if request.method == "POST":
        body = request.get_json(silent=True)
        if not isinstance(body, dict):
            return jsonify({"ok": False, "error": "Invalid JSON object"}), 400
        engine.set_devices(
            body.get("input_id", ""),
            body.get("intex_id", ""),
            body.get("boat_id", ""),
            body.get("laptop_id", ""),
        )
        return jsonify(_build_status_payload(force_vol_refresh=True))
    info = engine.refresh_device_selection(persist=True)
    info["win_vols"] = _build_status_payload(force_vol_refresh=True)["win_vols"]
    return jsonify(info)


@app.route("/api/state", methods=["GET"])
def state():
    return jsonify(_build_status_payload(force_vol_refresh=True))


@app.route("/api/telemetry", methods=["GET"])
def telemetry():
    return jsonify({
        "running": engine.running,
        "mixer": dict(engine.mixer),
        "meters": dict(engine.meters),
        "telemetry": dict(engine.telemetry),
        "win_vols": _build_status_payload(force_vol_refresh=False)["win_vols"],
    })


@app.route("/api/meters", methods=["GET"])
def meters_fast():
    """Ultra-lightweight meter endpoint avoiding COM and mixer serialization."""
    return jsonify({
        "running": engine.running,
        "meters": dict(engine.meters),
        "telemetry": dict(engine.telemetry),
        "master_vol": engine.mixer.get("master_vol", 23.0),
        "master_mute": engine.mixer.get("master_mute", False),
    })


@app.route("/api/mixer", methods=["POST"])
def mixer():
    body = request.get_json(silent=True)
    if not isinstance(body, dict):
        return jsonify({"ok": False, "error": "Invalid JSON object"}), 400
    updated = engine.update_mixer(body)
    return jsonify({"ok": True, "mixer": updated})


@app.route("/api/start", methods=["POST"])
def start_engine():
    engine.start()
    return jsonify(_build_status_payload(force_vol_refresh=True))


@app.route("/api/stop", methods=["POST"])
def stop_engine():
    engine.stop()
    return jsonify(_build_status_payload(force_vol_refresh=False))


@app.route("/api/boost_capture", methods=["POST"])
def boost_capture():
    in_name = engine.selected_names.get("input_name", "")
    ok = set_windows_endpoint_volume(in_name, 100.0, unmute=True)
    return jsonify({"ok": ok, **_build_status_payload(force_vol_refresh=True)})


@app.route("/api/set_win_vol", methods=["POST"])
def set_win_vol():
    body = request.get_json(silent=True)
    if not isinstance(body, dict):
        return jsonify({"ok": False, "error": "Invalid JSON object"}), 400
    role = str(body.get("role", ""))
    if role not in ("input", "intex", "boat", "laptop"):
        return jsonify({"ok": False, "error": "Invalid role"}), 400
    try:
        vol = float(body.get("volume_pct", 100.0))
        if not math.isfinite(vol):
            raise ValueError("non-finite")
    except (TypeError, ValueError):
        return jsonify({"ok": False, "error": "Invalid volume"}), 400
    vol = max(0.0, min(100.0, vol))
    target_name = engine.selected_names.get(f"{role}_name", "")
    ok = set_windows_endpoint_volume(target_name, vol, unmute=True)
    return jsonify({"ok": ok, **_build_status_payload(force_vol_refresh=True)})


@app.route("/api/open_sound_settings", methods=["POST"])
def open_sound_settings():
    try:
        os.startfile("ms-settings:sound")
    except Exception:
        subprocess.Popen(["control", "mmsys.cpl"], shell=False)
    return jsonify({"ok": True})


def _open_browser_later(url: str) -> None:
    threading.Timer(0.9, lambda: webbrowser.open(url)).start()


if __name__ == "__main__":
    port = int(os.environ.get("MIXER_PORT", "5055"))
    url = f"http://127.0.0.1:{port}"
    print(f"Starting Intex 2.2 + boAt Center Mixer Control Panel at {url}")
    engine.start()
    if os.environ.get("NO_BROWSER") != "1":
        _open_browser_later(url)
    try:
        app.run(host="127.0.0.1", port=port, debug=False, threaded=True)
    finally:
        engine.stop()
