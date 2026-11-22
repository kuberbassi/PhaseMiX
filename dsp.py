"""Real-time audio DSP helpers: biquad EQ/crossover filters, resampling, and test signals."""

from __future__ import annotations

import math
import numpy as np
from scipy import signal


def design_low_shelf_sos(freq_hz: float, gain_db: float, fs: float, q: float = 0.707) -> np.ndarray:
    """Design a 2nd-order Audio-EQ-Cookbook low-shelf filter in SOS format."""
    if abs(gain_db) < 0.05:
        return np.array([[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]], dtype=np.float64)
    a_amp = 10.0 ** (gain_db / 40.0)
    w0 = 2.0 * math.pi * max(20.0, min(freq_hz, fs * 0.4)) / fs
    cos_w0 = math.cos(w0)
    sin_w0 = math.sin(w0)
    alpha = sin_w0 / (2.0 * q)
    two_sqrt_a_alpha = 2.0 * math.sqrt(a_amp) * alpha

    b0 = a_amp * ((a_amp + 1.0) - (a_amp - 1.0) * cos_w0 + two_sqrt_a_alpha)
    b1 = 2.0 * a_amp * ((a_amp - 1.0) - (a_amp + 1.0) * cos_w0)
    b2 = a_amp * ((a_amp + 1.0) - (a_amp - 1.0) * cos_w0 - two_sqrt_a_alpha)
    a0 = (a_amp + 1.0) + (a_amp - 1.0) * cos_w0 + two_sqrt_a_alpha
    a1 = -2.0 * ((a_amp - 1.0) + (a_amp + 1.0) * cos_w0)
    a2 = (a_amp + 1.0) + (a_amp - 1.0) * cos_w0 - two_sqrt_a_alpha
    return np.array([[b0 / a0, b1 / a0, b2 / a0, 1.0, a1 / a0, a2 / a0]], dtype=np.float64)


def design_peaking_sos(freq_hz: float, gain_db: float, fs: float, q: float = 0.75) -> np.ndarray:
    """Design a 2nd-order Audio-EQ-Cookbook peaking bell filter in SOS format."""
    if abs(gain_db) < 0.05:
        return np.array([[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]], dtype=np.float64)
    a_amp = 10.0 ** (gain_db / 40.0)
    w0 = 2.0 * math.pi * max(20.0, min(freq_hz, fs * 0.45)) / fs
    cos_w0 = math.cos(w0)
    alpha = math.sin(w0) / (2.0 * q)

    b0 = 1.0 + alpha * a_amp
    b1 = -2.0 * cos_w0
    b2 = 1.0 - alpha * a_amp
    a0 = 1.0 + alpha / a_amp
    a1 = -2.0 * cos_w0
    a2 = 1.0 - alpha / a_amp
    return np.array([[b0 / a0, b1 / a0, b2 / a0, 1.0, a1 / a0, a2 / a0]], dtype=np.float64)


def design_high_shelf_sos(freq_hz: float, gain_db: float, fs: float, q: float = 0.707) -> np.ndarray:
    """Design a 2nd-order Audio-EQ-Cookbook high-shelf filter in SOS format."""
    if abs(gain_db) < 0.05:
        return np.array([[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]], dtype=np.float64)
    a_amp = 10.0 ** (gain_db / 40.0)
    w0 = 2.0 * math.pi * max(200.0, min(freq_hz, fs * 0.45)) / fs
    cos_w0 = math.cos(w0)
    sin_w0 = math.sin(w0)
    alpha = sin_w0 / (2.0 * q)
    two_sqrt_a_alpha = 2.0 * math.sqrt(a_amp) * alpha

    b0 = a_amp * ((a_amp + 1.0) + (a_amp - 1.0) * cos_w0 + two_sqrt_a_alpha)
    b1 = -2.0 * a_amp * ((a_amp - 1.0) + (a_amp + 1.0) * cos_w0)
    b2 = a_amp * ((a_amp + 1.0) + (a_amp - 1.0) * cos_w0 - two_sqrt_a_alpha)
    a0 = (a_amp + 1.0) - (a_amp - 1.0) * cos_w0 + two_sqrt_a_alpha
    a1 = 2.0 * ((a_amp - 1.0) - (a_amp + 1.0) * cos_w0)
    a2 = (a_amp + 1.0) - (a_amp - 1.0) * cos_w0 - two_sqrt_a_alpha
    return np.array([[b0 / a0, b1 / a0, b2 / a0, 1.0, a1 / a0, a2 / a0]], dtype=np.float64)


