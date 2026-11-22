<div align="center">

# PhaseMiX

**Sync wired, Bluetooth, and built-in speakers into one phase-aligned Windows soundstage.**

![OS](https://img.shields.io/badge/Windows-10%20%2F%2011-0f1014?style=flat-square&logo=windows&logoColor=white)
![Engine](https://img.shields.io/badge/Audio-WASAPI%20Loopback%2048kHz-181a20?style=flat-square)
![UI](https://img.shields.io/badge/UI-React%2019%20%2B%20Three.js-181a20?style=flat-square&logo=react&logoColor=61DAFB)
![Status](https://img.shields.io/badge/Roadmap-Rust%20Core%20Next-22c55e?style=flat-square)

</div>

---

## Signal Flow

```mermaid
flowchart LR
    WIN["Windows Audio\n(48 kHz Loopback)"] --> RING["Lockless Ring Buffer\nPer-Channel Delay (0-450 ms)"]

    RING --> C1["Channel 1 · Primary 2.1 / AUX\nWidth · Balance · 3-Band EQ · PeakGuard"]
    RING --> C2["Channel 2 · Center / Wireless\nHPF/LPF · Mono/Vocal/Side · 3-Band EQ"]
    RING --> C3["Channel 3 · Built-In / Near-Field\nHPF/LPF · Phase 180° · 3-Band EQ"]

    C1 --> OUT1["Wired Speakers"]
    C2 --> OUT2["Bluetooth Speaker"]
    C3 --> OUT3["Internal / Tweeters"]
```

---

## Core Modules

| Feature | What It Does | Specs |
| :--- | :--- | :--- |
| **Phase & Delay Sync** | Delays fast wired/internal speakers to match Bluetooth lag | `0 – 450 ms`, `±2 ms` fine step, drift resampling |
| **3-Band EQ + Crossover** | Sculpt lows, vocals & highs independently per output | `110 Hz` shelf · `1 kHz` bell · `4.5 kHz` shelf (`±12 dB`), `HPF`/`LPF` |
| **Ear-Safe Protection** | Prevents sudden system volume blasts or clipping | Master cap (`15–200%`), C¹ soft-knee limiter (`-30` to `-0.5 dB`) |
| **Position Presets** | Instant stage layouts synced with live 3D room visualizer | `Desk Triangle`, `Cinema 3.2`, `Wide Room`, `Night Safe`, `Offset`, `Flat` |
| **Windows Integration** | Keyboard Volume/Mute bridge + per-endpoint level sync | `pycaw` `IAudioEndpointVolume` + virtual loopback unity lock |

---

## Position Presets

| Preset | Stage Strategy | Safety / Focus |
| :--- | :--- | :--- |
| **Desk Triangle** | Flanking L/R + `260 Hz+` vocal center + bright front air | Everyday balanced listening |
| **Cinema 3.2 Dialogue** | Widened L/R (`135%`) + `+3.5 dB` speech lock + side ambience | Movies & dialogue clarity |
| **Wide Room Music** | Boosted subs + full-mid center + sparkling tweeters | Full-room music punch |
| **Night Ear-Safe** | Tamed lows + soft vocal center | Strict `60%` cap & `-15 dB` limiter |
| **Offset Desk** | Compensates asymmetric L/R speaker placement (`+18` balance) | Off-center desks |
| **Pure Flat** | `0 dB` across all 9 bands | Direct stereo reference |

---

## Tech Stack

| Layer | Current Stack | Next (`Phase 2`) |
| :--- | :--- | :--- |
| **Frontend & 3D** | React 19 · TypeScript · Tailwind v4 · Three.js · Phosphor Icons · Aurora WebGL | Same React/Three.js UI |
| **Audio Routing** | Python 3.14 · `PyAudioWPatch` WASAPI · `pycaw` | Rust + Tauri v2 (`wasapi` / zero-alloc ring buffers) |
| **DSP Pipeline** | `NumPy` + `SciPy` SOS filters · C¹ soft-knee `PeakGuard` | Pure Rust inline biquads |

---

## Windows Installer & Desktop App

Run `PhaseMiX-Setup.exe` for a standalone Edge WebView2 window with system tray playback, single-instance focus, and settings stored cleanly in `%APPDATA%\PhaseMiX\config.json`.

To build the `.exe` installer from source (requires Inno Setup 6):

```powershell
powershell -ExecutionPolicy Bypass -File installer\build_installer.ps1
```

---

## Quick Start (Dev Mode)

```powershell
# 1. Build frontend UI
cd frontend && npm install && npm run build && cd ..

# 2. Install Python dependencies & launch mixer
pip install -r requirements.txt
python app.py
```

Open `http://127.0.0.1:5055` (or use `start_mixer.bat`).
