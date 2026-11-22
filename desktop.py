"""Native desktop wrapper for PhaseMiX using Edge WebView2, Waitress, and System Tray."""

from __future__ import annotations

import ctypes
import json
import os
import socket
import sys
import threading
import time
import urllib.request

# Guard against None stdio in PyInstaller --noconsole GUI builds
if sys.stdout is None:
    sys.stdout = open(os.devnull, "w", encoding="utf-8")
if sys.stderr is None:
    sys.stderr = open(os.devnull, "w", encoding="utf-8")

from PIL import Image
import pystray
from waitress import create_server
import webview

from app import CONFIG_PATH, RESOURCE_DIR, _get_cached_win_vols, app, engine


def _set_app_user_model_id() -> None:
    try:
        ctypes.windll.shell32.SetCurrentProcessExplicitAppUserModelID("PhaseMiX.Audio.Console")
    except Exception:
        pass


def _acquire_single_instance() -> bool:
    """Prevent duplicate audio engines from locking the same WASAPI streams."""
    kernel32 = ctypes.windll.kernel32
    mutex = kernel32.CreateMutexW(None, False, "Local\\PhaseMiXDesktopApp")
    if mutex and kernel32.GetLastError() == 183:  # ERROR_ALREADY_EXISTS
        hwnd = ctypes.windll.user32.FindWindowW(None, "PhaseMiX")
        if hwnd:
            ctypes.windll.user32.ShowWindow(hwnd, 9)  # SW_RESTORE
            ctypes.windll.user32.SetForegroundWindow(hwnd)
        return False
    _acquire_single_instance._mutex = mutex  # keep handle alive for process lifetime
    return True


def _pick_free_port(preferred: int = 5055) -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        try:
            s.bind(("127.0.0.1", preferred))
            return preferred
        except OSError:
            pass
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return int(s.getsockname()[1])


def _icon_path() -> str | None:
    for candidate in (
        os.path.join(RESOURCE_DIR, "assets", "phasemix.ico"),
        os.path.join(os.path.dirname(os.path.abspath(__file__)), "installer", "phasemix.ico"),
    ):
        if os.path.exists(candidate):
            return candidate
    return None


def _run_smoke_test(port: int, server, ico: str | None) -> int:
    """Verify frozen WSGI server, built frontend HTML, meters API, and live Edge WebView2 React mount."""
    base = f"http://127.0.0.1:{port}"
    log_path = os.environ.get("PHASEMIX_SMOKE_LOG")
    result = {"ok": False, "err": ""}
    try:
        for _ in range(40):
            try:
                with urllib.request.urlopen(f"{base}/api/meters", timeout=2.0) as r:
                    if r.status == 200:
                        break
            except Exception:
                time.sleep(0.1)
        with urllib.request.urlopen(f"{base}/", timeout=5.0) as r_html:
            html = r_html.read().decode("utf-8", errors="ignore")
            assert r_html.status == 200 and "<title>PhaseMiX</title>" in html, "Frontend HTML check failed"
        with urllib.request.urlopen(f"{base}/api/state", timeout=10.0) as r_state:
            data = json.loads(r_state.read().decode("utf-8"))
            assert r_state.status == 200 and data.get("running") is True, "Engine failed to report running"

        if "--smoke-gui" in sys.argv:
            win = webview.create_window(
                "PhaseMiX",
                f"{base}/",
                width=1100,
                height=720,
                hidden=True,
                background_color="#0f1014",
            )

            def verify_dom() -> None:
                try:
                    deadline = time.monotonic() + 10.0
                    while time.monotonic() < deadline:
                        try:
                            title = win.evaluate_js("document.title")
                            h1 = win.evaluate_js("document.querySelector('h1')?.textContent || ''")
                            if title == "PhaseMiX" and "PhaseMiX" in str(h1):
                                result["ok"] = True
                                break
                        except Exception:
                            pass
                        time.sleep(0.2)
                    if not result["ok"]:
                        result["err"] = "React root did not render PhaseMiX h1 before timeout"
                    else:
                        # Brief settle before closing WebView2 to avoid WinForms/GPU shutdown race
                        time.sleep(0.15)
                finally:
                    try:
                        win.destroy()
                    except Exception:
                        pass

            wv_cache = os.path.join(os.path.dirname(CONFIG_PATH), "webview_smoke")
            webview.start(verify_dom, gui="edgechromium", icon=ico, private_mode=False, storage_path=wv_cache)
            assert result["ok"], result["err"] or "WebView2 verification failed"

        msg = f"PHASEMIX_EXE_SMOKE_PASS port={port} gui={'--smoke-gui' in sys.argv}"
        if log_path:
            with open(log_path, "w", encoding="utf-8") as f:
                f.write(msg)
        print(msg)
        return 0
    except Exception as exc:
        err_msg = f"SMOKE_FAIL: {exc!r}"
        if log_path:
            with open(log_path, "w", encoding="utf-8") as f:
                f.write(err_msg)
        print(err_msg)
        return 1
    finally:
        try:
            engine.stop()
        except Exception:
            pass
        try:
            server.close()
        except Exception:
            pass
        try:
            sys.stdout.flush()
            sys.stderr.flush()
        except Exception:
            pass


