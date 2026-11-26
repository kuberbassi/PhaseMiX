<div align="center">

<img src="src-tauri/icons/128x128@2x.png" alt="PhaseMiX Logo" width="84" />

# PhaseMiX

**Sync wired, Bluetooth, and built-in speakers into one phase-aligned Windows soundstage.**

![OS](https://img.shields.io/badge/Windows-10%20%2F%2011-0f1014?style=flat-square&logo=windows&logoColor=white)
![Engine](https://img.shields.io/badge/Audio-WASAPI%20Loopback%2048kHz-181a20?style=flat-square)
![UI](https://img.shields.io/badge/UI-React%2019%20%2B%20Three.js-181a20?style=flat-square&logo=react&logoColor=61DAFB)
![Core](https://img.shields.io/badge/Core-Rust%20%2B%20Tauri%20v2-22c55e?style=flat-square&logo=rust&logoColor=white)
![License](https://img.shields.io/badge/License-MIT-181a20?style=flat-square)

</div>

---

## Signal Flow

```mermaid
flowchart LR
    WIN["Windows Audio\n(48 kHz Loopback)"] --> RING["Lockless Ring Buffer\nPer-Channel Delay (0-450 ms)"]

    RING --> C1["Channel 1 · Primary 2.2 (Wired AUX or Bluetooth)\nWidth · Balance · BT Gain Curve · 3-Band EQ · PeakGuard"]
    RING --> C2["Channel 2 · Built-In Speakers (Near-Field 2.0)\nHPF/LPF · Phase 180° · 3-Band EQ · PeakGuard"]
    RING --> C3["Channel 3 · Optional Auxiliary Speaker\nHPF/LPF · Mono/Vocal/Side · 3-Band EQ"]

    C1 --> OUT1["Primary 2.2 (AUX / BT)"]
    C2 --> OUT2["Built-In / Near-Field"]
    C3 --> OUT3["Optional 3rd Speaker"]
```

---

## Core Modules

| Feature | What It Does | Specs |
| :--- | :--- | :--- |
| **4.2 Mode Switch & Delay Sync** | One-click `Wired AUX` (`0 ms`) ↔ `Bluetooth` (`224 ms`) switch + 2-burst mic chirp align | `0 – 450 ms`, `±2 ms` step, Catmull-Rom drift sync, preserved across presets |
| **Full-Range 3-Band EQ + Curve** | Seamless `20 Hz – 20 kHz` 4.2 crossover with real-time response curve per output | `110 Hz` · `1 kHz` · `4.5 kHz` (`±12 dB`), `HPF`/`LPF` |
| **Ear-Safe Protection** | Prevents sudden system volume blasts or clipping | Master cap (`15–200%`), hold-release PeakGuard (`-3 dB` day / `-15 dB` night) |
| **Responsive UI & 3D Stage** | Container-responsive console (`720×560` min), 4.2/5.2 3D physics, click-to-solo, dock mode | `Desk 4.2`, `Cinema 4.2`, `Wide Room 4.2`, `Night Safe`, `Offset 4.2`, `Pure Flat 4.2` |
| **Windows & Bluetooth Gain Sync** | Dedicated BT master curve, session unity, saved HW levels, 1 ms timer lock, hot-plug BT | `wireless_master_gain`, `1–2%` media keys, `70 ms` BT / `35 ms` wired slack |

---

## Audio Routing Architecture

- **System Loopback Capture (`48 kHz` Stereo `f32`):** PhaseMiX captures a Windows render loopback endpoint (preferring **VB-Audio CABLE Input** first, then **Steam Streaming Speakers** or **VoiceMeeter**, falling back to standard render loopback or physical inputs). Capture uses WASAPI `EventsShared` (`autoconvert: true`) with an **`80 ms`** Windows engine loopback buffer (`PollingShared` fallback in native mix format), while locking OS timer resolution to `1 ms` via `timeBeginPeriod(1)`.
- **Central `SharedRing` Buffer & Independent Workers:** Captured blocks write into a `4`-second (`192,000`-frame) ring pre-seeded by `600 ms` with a **`125 ms`** base playback cushion plus per-speaker delay (`0–450 ms`). Each active output slot spawns a dedicated `Pro Audio` thread opening WASAPI render streams (`70 ms` shared slack for detected wireless/BT endpoints, `35 ms` wired) with non-blocking `try_write()` telemetry, duplicate-endpoint prevention, and monotonic underrun handling (never rewinds backwards over already-played audio).
- **Independent Clock-Drift Synchronization:** Separate audio hardware devices (Realtek DAC, USB/HDMI, Bluetooth A2DP) do not share a hardware clock crystal. Each output thread smooths ring-buffer offset with a `3.0 s` time-normalized EMA, ignores `< 1.5 ms` (`72`-frame) packet arrival jitter, and applies 4-point Catmull-Rom fractional resampling capped at **`±400 ppm`** (`±0.04%`) so speakers stay phase-aligned without audible pitch drift.
- **Dual Master Volume Curves, Session Unity & Persistent Device Levels:** Wired/APO outputs use a Windows-matched perceptual curve (`x * 10^(-(1 - x))`), while Bluetooth/wireless outputs automatically use a compensated `wireless_master_gain` curve (`(v / 60)^0.80`, reaching `0 dB` unity at `60%` Master) so raw A2DP streams match the loudness of Realtek/Dolby APO endpoints across the entire slider range. Every render stream enforces `ISimpleAudioVolume` unity (`100%` unmuted, defeating Windows Communications auto-ducking), auto-normalizes unpinned severely-attenuated Bluetooth endpoints (`< -6 dB`) when virtual loopback owns playback, and restores per-device Windows volumes (`win_device_vols`) across boots and reconnects. On virtual loopback inputs, `run_win_key_bridge` polls Windows `IAudioEndpointVolume` every `20 ms`, resets endpoint scalar to `0.98`, and converts media-key presses into `1%` (`< 14%`) / `2%` (`>= 14%`) Master steps.

---

## Device Compatibility & Windows Setup

- **Virtual Audio Cable Source (**[**VB-Audio Virtual Cable**](https://vb-audio.com/Cable/) — **Preferred**):**
  - Download from the official site ([`vb-audio.com/Cable`](https://vb-audio.com/Cable/)), extract the archive, right-click **`VBCABLE_Setup_x64.exe`** -> **Run as administrator** -> click **Install Driver** (reboot if prompted).
  - Set **`CABLE Input (VB-Audio Virtual Cable)`** as the Windows **Default Playback Device**, keep your real **`Microphone Array`** as your Windows **Default Recording Mic** (do not leave it on `CABLE Output`), and select **`VB-Audio Cable` (`LOOP` tag)** in PhaseMiX Input (PhaseMiX prioritizes it automatically).
  - *Why VB-CABLE over Steam Streaming Speakers?* `VB-CABLE` is a standalone Windows driver with no launcher app behind it — `Steam Streaming Speakers` works as a fallback, but launching `steam.exe` can briefly reset the driver or switch Windows default playback back to physical AUX.
- **Supported Output Devices:** Works with any active Windows WASAPI `Render` endpoint — **Realtek / analog `3.5 mm` speakers**, **built-in laptop speakers**, **USB DACs / USB headphones**, **HDMI/DisplayPort audio**, and **stereo Bluetooth (`A2DP`) speakers**.
- **Two Bluetooth Speakers Simultaneously:** Standard laptop Bluetooth adapters typically sustain only **one active `A2DP` stereo stream cleanly** at a time. To run two wireless speakers simultaneously, plug in a **USB Bluetooth Audio Transmitter dongle** (which Windows exposes as an independent `USB` sound device) alongside your laptop's built-in Bluetooth, unlink `AUX+PC`, and align both wireless delays manually using `Sync Click`.
- **Bluetooth Profile Tip:** PhaseMiX hides duplicate `Hands-Free / AG Audio` endpoints whenever stereo `Headphones` devices exist. Keep your default Windows microphone on your laptop/USB mic rather than a Bluetooth headset mic so voice apps do not force Bluetooth out of stereo `A2DP` into `16 kHz` mono Hands-Free mode.

---

## Latency Calibration Method

1. **Dual-Burst Acoustic Chirp Alignment (`Auto-Align Delay via Mic`):**
   - Requires routing running with both your **Bluetooth Primary** and **Built-in Speakers** assigned (or an optional 3rd Bluetooth speaker + Wired Primary) plus a working Windows default microphone.
   - After draining `160 ms` of cold mic audio, PhaseMiX injects **4 alternating `44 ms` Hann-windowed logarithmic chirps (`600 Hz -> 3.4 kHz`)** over `3.05 s` (`BT 1` at `+180 ms`, `Wired 1` at `+980 ms`, `BT 2` at `+1750 ms`, `Wired 2` at `+2550 ms`) with any unused channel muted during the sweep.
   - An analytic quadrature matched filter (`find_chirp_arrival`, SNR `>= 4.0` threshold and earliest-direct-path crest scan within `6 ms`) measures exact arrival offset minus each thread's ring lag.
   - Both burst pairs must agree within **`<= 18 ms`** (rejecting noisy/echoed takes); the averaged offset (`0–450 ms`) locks your wired/built-in delay to match Bluetooth (`0 ms` BT anchor) and is retained across preset changes (`calibrated_delay_ms`).
2. **Manual Ear-Tuning Signals:** Built-in `Sync Click` (`2 ms` transient impulse), `Bass + Vocal`, and `L / C / R` generators let you fine-tune delays by hand in `±2 ms` steps when using asymmetric setups or a second wireless USB transmitter.

---

## Limitations & Windows Configuration Notes

- **Not Low-Latency for Competitive Gaming / Live Monitoring:** With a `125 ms` ring cushion plus typical Bluetooth A2DP latency (`~180–250 ms`), PhaseMiX targets **music, YouTube, movies, and general media** rather than real-time gaming or DAW instrument monitoring (use `Wired AUX` mode for `0 ms` delay offset between speakers).
- **WASAPI Shared Mode & OEM Audio Enhancements:** PhaseMiX uses WASAPI `Shared Mode` (`EventsShared`) for broad compatibility and sample-rate autoconversion. Downstream Windows/OEM APO processing (**Realtek SmartAmp, Dolby/DTS, Loudness Equalization**) varies per machine/driver — PhaseMiX's `wireless_master_gain` curve automatically bridges the loudness gap between APO-boosted built-in speakers and raw Bluetooth A2DP streams, while `ISimpleAudioVolume` unity prevents Windows Communications auto-ducking.
- **2-Speaker Auto-Align Scope:** Mic Auto-Align measures the lag differential between your active **Bluetooth** speaker and **Built-in / Wired** speaker pair. Multi-wireless setups (e.g., laptop BT + USB BT transmitter) require manual `Sync Click` tuning.
- **Non-`48 kHz` Fallback Resampling:** Most drivers accept `48 kHz` `EventsShared` directly via Windows SRC; if a strict capture driver rejects it and falls back to `PollingShared` at a different rate (e.g., `44.1 kHz`), capture conversion uses stateless per-packet linear interpolation.
- **Bluetooth RF Stability:** While PhaseMiX cushions `70 ms` in WASAPI and auto-restores endpoints/volumes on reconnect, wireless dropouts from `2.4 GHz` Wi-Fi congestion or range happen below the Windows audio stack.

---

## Position Presets

| Preset | Stage Strategy | Safety / Focus |
| :--- | :--- | :--- |
| **Desk 4.2 Full Range** | Primary 2.2 full subs & stereo + Built-in `210 Hz–20 kHz` near-field vocals & air | Everyday balanced 4.2 listening |
| **Cinema 4.2 Dialogue** | Widened Primary L/R (`132%`) + deep rumble subs + screen-locked dialogue | Movies & dialogue clarity |
| **Wide Room 4.2 Music** | Punchy 2.2 dual subs (`+3.5 dB`) + sparkling near-field air | Full-room music punch |
| **Night Ear-Safe** | Tamed dual subs (`-4.5 dB`) + clear near-field voices | Strict `60%` cap & `-15 dB` limiter |
| **Offset Desk 4.2** | Compensates asymmetric primary L/R placement (`+16` balance) | Off-center desks |
| **Pure Flat 4.2** | `0 dB` flat EQ across Primary 2.2 & Built-in 2.0 (`180 Hz` sub-guard) | Direct stereo reference |

---

## Tech Stack

| Layer | Architecture | Details |
| :--- | :--- | :--- |
| **Desktop Shell & IPC** | Rust + Tauri v2 | Zero-HTTP IPC commands, `16 ms` (`60 Hz`) live meter push, system tray, single-instance lock |
| **Audio Routing** | Native WASAPI (`wasapi` + `windows` crates) | Lockless `SharedRing`, `1.5 ms` phase lock, `±400 ppm` drift sync, BT auto-reconnect |
| **DSP Pipeline** | Pure Rust (`src-tauri/src/dsp.rs`) | Direct Form II transposed biquads, Windows perceptual curve, hold-release `PeakGuard` |
| **Frontend & 3D** | React 19 · TypeScript · Tailwind v4 · Three.js | Phosphor Icons, WebGL Aurora, 60 FPS 3D stage orbit view |

---

## Windows Installer & Desktop App

Run `PhaseMiX-Setup.exe` (NSIS, current-user install) for a standalone Edge WebView2 window with native tray controls, single-instance focus restore, and config stored in `%APPDATA%\PhaseMiX\config.json`.

To build the `.exe` installer from source (from the repository root):

```powershell
npm --prefix frontend install
.\frontend\node_modules\.bin\tauri build --bundles nsis
```

---

## Quick Start (Dev Mode)

Requires **Node.js 18+**, **Rust (Cargo)**, and **Windows Tauri v2 prerequisites** (Visual Studio C++ Build Tools + WebView2).

```powershell
git clone https://github.com/kuberbassi/PhaseMiX.git
cd PhaseMiX
npm --prefix frontend install
.\frontend\node_modules\.bin\tauri dev
```

---

## License

Released under the [MIT License](LICENSE) — free to use, modify, and customize for your own setups.
