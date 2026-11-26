//! Native Windows WASAPI loopback + multi-output synced audio router with lockless-fast ring buffering.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex, RwLock};
use serde::{Deserialize, Serialize};
use wasapi::{
    initialize_mta, AudioClient, Device, DeviceEnumerator, DeviceState, Direction, SampleType,
    StreamMode, WaveFormat,
};
use windows::core::w;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::IAudioSessionManager;
use windows::Win32::System::Com::CLSCTX_ALL;
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, GetCurrentThread,
    SetThreadPriority, THREAD_PRIORITY_HIGHEST,
};

use crate::dsp::{
    find_chirp_arrival, generate_cal_chirp_pair, meter_db, AuxChannelProcessor, AuxMode, AuxParams,
    PrimaryParams, PrimaryProcessor, TestMode, TestSignalGenerator,
};

pub const MASTER_RATE: u32 = 48_000;
const MASTER_RATE_F64: f64 = 48_000.0;
const RING_SECONDS: usize = 4;
const RING_FRAMES: usize = MASTER_RATE as usize * RING_SECONDS;
const BASE_BUFFER_MS: f64 = 125.0;
const PUMP_BLOCK_FRAMES: usize = 480; // 10 ms at 48 kHz
const OUTPUT_ROLES: [&str; 3] = ["intex", "boat", "laptop"];

#[link(name = "winmm")]
unsafe extern "system" {
    fn timeBeginPeriod(u_period: u32) -> u32;
    fn timeEndPeriod(u_period: u32) -> u32;
}

/// Locks Windows OS timer granularity to 1 ms while active so audio threads never jitter in tray/idle.
struct WinTimerGuard;

impl WinTimerGuard {
    fn acquire() -> Self {
        unsafe {
            let _ = timeBeginPeriod(1);
        }
        Self
    }
}

impl Drop for WinTimerGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = timeEndPeriod(1);
        }
    }
}

/// Zero-allocation Copy snapshot of MixerState read by Pro Audio threads without heap alloc or UI contention.
#[derive(Clone, Copy)]
struct FastAudioParams {
    master_vol: f64,
    master_mute: bool,
    protection_enabled: bool,
    protection_max_master: f64,
    protection_ceiling_db: f64,
    intex_vol: f64,
    intex_mute: bool,
    intex_delay_ms: f64,
    intex_balance: f64,
    intex_bass_db: f64,
    intex_mid_db: f64,
    intex_treble_db: f64,
    intex_width: f64,
    boat_vol: f64,
    boat_mute: bool,
    boat_delay_ms: f64,
    boat_mode: AuxMode,
    boat_hpf_hz: f64,
    boat_lpf_hz: f64,
    boat_bass_db: f64,
    boat_mid_db: f64,
    boat_treble_db: f64,
    boat_invert: bool,
    laptop_vol: f64,
    laptop_mute: bool,
    laptop_delay_ms: f64,
    laptop_mode: AuxMode,
    laptop_hpf_hz: f64,
    laptop_lpf_hz: f64,
    laptop_bass_db: f64,
    laptop_mid_db: f64,
    laptop_treble_db: f64,
    laptop_invert: bool,
    solo_role: u8, // 0 = none, 1 = intex, 2 = boat, 3 = laptop
    test_mode: TestMode,
}

impl FastAudioParams {
    fn from_mixer(m: &MixerState) -> Self {
        let solo_role = match m.solo.as_deref() {
            Some("intex") => 1,
            Some("boat") => 2,
            Some("laptop") => 3,
            _ => 0,
        };
        Self {
            master_vol: m.master_vol,
            master_mute: m.master_mute,
            protection_enabled: m.protection_enabled,
            protection_max_master: m.protection_max_master,
            protection_ceiling_db: m.protection_ceiling_db,
            intex_vol: m.intex_vol,
            intex_mute: m.intex_mute,
            intex_delay_ms: m.intex_delay_ms,
            intex_balance: m.intex_balance,
            intex_bass_db: m.intex_bass_db,
            intex_mid_db: m.intex_mid_db,
            intex_treble_db: m.intex_treble_db,
            intex_width: m.intex_width,
            boat_vol: m.boat_vol,
            boat_mute: m.boat_mute,
            boat_delay_ms: m.boat_delay_ms,
            boat_mode: AuxMode::from_str(&m.boat_mode),
            boat_hpf_hz: m.boat_hpf_hz,
            boat_lpf_hz: m.boat_lpf_hz,
            boat_bass_db: m.boat_bass_db,
            boat_mid_db: m.boat_mid_db,
            boat_treble_db: m.boat_treble_db,
            boat_invert: m.boat_invert,
            laptop_vol: m.laptop_vol,
            laptop_mute: m.laptop_mute,
            laptop_delay_ms: m.laptop_delay_ms,
            laptop_mode: AuxMode::from_str(&m.laptop_mode),
            laptop_hpf_hz: m.laptop_hpf_hz,
            laptop_lpf_hz: m.laptop_lpf_hz,
            laptop_bass_db: m.laptop_bass_db,
            laptop_mid_db: m.laptop_mid_db,
            laptop_treble_db: m.laptop_treble_db,
            laptop_invert: m.laptop_invert,
            solo_role,
            test_mode: TestMode::from_str(&m.test_mode),
        }
    }
}

#[inline]
fn ensure_com() {
    let _ = initialize_mta();
}

struct AudioThreadGuard(Option<HANDLE>);

impl Drop for AudioThreadGuard {
    fn drop(&mut self) {
        if let Some(h) = self.0.take() {
            unsafe {
                let _ = AvRevertMmThreadCharacteristics(h);
            }
        }
    }
}

fn elevate_audio_thread() -> AudioThreadGuard {
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
        let mut task_idx: u32 = 0;
        let mmcss = AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task_idx).ok();
        AudioThreadGuard(mmcss)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MixerState {
    pub master_vol: f64,
    pub master_mute: bool,
    pub protection_enabled: bool,
    pub protection_max_master: f64,
    pub protection_ceiling_db: f64,
    pub position_preset: String,

    pub intex_vol: f64,
    pub intex_mute: bool,
    pub intex_delay_ms: f64,
    pub intex_balance: f64,
    pub intex_bass_db: f64,
    pub intex_mid_db: f64,
    pub intex_treble_db: f64,
    pub intex_width: f64,

    pub boat_vol: f64,
    pub boat_mute: bool,
    pub boat_delay_ms: f64,
    pub boat_mode: String,
    pub boat_hpf_hz: f64,
    pub boat_lpf_hz: f64,
    pub boat_bass_db: f64,
    pub boat_mid_db: f64,
    pub boat_treble_db: f64,
    pub boat_invert: bool,

    pub laptop_vol: f64,
    pub laptop_mute: bool,
    pub laptop_delay_ms: f64,
    pub laptop_mode: String,
    pub laptop_hpf_hz: f64,
    pub laptop_lpf_hz: f64,
    pub laptop_bass_db: f64,
    pub laptop_mid_db: f64,
    pub laptop_treble_db: f64,
    pub laptop_invert: bool,

    pub solo: Option<String>,
    pub test_mode: String,
    pub calibrated_delay_ms: Option<f64>,
}

impl Default for MixerState {
    fn default() -> Self {
        Self {
            master_vol: 25.0,
            master_mute: false,
            protection_enabled: true,
            protection_max_master: 60.0,
            protection_ceiling_db: -3.0,
            position_preset: "desk_triangle".to_string(),
            intex_vol: 105.0,
            intex_mute: false,
            intex_delay_ms: 0.0,
            intex_balance: 0.0,
            intex_bass_db: 1.0,
            intex_mid_db: -0.5,
            intex_treble_db: 1.0,
            intex_width: 108.0,
            boat_vol: 95.0,
            boat_mute: false,
            boat_delay_ms: 0.0,
            boat_mode: "stereo".to_string(),
            boat_hpf_hz: 120.0,
            boat_lpf_hz: 18000.0,
            boat_bass_db: 0.0,
            boat_mid_db: 0.5,
            boat_treble_db: 0.5,
            boat_invert: false,
            laptop_vol: 115.0,
            laptop_mute: false,
            laptop_delay_ms: 0.0,
            laptop_mode: "stereo".to_string(),
            laptop_hpf_hz: 210.0,
            laptop_lpf_hz: 18000.0,
            laptop_bass_db: -1.0,
            laptop_mid_db: 1.5,
            laptop_treble_db: 2.0,
            laptop_invert: false,
            solo: None,
            test_mode: "off".to_string(),
            calibrated_delay_ms: None,
        }
    }
}

pub fn preset_patch(id: &str) -> Option<MixerPatch> {
    let patch = match id {
        "desk_triangle" => MixerPatch {
            position_preset: Some("desk_triangle".into()),
            protection_max_master: Some(60.0),
            protection_ceiling_db: Some(-3.0),
            intex_vol: Some(105.0),
            intex_balance: Some(0.0),
            intex_width: Some(108.0),
            intex_bass_db: Some(1.0),
            intex_mid_db: Some(-0.5),
            intex_treble_db: Some(1.0),
            boat_vol: Some(95.0),
            boat_mode: Some("stereo".into()),
            boat_hpf_hz: Some(120.0),
            boat_lpf_hz: Some(18000.0),
            boat_bass_db: Some(0.0),
            boat_mid_db: Some(0.5),
            boat_treble_db: Some(0.5),
            laptop_vol: Some(115.0),
            laptop_mode: Some("stereo".into()),
            laptop_hpf_hz: Some(210.0),
            laptop_lpf_hz: Some(18000.0),
            laptop_bass_db: Some(-1.0),
            laptop_mid_db: Some(1.5),
            laptop_treble_db: Some(2.0),
            ..Default::default()
        },
        "cinema_center" => MixerPatch {
            position_preset: Some("cinema_center".into()),
            protection_max_master: Some(60.0),
            protection_ceiling_db: Some(-3.0),
            intex_vol: Some(105.0),
            intex_balance: Some(0.0),
            intex_width: Some(132.0),
            intex_bass_db: Some(3.0),
            intex_mid_db: Some(-2.0),
            intex_treble_db: Some(0.5),
            boat_vol: Some(95.0),
            boat_mode: Some("vocal_mid".into()),
            boat_hpf_hz: Some(160.0),
            boat_lpf_hz: Some(16000.0),
            boat_bass_db: Some(-1.5),
            boat_mid_db: Some(2.5),
            boat_treble_db: Some(1.0),
            laptop_vol: Some(125.0),
            laptop_mode: Some("vocal_mid".into()),
            laptop_hpf_hz: Some(210.0),
            laptop_lpf_hz: Some(18000.0),
            laptop_bass_db: Some(-2.0),
            laptop_mid_db: Some(3.0),
            laptop_treble_db: Some(1.5),
            ..Default::default()
        },
        "wide_room" => MixerPatch {
            position_preset: Some("wide_room".into()),
            protection_max_master: Some(60.0),
            protection_ceiling_db: Some(-3.0),
            intex_vol: Some(115.0),
            intex_balance: Some(0.0),
            intex_width: Some(122.0),
            intex_bass_db: Some(3.5),
            intex_mid_db: Some(0.5),
            intex_treble_db: Some(2.0),
            boat_vol: Some(100.0),
            boat_mode: Some("stereo".into()),
            boat_hpf_hz: Some(100.0),
            boat_lpf_hz: Some(18000.0),
            boat_bass_db: Some(1.0),
            boat_mid_db: Some(0.5),
            boat_treble_db: Some(1.5),
            laptop_vol: Some(120.0),
            laptop_mode: Some("stereo".into()),
            laptop_hpf_hz: Some(190.0),
            laptop_lpf_hz: Some(18000.0),
            laptop_bass_db: Some(-0.5),
            laptop_mid_db: Some(1.0),
            laptop_treble_db: Some(2.5),
            ..Default::default()
        },
        "night_safe" => MixerPatch {
            position_preset: Some("night_safe".into()),
            protection_enabled: Some(true),
            protection_max_master: Some(60.0),
            protection_ceiling_db: Some(-15.0),
            intex_vol: Some(80.0),
            intex_balance: Some(0.0),
            intex_width: Some(96.0),
            intex_bass_db: Some(-4.5),
            intex_mid_db: Some(0.5),
            intex_treble_db: Some(-1.5),
            boat_vol: Some(80.0),
            boat_mode: Some("vocal_mid".into()),
            boat_hpf_hz: Some(200.0),
            boat_lpf_hz: Some(12000.0),
            boat_bass_db: Some(-2.5),
            boat_mid_db: Some(1.5),
            boat_treble_db: Some(-1.5),
            laptop_vol: Some(85.0),
            laptop_mode: Some("stereo".into()),
            laptop_hpf_hz: Some(230.0),
            laptop_lpf_hz: Some(14000.0),
            laptop_bass_db: Some(-2.5),
            laptop_mid_db: Some(1.5),
            laptop_treble_db: Some(-1.5),
            ..Default::default()
        },
        "asym_desk" => MixerPatch {
            position_preset: Some("asym_desk".into()),
            protection_max_master: Some(60.0),
            protection_ceiling_db: Some(-3.0),
            intex_vol: Some(105.0),
            intex_balance: Some(16.0),
            intex_width: Some(108.0),
            intex_bass_db: Some(1.5),
            intex_mid_db: Some(-0.5),
            intex_treble_db: Some(1.0),
            boat_vol: Some(95.0),
            boat_mode: Some("stereo".into()),
            boat_hpf_hz: Some(140.0),
            boat_lpf_hz: Some(18000.0),
            boat_bass_db: Some(0.0),
            boat_mid_db: Some(1.0),
            boat_treble_db: Some(1.0),
            laptop_vol: Some(125.0),
            laptop_mode: Some("stereo".into()),
            laptop_hpf_hz: Some(210.0),
            laptop_lpf_hz: Some(18000.0),
            laptop_bass_db: Some(-1.0),
            laptop_mid_db: Some(1.0),
            laptop_treble_db: Some(2.0),
            ..Default::default()
        },
        "pure_mirror" => MixerPatch {
            position_preset: Some("pure_mirror".into()),
            protection_max_master: Some(60.0),
            protection_ceiling_db: Some(-3.0),
            intex_vol: Some(100.0),
            intex_balance: Some(0.0),
            intex_width: Some(100.0),
            intex_bass_db: Some(0.0),
            intex_mid_db: Some(0.0),
            intex_treble_db: Some(0.0),
            boat_vol: Some(100.0),
            boat_mode: Some("stereo".into()),
            boat_hpf_hz: Some(0.0),
            boat_lpf_hz: Some(18000.0),
            boat_bass_db: Some(0.0),
            boat_mid_db: Some(0.0),
            boat_treble_db: Some(0.0),
            laptop_vol: Some(100.0),
            laptop_mode: Some("stereo".into()),
            laptop_hpf_hz: Some(180.0),
            laptop_lpf_hz: Some(18000.0),
            laptop_bass_db: Some(0.0),
            laptop_mid_db: Some(0.0),
            laptop_treble_db: Some(0.0),
            ..Default::default()
        },
        _ => return None,
    };
    Some(patch)
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct MixerPatch {
    pub master_vol: Option<f64>,
    pub master_mute: Option<bool>,
    pub protection_enabled: Option<bool>,
    pub protection_max_master: Option<f64>,
    pub protection_ceiling_db: Option<f64>,
    pub position_preset: Option<String>,

    pub intex_vol: Option<f64>,
    pub intex_mute: Option<bool>,
    pub intex_delay_ms: Option<f64>,
    pub intex_balance: Option<f64>,
    pub intex_bass_db: Option<f64>,
    pub intex_mid_db: Option<f64>,
    pub intex_treble_db: Option<f64>,
    pub intex_width: Option<f64>,

    pub boat_vol: Option<f64>,
    pub boat_mute: Option<bool>,
    pub boat_delay_ms: Option<f64>,
    pub boat_mode: Option<String>,
    pub boat_hpf_hz: Option<f64>,
    pub boat_lpf_hz: Option<f64>,
    pub boat_bass_db: Option<f64>,
    pub boat_mid_db: Option<f64>,
    pub boat_treble_db: Option<f64>,
    pub boat_invert: Option<bool>,

    pub laptop_vol: Option<f64>,
    pub laptop_mute: Option<bool>,
    pub laptop_delay_ms: Option<f64>,
    pub laptop_mode: Option<String>,
    pub laptop_hpf_hz: Option<f64>,
    pub laptop_lpf_hz: Option<f64>,
    pub laptop_bass_db: Option<f64>,
    pub laptop_mid_db: Option<f64>,
    pub laptop_treble_db: Option<f64>,
    pub laptop_invert: Option<bool>,

    // Double Option distinguishes omitted `solo` from explicit JSON `"solo": null`
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub solo: Option<Option<String>>,
    pub test_mode: Option<String>,
}

fn deserialize_double_option<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::<String>::deserialize(deserializer)?))
}

