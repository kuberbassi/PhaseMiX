//! Native Windows WASAPI loopback + multi-output synced audio router with lockless-fast ring buffering.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
use windows::Win32::System::Com::CLSCTX_ALL;
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, GetCurrentThread,
    SetThreadPriority, THREAD_PRIORITY_HIGHEST,
};

use crate::dsp::{
    block_db, find_chirp_arrival, generate_cal_chirp_pair, AuxChannelProcessor, AuxMode,
    AuxParams, PrimaryParams, PrimaryProcessor, TestMode, TestSignalGenerator,
};

pub const MASTER_RATE: u32 = 48_000;
const MASTER_RATE_F64: f64 = 48_000.0;
const RING_SECONDS: usize = 3;
const RING_FRAMES: usize = MASTER_RATE as usize * RING_SECONDS;
const BASE_BUFFER_MS: f64 = 90.0;
const PUMP_BLOCK_FRAMES: usize = 480; // 10 ms at 48 kHz
const OUTPUT_ROLES: [&str; 3] = ["intex", "boat", "laptop"];

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
            protection_ceiling_db: -6.0,
            position_preset: "desk_triangle".to_string(),
            intex_vol: 100.0,
            intex_mute: false,
            intex_delay_ms: 224.0,
            intex_balance: 0.0,
            intex_bass_db: 0.5,
            intex_mid_db: -2.0,
            intex_treble_db: 0.5,
            intex_width: 106.0,
            boat_vol: 60.0,
            boat_mute: false,
            boat_delay_ms: 0.0,
            boat_mode: "vocal_mid".to_string(),
            boat_hpf_hz: 260.0,
            boat_lpf_hz: 14000.0,
            boat_bass_db: -2.5,
            boat_mid_db: 2.0,
            boat_treble_db: 0.5,
            boat_invert: false,
            laptop_vol: 125.0,
            laptop_mute: false,
            laptop_delay_ms: 224.0,
            laptop_mode: "stereo".to_string(),
            laptop_hpf_hz: 300.0,
            laptop_lpf_hz: 18000.0,
            laptop_bass_db: -2.0,
            laptop_mid_db: 0.0,
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
            protection_ceiling_db: Some(-6.0),
            intex_vol: Some(100.0),
            intex_balance: Some(0.0),
            intex_width: Some(106.0),
            intex_bass_db: Some(0.5),
            intex_mid_db: Some(-2.0),
            intex_treble_db: Some(0.5),
            boat_vol: Some(60.0),
            boat_mode: Some("vocal_mid".into()),
            boat_hpf_hz: Some(260.0),
            boat_lpf_hz: Some(14000.0),
            boat_bass_db: Some(-2.5),
            boat_mid_db: Some(2.0),
            boat_treble_db: Some(0.5),
            laptop_vol: Some(125.0),
            laptop_mode: Some("stereo".into()),
            laptop_hpf_hz: Some(300.0),
            laptop_lpf_hz: Some(18000.0),
            laptop_bass_db: Some(-2.0),
            laptop_mid_db: Some(0.0),
            laptop_treble_db: Some(2.0),
            ..Default::default()
        },
        "cinema_center" => MixerPatch {
            position_preset: Some("cinema_center".into()),
            protection_max_master: Some(60.0),
            protection_ceiling_db: Some(-6.0),
            intex_vol: Some(100.0),
            intex_balance: Some(0.0),
            intex_width: Some(135.0),
            intex_bass_db: Some(2.5),
            intex_mid_db: Some(-3.5),
            intex_treble_db: Some(0.0),
            boat_vol: Some(72.0),
            boat_mode: Some("vocal_mid".into()),
            boat_hpf_hz: Some(220.0),
            boat_lpf_hz: Some(12000.0),
            boat_bass_db: Some(-3.0),
            boat_mid_db: Some(3.5),
            boat_treble_db: Some(1.0),
            laptop_vol: Some(120.0),
            laptop_mode: Some("side_ambience".into()),
            laptop_hpf_hz: Some(340.0),
            laptop_lpf_hz: Some(15000.0),
            laptop_bass_db: Some(-3.0),
            laptop_mid_db: Some(0.0),
            laptop_treble_db: Some(2.0),
            ..Default::default()
        },
        "wide_room" => MixerPatch {
            position_preset: Some("wide_room".into()),
            protection_max_master: Some(60.0),
            protection_ceiling_db: Some(-6.0),
            intex_vol: Some(110.0),
            intex_balance: Some(0.0),
            intex_width: Some(120.0),
            intex_bass_db: Some(4.0),
            intex_mid_db: Some(0.0),
            intex_treble_db: Some(1.5),
            boat_vol: Some(75.0),
            boat_mode: Some("mono_center".into()),
            boat_hpf_hz: Some(185.0),
            boat_lpf_hz: Some(17500.0),
            boat_bass_db: Some(0.0),
            boat_mid_db: Some(1.5),
            boat_treble_db: Some(1.5),
            laptop_vol: Some(160.0),
            laptop_mode: Some("stereo".into()),
            laptop_hpf_hz: Some(250.0),
            laptop_lpf_hz: Some(18000.0),
            laptop_bass_db: Some(0.0),
            laptop_mid_db: Some(1.0),
            laptop_treble_db: Some(2.0),
            ..Default::default()
        },
        "night_safe" => MixerPatch {
            position_preset: Some("night_safe".into()),
            protection_enabled: Some(true),
            protection_max_master: Some(60.0),
            protection_ceiling_db: Some(-15.0),
            intex_vol: Some(75.0),
            intex_balance: Some(0.0),
            intex_width: Some(95.0),
            intex_bass_db: Some(-5.0),
            intex_mid_db: Some(0.0),
            intex_treble_db: Some(-2.0),
            boat_vol: Some(65.0),
            boat_mode: Some("vocal_mid".into()),
            boat_hpf_hz: Some(240.0),
            boat_lpf_hz: Some(10000.0),
            boat_bass_db: Some(-2.5),
            boat_mid_db: Some(2.0),
            boat_treble_db: Some(-1.5),
            laptop_vol: Some(80.0),
            laptop_mode: Some("stereo".into()),
            laptop_hpf_hz: Some(280.0),
            laptop_lpf_hz: Some(12000.0),
            laptop_bass_db: Some(-3.0),
            laptop_mid_db: Some(1.0),
            laptop_treble_db: Some(-2.0),
            ..Default::default()
        },
        "asym_desk" => MixerPatch {
            position_preset: Some("asym_desk".into()),
            protection_max_master: Some(60.0),
            protection_ceiling_db: Some(-6.0),
            intex_vol: Some(100.0),
            intex_balance: Some(18.0),
            intex_width: Some(105.0),
            intex_bass_db: Some(1.0),
            intex_mid_db: Some(-1.5),
            intex_treble_db: Some(0.5),
            boat_vol: Some(65.0),
            boat_mode: Some("vocal_mid".into()),
            boat_hpf_hz: Some(280.0),
            boat_lpf_hz: Some(16000.0),
            boat_bass_db: Some(-2.0),
            boat_mid_db: Some(2.0),
            boat_treble_db: Some(1.0),
            laptop_vol: Some(165.0),
            laptop_mode: Some("stereo".into()),
            laptop_hpf_hz: Some(260.0),
            laptop_lpf_hz: Some(18000.0),
            laptop_bass_db: Some(-1.0),
            laptop_mid_db: Some(0.5),
            laptop_treble_db: Some(1.5),
            ..Default::default()
        },
        "pure_mirror" => MixerPatch {
            position_preset: Some("pure_mirror".into()),
            protection_max_master: Some(60.0),
            protection_ceiling_db: Some(-6.0),
            intex_vol: Some(100.0),
            intex_balance: Some(0.0),
            intex_width: Some(100.0),
            intex_bass_db: Some(0.0),
            intex_mid_db: Some(0.0),
            intex_treble_db: Some(0.0),
            boat_vol: Some(65.0),
            boat_mode: Some("stereo".into()),
            boat_hpf_hz: Some(0.0),
            boat_lpf_hz: Some(18000.0),
            boat_bass_db: Some(0.0),
            boat_mid_db: Some(0.0),
            boat_treble_db: Some(0.0),
            laptop_vol: Some(130.0),
            laptop_mode: Some("stereo".into()),
            laptop_hpf_hz: Some(220.0),
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
            in_db: -60.0,
            intex_db: -60.0,
            boat_db: -60.0,
            laptop_db: -60.0,
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
        self.write_idx = self.write_idx.wrapping_add(frames.len() as u64);
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
            let s = (idx_start as usize) % RING_FRAMES;
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

        let min_pos = w.saturating_sub(max_age) as f64;
        let w_lim = w.saturating_sub(1) as f64;
        let hermite_lim = w.saturating_sub(2) as f64;
        for (i, frame) in out.iter_mut().enumerate() {
            let pos = read_pos + (i as f64) * step;
            if pos < min_pos || pos >= w_lim {
                *frame = [0.0, 0.0];
                continue;
            }
            let idx1 = pos.floor() as usize;
            let t = (pos - idx1 as f64) as f32;
            if pos >= min_pos + 1.0 && pos < hermite_lim && idx1 >= 1 {
                let s0 = self.buf[(idx1 - 1) % RING_FRAMES];
                let s1 = self.buf[idx1 % RING_FRAMES];
                let s2 = self.buf[(idx1 + 1) % RING_FRAMES];
                let s3 = self.buf[(idx1 + 2) % RING_FRAMES];
                let t2 = t * t;
                let t3 = t2 * t;
                let interp = |y0: f32, y1: f32, y2: f32, y3: f32| -> f32 {
                    0.5 * ((2.0 * y1)
                        + (-y0 + y2) * t
                        + (2.0 * y0 - 5.0 * y1 + 4.0 * y2 - y3) * t2
                        + (-y0 + 3.0 * y1 - 3.0 * y2 + y3) * t3)
                };
                *frame = [
                    interp(s0[0], s1[0], s2[0], s3[0]),
                    interp(s0[1], s1[1], s2[1], s3[1]),
                ];
            } else {
                let s1 = self.buf[idx1 % RING_FRAMES];
                let s2 = self.buf[(idx1 + 1) % RING_FRAMES];
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
        } else if nl.contains("stone") {
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

    let mut intex_id = find_id_by_name(outputs, &saved.intex_name).unwrap_or_default();
    if intex_id.is_empty() && saved.intex_name.is_empty() {
        for pat in ["headphones (realtek", "speakers (realtek"] {
            if let Some(d) = outputs.iter().find(|d| d.name.to_lowercase().contains(pat)) {
                intex_id = d.id.clone();
                break;
            }
        }
    }

    let mut boat_id = find_id_by_name(outputs, &saved.boat_name).unwrap_or_default();
    if boat_id.is_empty() && saved.boat_name.is_empty() {
        for pat in [
            "headphones (stone",
            "stone 358",
            "headphones (3.0 bt",
            "bluetooth",
            "bt",
            "soundcore",
            "jbl",
            "sony",
            "bose",
            "marshall",
            "headphones (",
        ] {
            if let Some(d) = outputs.iter().find(|d| {
                let nl = d.name.to_lowercase();
                nl.contains(pat)
                    && !nl.contains("realtek")
                    && !nl.contains("hands-free")
                    && !nl.contains("ag audio")
                    && !is_virtual_audio_name(&nl)
                    && d.id != intex_id
            }) {
                boat_id = d.id.clone();
                break;
            }
        }
    }

    let mut laptop_id = find_id_by_name(outputs, &saved.laptop_name).unwrap_or_default();
    if laptop_id.is_empty() && saved.laptop_name.is_empty() {
        if let Some(d) = outputs.iter().find(|d| {
            let nl = d.name.to_lowercase();
            nl.contains("speakers (realtek") && d.id != intex_id && d.id != boat_id
        }) {
            laptop_id = d.id.clone();
        }
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
    })
}

pub fn query_selected_win_vols(ids: &SelectedIds) -> WinVols {
    ensure_com();
    let Ok(enumerator) = DeviceEnumerator::new() else {
        return WinVols::default();
    };
    let get_one = |id: &str| -> Option<WinEndpointVol> {
        if id.is_empty() {
            return None;
        }
        let dev = enumerator.get_device(id).ok()?;
        let name = dev.get_friendlyname().unwrap_or_default();
        read_device_endpoint_vol(&dev, id, &name)
    };
    WinVols {
        input: get_one(&ids.input_id),
        intex: get_one(&ids.intex_id),
        boat: get_one(&ids.boat_id),
        laptop: get_one(&ids.laptop_id),
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
    prev_day_preset: RwLock<String>,
    selected_ids: RwLock<SelectedIds>,
    selected_names: RwLock<SelectedNames>,
    preferred_names: RwLock<SelectedNames>,
    meters: RwLock<Meters>,
    telemetry: RwLock<Telemetry>,
    ring: Mutex<SharedRing>,
    cal: CalibrationSync,
    running: AtomicBool,
    stop_flag: Arc<AtomicBool>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    save_signal: (Mutex<Option<Instant>>, Condvar),
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

        let inner = Arc::new(EngineInner {
            config_path,
            mixer: RwLock::new(MixerState::default()),
            prev_day_preset: RwLock::new("desk_triangle".to_string()),
            selected_ids: RwLock::new(SelectedIds::default()),
            selected_names: RwLock::new(SelectedNames::default()),
            preferred_names: RwLock::new(SelectedNames::default()),
            meters: RwLock::new(Meters::default()),
            telemetry: RwLock::new(Telemetry::default()),
            ring: Mutex::new(SharedRing::new()),
            cal: CalibrationSync::default(),
            running: AtomicBool::new(false),
            stop_flag: Arc::new(AtomicBool::new(false)),
            workers: Mutex::new(Vec::new()),
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
        m.enforce_protection_cap();
        m.test_mode = "off".to_string();
        m.master_mute = false;
        m.intex_mute = false;
        m.boat_mute = false;
        m.laptop_mute = false;
        m.solo = None;

        if m.position_preset != "night_safe" && m.position_preset != "custom" {
            *inner.prev_day_preset.write() = m.position_preset.clone();
        }
        *inner.mixer.write() = m;
        *inner.selected_names.write() = parsed.devices.clone();
        *inner.preferred_names.write() = parsed.devices;
    }

    fn save_config_now(inner: &EngineInner) {
        let payload = PersistedConfig {
            mixer: inner.mixer.read().clone(),
            devices: inner.preferred_names.read().clone(),
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

        *inner.selected_ids.write() = picked.clone();
        *inner.selected_names.write() = new_names.clone();
        *inner.preferred_names.write() = merged_preferred;
        if persist {
            Self::save_config_now(inner);
        }

        let win_vols = query_selected_win_vols(&picked);
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

    pub fn set_devices(&self, ids: SelectedIds) -> StatusPayload {
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
        *self.inner.mixer.write() = m;
        if !parsed.devices.input_name.is_empty()
            || !parsed.devices.intex_name.is_empty()
            || !parsed.devices.boat_name.is_empty()
            || !parsed.devices.laptop_name.is_empty()
        {
            *self.inner.preferred_names.write() = parsed.devices;
            Self::refresh_device_selection_inner(&self.inner, true);
        } else {
            Self::save_config_now(&self.inner);
        }
        Ok(self.status_payload())
    }

    pub fn reset_factory(&self) -> StatusPayload {
        *self.inner.prev_day_preset.write() = "desk_triangle".to_string();
        *self.inner.mixer.write() = MixerState::default();
        Self::save_config_now(&self.inner);
        self.status_payload()
    }

    /// Measure acoustic BT vs Primary AUX lag differential via default laptop mic using FM chirps.
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
        if ids.boat_id.is_empty() || ids.intex_id.is_empty() {
            return CalibrationResult {
                ok: false,
                measured_ms: 0.0,
                confidence: 0.0,
                message: "Assign both Primary AUX and Center Wireless Bluetooth first".to_string(),
                status: self.status_payload(),
            };
        }

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
            m.intex_delay_ms = measured_ms;
            m.laptop_delay_ms = measured_ms;
            m.calibrated_delay_ms = Some(measured_ms);
        }
        Self::save_config_now(&self.inner);

        let spread_i = spread.round();
        CalibrationResult {
            ok: true,
            measured_ms,
            confidence: spread_i,
            message: if spread_i < 1.0 {
                format!("Aligned AUX & Built-in to {measured_ms:.0} ms")
            } else {
                format!("Aligned AUX & Built-in to {measured_ms:.0} ms (±{spread_i:.0} ms)")
            },
            status: self.status_payload(),
        }
    }

    pub fn set_win_vol(&self, role: &str, volume_pct: f64) -> (bool, StatusPayload) {
        let ids = self.inner.selected_ids.read().clone();
        let dev_id = match role {
            "input" => ids.input_id.as_str(),
            "intex" => ids.intex_id.as_str(),
            "boat" => ids.boat_id.as_str(),
            "laptop" => ids.laptop_id.as_str(),
            _ => return (false, self.status_payload()),
        };
        // Ear-Safe guard caps manual Windows hardware endpoint changes at 50% when active
        let max_pct = if role != "input" && self.inner.mixer.read().protection_enabled {
            50.0
        } else {
            100.0
        };
        let ok = set_device_endpoint_volume(dev_id, volume_pct.clamp(0.0, max_pct), true);
        (ok, self.status_payload())
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
        let m = self.inner.mixer.read();
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
            m.intex_delay_ms,
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
            win_vols: query_selected_win_vols(&ids),
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

fn set_output_meter(inner: &EngineInner, role: &str, db: f64) {
    let mut m = inner.meters.write();
    match role {
        "intex" => m.intex_db = db,
        "boat" => m.boat_db = db,
        "laptop" => m.laptop_db = db,
        _ => {}
    }
}

fn set_output_telemetry(inner: &EngineInner, role: &str, actual_ms: f64, ppm: f64, gr_db: f64) {
    let mut t = inner.telemetry.write();
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
                    f32::from_le_bytes(frame[off..off + 4].try_into().unwrap_or([0; 4]))
                }
                (SampleType::Int, 2) => {
                    let s = i16::from_le_bytes(frame[off..off + 2].try_into().unwrap_or([0; 2]));
                    s as f32 / 32768.0
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

    // 1. Try low-jitter EventsShared + autoconvert directly at 48 kHz float stereo (35 ms headroom)
    if client
        .initialize_client(
            &desired,
            Direction::Capture,
            StreamMode::EventsShared {
                autoconvert: true,
                buffer_duration_hns: def_period.max(350_000),
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
                buffer_duration_hns: def_period.max(400_000),
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
    let mut out_48k: Vec<[f32; 2]> = Vec::with_capacity(2048);

    while !inner.stop_flag.load(Ordering::Relaxed) {
        let mode_now = TestMode::from_str(&inner.mixer.read().test_mode);
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
                let m = TestMode::from_str(&inner.mixer.read().test_mode);
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
                    inner.meters.write().in_db = block_db(&test_buf);
                } else if m == TestMode::Off {
                    inner.ring.lock().write(&silence);
                    inner.meters.write().in_db = -60.0;
                } else {
                    sig_gen.generate(m, &mut test_buf);
                    inner.ring.lock().write(&test_buf);
                    inner.meters.write().in_db = block_db(&test_buf);
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
                        thread::sleep(Duration::from_millis(500));
                        continue;
                    }
                };
                if let Err(e) = client.start_stream() {
                    inner.telemetry.write().input_status = format!("Input Error ({e})");
                    thread::sleep(Duration::from_millis(500));
                    continue;
                }

                let cap_rate = fmt.get_samplespersec();
                let frame_bytes =
                    (fmt.get_nchannels().max(1) as usize) * ((fmt.get_bitspersample() / 8).max(1) as usize);
                inner.telemetry.write().input_status = format!("Capturing ({cap_rate} Hz)");
                let mut last_audio_ts = Instant::now();
                let mut silence_pace = Instant::now();

                'capture_stream: while !inner.stop_flag.load(Ordering::Relaxed) {
                    if TestMode::from_str(&inner.mixer.read().test_mode) != TestMode::Off
                        || inner.cal.active.load(Ordering::Relaxed)
                    {
                        break;
                    }
                    if let Some(ref ev) = h_event {
                        if let Err(e) = ev.wait_for_event(10) {
                            let msg = format!("{e}");
                            if !msg.contains("timed out") && !msg.contains("Timeout") {
                                inner.telemetry.write().input_status =
                                    format!("Reconnecting Input ({msg})");
                                break 'capture_stream;
                            }
                        }
                    } else {
                        thread::sleep(Duration::from_millis(8));
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
                                    let count = (read_n as usize)
                                        .min(PUMP_BLOCK_FRAMES * 4);
                                    out_48k.clear();
                                    out_48k.resize(count, [0.0, 0.0]);
                                    inner.ring.lock().write(&out_48k);
                                    inner.meters.write().in_db = -60.0;
                                } else {
                                    let bytes_read = (read_n as usize) * frame_bytes;
                                    decode_bytes_to_stereo_48k(
                                        &raw_bytes[..bytes_read],
                                        &fmt,
                                        &mut scratch_stereo,
                                        &mut out_48k,
                                    );
                                    if !out_48k.is_empty() {
                                        inner.ring.lock().write(&out_48k);
                                        inner.meters.write().in_db = block_db(&out_48k);
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

                    if !got_frames && last_audio_ts.elapsed() > Duration::from_millis(15) {
                        let now = Instant::now();
                        if now.duration_since(silence_pace) > Duration::from_millis(50) {
                            silence_pace = now;
                        }
                        let mut wrote = 0;
                        while now.duration_since(silence_pace) >= Duration::from_millis(10)
                            && wrote < 3
                        {
                            silence_pace += Duration::from_millis(10);
                            inner.ring.lock().write(&silence);
                            wrote += 1;
                        }
                        if wrote > 0 {
                            inner.meters.write().in_db = -60.0;
                        }
                    } else if got_frames {
                        silence_pace = Instant::now();
                    }
                }
                let _ = client.stop_stream();
                if !inner.stop_flag.load(Ordering::Relaxed)
                    && TestMode::from_str(&inner.mixer.read().test_mode) == TestMode::Off
                    && !inner.cal.active.load(Ordering::Relaxed)
                {
                    thread::sleep(Duration::from_millis(250));
                }
            }
            Err(err) => {
                inner.telemetry.write().input_status = format!("Input Error ({err})");
                thread::sleep(Duration::from_millis(600));
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

fn open_render_client(dev: &Device) -> Result<(AudioClient, WaveFormat, wasapi::Handle), String> {
    let mut client = dev.get_iaudioclient().map_err(|e| format!("{e}"))?;
    let (def_period, _min_period) = client.get_device_period().unwrap_or((100_000, 100_000));
    let is_bt = dev
        .get_friendlyname()
        .map(|n| is_wireless_device_name(&n))
        .unwrap_or(false);
    // Give Bluetooth 45 ms and wired/built-in speakers 25 ms shared engine buffer slack to stop crackles
    let min_hns = if is_bt { 450_000 } else { 250_000 };
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
        return Ok((client, desired, ev));
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
    Ok((client, fmt, ev))
}

fn frames_to_le_bytes(frames: &[[f32; 2]], byte_buf: &mut Vec<u8>) {
    byte_buf.clear();
    byte_buf.reserve(frames.len() * 8);
    for f in frames {
        byte_buf.extend_from_slice(&f[0].to_le_bytes());
        byte_buf.extend_from_slice(&f[1].to_le_bytes());
    }
}

fn run_output_loop(inner: Arc<EngineInner>, role: String, initial_id: String) {
    ensure_com();
    let _prio_guard = elevate_audio_thread();
    let mut dev_id = initial_id;
    let mut block_buf: Vec<[f32; 2]> = vec![[0.0; 2]; 2048];
    let mut byte_buf: Vec<u8> = Vec::with_capacity(2048 * 8);

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
            Ok((client, fmt, h_event)) => {
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

                let init_delay = {
                    let m = inner.mixer.read();
                    match role.as_str() {
                        "intex" => m.intex_delay_ms,
                        "boat" => m.boat_delay_ms,
                        _ => m.laptop_delay_ms,
                    }
                };
                let mut prev_delay_ms = init_delay;
                let w0 = inner.ring.lock().write_idx;
                let mut read_pos = (w0 as f64
                    - (BASE_BUFFER_MS + init_delay) * MASTER_RATE_F64 / 1000.0)
                    .max(0.0);
                let mut smoothed_diff = 0.0_f64;
                let mut fade_in = 1.0_f32;

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

                    let params = inner.mixer.read().clone();
                    let cal_now = inner.cal.active.load(Ordering::Relaxed);
                    let delay_ms = if cal_now {
                        0.0
                    } else {
                        match role.as_str() {
                            "intex" => params.intex_delay_ms,
                            "boat" => params.boat_delay_ms,
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

                        // Snap cleanly with micro-fade on delay changes, >140 ms lag shifts, underruns, or ring wrap
                        let step_est = MASTER_RATE_F64 / (out_rate as f64);
                        let block_need = (avail as f64) * step_est + 64.0;
                        let underrun = read_pos + block_need >= w as f64;
                        let overflow = read_pos < min_valid;

                        if (delay_ms - prev_delay_ms).abs() > 0.5
                            || diff.abs() > MASTER_RATE_F64 * 0.14
                            || underrun
                            || overflow
                        {
                            prev_delay_ms = delay_ms;
                            if diff.abs() > 240.0 || underrun || overflow {
                                read_pos = desired_pos.min(w as f64 - block_need).max(min_valid);
                                smoothed_diff = 0.0;
                                fade_in = 0.0;
                                diff = 0.0;
                            }
                        }

                        // 6 ms jitter deadband + ±1200 ppm smooth clock drift tracking (zero audible warble/tearing)
                        smoothed_diff = 0.995 * smoothed_diff + 0.005 * diff;
                        let rate_adj = if smoothed_diff.abs() < 288.0 {
                            1.0
                        } else {
                            1.0 + (smoothed_diff / (MASTER_RATE_F64 * 14.0)).clamp(-0.0012, 0.0012)
                        };
                        let step = step_est * rate_adj;
                        let read_start = read_pos;
                        ring.read_interpolated(read_pos, &mut block_buf, step);
                        read_pos += (avail as f64) * step;
                        (w, rate_adj, read_start, step)
                    };

                    let (w_after, rate_adj, read_start, step) = w_now;

                    if fade_in < 1.0 {
                        let next_fade = (fade_in + 0.25).min(1.0);
                        let span = avail.max(1) as f32;
                        for (idx, frame) in block_buf.iter_mut().enumerate() {
                            let g = fade_in + (next_fade - fade_in) * (idx as f32 / span);
                            frame[0] *= g;
                            frame[1] *= g;
                        }
                        fade_in = next_fade;
                    }

                    let master_vol = if params.protection_enabled {
                        params.master_vol.min(params.protection_max_master)
                    } else {
                        params.master_vol
                    };
                    let solo_blocked = params.solo.as_deref().is_some_and(|s| s != role);

                    let gr_db = if cal_now {
                        if role == "laptop" {
                            block_buf.fill([0.0, 0.0]);
                        } else {
                            let wins = if role == "boat" {
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
                    } else if role == "intex" {
                        let p = PrimaryParams {
                            master_vol,
                            vol: params.intex_vol,
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
                    } else if role == "boat" {
                        let p = AuxParams {
                            master_vol,
                            vol: params.boat_vol,
                            mode: AuxMode::from_str(&params.boat_mode),
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
                            mode: AuxMode::from_str(&params.laptop_mode),
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

                    frames_to_le_bytes(&block_buf, &mut byte_buf);
                    if render_client.write_to_device(&byte_buf, None).is_err() {
                        break;
                    }

                    let db = block_db(&block_buf);
                    set_output_meter(&inner, &role, db);
                    let actual_ms = ((w_after as f64 - read_pos) * 1000.0 / MASTER_RATE_F64
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
                set_output_meter(&inner, &role, -60.0);
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

    let Ok(enumerator) = DeviceEnumerator::new() else {
        return;
    };
    let Ok(dev) = enumerator.get_device(&input_id) else {
        return;
    };
    let Ok(ep) = (unsafe { dev.as_immdevice().Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) })
    else {
        return;
    };

    unsafe {
        let _ = ep.SetMute(false, std::ptr::null());
        let _ = ep.SetMasterVolumeLevelScalar(UNITY_SCALAR, std::ptr::null());
    }

    while !inner.stop_flag.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(60));
        if inner.stop_flag.load(Ordering::Relaxed) {
            break;
        }
        let muted = unsafe { ep.GetMute().map(|b| b.as_bool()).unwrap_or(false) };
        if muted {
            {
                let mut m = inner.mixer.write();
                m.master_mute = !m.master_mute;
            }
            AudioRouterEngine::save_config_now(&inner);
            unsafe {
                let _ = ep.SetMute(false, std::ptr::null());
            }
        }
        if let Ok(scalar) = unsafe { ep.GetMasterVolumeLevelScalar() } {
            let diff = (scalar - UNITY_SCALAR) as f64;
            if diff.abs() >= 0.012 {
                let steps = (diff / 0.02).round();
                let delta = if steps.abs() <= 3.0 {
                    steps * 4.0
                } else {
                    diff * 120.0
                };
                {
                    let mut m = inner.mixer.write();
                    let cap = if m.protection_enabled {
                        m.protection_max_master
                    } else {
                        200.0
                    };
                    m.master_vol = (m.master_vol + delta).clamp(0.0, cap).round();
                }
                AudioRouterEngine::schedule_save(&inner);
                unsafe {
                    let _ = ep.SetMasterVolumeLevelScalar(UNITY_SCALAR, std::ptr::null());
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
        assert_eq!(m.protection_ceiling_db, -6.0);
        assert_eq!(m.intex_delay_ms, 224.0);
        assert_eq!(m.laptop_delay_ms, 224.0);

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
        assert_eq!(m.protection_ceiling_db, -6.0);
        assert_eq!(m.position_preset, "wide_room");
        assert_eq!(m.boat_hpf_hz, 185.0);
        assert_eq!(m.laptop_treble_db, 2.0);
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
        assert_eq!(factory.mixer.intex_delay_ms, 224.0);
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
}