def main() -> int:
    smoke = "--smoke-test" in sys.argv
    if not smoke and not _acquire_single_instance():
        return 0

    _set_app_user_model_id()
    port = _pick_free_port(int(os.environ.get("MIXER_PORT", "5055")))
    server = create_server(app, host="127.0.0.1", port=port, threads=8)
    srv_thread = threading.Thread(target=server.run, name="phasemix-http", daemon=True)
    srv_thread.start()

    ico = _icon_path()
    engine.start()
    # Pre-warm pycaw COM volume cache off the HTTP thread so first /api/state returns immediately
    threading.Thread(target=lambda: _get_cached_win_vols(force=True), name="winvol-warmup", daemon=True).start()
    if smoke:
        code = _run_smoke_test(port, server, ico)
        os._exit(code)

    state = {"quitting": False, "window": None, "tray": None}

    def show_window() -> None:
        win = state["window"]
        if win is not None:
            try:
                win.show()
                win.restore()
            except Exception:
                pass

    def quit_app() -> None:
        state["quitting"] = True
        tray = state["tray"]
        if tray is not None:
            try:
                tray.stop()
            except Exception:
                pass
        win = state["window"]
        if win is not None:
            try:
                win.destroy()
            except Exception:
                pass

    if ico:
        try:
            img = Image.open(ico)
            menu = pystray.Menu(
                pystray.MenuItem("Open PhaseMiX", lambda *_: show_window(), default=True),
                pystray.MenuItem(
                    "Mute / Unmute All",
                    lambda *_: engine.update_mixer({"master_mute": not bool(engine.mixer.get("master_mute", False))}),
                ),
                pystray.Menu.SEPARATOR,
                pystray.MenuItem("Quit PhaseMiX", lambda *_: quit_app()),
            )
            tray = pystray.Icon("PhaseMiX", img, "PhaseMiX Audio Console", menu)
            state["tray"] = tray
            threading.Thread(target=tray.run, name="phasemix-tray", daemon=True).start()
        except Exception:
            state["tray"] = None

    window = webview.create_window(
        "PhaseMiX",
        f"http://127.0.0.1:{port}/",
        width=1380,
        height=880,
        min_size=(940, 620),
        background_color="#0f1014",
    )
    state["window"] = window

    def on_closing() -> bool:
        if state["quitting"] or not engine.running or state["tray"] is None:
            return True
        window.hide()
        return False

    window.events.closing += on_closing
    wv_cache = os.path.join(os.path.dirname(CONFIG_PATH), "webview")

    try:
        webview.start(
            gui="edgechromium",
            icon=ico,
            private_mode=False,
            storage_path=wv_cache,
        )
    finally:
        state["quitting"] = True
        if state["tray"] is not None:
            try:
                state["tray"].stop()
            except Exception:
                pass
        engine.stop()
        try:
            server.close()
        except Exception:
            pass
    os._exit(0)


if __name__ == "__main__":
    raise SystemExit(main())