class StatefulSosFilter:
    """Stereo stateful IIR SOS filter that preserves zi across streaming blocks."""

    def __init__(self, channels: int = 2) -> None:
        self.channels = channels
        self.sos: np.ndarray | None = None
        self.zi: np.ndarray | None = None
        self._key: tuple | None = None

    def configure(self, key: tuple, builder) -> None:
        """Rebuild filter coefficients only when parameters change."""
        if key == self._key:
            return
        self._key = key
        sos = builder()
        if sos is None:
            self.sos = None
            self.zi = None
            return
        self.sos = np.asarray(sos, dtype=np.float64)
        base_zi = signal.sosfilt_zi(self.sos)
        # sosfilt_zi shape: (n_sections, 2); broadcast to (n_sections, 2, channels)
        self.zi = np.repeat(base_zi[:, :, None], self.channels, axis=2) * 0.0

    def process(self, block: np.ndarray) -> np.ndarray:
        """Apply filter along axis=0 for shape (N, channels)."""
        if self.sos is None or self.zi is None:
            return block
        out, self.zi = signal.sosfilt(self.sos, block, axis=0, zi=self.zi)
        return out.astype(np.float32, copy=False)


class ThreeBandEq:
    """Stateful 3-band EQ (110 Hz low-shelf, 1 kHz mid bell, 4.5 kHz high-shelf) per channel."""

    def __init__(self, fs: float, role: str) -> None:
        self.fs = fs
        self.role = role
        self.bass = StatefulSosFilter(channels=2)
        self.mid = StatefulSosFilter(channels=2)
        self.treble = StatefulSosFilter(channels=2)

    def process(self, x: np.ndarray, params: dict) -> np.ndarray:
        r = self.role
        bass_db = max(-12.0, min(12.0, float(params.get(f"{r}_bass_db", 0.0))))
        mid_db = max(-12.0, min(12.0, float(params.get(f"{r}_mid_db", 0.0))))
        treble_db = max(-12.0, min(12.0, float(params.get(f"{r}_treble_db", 0.0))))

        self.bass.configure(
            ("bass", round(bass_db, 2), self.fs),
            lambda: None if abs(bass_db) < 0.05 else design_low_shelf_sos(110.0, bass_db, self.fs),
        )
        self.mid.configure(
            ("mid", round(mid_db, 2), self.fs),
            lambda: None if abs(mid_db) < 0.05 else design_peaking_sos(1000.0, mid_db, self.fs, q=0.75),
        )
        self.treble.configure(
            ("treble", round(treble_db, 2), self.fs),
            lambda: None if abs(treble_db) < 0.05 else design_high_shelf_sos(4500.0, treble_db, self.fs),
        )
        x = self.bass.process(x)
        x = self.mid.process(x)
        return self.treble.process(x)


class PeakGuard:
    """Smooth ear-safe output peak limiter preventing sudden loudness spikes."""

    def __init__(self, fs: float) -> None:
        self.fs = fs
        self.env = 1.0
        self.last_gr_db = 0.0

    def process(self, x: np.ndarray, enabled: bool, ceiling_db: float) -> np.ndarray:
        if not enabled or x.size == 0:
            self.env = min(1.0, self.env + 0.25)
            self.last_gr_db = 0.0
            return soft_limit(np.nan_to_num(x, copy=False, nan=0.0, posinf=1.0, neginf=-1.0))

        x = np.nan_to_num(x, copy=False, nan=0.0, posinf=1.0, neginf=-1.0)
        ceiling_lin = max(0.05, min(0.98, 10.0 ** (max(-30.0, min(-0.5, ceiling_db)) / 20.0)))
        peak = float(np.max(np.abs(x)))
        target_gain = min(1.0, ceiling_lin / peak) if peak > ceiling_lin else 1.0
        # Fast attack when catching spikes, gentle release back toward unity
        alpha = 0.9 if target_gain < self.env else 0.14
        next_env = (1.0 - alpha) * self.env + alpha * target_gain
        curve = np.linspace(self.env, next_env, x.shape[0], dtype=np.float32)[:, None]
        self.env = next_env
        self.last_gr_db = round(20.0 * math.log10(max(1e-4, next_env)), 1) if next_env < 0.995 else 0.0

        y = (x * curve).astype(np.float32, copy=False)
        if float(np.max(np.abs(y))) > ceiling_lin:
            # C1-continuous soft-knee saturation prevents first-block transients exceeding ceiling_lin
            knee = ceiling_lin * 0.85
            headroom = max(1e-4, ceiling_lin - knee)
            abs_y = np.abs(y)
            over = abs_y > knee
            if np.any(over):
                mag = knee + headroom * np.tanh((abs_y[over] - knee) / headroom)
                y[over] = np.sign(y[over]) * mag
        return soft_limit(y)