fn clamp_finite(cur: f64, val: Option<f64>, lo: f64, hi: f64) -> f64 {
    match val {
        Some(v) if v.is_finite() => v.clamp(lo, hi),
        _ => cur,
    }
}

fn valid_aux_mode(s: &str) -> bool {
    matches!(s, "stereo" | "mono_center" | "vocal_mid" | "side_ambience")
}

fn valid_preset(s: &str) -> bool {
    matches!(
        s,
        "custom"
            | "desk_triangle"
            | "cinema_center"
            | "wide_room"
            | "night_safe"
            | "asym_desk"
            | "pure_mirror"
    )
}

impl MixerState {
    pub fn apply_patch(&mut self, p: &MixerPatch) {
        self.master_vol = clamp_finite(self.master_vol, p.master_vol, 0.0, 200.0);
        if let Some(v) = p.master_mute {
            self.master_mute = v;
        }
        if let Some(v) = p.protection_enabled {
            self.protection_enabled = v;
        }
        self.protection_max_master =
            clamp_finite(self.protection_max_master, p.protection_max_master, 15.0, 200.0);
        self.protection_ceiling_db =
            clamp_finite(self.protection_ceiling_db, p.protection_ceiling_db, -30.0, -0.5);
        if let Some(ref s) = p.position_preset {
            if valid_preset(s) {
                self.position_preset = s.clone();
            }
        }

        self.intex_vol = clamp_finite(self.intex_vol, p.intex_vol, 0.0, 200.0);
        if let Some(v) = p.intex_mute {
            self.intex_mute = v;
        }
        self.intex_delay_ms = clamp_finite(self.intex_delay_ms, p.intex_delay_ms, 0.0, 450.0);
        self.intex_balance = clamp_finite(self.intex_balance, p.intex_balance, -100.0, 100.0);
        self.intex_bass_db = clamp_finite(self.intex_bass_db, p.intex_bass_db, -12.0, 12.0);
        self.intex_mid_db = clamp_finite(self.intex_mid_db, p.intex_mid_db, -12.0, 12.0);
        self.intex_treble_db = clamp_finite(self.intex_treble_db, p.intex_treble_db, -12.0, 12.0);
        self.intex_width = clamp_finite(self.intex_width, p.intex_width, 50.0, 150.0);

        self.boat_vol = clamp_finite(self.boat_vol, p.boat_vol, 0.0, 200.0);
        if let Some(v) = p.boat_mute {
            self.boat_mute = v;
        }
        self.boat_delay_ms = clamp_finite(self.boat_delay_ms, p.boat_delay_ms, 0.0, 450.0);
        if let Some(ref s) = p.boat_mode {
            if valid_aux_mode(s) {
                self.boat_mode = s.clone();
            }
        }
        self.boat_hpf_hz = clamp_finite(self.boat_hpf_hz, p.boat_hpf_hz, 0.0, 500.0);
        self.boat_lpf_hz = clamp_finite(self.boat_lpf_hz, p.boat_lpf_hz, 3000.0, 18000.0);
        self.boat_bass_db = clamp_finite(self.boat_bass_db, p.boat_bass_db, -12.0, 12.0);
        self.boat_mid_db = clamp_finite(self.boat_mid_db, p.boat_mid_db, -12.0, 12.0);
        self.boat_treble_db = clamp_finite(self.boat_treble_db, p.boat_treble_db, -12.0, 12.0);
        if let Some(v) = p.boat_invert {
            self.boat_invert = v;
        }

        self.laptop_vol = clamp_finite(self.laptop_vol, p.laptop_vol, 0.0, 200.0);
        if let Some(v) = p.laptop_mute {
            self.laptop_mute = v;
        }
        self.laptop_delay_ms = clamp_finite(self.laptop_delay_ms, p.laptop_delay_ms, 0.0, 450.0);
        if let Some(ref s) = p.laptop_mode {
            if valid_aux_mode(s) {
                self.laptop_mode = s.clone();
            }
        }
        self.laptop_hpf_hz = clamp_finite(self.laptop_hpf_hz, p.laptop_hpf_hz, 0.0, 500.0);
        self.laptop_lpf_hz = clamp_finite(self.laptop_lpf_hz, p.laptop_lpf_hz, 3000.0, 18000.0);
        self.laptop_bass_db = clamp_finite(self.laptop_bass_db, p.laptop_bass_db, -12.0, 12.0);
        self.laptop_mid_db = clamp_finite(self.laptop_mid_db, p.laptop_mid_db, -12.0, 12.0);
        self.laptop_treble_db = clamp_finite(self.laptop_treble_db, p.laptop_treble_db, -12.0, 12.0);
        if let Some(v) = p.laptop_invert {
            self.laptop_invert = v;
        }

        if let Some(ref maybe_solo) = p.solo {
            self.solo = match maybe_solo {
                Some(r) if OUTPUT_ROLES.contains(&r.as_str()) => Some(r.clone()),
                _ => None,
            };
        }
        if let Some(ref m) = p.test_mode {
            if matches!(m.as_str(), "off" | "click" | "chirp" | "lr") {
                self.test_mode = m.clone();
            }
        }

        self.enforce_protection_cap();
    }

