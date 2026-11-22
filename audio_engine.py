"""Dual-output WASAPI audio router with real-time delay sync and Windows volume integration."""

from __future__ import annotations

import collections
import json
import math
import os
import threading
import time
import warnings
from dataclasses import dataclass
from typing import Any

import numpy as np
import pyaudiowpatch as pa
from pycaw.pycaw import AudioUtilities

warnings.filterwarnings("ignore", category=UserWarning, module=r"pycaw(\..*)?")

from dsp import AuxChannelProcessor, IntexProcessor, TestSignalGenerator, block_db

MASTER_RATE = 48000
RING_SECONDS = 3
RING_FRAMES = MASTER_RATE * RING_SECONDS
BASE_BUFFER_MS = 90.0
OUT_BLOCK_FRAMES = 480  # 10 ms at 48 kHz for crisp instant response
PUMP_BLOCK_FRAMES = 480  # 10 ms at 48 kHz
OUTPUT_ROLES = ("intex", "boat", "laptop")

ALLOWED_AUX_MODES = {"stereo", "mono_center", "vocal_mid", "side_ambience"}
ALLOWED_TEST_MODES = {"off", "click", "chirp", "lr"}
ALLOWED_PRESETS = {"custom", "desk_triangle", "cinema_center", "wide_room", "night_safe", "asym_desk", "pure_mirror"}
NUMERIC_BOUNDS: dict[str, tuple[float, float]] = {
    "master_vol": (0.0, 200.0),
    "protection_max_master": (15.0, 200.0),
    "protection_ceiling_db": (-30.0, -0.5),
    "intex_vol": (0.0, 200.0),
    "intex_delay_ms": (0.0, 450.0),
    "intex_balance": (-100.0, 100.0),
    "intex_bass_db": (-12.0, 12.0),
    "intex_mid_db": (-12.0, 12.0),
    "intex_treble_db": (-12.0, 12.0),
    "intex_width": (50.0, 150.0),
    "boat_vol": (0.0, 200.0),
    "boat_delay_ms": (0.0, 450.0),
    "boat_hpf_hz": (0.0, 500.0),
    "boat_lpf_hz": (3000.0, 18000.0),
    "boat_bass_db": (-12.0, 12.0),
    "boat_mid_db": (-12.0, 12.0),
    "boat_treble_db": (-12.0, 12.0),
    "laptop_vol": (0.0, 200.0),
    "laptop_delay_ms": (0.0, 450.0),
    "laptop_hpf_hz": (0.0, 500.0),
    "laptop_lpf_hz": (3000.0, 18000.0),
    "laptop_bass_db": (-12.0, 12.0),
    "laptop_mid_db": (-12.0, 12.0),
    "laptop_treble_db": (-12.0, 12.0),
}
BOOL_KEYS = {"master_mute", "protection_enabled", "intex_mute", "boat_mute", "boat_invert", "laptop_mute", "laptop_invert"}


def sanitize_mixer_patch(patch: dict[str, Any]) -> dict[str, Any]:
    """Validate and clamp untrusted mixer parameter patches."""
    clean: dict[str, Any] = {}
    if not isinstance(patch, dict):
        return clean
    for k, v in patch.items():
        if k in NUMERIC_BOUNDS:
            try:
                num = float(v)
                if math.isfinite(num):
                    lo, hi = NUMERIC_BOUNDS[k]
                    clean[k] = max(lo, min(hi, num))
            except (TypeError, ValueError):
                continue
        elif k in BOOL_KEYS:
            clean[k] = bool(v)
        elif k in ("boat_mode", "laptop_mode") and v in ALLOWED_AUX_MODES:
            clean[k] = str(v)
        elif k == "test_mode" and v in ALLOWED_TEST_MODES:
            clean[k] = str(v)
        elif k == "position_preset" and v in ALLOWED_PRESETS:
            clean[k] = str(v)
        elif k == "solo":
            clean[k] = str(v) if v in OUTPUT_ROLES else None
    return clean