class IntexProcessor:
    """Stereo width, 3-band EQ, L/R balance, smoothed gain, and ear-safe guard for Intex 2.2."""

    def __init__(self, fs: float) -> None:
        self.fs = fs
        self.eq = ThreeBandEq(fs, role="intex")
        self.guard = PeakGuard(fs)
        self.prev_gain = 0.0

    def process(self, block: np.ndarray, params: dict, master_vol: float, active_mute: bool) -> np.ndarray:
        n = block.shape[0]
        if n == 0:
            return block

        width = float(params.get("intex_width", 100.0)) / 100.0
        bal = max(-100.0, min(100.0, float(params.get("intex_balance", 0.0))))
        target_gain = 0.0 if active_mute else (master_vol / 100.0) * (float(params.get("intex_vol", 100.0)) / 100.0)

        if target_gain == 0.0 and self.prev_gain == 0.0:
            self.guard.last_gr_db = 0.0
            return np.zeros((n, 2), dtype=np.float32)

        x = block.astype(np.float32, copy=True)
        if abs(width - 1.0) > 1e-3:
            mid = 0.5 * (x[:, 0] + x[:, 1])
            side = 0.5 * (x[:, 0] - x[:, 1]) * width
            x[:, 0] = mid + side
            x[:, 1] = mid - side

        x = self.eq.process(x, params)

        if bal < -0.5:
            x[:, 1] *= 1.0 + bal / 100.0
        elif bal > 0.5:
            x[:, 0] *= 1.0 - bal / 100.0

        gain_curve = np.linspace(self.prev_gain, target_gain, n, dtype=np.float32)[:, None]
        self.prev_gain = target_gain
        return self.guard.process(
            x * gain_curve,
            bool(params.get("protection_enabled", True)),
            float(params.get("protection_ceiling_db", -9.0)),
        )


class AuxChannelProcessor:
    """Parameterized 3-band EQ, crossover, center/ambience mode, and ear-safe guard."""

    def __init__(self, fs: float, role: str = "boat") -> None:
        self.fs = fs
        self.role = role
        self.hpf = StatefulSosFilter(channels=2)
        self.lpf = StatefulSosFilter(channels=2)
        self.vocal = StatefulSosFilter(channels=2)
        self.eq = ThreeBandEq(fs, role=role)
        self.guard = PeakGuard(fs)
        self.prev_gain = 0.0

    def process(self, block: np.ndarray, params: dict, master_vol: float, active_mute: bool) -> np.ndarray:
        n = block.shape[0]
        if n == 0:
            return block

        r = self.role
        default_mode = "mono_center" if r == "boat" else "stereo"
        default_hpf = 160.0 if r == "boat" else 250.0
        default_vol = 85.0 if r == "boat" else 75.0

        mode = params.get(f"{r}_mode", default_mode)
        hpf_hz = float(params.get(f"{r}_hpf_hz", default_hpf))
        lpf_hz = float(params.get(f"{r}_lpf_hz", 18000.0))
        invert = -1.0 if params.get(f"{r}_invert", False) else 1.0
        target_gain = 0.0 if active_mute else (master_vol / 100.0) * (float(params.get(f"{r}_vol", default_vol)) / 100.0)

        if target_gain == 0.0 and self.prev_gain == 0.0:
            self.guard.last_gr_db = 0.0
            return np.zeros((n, 2), dtype=np.float32)

        x = block.astype(np.float32, copy=True)
        if mode in ("mono_center", "vocal_mid"):
            center = 0.5 * (x[:, 0] + x[:, 1])
            x[:, 0] = center
            x[:, 1] = center
        elif mode == "side_ambience":
            side = 0.5 * (x[:, 0] - x[:, 1])
            x[:, 0] = side
            x[:, 1] = -side

        vocal_db = 3.5 if mode == "vocal_mid" else 0.0
        self.vocal.configure(
            ("vocal", vocal_db, self.fs),
            lambda: None if vocal_db == 0.0 else design_peaking_sos(1800.0, vocal_db, self.fs, q=0.75),
        )
        x = self.vocal.process(x)

        nyq = 0.5 * self.fs
        self.hpf.configure(
            ("hpf", round(hpf_hz, 1), self.fs),
            lambda: None if hpf_hz <= 20.0 else signal.butter(2, min(hpf_hz, nyq * 0.45), btype="highpass", fs=self.fs, output="sos"),
        )
        x = self.hpf.process(x)

        self.lpf.configure(
            ("lpf", round(lpf_hz, 1), self.fs),
            lambda: None if lpf_hz >= 17500.0 else signal.butter(2, min(lpf_hz, nyq * 0.9), btype="lowpass", fs=self.fs, output="sos"),
        )
        x = self.lpf.process(x)

        x = self.eq.process(x, params)

        gain_curve = (np.linspace(self.prev_gain, target_gain, n, dtype=np.float32) * invert)[:, None]
        self.prev_gain = target_gain
        return self.guard.process(
            x * gain_curve,
            bool(params.get("protection_enabled", True)),
            float(params.get("protection_ceiling_db", -9.0)),
        )