    pub fn enforce_protection_cap(&mut self) {
        self.protection_max_master = clamp_finite(
            self.protection_max_master,
            Some(self.protection_max_master),
            15.0,
            200.0,
        );
        self.protection_ceiling_db = clamp_finite(
            self.protection_ceiling_db,
            Some(self.protection_ceiling_db),
            -30.0,
            -0.5,
        );
        if self.protection_enabled && self.master_vol > self.protection_max_master {
            self.master_vol = self.protection_max_master;
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub rate: u32,
    pub channels: u16,
    pub is_loopback: bool,
    pub label: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SelectedIds {
    pub input_id: String,
    pub intex_id: String,
    pub boat_id: String,
    pub laptop_id: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SelectedNames {
    pub input_name: String,
    pub intex_name: String,
    pub boat_name: String,
    pub laptop_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WinEndpointVol {
    pub id: String,
    pub name: String,
    pub volume_pct: f64,
    pub db: f64,
    pub mute: bool,
    pub saved_pct: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WinVols {
    pub input: Option<WinEndpointVol>,
    pub intex: Option<WinEndpointVol>,
    pub boat: Option<WinEndpointVol>,
    pub laptop: Option<WinEndpointVol>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Meters {
    pub in_db: f64,
    pub intex_db: f64,
    pub boat_db: f64,
    pub laptop_db: f64,
}

impl Default for Meters {
    fn default() -> Self {
        Self {
            in_db: -92.0,
            intex_db: -92.0,
            boat_db: -92.0,
            laptop_db: -92.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Telemetry {
    pub sample_rate: u32,
    pub input_status: String,
    pub intex_status: String,
    pub boat_status: String,
    pub laptop_status: String,
    pub intex_actual_ms: f64,
    pub boat_actual_ms: f64,
    pub laptop_actual_ms: f64,
    pub intex_rate_ppm: f64,
    pub boat_rate_ppm: f64,
    pub laptop_rate_ppm: f64,
    pub intex_gr_db: f64,
    pub boat_gr_db: f64,
    pub laptop_gr_db: f64,
}

impl Default for Telemetry {
    fn default() -> Self {
        Self {
            sample_rate: MASTER_RATE,
            input_status: "Stopped".to_string(),
            intex_status: "Stopped".to_string(),
            boat_status: "Stopped".to_string(),
            laptop_status: "Stopped".to_string(),
            intex_actual_ms: 0.0,
            boat_actual_ms: 0.0,
            laptop_actual_ms: 0.0,
            intex_rate_ppm: 0.0,
            boat_rate_ppm: 0.0,
            laptop_rate_ppm: 0.0,
            intex_gr_db: 0.0,
            boat_gr_db: 0.0,
            laptop_gr_db: 0.0,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FastMetersEvent {
    pub running: bool,
    pub meters: Meters,
    pub telemetry: Telemetry,
    pub master_vol: f64,
    pub master_mute: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct StatusPayload {
    pub running: bool,
    pub mixer: MixerState,
    pub meters: Meters,
    pub telemetry: Telemetry,
    pub selected: SelectedIds,
    pub selected_names: SelectedNames,
    pub win_vols: WinVols,
    pub default_output_name: String,
    pub default_output_is_virtual: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct DevicesResponse {
    pub inputs: Vec<DeviceInfo>,
    pub outputs: Vec<DeviceInfo>,
    pub selected: SelectedIds,
    pub selected_names: SelectedNames,
    pub has_virtual_cable: bool,
    pub win_vols: WinVols,
    pub default_output_name: String,
    pub default_output_is_virtual: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct PersistedConfig {
    mixer: MixerState,
    devices: SelectedNames,
    win_device_vols: HashMap<String, f64>,
}

struct SharedRing {
    buf: Vec<[f32; 2]>,
    write_idx: u64,
}

impl SharedRing {
    fn new() -> Self {
        Self {
            buf: vec![[0.0; 2]; RING_FRAMES],
            write_idx: 0,
        }
    }

    fn reset_preseed(&mut self) {
        self.buf.fill([0.0, 0.0]);
        // Seed 600 ms into silence so outputs calibrated to 222-450 ms start aligned immediately
        self.write_idx = (MASTER_RATE as f64 * 0.6) as u64;
    }

    fn write(&mut self, frames: &[[f32; 2]]) {
        if frames.is_empty() {
            return;
        }
        let mut src = frames;
        if src.len() > RING_FRAMES {
            src = &src[src.len() - RING_FRAMES..];
        }
        let start = (self.write_idx as usize) % RING_FRAMES;
        let first = src.len().min(RING_FRAMES - start);
        self.buf[start..start + first].copy_from_slice(&src[..first]);
        let rem = src.len() - first;
        if rem > 0 {
            self.buf[..rem].copy_from_slice(&src[first..]);
        }
        self.write_idx = self.write_idx.wrapping_add(src.len() as u64);
    }

    fn read_interpolated(&self, read_pos: f64, out: &mut [[f32; 2]], step: f64) {
        let out_frames = out.len();
        if out_frames == 0 {
            return;
        }
        let w = self.write_idx;
        let max_age = (RING_FRAMES - 128) as u64;
        if (step - 1.0).abs() < 1e-6 && (read_pos - read_pos.round()).abs() < 0.05 {
            let idx_start = read_pos.round().max(0.0) as u64;
            let age = w.saturating_sub(idx_start);
            if age == 0 || age >= max_age {
                out.fill([0.0, 0.0]);
                return;
            }
            let avail = age.min(out_frames as u64) as usize;
            let s = (idx_start % RING_FRAMES as u64) as usize;
            let first = avail.min(RING_FRAMES - s);
            out[..first].copy_from_slice(&self.buf[s..s + first]);
            if first < avail {
                out[first..avail].copy_from_slice(&self.buf[..avail - first]);
            }
            if avail < out_frames {
                out[avail..].fill([0.0, 0.0]);
            }
            return;
        }

        let ring_u64 = RING_FRAMES as u64;
        let min_pos = w.saturating_sub(max_age) as f64;
        let w_lim = w.saturating_sub(1) as f64;
        let hermite_lim = w.saturating_sub(2) as f64;
        for (i, frame) in out.iter_mut().enumerate() {
            let pos = read_pos + (i as f64) * step;
            if pos < min_pos || pos >= w_lim || !pos.is_finite() {
                *frame = [0.0, 0.0];
                continue;
            }
            let idx1 = pos.floor() as u64;
            let t = (pos - idx1 as f64) as f32;
            if pos >= min_pos + 1.0 && pos < hermite_lim && idx1 >= 1 {
                let s0 = self.buf[((idx1 - 1) % ring_u64) as usize];
                let s1 = self.buf[(idx1 % ring_u64) as usize];
                let s2 = self.buf[((idx1 + 1) % ring_u64) as usize];
                let s3 = self.buf[((idx1 + 2) % ring_u64) as usize];
                let t2 = t * t;
                let t3 = t2 * t;
                let interp = |y0: f32, y1: f32, y2: f32, y3: f32| -> f32 {
                    let v = 0.5
                        * ((2.0 * y1)
                            + (-y0 + y2) * t
                            + (2.0 * y0 - 5.0 * y1 + 4.0 * y2 - y3) * t2
                            + (-y0 + 3.0 * y1 - 3.0 * y2 + y3) * t3);
                    if v.is_finite() {
                        v.clamp(-4.0, 4.0)
                    } else {
                        0.0
                    }
                };
                *frame = [
                    interp(s0[0], s1[0], s2[0], s3[0]),
                    interp(s0[1], s1[1], s2[1], s3[1]),
                ];
            } else {
                let s1 = self.buf[(idx1 % ring_u64) as usize];
                let s2 = self.buf[((idx1 + 1) % ring_u64) as usize];
                *frame = [s1[0] + (s2[0] - s1[0]) * t, s1[1] + (s2[1] - s1[1]) * t];
            }
        }
    }
}

pub fn list_wasapi_devices() -> (Vec<DeviceInfo>, Vec<DeviceInfo>, bool) {
    ensure_com();
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    let mut has_virtual_cable = false;

    let Ok(enumerator) = DeviceEnumerator::new() else {
        return (inputs, outputs, has_virtual_cable);
    };

    // Enumerate Render endpoints (both loopback capture sources and playback outputs)
    if let Ok(col) = enumerator.get_device_collection(Direction::Render) {
        for dev in (&col).into_iter().flatten() {
            if !matches!(dev.get_state(), Ok(DeviceState::Active)) {
                continue;
            }
            let Ok(id) = dev.get_id() else { continue };
            let name = dev.get_friendlyname().unwrap_or_else(|_| "Audio Output".into());
            let lower = name.to_lowercase();
            let fmt = dev.get_device_format().ok();
            let rate = fmt.as_ref().map(|f| f.get_samplespersec()).unwrap_or(MASTER_RATE);
            let ch = fmt.as_ref().map(|f| f.get_nchannels()).unwrap_or(2).clamp(1, 2);

            if lower.contains("vb-audio") || lower.contains("cable") || lower.contains("voicemeeter") {
                has_virtual_cable = true;
            }

            let in_tag = if lower.contains("cable")
                || lower.contains("voicemeeter")
                || lower.contains("steam streaming speakers")
            {
                "Virtual • Loopback"
            } else {
                "Loopback"
            };
            inputs.push(DeviceInfo {
                id: id.clone(),
                name: format!("{name} [Loopback]"),
                rate,
                channels: ch,
                is_loopback: true,
                label: format!("[{in_tag}] {name} ({rate} Hz)"),
            });

            let out_tag = if lower.contains("stone")
                || lower.contains("bt")
                || lower.contains("bluetooth")
                || lower.contains("anc")
            {
                "Bluetooth"
            } else if lower.contains("realtek") {
                "Wired / Realtek"
            } else if lower.contains("steam")
                || lower.contains("cable")
                || lower.contains("voicemeeter")
            {
                "Virtual"
            } else {
                "Output"
            };
            outputs.push(DeviceInfo {
                id,
                name: name.clone(),
                rate,
                channels: ch,
                is_loopback: false,
                label: format!("[{out_tag}] {name} ({rate} Hz)"),
            });
        }
    }

    // Physical capture microphones / line-ins
    if let Ok(col) = enumerator.get_device_collection(Direction::Capture) {
        for dev in (&col).into_iter().flatten() {
            if !matches!(dev.get_state(), Ok(DeviceState::Active)) {
                continue;
            }
            let Ok(id) = dev.get_id() else { continue };
            let name = dev.get_friendlyname().unwrap_or_else(|_| "Audio Input".into());
            let lower = name.to_lowercase();
            let fmt = dev.get_device_format().ok();
            let rate = fmt.as_ref().map(|f| f.get_samplespersec()).unwrap_or(MASTER_RATE);
            let ch = fmt.as_ref().map(|f| f.get_nchannels()).unwrap_or(2).clamp(1, 2);

            if lower.contains("vb-audio") || lower.contains("cable") || lower.contains("voicemeeter") {
                has_virtual_cable = true;
            }
            let tag = if lower.contains("cable") || lower.contains("voicemeeter") {
                "Virtual • Mic/Line"
            } else {
                "Mic/Line"
            };
            inputs.push(DeviceInfo {
                id,
                name: name.clone(),
                rate,
                channels: ch,
                is_loopback: false,
                label: format!("[{tag}] {name} ({rate} Hz)"),
            });
        }
    }

    inputs.sort_by_key(|d| {
        let nl = d.name.to_lowercase();
        let rank = if nl.contains("cable input") && d.is_loopback {
            0
        } else if nl.contains("steam streaming speakers") && d.is_loopback {
            1
        } else if nl.contains("cable output") {
            2
        } else if d.is_loopback {
            3
        } else {
            4
        };
        (rank, nl)
    });

    outputs.sort_by_key(|d| {
        let nl = d.name.to_lowercase();
        let rank = if nl.contains("headphones (realtek") {
            0
        } else if nl.contains("3.0 bt") {
            1
        } else if nl.contains("speakers (realtek") {
            2
        } else if nl.contains("bluetooth") || nl.contains("bt") {
            3
        } else {
            4
        };
        (rank, nl)
    });

    (inputs, outputs, has_virtual_cable)
}

fn is_virtual_audio_name(name: &str) -> bool {
    let nl = name.to_lowercase();
    nl.contains("steam streaming")
        || nl.contains("vb-audio")
        || nl.contains("cable")
        || nl.contains("voicemeeter")
}

pub fn default_render_endpoint_info() -> (String, String, bool) {
    ensure_com();
    let Ok(enumerator) = DeviceEnumerator::new() else {
        return (String::new(), String::new(), true);
    };
    let Ok(dev) = enumerator.get_default_device(Direction::Render) else {
        return (String::new(), String::new(), true);
    };
    let id = dev.get_id().unwrap_or_default();
    let name = dev.get_friendlyname().unwrap_or_default();
    let is_virtual = is_virtual_audio_name(&name);
    (id, name, is_virtual)
}

pub fn current_render_signature() -> String {
    ensure_com();
    let Ok(enumerator) = DeviceEnumerator::new() else {
        return String::new();
    };
    let def_id = enumerator
        .get_default_device(Direction::Render)
        .and_then(|d| d.get_id())
        .unwrap_or_default();
    let mut ids: Vec<String> = Vec::new();
    if let Ok(col) = enumerator.get_device_collection(Direction::Render) {
        for dev in (&col).into_iter().flatten() {
            if matches!(dev.get_state(), Ok(DeviceState::Active)) {
                if let Ok(id) = dev.get_id() {
                    ids.push(id);
                }
            }
        }
    }
    ids.sort();
    format!("{def_id}|{}", ids.join(","))
}

fn find_id_by_name(list: &[DeviceInfo], saved: &str) -> Option<String> {
    if saved.is_empty() {
        return None;
    }
    let s_low = saved.to_lowercase();
    let s_clean = s_low.replace(" [loopback]", "").trim().to_string();
    for d in list {
        if d.name.to_lowercase() == s_low {
            return Some(d.id.clone());
        }
    }
    for d in list {
        if d.name.to_lowercase().contains(&s_clean) {
            return Some(d.id.clone());
        }
    }
    None
}

fn is_valid_primary_candidate(d: &DeviceInfo, laptop_id: &str) -> bool {
    let nl = d.name.to_lowercase();
    !nl.contains("hands-free")
        && !nl.contains("ag audio")
        && !nl.contains("speakers (realtek")
        && !is_virtual_audio_name(&nl)
        && d.id != laptop_id
}

fn pick_default_devices(
    inputs: &[DeviceInfo],
    outputs: &[DeviceInfo],
    saved: &SelectedNames,
) -> SelectedIds {
    let mut in_id = find_id_by_name(inputs, &saved.input_name).unwrap_or_default();
    if in_id.is_empty() && saved.input_name.is_empty() {
        for pat in [
            "cable input",
            "steam streaming speakers",
            "cable output",
            "voicemeeter",
        ] {
            if let Some(d) = inputs.iter().find(|d| d.name.to_lowercase().contains(pat)) {
                in_id = d.id.clone();
                break;
            }
        }
    }

    // 1. Always reserve Laptop Built-in Speakers first so Primary Intex never double-routes onto it
    let mut laptop_id = find_id_by_name(outputs, &saved.laptop_name).unwrap_or_default();
    if laptop_id.is_empty() && saved.laptop_name.is_empty() {
        if let Some(d) = outputs.iter().find(|d| {
            let nl = d.name.to_lowercase();
            nl.contains("speakers (realtek") && !is_virtual_audio_name(&nl)
        }) {
            laptop_id = d.id.clone();
        }
    }

    // 2. Primary Intex IT-3000 (2.2): prefers Wired AUX (Headphones Realtek), seamlessly falls over to Intex 3.0 BT
    let saved_intex_is_laptop = !saved.intex_name.is_empty()
        && !saved.laptop_name.is_empty()
        && saved.intex_name.eq_ignore_ascii_case(&saved.laptop_name);
    let mut intex_id = if saved_intex_is_laptop {
        String::new()
    } else {
        find_id_by_name(outputs, &saved.intex_name).unwrap_or_default()
    };
    if intex_id == laptop_id {
        intex_id.clear();
    }
    let saved_intex_low = saved.intex_name.to_lowercase();
    let allow_intex_failover = saved.intex_name.is_empty()
        || saved_intex_is_laptop
        || saved_intex_low.contains("headphones (realtek")
        || saved_intex_low.contains("3.0 bt");
    if intex_id.is_empty() && allow_intex_failover {
        for pat in ["headphones (realtek", "headphones (3.0 bt", "3.0 bt"] {
            if let Some(d) = outputs
                .iter()
                .find(|d| d.name.to_lowercase().contains(pat) && is_valid_primary_candidate(d, &laptop_id))
            {
                intex_id = d.id.clone();
                break;
            }
        }
    }

    // 3. Optional 3rd speaker (`boat`): only bind if explicitly saved and distinct from Intex & Laptop
    let mut boat_id = find_id_by_name(outputs, &saved.boat_name).unwrap_or_default();
    if !boat_id.is_empty() && (boat_id == intex_id || boat_id == laptop_id) {
        boat_id.clear();
    }

    SelectedIds {
        input_id: in_id,
        intex_id,
        boat_id,
        laptop_id,
    }
}

fn read_device_endpoint_vol(dev: &Device, id: &str, name: &str) -> Option<WinEndpointVol> {
    let ep: IAudioEndpointVolume = unsafe { dev.as_immdevice().Activate(CLSCTX_ALL, None).ok()? };
    let scalar = unsafe { ep.GetMasterVolumeLevelScalar().ok()? };
    let db = unsafe { ep.GetMasterVolumeLevel().unwrap_or(0.0) };
    let mute = unsafe { ep.GetMute().map(|b| b.as_bool()).unwrap_or(false) };
    Some(WinEndpointVol {
        id: id.to_string(),
        name: name.to_string(),
        volume_pct: ((scalar * 100.0) as f64 * 10.0).round() / 10.0,
        db: ((db as f64) * 10.0).round() / 10.0,
        mute,
        saved_pct: None,
    })
}

pub fn query_selected_win_vols(ids: &SelectedIds) -> WinVols {
    query_win_vols_with_saved(ids, None)
}

fn query_win_vols_with_saved(ids: &SelectedIds, saved: Option<&HashMap<String, f64>>) -> WinVols {
    ensure_com();
    let Ok(enumerator) = DeviceEnumerator::new() else {
        return WinVols::default();
    };
    let get_one = |id: &str, use_saved: bool| -> Option<WinEndpointVol> {
        if id.is_empty() {
            return None;
        }
        let dev = enumerator.get_device(id).ok()?;
        let name = dev.get_friendlyname().unwrap_or_default();
        let mut info = read_device_endpoint_vol(&dev, id, &name)?;
        if use_saved {
            info.saved_pct = saved.and_then(|m| m.get(&name).copied());
        }
        Some(info)
    };
    WinVols {
        input: get_one(&ids.input_id, false),
        intex: get_one(&ids.intex_id, true),
        boat: get_one(&ids.boat_id, true),
        laptop: get_one(&ids.laptop_id, true),
    }
}

pub fn set_device_endpoint_volume(device_id: &str, volume_pct: f64, unmute: bool) -> bool {
    if device_id.is_empty() {
        return false;
    }
    ensure_com();
    let Ok(enumerator) = DeviceEnumerator::new() else {
        return false;
    };
    let Ok(dev) = enumerator.get_device(device_id) else {
        return false;
    };
    let Ok(ep) = (unsafe { dev.as_immdevice().Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) }) else {
        return false;
    };
    let scalar = (volume_pct / 100.0).clamp(0.0, 1.0) as f32;
    unsafe {
        if unmute {
            let _ = ep.SetMute(false, std::ptr::null());
        }
        ep.SetMasterVolumeLevelScalar(scalar, std::ptr::null()).is_ok()
    }
}

pub fn default_config_path() -> PathBuf {
    if let Ok(p) = std::env::var("PHASEMIX_CONFIG") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(appdata).join("PhaseMiX").join("config.json")
}

struct CalibrationSync {
    active: AtomicBool,
    bt_role: AtomicU8,
    wired_role: AtomicU8,
    s0: AtomicU64,
    s1: AtomicU64,
    s2: AtomicU64,
    s3: AtomicU64,
    bt1_lag: AtomicU64,
    aux1_lag: AtomicU64,
    bt2_lag: AtomicU64,
    aux2_lag: AtomicU64,
}

impl Default for CalibrationSync {
    fn default() -> Self {
        Self {
            active: AtomicBool::new(false),
            bt_role: AtomicU8::new(1),
            wired_role: AtomicU8::new(3),
            s0: AtomicU64::new(0),
            s1: AtomicU64::new(0),
            s2: AtomicU64::new(0),
            s3: AtomicU64::new(0),
            bt1_lag: AtomicU64::new(0),
            aux1_lag: AtomicU64::new(0),
            bt2_lag: AtomicU64::new(0),
            aux2_lag: AtomicU64::new(0),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CalibrationResult {
    pub ok: bool,
    pub measured_ms: f64,
    pub confidence: f64,
    pub message: String,
    pub status: StatusPayload,
}

struct EngineInner {
    config_path: PathBuf,
    mixer: RwLock<MixerState>,
    fast_params: RwLock<FastAudioParams>,
    prev_day_preset: RwLock<String>,
    selected_ids: RwLock<SelectedIds>,
    selected_names: RwLock<SelectedNames>,
    preferred_names: RwLock<SelectedNames>,
    win_device_vols: RwLock<HashMap<String, f64>>,
    meters: RwLock<Meters>,
    telemetry: RwLock<Telemetry>,
    ring: Mutex<SharedRing>,
    cal: CalibrationSync,
    running: AtomicBool,
    stop_flag: Arc<AtomicBool>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    timer_guard: Mutex<Option<WinTimerGuard>>,
    save_signal: (Mutex<Option<Instant>>, Condvar),
}

fn sanitize_win_vol_map(raw: HashMap<String, f64>) -> HashMap<String, f64> {
    raw.into_iter()
        .filter_map(|(k, v)| {
            let clean_k = k.trim().to_string();
            (!clean_k.is_empty() && v.is_finite()).then(|| (clean_k, v.clamp(0.0, 100.0).round()))
        })
        .collect()
}

/// Ensures PhaseMiX's own per-app Windows Volume Mixer session is always 100% unmuted (beats Communications auto-ducking).
fn ensure_session_unity(dev: &Device) {
    unsafe {
        if let Ok(mgr) = dev
            .as_immdevice()
            .Activate::<IAudioSessionManager>(CLSCTX_ALL, None)
        {
            if let Ok(sv) = mgr.GetSimpleAudioVolume(None, 0) {
                let muted = sv.GetMute().map(|b| b.as_bool()).unwrap_or(false);
                if muted {
                    let _ = sv.SetMute(false, std::ptr::null());
                }
                if let Ok(v) = sv.GetMasterVolume() {
                    if v < 0.99 {
                        let _ = sv.SetMasterVolume(1.0, std::ptr::null());
                    }
                }
            }
        }
    }
}

fn apply_saved_win_vol_for(inner: &EngineInner, dev_id: &str, dev_name: &str) {
    if dev_id.is_empty() || dev_name.is_empty() {
        return;
    }
    let saved = inner.win_device_vols.read().get(dev_name).copied();
    if let Some(pct) = saved {
        let _ = set_device_endpoint_volume(dev_id, pct.clamp(0.0, 100.0), true);
        return;
    }
    // Bluetooth A2DP endpoints on Windows use a -130 dB taper (32% = -17.3 dB!), making BT barely audible
    // once PhaseMiX Master Bus also attenuates. When virtual loopback owns Windows playback, normalize
    // unpinned severely-attenuated BT endpoints to 95% (~-0.9 dB) so BT matches wired Realtek drive.
    if is_wireless_device_name(dev_name) && default_render_endpoint_info().2 {
        ensure_com();
        if let Ok(enumerator) = DeviceEnumerator::new() {
            if let Ok(dev) = enumerator.get_device(dev_id) {
                if let Some(info) = read_device_endpoint_vol(&dev, dev_id, dev_name) {
                    if info.db < -6.0 || info.mute {
                        let _ = set_device_endpoint_volume(dev_id, 95.0, true);
                    }
                }
            }
        }
    }
}

/// Auto-syncs delays when Intex 2.2 switches between Wired AUX (0/0 ms) and Bluetooth (0/224 ms).
fn sync_delays_for_intex_mode(inner: &EngineInner, intex_name: &str, boat_name: &str) {
    if !boat_name.is_empty() || intex_name.is_empty() {
        return;
    }
    let intex_is_bt = is_wireless_device_name(intex_name);
    let mut m = inner.mixer.write();
    if intex_is_bt {
        let target_ms = m.calibrated_delay_ms.unwrap_or(224.0);
        m.intex_delay_ms = 0.0;
        if m.laptop_delay_ms < 20.0 {
            m.laptop_delay_ms = target_ms;
        }
    } else {
        m.intex_delay_ms = 0.0;
        m.laptop_delay_ms = 0.0;
    }
    *inner.fast_params.write() = FastAudioParams::from_mixer(&m);
}

pub struct AudioRouterEngine {
    inner: Arc<EngineInner>,
    _saver_thread: Option<JoinHandle<()>>,
}

impl AudioRouterEngine {
    pub fn new(config_path: PathBuf) -> Self {
        if let Some(parent) = config_path.parent() {
            if !parent.as_os_str().is_empty() {
                let _ = fs::create_dir_all(parent);
            }
        }

        let default_m = MixerState::default();
        let default_fast = FastAudioParams::from_mixer(&default_m);
        let inner = Arc::new(EngineInner {
            config_path,
            mixer: RwLock::new(default_m),
            fast_params: RwLock::new(default_fast),
            prev_day_preset: RwLock::new("desk_triangle".to_string()),
            selected_ids: RwLock::new(SelectedIds::default()),
            selected_names: RwLock::new(SelectedNames::default()),
            preferred_names: RwLock::new(SelectedNames::default()),
            win_device_vols: RwLock::new(HashMap::new()),
            meters: RwLock::new(Meters::default()),
            telemetry: RwLock::new(Telemetry::default()),
            ring: Mutex::new(SharedRing::new()),
            cal: CalibrationSync::default(),
            running: AtomicBool::new(false),
            stop_flag: Arc::new(AtomicBool::new(false)),
            workers: Mutex::new(Vec::new()),
            timer_guard: Mutex::new(None),
            save_signal: (Mutex::new(None), Condvar::new()),
        });

        Self::load_config(&inner);
        // Isolate WASAPI MTA COM initialization off the caller/main UI thread (tao needs STA/OleInitialize)
        let init_inner = inner.clone();
        if let Ok(h) = thread::Builder::new()
            .name("wasapi-init".to_string())
            .spawn(move || {
                Self::refresh_device_selection_inner(&init_inner, false);
            })
        {
            let _ = h.join();
        }

        // Single background debounced config writer thread (never spawns per drag!)
        let saver_inner = inner.clone();
        let saver = thread::Builder::new()
            .name("config-saver".to_string())
            .spawn(move || loop {
                let (lock, cvar) = &saver_inner.save_signal;
                let mut deadline = lock.lock();
                while deadline.is_none() {
                    cvar.wait(&mut deadline);
                }
                if let Some(target_t) = *deadline {
                    let now = Instant::now();
                    if now < target_t {
                        cvar.wait_for(&mut deadline, target_t - now);
                        continue;
                    }
                    *deadline = None;
                    drop(deadline);
                    Self::save_config_now(&saver_inner);
                }
            })
            .ok();

        Self {
            inner,
            _saver_thread: saver,
        }
    }

    fn backup_path(path: &PathBuf) -> PathBuf {
        PathBuf::from(format!("{}.bak", path.display()))
    }

    fn parse_persisted_config(path: &PathBuf) -> Option<PersistedConfig> {
        let raw = fs::read_to_string(path).ok()?;
        serde_json::from_str::<PersistedConfig>(&raw).ok()
    }

    fn load_config(inner: &EngineInner) {
        let parsed = Self::parse_persisted_config(&inner.config_path)
            .or_else(|| Self::parse_persisted_config(&Self::backup_path(&inner.config_path)));
        let Some(parsed) = parsed else {
            return;
        };
        let mut m = parsed.mixer;
        let mut devs = parsed.devices;
        m.enforce_protection_cap();
        m.test_mode = "off".to_string();
        m.master_mute = false;
        m.intex_mute = false;
        m.boat_mute = false;
        m.laptop_mute = false;
        m.solo = None;

        // Auto-heal legacy 5.2 configs where unplugged AUX caused Intex to duplicate Laptop Speakers
        // while Intex 3.0 BT sat inside the vocal-filtered `boat` center slot.
        let intex_dup_laptop = !devs.intex_name.is_empty()
            && !devs.laptop_name.is_empty()
            && devs.intex_name.eq_ignore_ascii_case(&devs.laptop_name);
        if intex_dup_laptop {
            if devs.boat_name.to_lowercase().contains("3.0 bt") {
                devs.intex_name = std::mem::take(&mut devs.boat_name);
                m.intex_delay_ms = 0.0;
                m.laptop_delay_ms = m.calibrated_delay_ms.unwrap_or(224.0);
            } else {
                devs.intex_name.clear();
            }
        }

        if m.position_preset != "night_safe" && m.position_preset != "custom" {
            *inner.prev_day_preset.write() = m.position_preset.clone();
        }
        *inner.fast_params.write() = FastAudioParams::from_mixer(&m);
        *inner.mixer.write() = m;
        *inner.selected_names.write() = devs.clone();
        *inner.preferred_names.write() = devs;
        *inner.win_device_vols.write() = sanitize_win_vol_map(parsed.win_device_vols);
    }

    fn save_config_now(inner: &EngineInner) {
        let payload = PersistedConfig {
            mixer: inner.mixer.read().clone(),
            devices: inner.preferred_names.read().clone(),
            win_device_vols: inner.win_device_vols.read().clone(),
        };
        let Ok(json) = serde_json::to_string_pretty(&payload) else {
            return;
        };
        let tmp = PathBuf::from(format!("{}.tmp", inner.config_path.display()));
        if fs::write(&tmp, &json).is_ok() {
            if inner.config_path.exists() {
                let _ = fs::copy(&inner.config_path, Self::backup_path(&inner.config_path));
            }
            let _ = fs::rename(&tmp, &inner.config_path);
        }
    }

    fn schedule_save(inner: &EngineInner) {
        let (lock, cvar) = &inner.save_signal;
        *lock.lock() = Some(Instant::now() + Duration::from_millis(850));
        cvar.notify_one();
    }

    fn refresh_device_selection_inner(inner: &Arc<EngineInner>, persist: bool) -> DevicesResponse {
        let (inputs, outputs, has_vc) = list_wasapi_devices();
        let prev_ids = inner.selected_ids.read().clone();
        let saved_names = inner.preferred_names.read().clone();
        let picked = pick_default_devices(&inputs, &outputs, &saved_names);

        let id_to_in: HashMap<&str, &str> =
            inputs.iter().map(|d| (d.id.as_str(), d.name.as_str())).collect();
        let id_to_out: HashMap<&str, &str> =
            outputs.iter().map(|d| (d.id.as_str(), d.name.as_str())).collect();

        let new_names = SelectedNames {
            input_name: id_to_in.get(picked.input_id.as_str()).unwrap_or(&"").to_string(),
            intex_name: id_to_out.get(picked.intex_id.as_str()).unwrap_or(&"").to_string(),
            boat_name: id_to_out.get(picked.boat_id.as_str()).unwrap_or(&"").to_string(),
            laptop_name: id_to_out.get(picked.laptop_id.as_str()).unwrap_or(&"").to_string(),
        };

        // Preserve previously remembered offline device names so booting without a speaker never wipes config
        let merged_preferred = SelectedNames {
            input_name: if !new_names.input_name.is_empty() {
                new_names.input_name.clone()
            } else {
                saved_names.input_name
            },
            intex_name: if !new_names.intex_name.is_empty() {
                new_names.intex_name.clone()
            } else {
                saved_names.intex_name
            },
            boat_name: if !new_names.boat_name.is_empty() {
                new_names.boat_name.clone()
            } else {
                saved_names.boat_name
            },
            laptop_name: if !new_names.laptop_name.is_empty() {
                new_names.laptop_name.clone()
            } else {
                saved_names.laptop_name
            },
        };

        let prev_names = inner.selected_names.read().clone();
        *inner.selected_ids.write() = picked.clone();
        *inner.selected_names.write() = new_names.clone();
        *inner.preferred_names.write() = merged_preferred;

        // If Primary Intex switched between Wired AUX and Wireless BT (e.g. hot-plugging AUX or connecting 3.0 BT),
        // align delays automatically for zero-lag Wired 4.2 or phase-locked BT 4.2.
        let intex_mode_switched = !new_names.intex_name.is_empty()
            && (prev_names.intex_name.is_empty()
                || is_wireless_device_name(&new_names.intex_name)
                    != is_wireless_device_name(&prev_names.intex_name));
        if intex_mode_switched && new_names.boat_name.is_empty() {
            sync_delays_for_intex_mode(inner, &new_names.intex_name, &new_names.boat_name);
        }

        if persist {
            Self::save_config_now(inner);
        }

        // Auto-restore saved Windows Device Levels on startup or speaker hot-plug attach
        if !picked.intex_id.is_empty() && picked.intex_id != prev_ids.intex_id {
            apply_saved_win_vol_for(inner, &picked.intex_id, &new_names.intex_name);
        }
        if !picked.boat_id.is_empty() && picked.boat_id != prev_ids.boat_id {
            apply_saved_win_vol_for(inner, &picked.boat_id, &new_names.boat_name);
        }
        if !picked.laptop_id.is_empty() && picked.laptop_id != prev_ids.laptop_id {
            apply_saved_win_vol_for(inner, &picked.laptop_id, &new_names.laptop_name);
        }

        let win_vols = query_win_vols_with_saved(&picked, Some(&inner.win_device_vols.read()));
        let (_, default_output_name, default_output_is_virtual) = default_render_endpoint_info();
        let changed = picked.input_id != prev_ids.input_id
            || picked.intex_id != prev_ids.intex_id
            || picked.boat_id != prev_ids.boat_id
            || picked.laptop_id != prev_ids.laptop_id;
        if changed && inner.running.load(Ordering::SeqCst) {
            // Device graph changed underneath live stream -> restart cleanly
            Self::restart_inner(inner);
        }

        DevicesResponse {
            inputs,
            outputs,
            selected: picked,
            selected_names: new_names,
            has_virtual_cable: has_vc,
            win_vols,
            default_output_name,
            default_output_is_virtual,
        }
    }

    pub fn refresh_devices(&self) -> DevicesResponse {
        Self::refresh_device_selection_inner(&self.inner, true)
    }

    pub fn refresh_devices_passive(&self) -> DevicesResponse {
        Self::refresh_device_selection_inner(&self.inner, false)
    }

    pub fn set_devices(&self, mut ids: SelectedIds) -> StatusPayload {
        let prev_ids = self.inner.selected_ids.read().clone();
        let prev_names = self.inner.selected_names.read().clone();

        // Prevent accidental double-routing of the exact same Windows audio endpoint across two channels
        if !ids.intex_id.is_empty() && ids.intex_id == ids.laptop_id {
            if ids.intex_id != prev_ids.intex_id {
                ids.laptop_id.clear();
            } else {
                ids.intex_id.clear();
            }
        }
        if !ids.boat_id.is_empty() && (ids.boat_id == ids.intex_id || ids.boat_id == ids.laptop_id) {
            if ids.boat_id != prev_ids.boat_id {
                if ids.intex_id == ids.boat_id {
                    ids.intex_id.clear();
                }
                if ids.laptop_id == ids.boat_id {
                    ids.laptop_id.clear();
                }
            } else {
                ids.boat_id.clear();
            }
        }

        let (inputs, outputs, _) = list_wasapi_devices();
        let id_to_in: HashMap<&str, &str> =
            inputs.iter().map(|d| (d.id.as_str(), d.name.as_str())).collect();
        let id_to_out: HashMap<&str, &str> =
            outputs.iter().map(|d| (d.id.as_str(), d.name.as_str())).collect();

        let explicit = SelectedNames {
            input_name: id_to_in.get(ids.input_id.as_str()).unwrap_or(&"").to_string(),
            intex_name: id_to_out.get(ids.intex_id.as_str()).unwrap_or(&"").to_string(),
            boat_name: id_to_out.get(ids.boat_id.as_str()).unwrap_or(&"").to_string(),
            laptop_name: id_to_out.get(ids.laptop_id.as_str()).unwrap_or(&"").to_string(),
        };
        apply_saved_win_vol_for(&self.inner, &ids.intex_id, &explicit.intex_name);
        apply_saved_win_vol_for(&self.inner, &ids.boat_id, &explicit.boat_name);
        apply_saved_win_vol_for(&self.inner, &ids.laptop_id, &explicit.laptop_name);

        let intex_changed_mode = !explicit.intex_name.is_empty()
            && (prev_names.intex_name.is_empty()
                || is_wireless_device_name(&explicit.intex_name)
                    != is_wireless_device_name(&prev_names.intex_name));
        if intex_changed_mode && explicit.boat_name.is_empty() {
            sync_delays_for_intex_mode(&self.inner, &explicit.intex_name, &explicit.boat_name);
        }

        *self.inner.selected_names.write() = explicit.clone();
        *self.inner.preferred_names.write() = explicit;
        *self.inner.selected_ids.write() = ids;
        Self::save_config_now(&self.inner);

        if self.inner.running.load(Ordering::SeqCst) {
            Self::restart_inner(&self.inner);
        }
        self.status_payload()
    }

    pub fn update_mixer(&self, patch: MixerPatch) -> MixerState {
        let updated = {
            let mut m = self.inner.mixer.write();
            m.apply_patch(&patch);
            if m.position_preset != "night_safe" && m.position_preset != "custom" {
                *self.inner.prev_day_preset.write() = m.position_preset.clone();
            }
            *self.inner.fast_params.write() = FastAudioParams::from_mixer(&m);
            m.clone()
        };
        Self::schedule_save(&self.inner);
        updated
    }

    pub fn apply_preset(&self, id: &str) -> Option<MixerState> {
        let patch = preset_patch(id)?;
        let updated = {
            let mut m = self.inner.mixer.write();
            m.apply_patch(&patch);
            if id != "night_safe" && id != "custom" {
                *self.inner.prev_day_preset.write() = id.to_string();
            }
            *self.inner.fast_params.write() = FastAudioParams::from_mixer(&m);
            m.clone()
        };
        Self::save_config_now(&self.inner);
        Some(updated)
    }

    /// One-tap Midnight Guard toggle: jumps to Night Ear-Safe (-15 dB), or restores last day preset.
    pub fn toggle_night_guard(&self) -> MixerState {
        let is_night = self.inner.mixer.read().position_preset == "night_safe";
        let target = if is_night {
            let prev = self.inner.prev_day_preset.read().clone();
            if prev == "night_safe" || prev.is_empty() {
                "desk_triangle".to_string()
            } else {
                prev
            }
        } else {
            let cur = self.inner.mixer.read().position_preset.clone();
            if cur != "custom" {
                *self.inner.prev_day_preset.write() = cur;
            }
            "night_safe".to_string()
        };
        self.apply_preset(&target).unwrap_or_else(|| self.inner.mixer.read().clone())
    }

    /// Smart cycle across main listening modes: Desk -> Music -> Cinema -> Night Safe -> Desk.
    pub fn cycle_smart_preset(&self) -> MixerState {
        const ORDER: [&str; 4] = ["desk_triangle", "wide_room", "cinema_center", "night_safe"];
        let cur = self.inner.mixer.read().position_preset.clone();
        let next = match ORDER.iter().position(|&p| p == cur) {
            Some(i) => ORDER[(i + 1) % ORDER.len()],
            None => ORDER[0],
        };
        self.apply_preset(next).unwrap_or_else(|| self.inner.mixer.read().clone())
    }

    pub fn export_profile_json(&self) -> Result<String, String> {
        let payload = PersistedConfig {
            mixer: self.inner.mixer.read().clone(),
            devices: self.inner.preferred_names.read().clone(),
            win_device_vols: self.inner.win_device_vols.read().clone(),
        };
        serde_json::to_string_pretty(&payload).map_err(|e| format!("{e}"))
    }

    pub fn import_profile_json(&self, raw: &str) -> Result<StatusPayload, String> {
        let parsed = serde_json::from_str::<PersistedConfig>(raw)
            .map_err(|e| format!("Invalid profile JSON: {e}"))?;
        let mut m = parsed.mixer;
        m.enforce_protection_cap();
        m.test_mode = "off".to_string();
        m.master_mute = false;
        m.intex_mute = false;
        m.boat_mute = false;
        m.laptop_mute = false;
        m.solo = None;
        if m.position_preset != "night_safe" && m.position_preset != "custom" {
            *self.inner.prev_day_preset.write() = m.position_preset.clone();
        }
        *self.inner.fast_params.write() = FastAudioParams::from_mixer(&m);
        *self.inner.mixer.write() = m;
        *self.inner.win_device_vols.write() = sanitize_win_vol_map(parsed.win_device_vols);

        if !parsed.devices.input_name.is_empty()
            || !parsed.devices.intex_name.is_empty()
            || !parsed.devices.boat_name.is_empty()
            || !parsed.devices.laptop_name.is_empty()
        {
            *self.inner.preferred_names.write() = parsed.devices;
            Self::refresh_device_selection_inner(&self.inner, true);
        } else {
            let ids = self.inner.selected_ids.read().clone();
            let names = self.inner.selected_names.read().clone();
            apply_saved_win_vol_for(&self.inner, &ids.intex_id, &names.intex_name);
            apply_saved_win_vol_for(&self.inner, &ids.boat_id, &names.boat_name);
            apply_saved_win_vol_for(&self.inner, &ids.laptop_id, &names.laptop_name);
            Self::save_config_now(&self.inner);
        }
        Ok(self.status_payload())
    }

    pub fn reset_factory(&self) -> StatusPayload {
        *self.inner.prev_day_preset.write() = "desk_triangle".to_string();
        let def = MixerState::default();
        *self.inner.fast_params.write() = FastAudioParams::from_mixer(&def);
        *self.inner.mixer.write() = def;
        self.inner.win_device_vols.write().clear();
        Self::save_config_now(&self.inner);
        self.status_payload()
    }

    /// Measure acoustic BT vs Wired lag differential via default laptop mic using FM chirps.
    /// Supports both 4.2 Wireless mode (Intex 3.0 BT vs Laptop) and 3-speaker mode (Extra BT vs Wired Intex).
    pub fn calibrate_sync_delay(&self) -> CalibrationResult {
        ensure_com();
        if !self.is_running() {
            return CalibrationResult {
                ok: false,
                measured_ms: 0.0,
                confidence: 0.0,
                message: "Start audio routing before running auto-align".to_string(),
                status: self.status_payload(),
            };
        }
        let ids = self.inner.selected_ids.read().clone();
        let names = self.inner.selected_names.read().clone();
        let intex_is_bt = !ids.intex_id.is_empty() && is_wireless_device_name(&names.intex_name);
        let boat_is_bt = !ids.boat_id.is_empty() && is_wireless_device_name(&names.boat_name);

        let (bt_role, wired_role): (u8, u8) = if intex_is_bt && !ids.laptop_id.is_empty() {
            (1, 3) // 4.2 Wireless BT mode: Intex 3.0 BT vs Laptop Speakers
        } else if boat_is_bt && !ids.intex_id.is_empty() {
            (2, 1) // Optional 3rd BT speaker vs Wired Intex AUX
        } else if !ids.intex_id.is_empty() && !ids.laptop_id.is_empty() {
            (1, 3) // Generic 4.2 pair (e.g. USB BT dongle without BT in friendly name)
        } else if !ids.boat_id.is_empty() && !ids.intex_id.is_empty() {
            (2, 1)
        } else {
            return CalibrationResult {
                ok: false,
                measured_ms: 0.0,
                confidence: 0.0,
                message: "Assign both Primary (Bluetooth) and Built-in Speakers first".to_string(),
                status: self.status_payload(),
            };
        };

        struct CalGuard<'a>(&'a CalibrationSync);
        impl Drop for CalGuard<'_> {
            fn drop(&mut self) {
                self.0.active.store(false, Ordering::SeqCst);
            }
        }

        let mic_open = DeviceEnumerator::new()
            .and_then(|e| e.get_default_device(Direction::Capture))
            .map_err(|e| format!("No active default microphone ({e})"))
            .and_then(|dev| open_loopback_or_capture(&dev));

        let (client, fmt, h_event) = match mic_open {
            Ok(v) => v,
            Err(msg) => {
                return CalibrationResult {
                    ok: false,
                    measured_ms: 0.0,
                    confidence: 0.0,
                    message: msg,
                    status: self.status_payload(),
                };
            }
        };
        let cap_client = match client.get_audiocaptureclient() {
            Ok(c) => c,
            Err(e) => {
                return CalibrationResult {
                    ok: false,
                    measured_ms: 0.0,
                    confidence: 0.0,
                    message: format!("Microphone client error: {e}"),
                    status: self.status_payload(),
                };
            }
        };
        if let Err(e) = client.start_stream() {
            return CalibrationResult {
                ok: false,
                measured_ms: 0.0,
                confidence: 0.0,
                message: format!("Microphone start failed: {e}"),
                status: self.status_payload(),
            };
        }

        let (chirp_cos, chirp_sin) = generate_cal_chirp_pair(MASTER_RATE_F64);
        let frame_bytes =
            (fmt.get_nchannels().max(1) as usize) * ((fmt.get_bitspersample() / 8).max(1) as usize);
        let mut raw_bytes: Vec<u8> = Vec::with_capacity(48_000);
        let mut scratch_stereo: Vec<[f32; 2]> = Vec::with_capacity(2048);
        let mut out_48k: Vec<[f32; 2]> = Vec::with_capacity(2048);
        let mut mono: Vec<f32> = Vec::with_capacity(MASTER_RATE as usize * 4);

        let mut drain_mic = |dst: &mut Vec<f32>, target_len: usize, max_wait: Duration| {
            let t0 = Instant::now();
            while dst.len() < target_len && t0.elapsed() < max_wait {
                if let Some(ref ev) = h_event {
                    let _ = ev.wait_for_event(20);
                } else {
                    thread::sleep(Duration::from_millis(10));
                }
                loop {
                    let pkt = cap_client.get_next_packet_size().unwrap_or(Some(0)).unwrap_or(0);
                    if pkt == 0 {
                        break;
                    }
                    let needed = (pkt as usize) * frame_bytes;
                    if raw_bytes.len() < needed {
                        raw_bytes.resize(needed, 0);
                    }
                    match cap_client.read_from_device(&mut raw_bytes[..needed]) {
                        Ok((read_n, info)) if read_n > 0 => {
                            if info.flags.silent {
                                let add = (read_n as usize).min(4096);
                                dst.resize(dst.len() + add, 0.0);
                            } else {
                                let bytes_read = (read_n as usize) * frame_bytes;
                                decode_bytes_to_stereo_48k(
                                    &raw_bytes[..bytes_read],
                                    &fmt,
                                    &mut scratch_stereo,
                                    &mut out_48k,
                                );
                                for f in &out_48k {
                                    dst.push(0.5 * (f[0] + f[1]));
                                }
                            }
                        }
                        _ => break,
                    }
                }
            }
        };

        // Drain 160 ms cold-start mic frames before scheduling chirps so windows start deterministic
        drain_mic(&mut mono, (MASTER_RATE as usize * 16) / 100, Duration::from_millis(450));
        mono.clear();

        let w_base = self.inner.ring.lock().write_idx;
        let s0 = w_base + (MASTER_RATE as u64 * 18) / 100; // +180 ms: BT 1
        let s1 = w_base + (MASTER_RATE as u64 * 98) / 100; // +980 ms: AUX 1
        let s2 = w_base + (MASTER_RATE as u64 * 175) / 100; // +1750 ms: BT 2
        let s3 = w_base + (MASTER_RATE as u64 * 255) / 100; // +2550 ms: AUX 2
        self.inner.cal.bt_role.store(bt_role, Ordering::SeqCst);
        self.inner.cal.wired_role.store(wired_role, Ordering::SeqCst);
        self.inner.cal.s0.store(s0, Ordering::SeqCst);
        self.inner.cal.s1.store(s1, Ordering::SeqCst);
        self.inner.cal.s2.store(s2, Ordering::SeqCst);
        self.inner.cal.s3.store(s3, Ordering::SeqCst);
        self.inner.cal.bt1_lag.store(0, Ordering::SeqCst);
        self.inner.cal.aux1_lag.store(0, Ordering::SeqCst);
        self.inner.cal.bt2_lag.store(0, Ordering::SeqCst);
        self.inner.cal.aux2_lag.store(0, Ordering::SeqCst);
        self.inner.cal.active.store(true, Ordering::SeqCst);
        let _cal_guard = CalGuard(&self.inner.cal);

        let target_samples = (MASTER_RATE as usize * 305) / 100; // 3.05 s capture
        drain_mic(&mut mono, target_samples, Duration::from_millis(3500));
        let _ = client.stop_stream();
        self.inner.cal.active.store(false, Ordering::SeqCst);

        if mono.len() < (MASTER_RATE as usize * 27) / 10 {
            return CalibrationResult {
                ok: false,
                measured_ms: 0.0,
                confidence: 0.0,
                message: "Microphone capture too short — check Windows mic permissions".to_string(),
                status: self.status_payload(),
            };
        }

        let bt1_lag = self.inner.cal.bt1_lag.load(Ordering::Relaxed);
        let aux1_lag = self.inner.cal.aux1_lag.load(Ordering::Relaxed);
        let bt2_lag = self.inner.cal.bt2_lag.load(Ordering::Relaxed);
        let aux2_lag = self.inner.cal.aux2_lag.load(Ordering::Relaxed);
        if bt1_lag == 0 || aux1_lag == 0 || bt2_lag == 0 || aux2_lag == 0 {
            return CalibrationResult {
                ok: false,
                measured_ms: 0.0,
                confidence: 0.0,
                message: "A speaker did not output the calibration burst — check connection".to_string(),
                status: self.status_payload(),
            };
        }

        let b1_end = ((MASTER_RATE as usize * 88) / 100).min(mono.len());
        let a1_end = ((MASTER_RATE as usize * 162) / 100).min(mono.len());
        let b2_end = ((MASTER_RATE as usize * 244) / 100).min(mono.len());

        let w_bt1 = find_chirp_arrival(&mono, &chirp_cos, &chirp_sin, 0, b1_end);
        let w_aux1 = find_chirp_arrival(&mono, &chirp_cos, &chirp_sin, b1_end, a1_end);
        let w_bt2 = find_chirp_arrival(&mono, &chirp_cos, &chirp_sin, a1_end, b2_end);
        let w_aux2 = find_chirp_arrival(&mono, &chirp_cos, &chirp_sin, b2_end, mono.len());

        let (Some((p_bt1, s_bt1)), Some((p_aux1, s_aux1)), Some((p_bt2, s_bt2)), Some((p_aux2, s_aux2))) =
            (w_bt1, w_aux1, w_bt2, w_aux2)
        else {
            return CalibrationResult {
                ok: false,
                measured_ms: 0.0,
                confidence: 0.0,
                message: "Could not detect all 4 chirps — keep room quiet and speakers unmuted"
                    .to_string(),
                status: self.status_payload(),
            };
        };

        let min_snr = s_bt1.min(s_aux1).min(s_bt2).min(s_aux2);
        if min_snr < 4.0 {
            return CalibrationResult {
                ok: false,
                measured_ms: 0.0,
                confidence: 0.0,
                message: "Room too noisy or speakers too quiet — raise speaker level slightly"
                    .to_string(),
                status: self.status_payload(),
            };
        }

        let ms_scale = MASTER_RATE_F64 / 1000.0;
        let d1_ms = ((p_bt1 - (p_aux1 - (s1 - s0) as f64)) + (aux1_lag as f64 - bt1_lag as f64))
            / ms_scale;
        let d2_ms = ((p_bt2 - (p_aux2 - (s3 - s2) as f64)) + (aux2_lag as f64 - bt2_lag as f64))
            / ms_scale;
        let spread = (d1_ms - d2_ms).abs();
        if spread > 18.0 {
            return CalibrationResult {
                ok: false,
                measured_ms: 0.0,
                confidence: 0.0,
                message: format!(
                    "Unstable mic echo (±{:.0} ms spread) — move mic closer & retry",
                    spread.round()
                ),
                status: self.status_payload(),
            };
        }

        let measured_ms = ((d1_ms + d2_ms) * 0.5).round().clamp(0.0, 450.0);

        {
            let mut m = self.inner.mixer.write();
            if bt_role == 1 && wired_role == 3 {
                m.intex_delay_ms = 0.0;
                m.laptop_delay_ms = measured_ms;
            } else {
                m.intex_delay_ms = measured_ms;
                m.laptop_delay_ms = measured_ms;
                m.boat_delay_ms = 0.0;
            }
            m.calibrated_delay_ms = Some(measured_ms);
            *self.inner.fast_params.write() = FastAudioParams::from_mixer(&m);
        }
        Self::save_config_now(&self.inner);

        let spread_i = spread.round();
        let target_label = if bt_role == 1 && wired_role == 3 {
            "Built-in to Bluetooth"
        } else {
            "Wired & Built-in"
        };
        CalibrationResult {
            ok: true,
            measured_ms,
            confidence: spread_i,
            message: if spread_i < 1.0 {
                format!("Aligned {target_label} at {measured_ms:.0} ms")
            } else {
                format!("Aligned {target_label} at {measured_ms:.0} ms (±{spread_i:.0} ms)")
            },
            status: self.status_payload(),
        }
    }

    fn role_device_info(&self, role: &str) -> Option<(String, String)> {
        let ids = self.inner.selected_ids.read();
        let names = self.inner.selected_names.read();
        match role {
            "input" => Some((ids.input_id.clone(), names.input_name.clone())),
            "intex" => Some((ids.intex_id.clone(), names.intex_name.clone())),
            "boat" => Some((ids.boat_id.clone(), names.boat_name.clone())),
            "laptop" => Some((ids.laptop_id.clone(), names.laptop_name.clone())),
            _ => None,
        }
    }

    pub fn set_win_vol(&self, role: &str, volume_pct: f64) -> (bool, StatusPayload) {
        let Some((dev_id, dev_name)) = self.role_device_info(role) else {
            return (false, self.status_payload());
        };
        let pct = volume_pct.clamp(0.0, 100.0);
        let ok = set_device_endpoint_volume(&dev_id, pct, true);
        if ok && role != "input" && !dev_name.is_empty() {
            self.inner.win_device_vols.write().insert(dev_name, pct.round());
            Self::schedule_save(&self.inner);
        }
        (ok, self.status_payload())
    }

    pub fn pin_win_vol(&self, role: &str, pin: bool) -> (bool, StatusPayload) {
        if role == "input" {
            return (false, self.status_payload());
        }
        let Some((dev_id, dev_name)) = self.role_device_info(role) else {
            return (false, self.status_payload());
        };
        if dev_name.is_empty() {
            return (false, self.status_payload());
        }
        if pin {
            ensure_com();
            let cur_pct = DeviceEnumerator::new()
                .ok()
                .and_then(|e| e.get_device(&dev_id).ok())
                .and_then(|d| read_device_endpoint_vol(&d, &dev_id, &dev_name))
                .map(|v| v.volume_pct.clamp(0.0, 100.0).round());
            let Some(pct) = cur_pct else {
                return (false, self.status_payload());
            };
            self.inner.win_device_vols.write().insert(dev_name, pct);
        } else {
            self.inner.win_device_vols.write().remove(&dev_name);
        }
        Self::save_config_now(&self.inner);
        (true, self.status_payload())
    }

    pub fn boost_capture(&self) -> (bool, StatusPayload) {
        let input_id = self.inner.selected_ids.read().input_id.clone();
        let ok = set_device_endpoint_volume(&input_id, 100.0, true);
        (ok, self.status_payload())
    }

    pub fn is_running(&self) -> bool {
        self.inner.running.load(Ordering::SeqCst)
    }

    pub fn fast_meters_event(&self) -> FastMetersEvent {
        let m = *self.inner.fast_params.read();
        FastMetersEvent {
            running: self.inner.running.load(Ordering::SeqCst),
            meters: self.inner.meters.read().clone(),
            telemetry: self.inner.telemetry.read().clone(),
            master_vol: m.master_vol,
            master_mute: m.master_mute,
        }
    }

    pub fn tray_summary(&self) -> (String, f64, bool, f64, bool) {
        let m = self.inner.mixer.read();
        (
            m.position_preset.clone(),
            m.master_vol,
            m.master_mute,
            m.intex_delay_ms.max(m.laptop_delay_ms),
            self.inner.running.load(Ordering::SeqCst),
        )
    }

    pub fn status_payload(&self) -> StatusPayload {
        let ids = self.inner.selected_ids.read().clone();
        let (_, default_output_name, default_output_is_virtual) = default_render_endpoint_info();
        StatusPayload {
            running: self.inner.running.load(Ordering::SeqCst),
            mixer: self.inner.mixer.read().clone(),
            meters: self.inner.meters.read().clone(),
            telemetry: self.inner.telemetry.read().clone(),
            selected_names: self.inner.selected_names.read().clone(),
            win_vols: query_win_vols_with_saved(&ids, Some(&self.inner.win_device_vols.read())),
            selected: ids,
            default_output_name,
            default_output_is_virtual,
        }
    }

    pub fn start(&self) {
        Self::start_inner(&self.inner);
    }

    pub fn stop(&self) {
        Self::stop_inner(&self.inner);
    }

    fn restart_inner(inner: &Arc<EngineInner>) {
        Self::stop_inner(inner);
        thread::sleep(Duration::from_millis(60));
        Self::start_inner(inner);
    }

    fn start_inner(inner: &Arc<EngineInner>) {
        if inner.running.swap(true, Ordering::SeqCst) {
            return;
        }
        *inner.timer_guard.lock() = Some(WinTimerGuard::acquire());
        inner.stop_flag.store(false, Ordering::SeqCst);
        inner.ring.lock().reset_preseed();

        let ids = inner.selected_ids.read().clone();
        let mut handles = Vec::new();

        // 1. Capture + Test Signal Pump Thread
        let pump_inner = inner.clone();
        let in_id = ids.input_id.clone();
        if let Ok(h) = thread::Builder::new()
            .name("wasapi-capture".to_string())
            .spawn(move || run_capture_pump(pump_inner, in_id))
        {
            handles.push(h);
        }

        // 2. Keyboard Vol/Mute Key Bridge Thread
        let key_inner = inner.clone();
        let key_in_id = ids.input_id.clone();
        if let Ok(h) = thread::Builder::new()
            .name("win-key-bridge".to_string())
            .spawn(move || run_win_key_bridge(key_inner, key_in_id))
        {
            handles.push(h);
        }

        // 3. Dedicated Output Threads (Primary, Center/Wireless, Built-in)
        let pref = inner.preferred_names.read().clone();
        for (role, dev_id, pref_name) in [
            ("intex", ids.intex_id, pref.intex_name),
            ("boat", ids.boat_id, pref.boat_name),
            ("laptop", ids.laptop_id, pref.laptop_name),
        ] {
            if dev_id.is_empty() && pref_name.is_empty() {
                set_output_status(inner, role, "Disabled".to_string());
                continue;
            }
            let out_inner = inner.clone();
            let role_s = role.to_string();
            if let Ok(h) = thread::Builder::new()
                .name(format!("wasapi-out-{role}"))
                .spawn(move || run_output_loop(out_inner, role_s, dev_id))
            {
                handles.push(h);
            }
        }

        *inner.workers.lock() = handles;
    }

    fn stop_inner(inner: &Arc<EngineInner>) {
        if !inner.running.swap(false, Ordering::SeqCst) {
            return;
        }
        inner.stop_flag.store(true, Ordering::SeqCst);
        let handles: Vec<JoinHandle<()>> = inner.workers.lock().drain(..).collect();
        for h in handles {
            let _ = h.join();
        }
        *inner.timer_guard.lock() = None;
        *inner.meters.write() = Meters::default();
        {
            let mut t = inner.telemetry.write();
            t.input_status = "Stopped".to_string();
            t.intex_status = "Stopped".to_string();
            t.boat_status = "Stopped".to_string();
            t.laptop_status = "Stopped".to_string();
        }
        Self::save_config_now(inner);
    }
}

impl Drop for AudioRouterEngine {
    fn drop(&mut self) {
        self.stop();
    }
}

fn set_output_status(inner: &EngineInner, role: &str, status: String) {
    let mut t = inner.telemetry.write();
    match role {
        "intex" => t.intex_status = status,
        "boat" => t.boat_status = status,
        "laptop" => t.laptop_status = status,
        _ => {}
    }
}

#[inline]
fn set_output_meter(inner: &EngineInner, role: &str, db: f64) {
    // Non-blocking try_write ensures Pro Audio render callbacks never stall behind UI reads
    let Some(mut m) = inner.meters.try_write() else {
        return;
    };
    match role {
        "intex" => m.intex_db = db,
        "boat" => m.boat_db = db,
        "laptop" => m.laptop_db = db,
        _ => {}
    }
}

#[inline]
fn set_output_telemetry(inner: &EngineInner, role: &str, actual_ms: f64, ppm: f64, gr_db: f64) {
    let Some(mut t) = inner.telemetry.try_write() else {
        return;
    };
    match role {
        "intex" => {
            t.intex_actual_ms = actual_ms;
            t.intex_rate_ppm = ppm;
            t.intex_gr_db = gr_db;
        }
        "boat" => {
            t.boat_actual_ms = actual_ms;
            t.boat_rate_ppm = ppm;
            t.boat_gr_db = gr_db;
        }
        "laptop" => {
            t.laptop_actual_ms = actual_ms;
            t.laptop_rate_ppm = ppm;
            t.laptop_gr_db = gr_db;
        }
        _ => {}
    }
}

#[inline]
fn cosine_fade_in(buf: &mut [[f32; 2]], max_len: usize) {
    let n = buf.len().min(max_len);
    if n == 0 {
        return;
    }
    let inv = 1.0 / (n as f32);
    for (i, fr) in buf[..n].iter_mut().enumerate() {
        let g = 0.5 - 0.5 * (std::f32::consts::PI * ((i + 1) as f32 * inv)).cos();
        fr[0] *= g;
        fr[1] *= g;
    }
}

#[inline]
fn cosine_fade_out_tail(buf: &mut [[f32; 2]], max_len: usize) {
    let n = buf.len().min(max_len);
    if n == 0 {
        return;
    }
    let base = buf.len() - n;
    let inv = 1.0 / (n as f32);
    for (i, fr) in buf[base..].iter_mut().enumerate() {
        let g = 0.5 + 0.5 * (std::f32::consts::PI * ((i + 1) as f32 * inv)).cos();
        fr[0] *= g;
        fr[1] *= g;
    }
}

fn decode_bytes_to_stereo_48k(
    raw: &[u8],
    fmt: &WaveFormat,
    scratch_stereo: &mut Vec<[f32; 2]>,
    out_48k: &mut Vec<[f32; 2]>,
) {
    scratch_stereo.clear();
    out_48k.clear();

    let ch = fmt.get_nchannels().max(1) as usize;
    let bps = (fmt.get_bitspersample() / 8).max(1) as usize;
    let frame_bytes = ch * bps;
    let sample_type = fmt.get_subformat().unwrap_or(SampleType::Float);
    let src_rate = fmt.get_samplespersec();

    for frame in raw.chunks_exact(frame_bytes) {
        let read_ch = |c_idx: usize| -> f32 {
            let off = c_idx.min(ch - 1) * bps;
            match (sample_type, bps) {
                (SampleType::Float, 4) => {
                    let v = f32::from_le_bytes(frame[off..off + 4].try_into().unwrap_or([0; 4]));
                    if v.is_finite() {
                        v.clamp(-2.0, 2.0)
                    } else {
                        0.0
                    }
                }
                (SampleType::Int, 2) => {
                    let s = i16::from_le_bytes(frame[off..off + 2].try_into().unwrap_or([0; 2]));
                    s as f32 / 32768.0
                }
                (SampleType::Int, 3) => {
                    let b0 = frame[off] as i32;
                    let b1 = frame[off + 1] as i32;
                    let b2 = frame[off + 2] as i32;
                    let s = ((b0 | (b1 << 8) | (b2 << 16)) << 8) >> 8;
                    (s as f64 / 8388608.0) as f32
                }
                (SampleType::Int, 4) => {
                    let s = i32::from_le_bytes(frame[off..off + 4].try_into().unwrap_or([0; 4]));
                    (s as f64 / 2147483648.0) as f32
                }
                _ => 0.0,
            }
        };
        let l = read_ch(0);
        let r = if ch >= 2 { read_ch(1) } else { l };
        scratch_stereo.push([l, r]);
    }

    if src_rate == MASTER_RATE || scratch_stereo.is_empty() {
        out_48k.extend_from_slice(scratch_stereo);
        return;
    }

    let ratio = MASTER_RATE_F64 / (src_rate as f64);
    let out_len = ((scratch_stereo.len() as f64) * ratio).round().max(1.0) as usize;
    let src_last = (scratch_stereo.len() - 1) as f64;
    for i in 0..out_len {
        let pos = (i as f64 / ratio).clamp(0.0, src_last);
        let i0 = pos.floor() as usize;
        let i1 = (i0 + 1).min(scratch_stereo.len() - 1);
        let frac = (pos - i0 as f64) as f32;
        let a = scratch_stereo[i0];
        let b = scratch_stereo[i1];
        out_48k.push([a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac]);
    }
}

fn open_loopback_or_capture(
    dev: &Device,
) -> Result<(AudioClient, WaveFormat, Option<wasapi::Handle>), String> {
    let mut client = dev.get_iaudioclient().map_err(|e| format!("{e}"))?;
    let (def_period, _min_period) = client.get_device_period().unwrap_or((100_000, 100_000));
    let desired = WaveFormat::new(32, 32, SampleType::Float, MASTER_RATE as usize, 2, None);

    // 1. Try EventsShared + autoconvert at 48 kHz float stereo (80 ms loopback buffer prevents Windows audio drops)
    if client
        .initialize_client(
            &desired,
            Direction::Capture,
            StreamMode::EventsShared {
                autoconvert: true,
                buffer_duration_hns: def_period.max(800_000),
            },
            None,
        )
        .is_ok()
    {
        let h_event = client.set_get_eventhandle().ok();
        return Ok((client, desired, h_event));
    }

    // 2. Fallback: PollingShared in native mixformat (for strict loopback drivers)
    let mut client = dev.get_iaudioclient().map_err(|e| format!("{e}"))?;
    let mix_fmt = client.get_mixformat().map_err(|e| format!("{e}"))?;
    client
        .initialize_client(
            &mix_fmt,
            Direction::Capture,
            StreamMode::PollingShared {
                autoconvert: false,
                buffer_duration_hns: def_period.max(800_000),
            },
            None,
        )
        .map_err(|e| format!("{e}"))?;
    Ok((client, mix_fmt, None))
}

fn run_capture_pump(inner: Arc<EngineInner>, device_id: String) {
    ensure_com();
    let _prio_guard = elevate_audio_thread();
    let mut sig_gen = TestSignalGenerator::new(MASTER_RATE_F64);
    let (cal_chirp, _) = generate_cal_chirp_pair(MASTER_RATE_F64);
    let silence = [[0.0_f32; 2]; PUMP_BLOCK_FRAMES];
    let mut test_buf = [[0.0_f32; 2]; PUMP_BLOCK_FRAMES];
    let mut raw_bytes: Vec<u8> = Vec::with_capacity(48000);
    let mut scratch_stereo: Vec<[f32; 2]> = Vec::with_capacity(2048);
    let mut out_48k: Vec<[f32; 2]> = Vec::with_capacity(4096);

    let set_in_meter = |db: f64| {
        if let Some(mut m) = inner.meters.try_write() {
            m.in_db = db;
        }
    };

    while !inner.stop_flag.load(Ordering::Relaxed) {
        let mode_now = inner.fast_params.read().test_mode;
        let cal_now = inner.cal.active.load(Ordering::Relaxed);
        if device_id.is_empty() || mode_now != TestMode::Off || cal_now {
            inner.telemetry.write().input_status = if cal_now {
                "Auto-Align Calibration".to_string()
            } else if device_id.is_empty() {
                "Test Generator Only".to_string()
            } else {
                "Test Tone Active".to_string()
            };
            let mut next_tick = Instant::now();
            let dt = Duration::from_micros(10_000);
            while !inner.stop_flag.load(Ordering::Relaxed) {
                let m = inner.fast_params.read().test_mode;
                let cal_active = inner.cal.active.load(Ordering::Relaxed);
                if !cal_active && m == TestMode::Off && !device_id.is_empty() {
                    break;
                }
                if cal_active {
                    let slots = [
                        inner.cal.s0.load(Ordering::Relaxed),
                        inner.cal.s1.load(Ordering::Relaxed),
                        inner.cal.s2.load(Ordering::Relaxed),
                        inner.cal.s3.load(Ordering::Relaxed),
                    ];
                    let c_len = cal_chirp.len() as u64;
                    let mut ring = inner.ring.lock();
                    let w0 = ring.write_idx;
                    for (i, out) in test_buf.iter_mut().enumerate() {
                        let idx = w0 + i as u64;
                        let s = slots
                            .into_iter()
                            .find_map(|st| {
                                (idx >= st && idx < st + c_len).then(|| cal_chirp[(idx - st) as usize])
                            })
                            .unwrap_or(0.0);
                        *out = [s, s];
                    }
                    ring.write(&test_buf);
                    drop(ring);
                    set_in_meter(meter_db(&test_buf));
                } else if m == TestMode::Off {
                    inner.ring.lock().write(&silence);
                    set_in_meter(-92.0);
                } else {
                    sig_gen.generate(m, &mut test_buf);
                    inner.ring.lock().write(&test_buf);
                    set_in_meter(meter_db(&test_buf));
                }
                next_tick += dt;
                let now = Instant::now();
                if next_tick > now {
                    thread::sleep(next_tick - now);
                } else if now - next_tick > Duration::from_millis(80) {
                    next_tick = now;
                }
            }
            continue;
        }

        // Open WASAPI capture / loopback stream
        let opened = DeviceEnumerator::new()
            .and_then(|e| e.get_device(&device_id))
            .map_err(|e| format!("{e}"))
            .and_then(|dev| open_loopback_or_capture(&dev).map(|res| (dev, res)));

        match opened {
            Ok((_dev, (client, fmt, h_event))) => {
                let cap_client = match client.get_audiocaptureclient() {
                    Ok(c) => c,
                    Err(e) => {
                        inner.telemetry.write().input_status = format!("Input Error ({e})");
                        thread::sleep(Duration::from_millis(400));
                        continue;
                    }
                };
                if let Err(e) = client.start_stream() {
                    inner.telemetry.write().input_status = format!("Input Error ({e})");
                    thread::sleep(Duration::from_millis(400));
                    continue;
                }

                let cap_rate = fmt.get_samplespersec();
                let frame_bytes =
                    (fmt.get_nchannels().max(1) as usize) * ((fmt.get_bitspersample() / 8).max(1) as usize);
                inner.telemetry.write().input_status = format!("Capturing ({cap_rate} Hz)");
                let mut last_audio_ts = Instant::now();
                let mut silence_pace = Instant::now();
                let mut in_silence_state = false;
                let mut last_cap_frame = [0.0_f32; 2];

                'capture_stream: while !inner.stop_flag.load(Ordering::Relaxed) {
                    if inner.fast_params.read().test_mode != TestMode::Off
                        || inner.cal.active.load(Ordering::Relaxed)
                    {
                        break;
                    }
                    if let Some(ref ev) = h_event {
                        if let Err(e) = ev.wait_for_event(8) {
                            let msg = format!("{e}");
                            if !msg.contains("timed out") && !msg.contains("Timeout") {
                                inner.telemetry.write().input_status =
                                    format!("Reconnecting Input ({msg})");
                                break 'capture_stream;
                            }
                        }
                    } else {
                        thread::sleep(Duration::from_millis(6));
                    }

                    let mut got_frames = false;
                    loop {
                        let pkt = match cap_client.get_next_packet_size() {
                            Ok(p) => p.unwrap_or(0),
                            Err(e) => {
                                inner.telemetry.write().input_status =
                                    format!("Reconnecting Input ({e})");
                                break 'capture_stream;
                            }
                        };
                        if pkt == 0 {
                            break;
                        }
                        let needed = (pkt as usize) * frame_bytes;
                        if raw_bytes.len() < needed {
                            raw_bytes.resize(needed, 0);
                        }
                        match cap_client.read_from_device(&mut raw_bytes[..needed]) {
                            Ok((read_n, info)) if read_n > 0 => {
                                got_frames = true;
                                last_audio_ts = Instant::now();
                                if info.flags.silent {
                                    let count = (read_n as usize).min(RING_FRAMES / 2);
                                    out_48k.clear();
                                    out_48k.resize(count, [0.0, 0.0]);
                                    if !in_silence_state {
                                        in_silence_state = true;
                                        if last_cap_frame[0] != 0.0 || last_cap_frame[1] != 0.0 {
                                            let fade_n = count.min(32);
                                            out_48k[..fade_n].fill(last_cap_frame);
                                            cosine_fade_out_tail(&mut out_48k[..fade_n], fade_n);
                                            last_cap_frame = [0.0, 0.0];
                                        }
                                    }
                                    inner.ring.lock().write(&out_48k);
                                    set_in_meter(-92.0);
                                } else {
                                    let bytes_read = (read_n as usize) * frame_bytes;
                                    decode_bytes_to_stereo_48k(
                                        &raw_bytes[..bytes_read],
                                        &fmt,
                                        &mut scratch_stereo,
                                        &mut out_48k,
                                    );
                                    if !out_48k.is_empty() {
                                        if info.flags.data_discontinuity || in_silence_state {
                                            let fade_n =
                                                if info.flags.data_discontinuity { 64 } else { 32 };
                                            cosine_fade_in(&mut out_48k, fade_n);
                                            in_silence_state = false;
                                        }
                                        last_cap_frame = *out_48k.last().unwrap_or(&[0.0, 0.0]);
                                        inner.ring.lock().write(&out_48k);
                                        set_in_meter(meter_db(&out_48k));
                                    }
                                }
                            }
                            Ok(_) => break,
                            Err(e) => {
                                inner.telemetry.write().input_status =
                                    format!("Reconnecting Input ({e})");
                                break 'capture_stream;
                            }
                        }
                    }

                    // Only enter idle silence after >50 ms without loopback packets so Windows scheduling jitter never punches holes mid-song
                    if !got_frames && last_audio_ts.elapsed() > Duration::from_millis(50) {
                        let now = Instant::now();
                        if !in_silence_state {
                            in_silence_state = true;
                            silence_pace = now;
                            let mut tail = silence;
                            if last_cap_frame[0] != 0.0 || last_cap_frame[1] != 0.0 {
                                tail[..32].fill(last_cap_frame);
                                cosine_fade_out_tail(&mut tail[..32], 32);
                                last_cap_frame = [0.0, 0.0];
                            }
                            inner.ring.lock().write(&tail);
                            set_in_meter(-92.0);
                        } else {
                            if now.duration_since(silence_pace) > Duration::from_millis(120) {
                                silence_pace = now - Duration::from_millis(40);
                            }
                            let mut wrote = 0;
                            while now.duration_since(silence_pace) >= Duration::from_millis(10)
                                && wrote < 6
                            {
                                silence_pace += Duration::from_millis(10);
                                inner.ring.lock().write(&silence);
                                wrote += 1;
                            }
                            if wrote > 0 {
                                set_in_meter(-92.0);
                            }
                        }
                    } else if got_frames {
                        silence_pace = Instant::now();
                    }
                }
                let _ = client.stop_stream();
                // Keep output ring padded with silence during input reconnect backoff
                for _ in 0..25 {
                    if inner.stop_flag.load(Ordering::Relaxed)
                        || inner.fast_params.read().test_mode != TestMode::Off
                        || inner.cal.active.load(Ordering::Relaxed)
                    {
                        break;
                    }
                    inner.ring.lock().write(&silence);
                    set_in_meter(-92.0);
                    thread::sleep(Duration::from_millis(10));
                }
            }
            Err(err) => {
                inner.telemetry.write().input_status = format!("Input Error ({err})");
                for _ in 0..40 {
                    if inner.stop_flag.load(Ordering::Relaxed)
                        || inner.fast_params.read().test_mode != TestMode::Off
                        || inner.cal.active.load(Ordering::Relaxed)
                    {
                        break;
                    }
                    inner.ring.lock().write(&silence);
                    set_in_meter(-92.0);
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }
}

fn is_wireless_device_name(name: &str) -> bool {
    let nl = name.to_lowercase();
    [
        "bt", "bluetooth", "stone", "anc", "earbuds", "soundcore", "jbl", "sony", "bose",
        "marshall", "airpods", "sennheiser", "jabra", "beats",
    ]
    .iter()
    .any(|k| nl.contains(k))
}

fn open_render_client(dev: &Device) -> Result<(AudioClient, WaveFormat, wasapi::Handle, bool), String> {
    ensure_session_unity(dev);
    let mut client = dev.get_iaudioclient().map_err(|e| format!("{e}"))?;
    let (def_period, _min_period) = client.get_device_period().unwrap_or((100_000, 100_000));
    let is_bt = dev
        .get_friendlyname()
        .map(|n| is_wireless_device_name(&n))
        .unwrap_or(false);
    // Give Bluetooth 70 ms and wired/built-in speakers 35 ms shared engine buffer slack to stop crackles
    let min_hns = if is_bt { 700_000 } else { 350_000 };
    let desired = WaveFormat::new(32, 32, SampleType::Float, MASTER_RATE as usize, 2, None);

    // Prefer EventsShared with autoconvert so hardware (including 44.1kHz Bluetooth) runs smoothly
    if client
        .initialize_client(
            &desired,
            Direction::Render,
            StreamMode::EventsShared {
                autoconvert: true,
                buffer_duration_hns: def_period.max(min_hns),
            },
            None,
        )
        .is_ok()
    {
        let ev = client.set_get_eventhandle().map_err(|e| format!("{e}"))?;
        return Ok((client, desired, ev, is_bt));
    }

    // Fallback: device mixformat directly with generous shared buffer slack
    let mut client = dev.get_iaudioclient().map_err(|e| format!("{e}"))?;
    let mix = client.get_mixformat().map_err(|e| format!("{e}"))?;
    let fmt = WaveFormat::new(
        32,
        32,
        SampleType::Float,
        mix.get_samplespersec() as usize,
        2,
        None,
    );
    client
        .initialize_client(
            &fmt,
            Direction::Render,
            StreamMode::EventsShared {
                autoconvert: true,
                buffer_duration_hns: def_period.max(min_hns.max(300_000)),
            },
            None,
        )
        .map_err(|e| format!("{e}"))?;
    let ev = client.set_get_eventhandle().map_err(|e| format!("{e}"))?;
    Ok((client, fmt, ev, is_bt))
}

fn frames_to_le_bytes(frames: &[[f32; 2]], byte_buf: &mut Vec<u8>) {
    byte_buf.clear();
    byte_buf.reserve(frames.len() * 8);
    for f in frames {
        let l = if f[0].is_finite() {
            f[0].clamp(-0.9999, 0.9999)
        } else {
            0.0
        };
        let r = if f[1].is_finite() {
            f[1].clamp(-0.9999, 0.9999)
        } else {
            0.0
        };
        byte_buf.extend_from_slice(&l.to_le_bytes());
        byte_buf.extend_from_slice(&r.to_le_bytes());
    }
}

fn run_output_loop(inner: Arc<EngineInner>, role: String, initial_id: String) {
    ensure_com();
    let _prio_guard = elevate_audio_thread();
    let mut dev_id = initial_id;
    let role_idx: u8 = match role.as_str() {
        "intex" => 1,
        "boat" => 2,
        _ => 3,
    };
    let mut block_buf: Vec<[f32; 2]> = Vec::with_capacity(9600);
    block_buf.resize(2048, [0.0, 0.0]);
    let mut byte_buf: Vec<u8> = Vec::with_capacity(9600 * 8);

    while !inner.stop_flag.load(Ordering::Relaxed) {
        let opened = if dev_id.is_empty() {
            Err("Waiting for device".to_string())
        } else {
            DeviceEnumerator::new()
                .and_then(|e| e.get_device(&dev_id))
                .map_err(|e| format!("{e}"))
                .and_then(|dev| open_render_client(&dev))
        };

        match opened {
            Ok((client, fmt, h_event, is_wireless_out)) => {
                let out_rate = fmt.get_samplespersec();
                let render_client = match client.get_audiorenderclient() {
                    Ok(rc) => rc,
                    Err(e) => {
                        set_output_status(&inner, &role, format!("Reconnecting ({e})"));
                        thread::sleep(Duration::from_millis(800));
                        continue;
                    }
                };

                // Prime buffer cleanly with silence before start_stream per WASAPI guideline
                if let Ok(avail) = client.get_available_space_in_frames() {
                    let prime_n = (avail as usize).min(4800);
                    block_buf.resize(prime_n, [0.0, 0.0]);
                    block_buf.fill([0.0, 0.0]);
                    frames_to_le_bytes(&block_buf, &mut byte_buf);
                    let _ = render_client.write_to_device(&byte_buf, None);
                }

                if let Err(e) = client.start_stream() {
                    set_output_status(&inner, &role, format!("Reconnecting ({e})"));
                    thread::sleep(Duration::from_millis(800));
                    continue;
                }

                set_output_status(&inner, &role, format!("Playing ({out_rate} Hz)"));

                let mut primary_dsp = PrimaryProcessor::new(out_rate as f64);
                let mut aux_dsp = AuxChannelProcessor::new(out_rate as f64);
                primary_dsp.reset();
                aux_dsp.reset();

                let init_delay = {
                    let p = *inner.fast_params.read();
                    match role_idx {
                        1 => p.intex_delay_ms,
                        2 => p.boat_delay_ms,
                        _ => p.laptop_delay_ms,
                    }
                };
                let mut prev_delay_ms = init_delay;
                let w0 = inner.ring.lock().write_idx;
                let mut read_pos = (w0 as f64
                    - (BASE_BUFFER_MS + init_delay) * MASTER_RATE_F64 / 1000.0)
                    .max(0.0);
                let mut smoothed_diff = 0.0_f64;
                let mut do_resync_fade = true;
                let mut last_out = [0.0_f32; 2];

                while !inner.stop_flag.load(Ordering::Relaxed) {
                    if h_event.wait_for_event(500).is_err() {
                        if inner.stop_flag.load(Ordering::Relaxed) {
                            break;
                        }
                        break;
                    }
                    let avail = match client.get_available_space_in_frames() {
                        Ok(n) if n > 0 => n as usize,
                        Ok(_) => continue,
                        Err(_) => break,
                    };

                    let params = *inner.fast_params.read();
                    let cal_now = inner.cal.active.load(Ordering::Relaxed);
                    let delay_ms = if cal_now {
                        0.0
                    } else {
                        match role_idx {
                            1 => params.intex_delay_ms,
                            2 => params.boat_delay_ms,
                            _ => params.laptop_delay_ms,
                        }
                    };

                    if block_buf.len() != avail {
                        block_buf.resize(avail, [0.0, 0.0]);
                    }

                    let w_now = {
                        let ring = inner.ring.lock();
                        let w = ring.write_idx;
                        let min_valid = w.saturating_sub((RING_FRAMES - 4096) as u64) as f64;
                        let target_lag = (BASE_BUFFER_MS + delay_ms) * MASTER_RATE_F64 / 1000.0;
                        let desired_pos = (w as f64 - target_lag).max(min_valid);
                        let mut diff = desired_pos - read_pos;

                        let user_delay_changed = (delay_ms - prev_delay_ms).abs() > 0.5;
                        if user_delay_changed {
                            prev_delay_ms = delay_ms;
                        }
                        let step_est = MASTER_RATE_F64 / (out_rate as f64);
                        let block_need = (avail as f64) * step_est + 32.0;
                        let overflow = read_pos < min_valid;
                        let underrun = read_pos + block_need >= w as f64;
                        let excess_lag = diff > MASTER_RATE_F64 * 0.22
                            || (user_delay_changed && diff.abs() > 240.0);

                        let mut underrun_valid_frames: Option<usize> = None;
                        if overflow || excess_lag {
                            read_pos = desired_pos.min(w as f64 - block_need).max(min_valid);
                            smoothed_diff = 0.0;
                            do_resync_fade = true;
                            diff = 0.0;
                        } else if underrun {
                            // Never rewind backwards over already-played samples: consume unread tail smoothly
                            let unread = ((w as f64 - 2.0 - read_pos) / step_est)
                                .floor()
                                .max(0.0) as usize;
                            underrun_valid_frames = Some(unread.min(avail));
                            do_resync_fade = true;
                            smoothed_diff = 0.0;
                            diff = 0.0;
                        }

                        // Time-normalized EMA (tau = 3.0 s), 1.5 ms phase-lock deadband & gentle ±400 ppm slew
                        let block_dt = (avail as f64) / (out_rate as f64).max(8000.0);
                        let ema_alpha = (1.0 - (-block_dt / 3.0).exp()).clamp(0.001, 0.15);
                        smoothed_diff = (1.0 - ema_alpha) * smoothed_diff + ema_alpha * diff;
                        let rate_adj = if smoothed_diff.abs() < 72.0 {
                            1.0
                        } else {
                            1.0 + (smoothed_diff / (MASTER_RATE_F64 * 18.0))
                                .clamp(-0.0004, 0.0004)
                        };
                        let step = step_est * rate_adj;
                        let read_start = read_pos;
                        if let Some(valid_n) = underrun_valid_frames {
                            block_buf.fill([0.0, 0.0]);
                            if valid_n > 0 {
                                ring.read_interpolated(
                                    read_pos,
                                    &mut block_buf[..valid_n],
                                    step_est,
                                );
                                cosine_fade_out_tail(&mut block_buf[..valid_n], 32);
                                read_pos = (read_pos + (valid_n as f64) * step_est)
                                    .min((w as f64 - 2.0).max(min_valid));
                            }
                        } else {
                            ring.read_interpolated(read_pos, &mut block_buf, step);
                            read_pos += (avail as f64) * step;
                        }
                        (w, rate_adj, read_start, step)
                    };

                    let (w_after, rate_adj, read_start, step) = w_now;

                    let master_vol = if params.protection_enabled {
                        params.master_vol.min(params.protection_max_master)
                    } else {
                        params.master_vol
                    };
                    let solo_blocked = params.solo_role != 0 && params.solo_role != role_idx;

                    let gr_db = if cal_now {
                        let bt_role = inner.cal.bt_role.load(Ordering::Relaxed);
                        let wired_role = inner.cal.wired_role.load(Ordering::Relaxed);
                        if role_idx != bt_role && role_idx != wired_role {
                            block_buf.fill([0.0, 0.0]);
                        } else {
                            let wins = if role_idx == bt_role {
                                [
                                    (inner.cal.s0.load(Ordering::Relaxed), &inner.cal.bt1_lag),
                                    (inner.cal.s2.load(Ordering::Relaxed), &inner.cal.bt2_lag),
                                ]
                            } else {
                                [
                                    (inner.cal.s1.load(Ordering::Relaxed), &inner.cal.aux1_lag),
                                    (inner.cal.s3.load(Ordering::Relaxed), &inner.cal.aux2_lag),
                                ]
                            };
                            let read_end = read_start + (avail as f64) * step;
                            let matched = wins.into_iter().find(|(ws, _)| {
                                read_end > *ws as f64 && read_start < (*ws + 4800) as f64
                            });
                            if let Some((s_win_start, slot)) = matched {
                                let s_win_end = s_win_start + 4800;
                                if read_start <= s_win_start as f64
                                    && read_end > s_win_start as f64
                                {
                                    let lag = w_after
                                        .saturating_sub(read_start.max(0.0).round() as u64)
                                        .max(1);
                                    let _ = slot.compare_exchange(
                                        0,
                                        lag,
                                        Ordering::SeqCst,
                                        Ordering::Relaxed,
                                    );
                                }
                                for (idx, frame) in block_buf.iter_mut().enumerate() {
                                    let pos = read_start + (idx as f64) * step;
                                    if pos < s_win_start as f64 || pos >= s_win_end as f64 {
                                        *frame = [0.0, 0.0];
                                    } else {
                                        frame[0] = (frame[0] * 0.65).clamp(-0.5, 0.5);
                                        frame[1] = (frame[1] * 0.65).clamp(-0.5, 0.5);
                                    }
                                }
                            } else {
                                block_buf.fill([0.0, 0.0]);
                            }
                        }
                        0.0
                    } else if role_idx == 1 {
                        let p = PrimaryParams {
                            master_vol,
                            vol: params.intex_vol,
                            is_wireless: is_wireless_out,
                            width_pct: params.intex_width,
                            balance: params.intex_balance,
                            bass_db: params.intex_bass_db,
                            mid_db: params.intex_mid_db,
                            treble_db: params.intex_treble_db,
                            mute: params.master_mute || params.intex_mute || solo_blocked,
                            protection_enabled: params.protection_enabled,
                            protection_ceiling_db: params.protection_ceiling_db,
                        };
                        primary_dsp.process_in_place(&mut block_buf, &p);
                        primary_dsp.guard.last_gr_db
                    } else if role_idx == 2 {
                        let p = AuxParams {
                            master_vol,
                            vol: params.boat_vol,
                            is_wireless: is_wireless_out,
                            mode: params.boat_mode,
                            hpf_hz: params.boat_hpf_hz,
                            lpf_hz: params.boat_lpf_hz,
                            bass_db: params.boat_bass_db,
                            mid_db: params.boat_mid_db,
                            treble_db: params.boat_treble_db,
                            invert: params.boat_invert,
                            mute: params.master_mute || params.boat_mute || solo_blocked,
                            protection_enabled: params.protection_enabled,
                            protection_ceiling_db: params.protection_ceiling_db,
                        };
                        aux_dsp.process_in_place(&mut block_buf, &p);
                        aux_dsp.guard.last_gr_db
                    } else {
                        let p = AuxParams {
                            master_vol,
                            vol: params.laptop_vol,
                            is_wireless: is_wireless_out,
                            mode: params.laptop_mode,
                            hpf_hz: params.laptop_hpf_hz,
                            lpf_hz: params.laptop_lpf_hz,
                            bass_db: params.laptop_bass_db,
                            mid_db: params.laptop_mid_db,
                            treble_db: params.laptop_treble_db,
                            invert: params.laptop_invert,
                            mute: params.master_mute || params.laptop_mute || solo_blocked,
                            protection_enabled: params.protection_enabled,
                            protection_ceiling_db: params.protection_ceiling_db,
                        };
                        aux_dsp.process_in_place(&mut block_buf, &p);
                        aux_dsp.guard.last_gr_db
                    };

                    if do_resync_fade && !block_buf.is_empty() {
                        // Click-free 2 ms raised-cosine crossfade from previous block tail into resynced audio
                        let x_len = avail.min(96);
                        let inv_x = 1.0 / (x_len as f32);
                        for (idx, frame) in block_buf[..x_len].iter_mut().enumerate() {
                            let t = (idx + 1) as f32 * inv_x;
                            let g = 0.5 - 0.5 * (std::f32::consts::PI * t).cos();
                            frame[0] = last_out[0] * (1.0 - g) + frame[0] * g;
                            frame[1] = last_out[1] * (1.0 - g) + frame[1] * g;
                        }
                        do_resync_fade = false;
                    }
                    if let Some(&tail) = block_buf.last() {
                        last_out = tail;
                    }

                    frames_to_le_bytes(&block_buf, &mut byte_buf);
                    if render_client.write_to_device(&byte_buf, None).is_err() {
                        break;
                    }

                    let db = meter_db(&block_buf);
                    set_output_meter(&inner, &role, db);
                    let actual_ms = ((w_after as f64 - read_start) * 1000.0 / MASTER_RATE_F64
                        - BASE_BUFFER_MS)
                        .max(0.0);
                    set_output_telemetry(
                        &inner,
                        &role,
                        actual_ms,
                        (rate_adj - 1.0) * 1_000_000.0,
                        gr_db,
                    );
                }

                let _ = client.stop_stream();
            }
            Err(e) => {
                let waiting_offline = dev_id.is_empty();
                set_output_status(
                    &inner,
                    &role,
                    if waiting_offline {
                        "Waiting for device".to_string()
                    } else {
                        format!("Reconnecting ({e})")
                    },
                );
                set_output_meter(&inner, &role, -92.0);
                // Interruptible sleep before retrying device ID / re-resolving Bluetooth name
                let sleep_ticks = if waiting_offline { 15 } else { 8 };
                for _ in 0..sleep_ticks {
                    if inner.stop_flag.load(Ordering::Relaxed) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                let saved_name = {
                    let n = inner.preferred_names.read();
                    match role.as_str() {
                        "intex" => n.intex_name.clone(),
                        "boat" => n.boat_name.clone(),
                        _ => n.laptop_name.clone(),
                    }
                };
                if !saved_name.is_empty() {
                    let (_, outs, _) = list_wasapi_devices();
                    if let Some(found) = outs.iter().find(|d| d.name == saved_name) {
                        apply_saved_win_vol_for(&inner, &found.id, &found.name);
                        dev_id = found.id.clone();
                        let mut ids = inner.selected_ids.write();
                        let mut names = inner.selected_names.write();
                        match role.as_str() {
                            "intex" => {
                                ids.intex_id = found.id.clone();
                                names.intex_name = found.name.clone();
                            }
                            "boat" => {
                                ids.boat_id = found.id.clone();
                                names.boat_name = found.name.clone();
                            }
                            _ => {
                                ids.laptop_id = found.id.clone();
                                names.laptop_name = found.name.clone();
                            }
                        }
                    }
                }
            }
        }
    }
}

fn apply_win_key_delta(m: &mut MixerState, scalar: f32, unity_scalar: f32) -> bool {
    let diff = (scalar - unity_scalar) as f64;
    if diff.abs() < 0.012 {
        return false;
    }
    let steps = (diff / 0.02).round();
    let delta = if steps.abs() <= 3.0 {
        if m.master_vol < 14.0 || (steps < 0.0 && m.master_vol <= 14.0) {
            steps * 1.0
        } else {
            steps * 2.0
        }
    } else {
        diff * 100.0
    };
    let cap = if m.protection_enabled {
        m.protection_max_master
    } else {
        200.0
    };
    m.master_vol = (m.master_vol + delta).clamp(0.0, cap).round();
    true
}

fn run_win_key_bridge(inner: Arc<EngineInner>, input_id: String) {
    if input_id.is_empty() {
        return;
    }
    let in_name = inner.selected_names.read().input_name.to_lowercase();
    if !["steam streaming", "cable", "voicemeeter"]
        .iter()
        .any(|tag| in_name.contains(tag))
    {
        return;
    }
    ensure_com();
    const UNITY_SCALAR: f32 = 0.98;

    let activate_ep = || -> Option<IAudioEndpointVolume> {
        let enumerator = DeviceEnumerator::new().ok()?;
        let dev = enumerator.get_device(&input_id).ok()?;
        let ep: IAudioEndpointVolume = unsafe {
            dev.as_immdevice()
                .Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
                .ok()?
        };
        unsafe {
            let _ = ep.SetMute(false, std::ptr::null());
            let _ = ep.SetMasterVolumeLevelScalar(UNITY_SCALAR, std::ptr::null());
        }
        Some(ep)
    };

    let Some(mut ep) = activate_ep() else {
        return;
    };
    let mut com_err_streak: u32 = 0;

    while !inner.stop_flag.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(20));
        if inner.stop_flag.load(Ordering::Relaxed) {
            break;
        }
        let muted = unsafe { ep.GetMute().map(|b| b.as_bool()).unwrap_or(false) };
        if muted {
            {
                let mut m = inner.mixer.write();
                m.master_mute = !m.master_mute;
                *inner.fast_params.write() = FastAudioParams::from_mixer(&m);
            }
            AudioRouterEngine::save_config_now(&inner);
            unsafe {
                let _ = ep.SetMute(false, std::ptr::null());
            }
        }
        match unsafe { ep.GetMasterVolumeLevelScalar() } {
            Ok(scalar) => {
                com_err_streak = 0;
                let changed = {
                    let mut m = inner.mixer.write();
                    let ok = apply_win_key_delta(&mut m, scalar, UNITY_SCALAR);
                    if ok {
                        *inner.fast_params.write() = FastAudioParams::from_mixer(&m);
                    }
                    ok
                };
                if changed {
                    AudioRouterEngine::schedule_save(&inner);
                    unsafe {
                        let _ = ep.SetMasterVolumeLevelScalar(UNITY_SCALAR, std::ptr::null());
                    }
                }
            }
            Err(_) => {
                com_err_streak = com_err_streak.saturating_add(1);
                if com_err_streak >= 5 {
                    com_err_streak = 0;
                    thread::sleep(Duration::from_millis(400));
                    if inner.stop_flag.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Some(new_ep) = activate_ep() {
                        ep = new_ep;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn mixer_patch_clamps_bounds_and_enforces_protection_cap() {
        let mut m = MixerState::default();
        assert_eq!(m.protection_max_master, 60.0);
        assert_eq!(m.protection_ceiling_db, -3.0);
        assert_eq!(m.intex_delay_ms, 0.0);
        assert_eq!(m.laptop_delay_ms, 0.0);

        m.apply_patch(&MixerPatch {
            master_vol: Some(999.0),
            protection_enabled: Some(true),
            protection_max_master: Some(45.0),
            protection_ceiling_db: Some(-99.0),
            intex_delay_ms: Some(-20.0),
            boat_bass_db: Some(40.0),
            ..Default::default()
        });
        assert_eq!(m.master_vol, 45.0);
        assert_eq!(m.protection_ceiling_db, -30.0);
        assert_eq!(m.intex_delay_ms, 0.0);
        assert_eq!(m.boat_bass_db, 12.0);
    }

    #[test]
    fn preset_switch_restores_daytime_and_night_ceilings() {
        let mut m = MixerState::default();
        let night = preset_patch("night_safe").expect("night_safe preset missing");
        m.apply_patch(&night);
        assert_eq!(m.protection_ceiling_db, -15.0);
        assert_eq!(m.position_preset, "night_safe");

        let music = preset_patch("wide_room").expect("wide_room preset missing");
        m.apply_patch(&music);
        assert_eq!(m.protection_ceiling_db, -3.0);
        assert_eq!(m.position_preset, "wide_room");
        assert_eq!(m.intex_vol, 115.0);
        assert_eq!(m.laptop_hpf_hz, 190.0);
        assert_eq!(m.laptop_treble_db, 2.5);
    }

    #[test]
    fn legacy_duplicate_config_migrates_bt_to_primary_42() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let cfg = std::env::temp_dir().join(format!("phasemix_mig_test_{stamp}.json"));
        fs::write(
            &cfg,
            r#"{
              "mixer": {"master_vol": 20, "intex_delay_ms": 224, "laptop_delay_ms": 224},
              "devices": {
                "intex_name": "Speakers (Realtek(R) Audio)",
                "boat_name": "Headphones (3.0 BT)",
                "laptop_name": "Speakers (Realtek(R) Audio)"
              }
            }"#,
        )
        .unwrap();

        let eng = AudioRouterEngine::new(cfg.clone());
        let pref = eng.inner.preferred_names.read().clone();
        assert_eq!(pref.intex_name, "Headphones (3.0 BT)");
        assert_eq!(pref.boat_name, "");
        assert_eq!(pref.laptop_name, "Speakers (Realtek(R) Audio)");
        let st = eng.status_payload();
        assert_eq!(st.mixer.intex_delay_ms, 0.0);
        assert_eq!(st.mixer.laptop_delay_ms, 224.0);

        let _ = fs::remove_file(&cfg);
        let _ = fs::remove_file(AudioRouterEngine::backup_path(&cfg));
    }

    #[test]
    fn shared_ring_wraparound_fast_slice_and_interp() {
        let mut ring = SharedRing::new();
        ring.write_idx = (RING_FRAMES - 10) as u64;
        let payload = [[0.5_f32, -0.5_f32]; 25];
        ring.write(&payload);

        let mut out = [[0.0_f32; 2]; 25];
        ring.read_interpolated((RING_FRAMES - 10) as f64, &mut out, 1.0);
        assert!((out[0][0] - 0.5).abs() < 1e-6);
        assert!((out[24][1] + 0.5).abs() < 1e-6);

        // Resampled 44.1 kHz ratio (1.088435) uses Catmull-Rom cleanly without NaNs or wrap artifacts
        let mut cubic_out = [[0.0_f32; 2]; 12];
        ring.read_interpolated((RING_FRAMES - 8) as f64, &mut cubic_out, 48_000.0 / 44_100.0);
        assert!(cubic_out.iter().all(|f| f[0].is_finite() && (f[0] - 0.5).abs() < 0.05));

        // Stale read position older than ring history must output silence rather than overwritten audio
        ring.write_idx = (RING_FRAMES as u64) * 3;
        let mut stale_out = [[0.9_f32; 2]; 8];
        ring.read_interpolated(100.0, &mut stale_out, 1.0);
        assert!(stale_out.iter().all(|f| f[0] == 0.0 && f[1] == 0.0));
    }

    #[test]
    fn wasapi_enumeration_and_muted_temp_engine_cycle() {
        let (inputs, outputs, _) = list_wasapi_devices();
        assert!(!inputs.is_empty() && !outputs.is_empty());

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let cfg = std::env::temp_dir().join(format!("phasemix_rust_test_{stamp}.json"));
        fs::write(
            &cfg,
            r#"{"mixer":{"master_vol":0,"master_mute":true,"protection_enabled":true,"protection_max_master":60}}"#,
        )
        .unwrap();

        let eng = AudioRouterEngine::new(cfg.clone());
        eng.start();
        thread::sleep(Duration::from_millis(800));
        assert!(eng.is_running());
        let st = eng.status_payload();
        let (_, outs, _) = list_wasapi_devices();
        println!(
            "OUTS={:?} SELECTED={:?} STATUS=({:?}, {:?}, {:?}, {:?})",
            outs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            st.selected_names,
            st.telemetry.input_status,
            st.telemetry.intex_status,
            st.telemetry.boat_status,
            st.telemetry.laptop_status
        );
        assert!(
            !st.telemetry.input_status.starts_with("Input Error"),
            "input failed: {}",
            st.telemetry.input_status
        );

        let updated = eng.update_mixer(MixerPatch {
            intex_vol: Some(115.0),
            master_mute: Some(true),
            master_vol: Some(0.0),
            ..Default::default()
        });
        assert_eq!(updated.intex_vol, 115.0);
        assert_eq!(updated.master_vol, 0.0);
        eng.stop();
        assert!(!eng.is_running());

        let saved = fs::read_to_string(&cfg).unwrap();
        assert!(saved.contains("115"));
        let _ = fs::remove_file(&cfg);
        let _ = fs::remove_file(AudioRouterEngine::backup_path(&cfg));
    }

    #[test]
    fn config_backup_recovers_from_corrupt_primary() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let cfg = std::env::temp_dir().join(format!("phasemix_bak_test_{stamp}.json"));
        let bak = AudioRouterEngine::backup_path(&cfg);

        fs::write(&cfg, "{corrupt json").unwrap();
        fs::write(
            &bak,
            r#"{"mixer":{"master_vol":42,"position_preset":"wide_room"},"devices":{"boat_name":"Saved Boat Test"}}"#,
        )
        .unwrap();

        let eng = AudioRouterEngine::new(cfg.clone());
        let st = eng.status_payload();
        assert_eq!(st.mixer.master_vol, 42.0);
        assert_eq!(st.mixer.position_preset, "wide_room");
        assert_eq!(
            eng.inner.preferred_names.read().boat_name,
            "Saved Boat Test"
        );

        let _ = fs::remove_file(&cfg);
        let _ = fs::remove_file(&bak);
    }

    #[test]
    fn preferred_device_name_survives_offline_refresh_and_save() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let cfg = std::env::temp_dir().join(format!("phasemix_pref_test_{stamp}.json"));
        let bak = AudioRouterEngine::backup_path(&cfg);

        fs::write(
            &cfg,
            r#"{"mixer":{"master_vol":25},"devices":{"boat_name":"Offline BT Speaker 999"}}"#,
        )
        .unwrap();

        let eng = AudioRouterEngine::new(cfg.clone());
        assert_eq!(eng.inner.selected_ids.read().boat_id, "");
        assert_eq!(
            eng.inner.preferred_names.read().boat_name,
            "Offline BT Speaker 999"
        );

        let _ = eng.refresh_devices();
        let saved = fs::read_to_string(&cfg).unwrap();
        assert!(saved.contains("Offline BT Speaker 999"));

        let _ = fs::remove_file(&cfg);
        let _ = fs::remove_file(&bak);
    }

    #[test]
    fn night_guard_toggle_and_smart_cycle_restore_previous_preset() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let cfg = std::env::temp_dir().join(format!("phasemix_hk_test_{stamp}.json"));
        let eng = AudioRouterEngine::new(cfg.clone());

        let wide = eng.apply_preset("wide_room").unwrap();
        assert_eq!(wide.position_preset, "wide_room");

        let night = eng.toggle_night_guard();
        assert_eq!(night.position_preset, "night_safe");
        assert_eq!(night.protection_ceiling_db, -15.0);

        let back = eng.toggle_night_guard();
        assert_eq!(back.position_preset, "wide_room");

        let cycled = eng.cycle_smart_preset();
        assert_eq!(cycled.position_preset, "cinema_center");

        let _ = fs::remove_file(&cfg);
        let _ = fs::remove_file(AudioRouterEngine::backup_path(&cfg));
    }

    #[test]
    fn export_and_import_profile_roundtrip_enforces_safety_caps() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let cfg = std::env::temp_dir().join(format!("phasemix_io_test_{stamp}.json"));
        let eng = AudioRouterEngine::new(cfg.clone());

        eng.update_mixer(MixerPatch {
            intex_delay_ms: Some(218.0),
            laptop_delay_ms: Some(218.0),
            master_vol: Some(36.0),
            ..Default::default()
        });
        let raw = eng.export_profile_json().unwrap();
        assert!(raw.contains("218"));

        let imported = eng
            .import_profile_json(
                r#"{"mixer":{"master_vol":185,"protection_enabled":true,"protection_max_master":55,"master_mute":true,"intex_delay_ms":210,"position_preset":"cinema_center"},"devices":{}}"#,
            )
            .unwrap();
        assert_eq!(imported.mixer.master_vol, 55.0);
        assert!(!imported.mixer.master_mute);
        assert_eq!(imported.mixer.intex_delay_ms, 210.0);

        let factory = eng.reset_factory();
        assert_eq!(factory.mixer.master_vol, 25.0);
        assert_eq!(factory.mixer.intex_delay_ms, 0.0);
        assert_eq!(factory.mixer.position_preset, "desk_triangle");
        assert_eq!(factory.mixer.calibrated_delay_ms, None);

        let _ = fs::remove_file(&cfg);
        let _ = fs::remove_file(AudioRouterEngine::backup_path(&cfg));
    }

    #[test]
    fn preset_switch_preserves_calibrated_delay() {
        let mut m = MixerState::default();
        m.intex_delay_ms = 236.0;
        m.laptop_delay_ms = 236.0;
        m.calibrated_delay_ms = Some(236.0);

        for id in [
            "wide_room",
            "cinema_center",
            "night_safe",
            "asym_desk",
            "pure_mirror",
            "desk_triangle",
        ] {
            let p = preset_patch(id).unwrap();
            m.apply_patch(&p);
            assert_eq!(m.intex_delay_ms, 236.0, "preset {id} overwrote intex_delay");
            assert_eq!(m.laptop_delay_ms, 236.0, "preset {id} overwrote laptop_delay");
            assert_eq!(m.calibrated_delay_ms, Some(236.0));
        }
    }

    #[test]
    fn win_key_delta_and_fast_params_sync_consistently() {
        let mut m = MixerState::default();
        assert!(!apply_win_key_delta(&mut m, 0.985, 0.98));
        assert_eq!(m.master_vol, 25.0);

        // Above 14% uses Windows native 2% step
        assert!(apply_win_key_delta(&mut m, 1.00, 0.98));
        assert_eq!(m.master_vol, 27.0);

        // Low-end (< 14%) uses 1% micro-steps for smooth night control
        m.master_vol = 0.0;
        assert!(apply_win_key_delta(&mut m, 1.00, 0.98));
        assert_eq!(m.master_vol, 1.0);
        m.master_vol = 14.0;
        assert!(apply_win_key_delta(&mut m, 0.96, 0.98));
        assert_eq!(m.master_vol, 13.0);

        assert!(apply_win_key_delta(&mut m, 1.98, 0.98));
        assert_eq!(m.master_vol, 60.0);

        m.protection_enabled = false;
        assert!(apply_win_key_delta(&mut m, 1.98, 0.98));
        assert_eq!(m.master_vol, 160.0);
        let _fp = FastAudioParams::from_mixer(&m);

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let cfg = std::env::temp_dir().join(format!("phasemix_fastparams_{stamp}.json"));
        let eng = AudioRouterEngine::new(cfg.clone());
        let st = eng.update_mixer(MixerPatch {
            master_vol: Some(38.0),
            master_mute: Some(true),
            solo: Some(Some("boat".to_string())),
            ..Default::default()
        });
        assert_eq!(st.master_vol, 38.0);
        assert!(st.master_mute);
        assert_eq!(st.solo.as_deref(), Some("boat"));

        let preset = eng.apply_preset("wide_room").unwrap();
        assert_eq!(preset.position_preset, "wide_room");

        let _ = fs::remove_file(&cfg);
        let _ = fs::remove_file(AudioRouterEngine::backup_path(&cfg));
    }

    #[test]
    fn saved_win_device_vols_roundtrip_and_sanitize() {
        let mut dirty = HashMap::new();
        dirty.insert("".to_string(), 50.0);
        dirty.insert("Headphones (Stone 358 Pro)".to_string(), 140.0);
        dirty.insert("Headphones (Realtek(R) Audio)".to_string(), 38.4);
        dirty.insert("Bad NaN".to_string(), f64::NAN);
        let cleaned = sanitize_win_vol_map(dirty);
        assert_eq!(cleaned.len(), 2);
        assert_eq!(cleaned.get("Headphones (Stone 358 Pro)"), Some(&100.0));
        assert_eq!(cleaned.get("Headphones (Realtek(R) Audio)"), Some(&38.0));

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let cfg = std::env::temp_dir().join(format!("phasemix_winvol_{stamp}.json"));
        let eng = AudioRouterEngine::new(cfg.clone());
        let _ = eng
            .import_profile_json(
                r#"{"mixer":{"master_vol":20},"devices":{},"win_device_vols":{"Offline Test Speaker":42.7}}"#,
            )
            .unwrap();
        assert_eq!(
            eng.inner.win_device_vols.read().get("Offline Test Speaker"),
            Some(&43.0)
        );
        let exported = eng.export_profile_json().unwrap();
        assert!(exported.contains("Offline Test Speaker"));

        let _ = eng.reset_factory();
        assert!(eng.inner.win_device_vols.read().is_empty());

        let _ = fs::remove_file(&cfg);
        let _ = fs::remove_file(AudioRouterEngine::backup_path(&cfg));
    }
}