DEFAULT_MIXER = {
    "master_vol": 25.0,
    "master_mute": False,
    "protection_enabled": True,
    "protection_max_master": 60.0,
    "protection_ceiling_db": -15.0,
    "position_preset": "desk_triangle",
    "intex_vol": 100.0,
    "intex_mute": False,
    "intex_delay_ms": 222.0,
    "intex_balance": 0.0,
    "intex_bass_db": 0.0,
    "intex_mid_db": 0.0,
    "intex_treble_db": 0.0,
    "intex_width": 100.0,
    "boat_vol": 85.0,
    "boat_mute": False,
    "boat_delay_ms": 0.0,
    "boat_mode": "mono_center",
    "boat_hpf_hz": 160.0,
    "boat_lpf_hz": 18000.0,
    "boat_bass_db": 0.0,
    "boat_mid_db": 0.0,
    "boat_treble_db": 0.0,
    "boat_invert": False,
    "laptop_vol": 75.0,
    "laptop_mute": False,
    "laptop_delay_ms": 222.0,
    "laptop_mode": "stereo",
    "laptop_hpf_hz": 250.0,
    "laptop_lpf_hz": 18000.0,
    "laptop_bass_db": 0.0,
    "laptop_mid_db": 0.0,
    "laptop_treble_db": 0.0,
    "laptop_invert": False,
    "solo": None,
    "test_mode": "off",
}


@dataclass
class DeviceInfo:
    id: int
    name: str
    rate: int
    channels: int
    is_loopback: bool
    is_input: bool
    is_output: bool
    label: str


def _safe_com_init():
    try:
        import comtypes
        comtypes.CoInitialize()
    except Exception:
        pass


def list_wasapi_devices() -> tuple[list[DeviceInfo], list[DeviceInfo], bool]:
    """Enumerate WASAPI capture/loopback inputs and stereo output endpoints."""
    p = pa.PyAudio()
    inputs: list[DeviceInfo] = []
    outputs: list[DeviceInfo] = []
    has_virtual_cable = False
    try:
        wasapi = p.get_host_api_info_by_type(pa.paWASAPI)
        wasapi_idx = wasapi["index"]
        for i in range(p.get_device_count()):
            d = p.get_device_info_by_index(i)
            if d.get("hostApi") != wasapi_idx:
                continue
            name = str(d.get("name", f"Device {i}"))
            rate = int(d.get("defaultSampleRate", 48000))
            in_ch = int(d.get("maxInputChannels", 0))
            out_ch = int(d.get("maxOutputChannels", 0))
            is_loop = bool(d.get("isLoopbackDevice", False))
            lower = name.lower()

            if "vb-audio" in lower or "cable input" in lower or "cable output" in lower or "voicemeeter" in lower:
                has_virtual_cable = True

            if in_ch >= 1:
                tag = "Loopback" if is_loop else "Mic/Line"
                if "cable" in lower or "voicemeeter" in lower or "steam streaming speakers" in lower:
                    tag = f"Virtual • {tag}"
                inputs.append(
                    DeviceInfo(
                        id=i,
                        name=name,
                        rate=rate,
                        channels=min(2, in_ch),
                        is_loopback=is_loop,
                        is_input=True,
                        is_output=False,
                        label=f"[{tag}] {name} ({rate} Hz)",
                    )
                )

            if out_ch >= 2:
                tag = "Output"
                if "stone" in lower or "bt" in lower or "bluetooth" in lower or "anc" in lower:
                    tag = "Bluetooth"
                elif "realtek" in lower:
                    tag = "Wired / Realtek"
                elif "steam" in lower or "cable" in lower or "voicemeeter" in lower:
                    tag = "Virtual"
                outputs.append(
                    DeviceInfo(
                        id=i,
                        name=name,
                        rate=rate,
                        channels=2,
                        is_loopback=False,
                        is_input=False,
                        is_output=True,
                        label=f"[{tag}] {name} ({rate} Hz)",
                    )
                )
    finally:
        p.terminate()

    def input_sort_key(d: DeviceInfo) -> tuple[int, str]:
        nl = d.name.lower()
        if "cable input" in nl and d.is_loopback:
            return (0, nl)
        if "cable output" in nl:
            return (1, nl)
        if "voicemeeter" in nl and d.is_loopback:
            return (2, nl)
        if "steam streaming speakers" in nl and d.is_loopback:
            return (3, nl)
        if d.is_loopback:
            return (4, nl)
        return (5, nl)

    inputs.sort(key=input_sort_key)
    return inputs, outputs, has_virtual_cable