class BoatCenterProcessor(AuxChannelProcessor):
    """Backwards-compatible alias defaulting to the 'boat' middle-driver role."""

    def __init__(self, fs: float) -> None:
        super().__init__(fs, role="boat")


def soft_limit(x: np.ndarray) -> np.ndarray:
    """Gentle soft-clipping ceiling so >100% mixer gain never hard-clips harshly."""
    peak = float(np.max(np.abs(x))) if x.size else 0.0
    if peak <= 0.96:
        return x
    return np.tanh(x * 0.95).astype(np.float32, copy=False)


def block_db(x: np.ndarray) -> float:
    """Compute transient-responsive RMS+Peak level in dBFS floored at -60 dB."""
    if x.size == 0:
        return -60.0
    rms = float(np.sqrt(np.mean(np.square(x, dtype=np.float32))))
    peak = float(np.max(np.abs(x)))
    level = 0.65 * rms + 0.35 * peak
    if level <= 1e-4:
        return -60.0
    return max(-60.0, min(0.0, 20.0 * math.log10(level)))


class TestSignalGenerator:
    """Generates calibration clicks, bass+vocal sweeps, and L/C/R checks at 48 kHz."""

    def __init__(self, fs: int = 48000) -> None:
        self.fs = fs
        self.sample_idx = 0

    def generate(self, mode: str, n_frames: int) -> np.ndarray:
        t = (self.sample_idx + np.arange(n_frames, dtype=np.float64)) / self.fs
        self.sample_idx += n_frames
        out = np.zeros((n_frames, 2), dtype=np.float32)

        if mode == "click":
            # Sharp 8 ms percussive click every 0.75 s for pinpoint ear sync calibration
            period = 0.75
            phase = np.mod(t, period)
            mask = phase < 0.012
            if np.any(mask):
                pt = phase[mask]
                env = np.exp(-pt * 420.0)
                sig = env * (0.55 * np.sin(2 * math.pi * 1600.0 * pt) + 0.35 * np.sin(2 * math.pi * 450.0 * pt))
                out[mask, 0] = sig
                out[mask, 1] = sig
        elif mode == "chirp":
            # Alternates 1.5s warm 65 Hz sub bass + 1.5s crisp 900/1800 Hz mid vocal chord
            cycle = np.mod(t, 3.0)
            low_mask = cycle < 1.5
            mid_mask = ~low_mask
            if np.any(low_mask):
                lt = cycle[low_mask]
                env = np.sin(math.pi * np.clip(lt / 1.5, 0.0, 1.0))
                sig = 0.35 * env * np.sin(2 * math.pi * 65.0 * lt)
                out[low_mask, 0] = sig
                out[low_mask, 1] = sig
            if np.any(mid_mask):
                mt = cycle[mid_mask] - 1.5
                env = np.sin(math.pi * np.clip(mt / 1.5, 0.0, 1.0))
                sig = 0.22 * env * (
                    np.sin(2 * math.pi * 700.0 * mt)
                    + 0.7 * np.sin(2 * math.pi * 1400.0 * mt)
                    + 0.4 * np.sin(2 * math.pi * 2100.0 * mt)
                )
                out[mid_mask, 0] = sig
                out[mid_mask, 1] = sig
        elif mode == "lr":
            # 4-beat 2.4s loop: Left (0-0.6), Center (0.6-1.2), Right (1.2-1.8), Pause (1.8-2.4)
            cycle = np.mod(t, 2.4)
            for idx, (start, l_amp, r_amp, freq) in enumerate([
                (0.0, 0.32, 0.0, 523.25),
                (0.6, 0.25, 0.25, 783.99),
                (1.2, 0.0, 0.32, 659.25),
            ]):
                m = (cycle >= start) & (cycle < start + 0.42)
                if np.any(m):
                    lt = cycle[m] - start
                    env = np.sin(math.pi * np.clip(lt / 0.42, 0.0, 1.0))
                    tone = env * np.sin(2 * math.pi * freq * lt)
                    out[m, 0] = tone * l_amp
                    out[m, 1] = tone * r_amp
        return out
