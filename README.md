<div align="center">

<img src="src-tauri/icons/128x128@2x.png" alt="PhaseMiX Logo" width="84" />

# PhaseMiX

**Sync wired, Bluetooth, and built-in speakers into one phase-aligned Windows soundstage.**

![OS](https://img.shields.io/badge/Windows-10%20%2F%2011-0f1014?style=flat-square&logo=windows&logoColor=white)
![Engine](https://img.shields.io/badge/Audio-WASAPI%20Loopback%2048kHz-181a20?style=flat-square)
![UI](https://img.shields.io/badge/UI-React%2019%20%2B%20Three.js-181a20?style=flat-square&logo=react&logoColor=61DAFB)
![Core](https://img.shields.io/badge/Core-Rust%20%2B%20Tauri%20v2-22c55e?style=flat-square&logo=rust&logoColor=white)

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
| **Phase & Delay Sync** | Delays wired/internal speakers + 2-burst mic chirp align & link | `0 – 450 ms`, `±2 ms` step, Catmull-Rom drift sync, preserved across presets |
| **3-Band EQ + Live Curve** | Real-time 20 Hz–20 kHz curve, bass/mids/treble & crossovers per output | `110 Hz` · `1 kHz` · `4.5 kHz` (`±12 dB`), `HPF`/`LPF` |
| **Ear-Safe Protection** | Prevents sudden system volume blasts or clipping | Master cap (`15–200%`), C¹ limiter (`-6 dB` day / `-15 dB` night) |
| **Interactive 3D Stage & Presets** | Width/balance/phase physics, click-to-solo, live laptop meters, compact mode | `Desk Triangle`, `Cinema 3.2`, `Wide Room`, `Night Safe`, Import/Export `.json` |
| **Windows Integration** | Hot-plug BT refresh, 1 ms timer lock, anti-tear DSP, tray, ~6 MB boot | `Ctrl+Alt+M/P/N`, `45 ms` BT / `25 ms` wired slack, `IAudioEndpointVolume` |

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

| Layer | Architecture | Details |
| :--- | :--- | :--- |
| **Desktop Shell & IPC** | Rust + Tauri v2 | Zero-HTTP IPC commands, `33 ms` live meter push, system tray, single-instance lock |
| **Audio Routing** | Native WASAPI (`wasapi` + `windows` crates) | Zero-alloc `SharedRing`, `8 ms` jitter deadband, `±1500 ppm` drift sync, BT auto-reconnect |
| **DSP Pipeline** | Pure Rust (`src-tauri/src/dsp.rs`) | Direct Form II transposed biquads, M/S width, C¹ soft-knee `PeakGuard` |
| **Frontend & 3D** | React 19 · TypeScript · Tailwind v4 · Three.js | Phosphor Icons, WebGL Aurora, 3D stage orbit view |

---

## Windows Installer & Desktop App

Run `PhaseMiX-Setup.exe` (NSIS, current-user install) for a standalone Edge WebView2 window with native tray controls, single-instance focus restore, and config stored in `%APPDATA%\PhaseMiX\config.json`.

To build the `.exe` installer from source:

```powershell
cd frontend
npm install && npm run build
npx tauri build --bundles nsis
```

---

## Quick Start (Dev Mode)

```powershell
cd frontend && npm install
npx tauri dev
```