def pick_default_devices(inputs: list[DeviceInfo], outputs: list[DeviceInfo], saved: dict | None = None) -> dict[str, str]:
    """Match saved device names first; otherwise pick sensible Intex/boAt/Virtual defaults."""
    saved = saved or {}

    def find_by_name(pool: list[DeviceInfo], target_name: str | None) -> str:
        if not target_name:
            return ""
        for d in pool:
            if d.name == target_name:
                return str(d.id)
        return ""

    in_id = find_by_name(inputs, saved.get("input_name"))
    if not in_id:
        for pat in ("cable input", "cable output", "voicemeeter", "steam streaming speakers"):
            for d in inputs:
                if pat in d.name.lower() and (d.is_loopback or "cable output" in d.name.lower()):
                    in_id = str(d.id)
                    break
            if in_id:
                break

    intex_id = find_by_name(outputs, saved.get("intex_name"))
    if not intex_id:
        for pat in ("headphones (realtek", "speakers (realtek"):
            for d in outputs:
                if pat in d.name.lower():
                    intex_id = str(d.id)
                    break
            if intex_id:
                break

    boat_id = find_by_name(outputs, saved.get("boat_name"))
    if not boat_id:
        for pat in ("headphones (stone", "stone 358", "headphones (3.0 bt", "headphones ("):
            for d in outputs:
                if pat in d.name.lower() and "realtek" not in d.name.lower() and str(d.id) != intex_id:
                    boat_id = str(d.id)
                    break
            if boat_id:
                break

    laptop_id = find_by_name(outputs, saved.get("laptop_name"))
    if not laptop_id:
        for pat in ("speakers (realtek",):
            for d in outputs:
                if pat in d.name.lower() and str(d.id) not in (intex_id, boat_id):
                    laptop_id = str(d.id)
                    break
            if laptop_id:
                break

    return {"input_id": in_id, "intex_id": intex_id, "boat_id": boat_id, "laptop_id": laptop_id}


def get_windows_endpoint_volumes() -> list[dict[str, Any]]:
    """Return active Windows render endpoint volumes via pycaw."""
    _safe_com_init()
    results: list[dict[str, Any]] = []
    try:
        for dev in AudioUtilities.GetAllDevices():
            name = str(getattr(dev, "FriendlyName", ""))
            state = str(getattr(dev, "state", ""))
            if "Active" not in state:
                continue
            try:
                ep = dev.EndpointVolume
                vol = round(float(ep.GetMasterVolumeLevelScalar()) * 100.0, 1)
                db = round(float(ep.GetMasterVolumeLevel()), 1)
                mute = bool(ep.GetMute())
                results.append({"name": name, "volume_pct": vol, "db": db, "mute": mute, "id": getattr(dev, "id", "")})
            except Exception:
                continue
    except Exception:
        pass
    return results


def set_windows_endpoint_volume(target_name_substr: str, volume_pct: float, unmute: bool = True) -> bool:
    """Set Windows endpoint volume (0-100) for matching active playback device."""
    _safe_com_init()
    target = target_name_substr.lower().replace(" [loopback]", "").strip()
    changed = False
    try:
        for dev in AudioUtilities.GetAllDevices():
            name = str(getattr(dev, "FriendlyName", ""))
            state = str(getattr(dev, "state", ""))
            if "Active" not in state:
                continue
            if target and target in name.lower():
                try:
                    ep = dev.EndpointVolume
                    if unmute:
                        ep.SetMute(0, None)
                    ep.SetMasterVolumeLevelScalar(max(0.0, min(1.0, volume_pct / 100.0)), None)
                    changed = True
                except Exception:
                    pass
    except Exception:
        pass
    return changed


class AudioRouterEngine:
    """Coordinates WASAPI loopback capture, master delay ring buffer, and synced dual outputs."""

    def __init__(self, config_path: str) -> None:
        self.config_path = config_path
        self._lock = threading.RLock()
        self.mixer: dict[str, Any] = dict(DEFAULT_MIXER)
        self.selected_names: dict[str, str] = {"input_name": "", "intex_name": "", "boat_name": "", "laptop_name": ""}
        self.selected_ids: dict[str, str] = {"input_id": "", "intex_id": "", "boat_id": "", "laptop_id": ""}

        self.running = False
        self._stop_event = threading.Event()
        self._pa: pa.PyAudio | None = None
        self._threads: list[threading.Thread] = []

        # Shared 48 kHz stereo circular buffer
        self.ring = np.zeros((RING_FRAMES, 2), dtype=np.float32)
        self.write_idx = 0
        self._ring_lock = threading.Lock()

        # Loopback chunk queue populated by PyAudio callback
        self._in_queue: collections.deque[np.ndarray] = collections.deque(maxlen=40)
        self._last_loopback_ts = 0.0
        self._in_stream = None
        self._in_rate = MASTER_RATE

        # Live meters & telemetry
        self._save_timer: threading.Timer | None = None
        self.meters = {"in_db": -60.0, "intex_db": -60.0, "boat_db": -60.0, "laptop_db": -60.0}
        self.telemetry: dict[str, Any] = {
            "sample_rate": MASTER_RATE,
            "input_status": "Stopped",
            "intex_status": "Stopped",
            "boat_status": "Stopped",
            "laptop_status": "Stopped",
            "intex_actual_ms": 0.0,
            "boat_actual_ms": 0.0,
            "laptop_actual_ms": 0.0,
            "intex_rate_ppm": 0.0,
            "boat_rate_ppm": 0.0,
            "laptop_rate_ppm": 0.0,
            "intex_gr_db": 0.0,
            "boat_gr_db": 0.0,
            "laptop_gr_db": 0.0,
        }

        self.load_config()

    def load_config(self) -> None:
        if os.path.exists(self.config_path):
            try:
                with open(self.config_path, "r", encoding="utf-8") as f:
                    data = json.load(f)
                self.mixer.update(sanitize_mixer_patch(data.get("mixer", {})))
                if self.mixer.get("protection_enabled", True):
                    cap = float(self.mixer.get("protection_max_master", 50.0))
                    if float(self.mixer.get("master_vol", 0.0)) > cap:
                        self.mixer["master_vol"] = cap
                # Never auto-resume muted or calibration test tones on fresh launch
                self.mixer["test_mode"] = "off"
                self.mixer["master_mute"] = False
                for role in OUTPUT_ROLES:
                    self.mixer[f"{role}_mute"] = False
                self.mixer["solo"] = None
                if isinstance(data.get("devices"), dict):
                    for dk in self.selected_names:
                        if isinstance(data["devices"].get(dk), str):
                            self.selected_names[dk] = data["devices"][dk]
            except Exception:
                pass
        self.refresh_device_selection(persist=False)

    def save_config(self) -> None:
        with self._lock:
            if self._save_timer is not None:
                self._save_timer.cancel()
                self._save_timer = None
            payload = {"mixer": dict(self.mixer), "devices": dict(self.selected_names)}
        try:
            tmp_path = f"{self.config_path}.tmp"
            with open(tmp_path, "w", encoding="utf-8") as f:
                json.dump(payload, f, indent=2)
            os.replace(tmp_path, self.config_path)
        except Exception:
            pass

    def schedule_save(self, delay: float = 0.85) -> None:
        """Debounce disk writes during continuous slider drags."""
        with self._lock:
            if self._save_timer is not None:
                self._save_timer.cancel()
            t = threading.Timer(delay, self.save_config)
            t.daemon = True
            t.start()
            self._save_timer = t

    def refresh_device_selection(self, persist: bool = True) -> dict[str, Any]:
        prev_ids = dict(self.selected_ids)
        inputs, outputs, has_vc = list_wasapi_devices()
        picked = pick_default_devices(inputs, outputs, self.selected_names)
        self.selected_ids = picked

        id_to_in = {str(d.id): d.name for d in inputs}
        id_to_out = {str(d.id): d.name for d in outputs}
        self.selected_names = {
            "input_name": id_to_in.get(picked["input_id"], ""),
            "intex_name": id_to_out.get(picked["intex_id"], ""),
            "boat_name": id_to_out.get(picked["boat_id"], ""),
            "laptop_name": id_to_out.get(picked["laptop_id"], ""),
        }
        if persist:
            self.save_config()
        if self.running and picked != prev_ids:
            self.restart()
        return {
            "inputs": [d.__dict__ for d in inputs],
            "outputs": [d.__dict__ for d in outputs],
            "selected": self.selected_ids,
            "selected_names": self.selected_names,
            "has_virtual_cable": has_vc,
            "windows_volumes": get_windows_endpoint_volumes(),
        }

    def set_devices(self, input_id: str, intex_id: str, boat_id: str, laptop_id: str = "") -> None:
        inputs, outputs, _ = list_wasapi_devices()
        id_to_in = {str(d.id): d.name for d in inputs}
        id_to_out = {str(d.id): d.name for d in outputs}
        in_val = str(input_id or "") if str(input_id or "") in id_to_in else ""
        intex_val = str(intex_id or "") if str(intex_id or "") in id_to_out else ""
        boat_val = str(boat_id or "") if str(boat_id or "") in id_to_out else ""
        laptop_val = str(laptop_id or "") if str(laptop_id or "") in id_to_out else ""
        with self._lock:
            self.selected_ids = {
                "input_id": in_val,
                "intex_id": intex_val,
                "boat_id": boat_val,
                "laptop_id": laptop_val,
            }
            self.selected_names = {
                "input_name": id_to_in.get(in_val, ""),
                "intex_name": id_to_out.get(intex_val, ""),
                "boat_name": id_to_out.get(boat_val, ""),
                "laptop_name": id_to_out.get(laptop_val, ""),
            }
            self.save_config()
        if self.running:
            self.restart()

    def update_mixer(self, patch: dict[str, Any]) -> dict[str, Any]:
        clean = sanitize_mixer_patch(patch)
        with self._lock:
            for k, v in clean.items():
                if k in self.mixer:
                    self.mixer[k] = v
            if self.mixer.get("protection_enabled", True):
                cap = float(self.mixer.get("protection_max_master", 50.0))
                if float(self.mixer.get("master_vol", 0.0)) > cap:
                    self.mixer["master_vol"] = cap
            self.schedule_save()
            return dict(self.mixer)

    def start(self) -> None:
        with self._lock:
            if self.running:
                return
            self._stop_event.clear()
            self.ring.fill(0.0)
            # Pre-seed write cursor 600 ms into silent ring so high-delay outputs start synced without clamping
            self.write_idx = int(0.6 * MASTER_RATE)
            self._in_queue.clear()
            self._last_loopback_ts = time.perf_counter()
            self._pa = pa.PyAudio()
            self.running = True

            self._open_input_stream()

            pump = threading.Thread(target=self._pump_loop, name="audio-pump", daemon=True)
            pump.start()
            self._threads = [pump]

            bridge = threading.Thread(target=self._win_key_bridge_loop, name="win-key-bridge", daemon=True)
            bridge.start()
            self._threads.append(bridge)

            for role in OUTPUT_ROLES:
                dev_str = self.selected_ids.get(f"{role}_id", "")
                if dev_str:
                    t = threading.Thread(
                        target=self._output_loop,
                        args=(role, int(dev_str)),
                        name=f"out-{role}",
                        daemon=True,
                    )
                    t.start()
                    self._threads.append(t)
                else:
                    self.telemetry[f"{role}_status"] = "Disabled"

    def stop(self) -> None:
        with self._lock:
            if not self.running:
                return
            self._stop_event.set()
            self.running = False

        for t in self._threads:
            t.join(timeout=2.5)
        still_alive = any(t.is_alive() for t in self._threads)
        self._threads.clear()

        if self._in_stream is not None:
            try:
                self._in_stream.stop_stream()
                self._in_stream.close()
            except Exception:
                pass
            self._in_stream = None

        if self._pa is not None and not still_alive:
            try:
                self._pa.terminate()
            except Exception:
                pass
            self._pa = None

        self.meters = {"in_db": -60.0, "intex_db": -60.0, "boat_db": -60.0, "laptop_db": -60.0}
        self.telemetry["input_status"] = "Stopped"
        for role in OUTPUT_ROLES:
            self.telemetry[f"{role}_status"] = "Stopped"
        self.save_config()

    def restart(self) -> None:
        self.stop()
        time.sleep(0.1)
        self.start()

    def _open_input_stream(self) -> None:
        in_id_str = self.selected_ids.get("input_id", "")
        if not in_id_str or self._pa is None:
            self.telemetry["input_status"] = "Test Generator Only"
            return
        try:
            dev_idx = int(in_id_str)
            info = self._pa.get_device_info_by_index(dev_idx)
            self._in_rate = int(info.get("defaultSampleRate", MASTER_RATE))
            in_ch = max(1, min(2, int(info.get("maxInputChannels", 2))))

            def cb(in_data, frame_count, time_info, status):
                if self._stop_event.is_set():
                    return (None, pa.paComplete)
                arr = np.frombuffer(in_data, dtype=np.float32)
                if arr.size:
                    frames = arr.reshape(-1, in_ch)
                    if in_ch == 1:
                        frames = np.repeat(frames, 2, axis=1)
                    if self._in_rate != MASTER_RATE:
                        new_len = max(1, int(round(frames.shape[0] * MASTER_RATE / self._in_rate)))
                        xp = np.linspace(0.0, 1.0, frames.shape[0], endpoint=False)
                        xq = np.linspace(0.0, 1.0, new_len, endpoint=False)
                        frames = np.column_stack([np.interp(xq, xp, frames[:, 0]), np.interp(xq, xp, frames[:, 1])]).astype(np.float32)
                    self._in_queue.append(frames.copy())
                    self._last_loopback_ts = time.perf_counter()
                return (None, pa.paContinue)

            self._in_stream = self._pa.open(
                format=pa.paFloat32,
                channels=in_ch,
                rate=self._in_rate,
                input=True,
                input_device_index=dev_idx,
                frames_per_buffer=PUMP_BLOCK_FRAMES,
                stream_callback=cb,
            )
            self._in_stream.start_stream()
            self.telemetry["input_status"] = f"Capturing ({self._in_rate} Hz)"
        except Exception as exc:
            self.telemetry["input_status"] = f"Capture error: {exc}"
            self._in_stream = None

    def _win_key_bridge_loop(self) -> None:
        """Keep virtual loopback endpoint at ~unity (0.98) while bridging Windows Vol/Mute keys to master bus."""
        in_name = self.selected_names.get("input_name", "").lower()
        if not any(tag in in_name for tag in ("steam streaming", "cable input", "voicemeeter")):
            return
        _safe_com_init()
        ep = None
        needle = in_name.replace(" [loopback]", "").strip()
        try:
            for dev in AudioUtilities.GetAllDevices():
                name = str(getattr(dev, "FriendlyName", "")).lower()
                if "Active" in str(getattr(dev, "state", "")) and needle and needle in name:
                    ep = dev.EndpointVolume
                    break
        except Exception:
            return
        if ep is None:
            return

        UNITY_SCALAR = 0.98
        try:
            ep.SetMute(0, None)
            ep.SetMasterVolumeLevelScalar(UNITY_SCALAR, None)
        except Exception:
            pass

        while not self._stop_event.is_set():
            time.sleep(0.15)
            try:
                muted = bool(ep.GetMute())
                if muted:
                    with self._lock:
                        self.mixer["master_mute"] = not bool(self.mixer.get("master_mute", False))
                        self.save_config()
                    ep.SetMute(0, None)

                scalar = float(ep.GetMasterVolumeLevelScalar())
                diff = scalar - UNITY_SCALAR
                if abs(diff) >= 0.012:
                    steps = round(diff / 0.02)
                    delta = steps * 4.0 if abs(steps) <= 3 else diff * 120.0
                    with self._lock:
                        cur = float(self.mixer.get("master_vol", 25.0))
                        cap = float(self.mixer.get("protection_max_master", 50.0)) if self.mixer.get("protection_enabled", True) else 200.0
                        self.mixer["master_vol"] = round(max(0.0, min(cap, cur + delta)), 0)
                        self.schedule_save()
                    ep.SetMasterVolumeLevelScalar(UNITY_SCALAR, None)
            except Exception:
                break

    def _write_ring(self, frames: np.ndarray) -> None:
        n = frames.shape[0]
        if n <= 0:
            return
        with self._ring_lock:
            start = self.write_idx % RING_FRAMES
            first = min(n, RING_FRAMES - start)
            self.ring[start : start + first] = frames[:first]
            if first < n:
                self.ring[0 : n - first] = frames[first:]
            self.write_idx += n

    def _read_ring_interpolated(self, read_pos: float, out_frames: int, step: float) -> np.ndarray:
        # Fast contiguous slice path when playback rate matches 1:1 (avoids fancy indexing & float64 work)
        if abs(step - 1.0) < 1e-6 and abs(read_pos - round(read_pos)) < 0.05:
            idx_start = int(round(read_pos))
            with self._ring_lock:
                w = self.write_idx
                avail = max(0, min(out_frames, w - idx_start))
                if avail == 0:
                    return np.zeros((out_frames, 2), dtype=np.float32)
                s = idx_start % RING_FRAMES
                first = min(avail, RING_FRAMES - s)
                if avail == out_frames and first == out_frames:
                    return self.ring[s : s + out_frames].copy()
                out = np.zeros((out_frames, 2), dtype=np.float32)
                out[:first] = self.ring[s : s + first]
                if first < avail:
                    out[first:avail] = self.ring[0 : avail - first]
                return out

        positions = read_pos + np.arange(out_frames, dtype=np.float64) * step
        idx0 = np.floor(positions).astype(np.int64)
        frac = (positions - idx0).astype(np.float32)[:, None]
        i0 = idx0 % RING_FRAMES
        i1 = (idx0 + 1) % RING_FRAMES
        with self._ring_lock:
            w = self.write_idx
            s0 = self.ring[i0]
            s1 = self.ring[i1]
        out = s0 + (s1 - s0) * frac
        ahead = positions >= (w - 1)
        if np.any(ahead):
            out[ahead] = 0.0
        return out

    def _pump_loop(self) -> None:
        gen = TestSignalGenerator(MASTER_RATE)
        silence = np.zeros((PUMP_BLOCK_FRAMES, 2), dtype=np.float32)
        next_tick = time.perf_counter()
        dt = PUMP_BLOCK_FRAMES / MASTER_RATE

        while not self._stop_event.is_set():
            now = time.perf_counter()
            if now < next_tick:
                time.sleep(max(0.001, next_tick - now))
                continue
            if now - next_tick > 0.15:
                next_tick = now
            next_tick += dt

            mode = self.mixer.get("test_mode", "off")
            if mode != "off":
                self._in_queue.clear()
                blk = gen.generate(mode, PUMP_BLOCK_FRAMES)
                self._write_ring(blk)
                self.meters["in_db"] = block_db(blk)
                continue

            # Drain incoming WASAPI loopback chunks when audio is playing
            pulled_any = False
            peak_blk = None
            while self._in_queue:
                # Drop stale excess if producer queue grew too large
                if len(self._in_queue) > 12:
                    self._in_queue.popleft()
                    continue
                chunk = self._in_queue.popleft()
                self._write_ring(chunk)
                peak_blk = chunk
                pulled_any = True

            if pulled_any and peak_blk is not None:
                self.meters["in_db"] = block_db(peak_blk)
            elif (time.perf_counter() - self._last_loopback_ts) > 0.065:
                # WASAPI loopback pauses when Windows audio is silent; keep ring clock alive smoothly
                self._write_ring(silence)
                self.meters["in_db"] = -60.0

    def _output_loop(self, role: str, device_idx: int) -> None:
        status_key = f"{role}_status"
        actual_key = f"{role}_actual_ms"
        ppm_key = f"{role}_rate_ppm"
        meter_key = f"{role}_db"

        while not self._stop_event.is_set():
            stream = None
            out_rate = MASTER_RATE
            try:
                if self._pa is None:
                    break
                info = self._pa.get_device_info_by_index(device_idx)
                # Try native rate first, fall back to MASTER_RATE if needed
                native_rate = int(info.get("defaultSampleRate", MASTER_RATE))
                out_rate = native_rate if native_rate in (44100, 48000, 96000) else MASTER_RATE
                out_frames = max(256, int(round(OUT_BLOCK_FRAMES * out_rate / MASTER_RATE)))

                stream = self._pa.open(
                    format=pa.paFloat32,
                    channels=2,
                    rate=out_rate,
                    output=True,
                    output_device_index=device_idx,
                    frames_per_buffer=out_frames,
                )
                self.telemetry[status_key] = f"Playing ({out_rate} Hz)"

                dsp = IntexProcessor(out_rate) if role == "intex" else AuxChannelProcessor(out_rate, role=role)
                # Prime PortAudio output buffer silently so initial stream.write calls don't jump read_pos ahead
                prime_zeros = np.zeros((out_frames, 2), dtype=np.float32).tobytes()
                for _ in range(3):
                    if self._stop_event.is_set():
                        break
                    stream.write(prime_zeros, exception_on_underflow=False)

                with self._ring_lock:
                    w0 = self.write_idx
                delay_ms = float(self.mixer.get(f"{role}_delay_ms", 0.0))
                prev_delay_ms = delay_ms
                target_lag = (BASE_BUFFER_MS + delay_ms) * MASTER_RATE / 1000.0
                read_pos = float(max(0.0, w0 - target_lag))
                smoothed_diff = 0.0
                fade_in = 1.0

                while not self._stop_event.is_set():
                    with self._lock:
                        params = dict(self.mixer)
                    with self._ring_lock:
                        w = self.write_idx

                    delay_ms = float(params.get(f"{role}_delay_ms", 0.0))
                    target_lag = (BASE_BUFFER_MS + delay_ms) * MASTER_RATE / 1000.0
                    desired_pos = w - target_lag
                    diff = desired_pos - read_pos

                    # Snap cleanly when delay slider moves or stream jumps (> 250 ms), avoiding pitch bend
                    if abs(delay_ms - prev_delay_ms) > 0.5 or abs(diff) > MASTER_RATE * 0.25:
                        prev_delay_ms = delay_ms
                        if abs(diff) > 240.0:
                            read_pos = max(0.0, desired_pos)
                            smoothed_diff = 0.0
                            fade_in = 0.0
                            diff = 0.0

                    # Slow crystal-drift filter + 8 ms deadband prevents audible pitch warble on volume changes
                    smoothed_diff = 0.992 * smoothed_diff + 0.008 * diff
                    if abs(smoothed_diff) < 384.0:
                        rate_adj = 1.0
                    else:
                        rate_adj = 1.0 + float(np.clip(smoothed_diff / (MASTER_RATE * 12.0), -0.0015, 0.0015))

                    step = (MASTER_RATE / float(out_rate)) * rate_adj
                    raw = self._read_ring_interpolated(read_pos, out_frames, step)
                    read_pos += out_frames * step

                    if fade_in < 1.0:
                        next_fade = min(1.0, fade_in + 0.25)
                        ramp = np.linspace(fade_in, next_fade, out_frames, dtype=np.float32)[:, None]
                        raw *= ramp
                        fade_in = next_fade

                    master_vol = float(params.get("master_vol", 25.0))
                    if params.get("protection_enabled", True):
                        master_vol = min(master_vol, float(params.get("protection_max_master", 50.0)))
                    solo = params.get("solo")
                    active_mute = bool(params.get("master_mute", False)) or bool(params.get(f"{role}_mute", False))
                    if solo is not None and solo != role:
                        active_mute = True

                    processed = dsp.process(raw, params, master_vol, active_mute)
                    stream.write(processed.tobytes(), exception_on_underflow=False)

                    with self._ring_lock:
                        w_now = self.write_idx
                    self.meters[meter_key] = block_db(processed)
                    self.telemetry[actual_key] = max(0.0, (w_now - read_pos) * 1000.0 / MASTER_RATE - BASE_BUFFER_MS)
                    self.telemetry[ppm_key] = (rate_adj - 1.0) * 1_000_000.0
                    self.telemetry[f"{role}_gr_db"] = dsp.guard.last_gr_db

            except Exception as exc:
                self.telemetry[status_key] = f"Reconnecting ({exc})"
                self.meters[meter_key] = -60.0
                if not self._stop_event.is_set():
                    time.sleep(1.0)
                    # Re-resolve device index in case Bluetooth speaker reconnected under a new WASAPI slot
                    target_name = self.selected_names.get(f"{role}_name", "")
                    if target_name:
                        try:
                            _, outs, _ = list_wasapi_devices()
                            for d in outs:
                                if d.name == target_name:
                                    device_idx = d.id
                                    break
                        except Exception:
                            pass
            finally:
                if stream is not None:
                    try:
                        stream.stop_stream()
                        stream.close()
                    except Exception:
                        pass
