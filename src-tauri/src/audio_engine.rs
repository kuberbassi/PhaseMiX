//! Native Windows WASAPI loopback + multi-output synced audio router with lockless-fast ring buffering.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex, RwLock};
use serde::{Deserialize, Serialize};
use wasapi::{
    initialize_mta, AudioClient, Device, DeviceEnumerator, DeviceState, Direction, SampleType,
    StreamMode, WaveFormat,
};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::System::Com::CLSCTX_ALL;

use crate::dsp::{
    block_db, AuxChannelProcessor, AuxMode, AuxParams, PrimaryParams, PrimaryProcessor,
    TestMode, TestSignalGenerator,
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
}

impl Default for MixerState {
    fn default() -> Self {
        Self {
            master_vol: 25.0,
            master_mute: false,
            protection_enabled: true,
            protection_max_master: 60.0,
            protection_ceiling_db: -15.0,
            position_preset: "desk_triangle".to_string(),
            intex_vol: 100.0,
            intex_mute: false,
            intex_delay_ms: 222.0,
            intex_balance: 0.0,
            intex_bass_db: 0.0,
            intex_mid_db: 0.0,
            intex_treble_db: 0.0,
            intex_width: 100.0,
            boat_vol: 85.0,
            boat_mute: false,
            boat_delay_ms: 0.0,
            boat_mode: "mono_center".to_string(),
            boat_hpf_hz: 160.0,
            boat_lpf_hz: 18000.0,
            boat_bass_db: 0.0,
            boat_mid_db: 0.0,
            boat_treble_db: 0.0,
            boat_invert: false,
            laptop_vol: 75.0,
            laptop_mute: false,
            laptop_delay_ms: 222.0,
            laptop_mode: "stereo".to_string(),
            laptop_hpf_hz: 250.0,
            laptop_lpf_hz: 18000.0,
            laptop_bass_db: 0.0,
            laptop_mid_db: 0.0,
            laptop_treble_db: 0.0,
            laptop_invert: false,
            solo: None,
            test_mode: "off".to_string(),
        }
    }
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
pub struct SelectedIds {
    pub input_id: String,
    pub intex_id: String,
    pub boat_id: String,
    pub laptop_id: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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
}

#[derive(Clone, Debug, Serialize)]
pub struct DevicesResponse {
    pub inputs: Vec<DeviceInfo>,
    pub outputs: Vec<DeviceInfo>,
    pub selected: SelectedIds,
    pub selected_names: SelectedNames,
    pub has_virtual_cable: bool,
    pub win_vols: WinVols,
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
        if (step - 1.0).abs() < 1e-6 && (read_pos - read_pos.round()).abs() < 0.05 {
            let idx_start = read_pos.round().max(0.0) as u64;
            let avail = w.saturating_sub(idx_start).min(out_frames as u64) as usize;
            if avail == 0 {
                out.fill([0.0, 0.0]);
                return;
            }
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

        let w_lim = w.saturating_sub(1) as f64;
        for (i, frame) in out.iter_mut().enumerate() {
            let pos = read_pos + (i as f64) * step;
            if pos < 0.0 || pos >= w_lim {
                *frame = [0.0, 0.0];
                continue;
            }
            let idx0 = pos.floor() as usize;
            let frac = (pos - idx0 as f64) as f32;
            let i0 = idx0 % RING_FRAMES;
            let i1 = (idx0 + 1) % RING_FRAMES;
            let s0 = self.buf[i0];
            let s1 = self.buf[i1];
            *frame = [
                s0[0] + (s1[0] - s0[0]) * frac,
                s0[1] + (s1[1] - s0[1]) * frac,
            ];
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
    if in_id.is_empty() {
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
    if intex_id.is_empty() {
        for pat in ["headphones (realtek", "speakers (realtek"] {
            if let Some(d) = outputs.iter().find(|d| d.name.to_lowercase().contains(pat)) {
                intex_id = d.id.clone();
                break;
            }
        }
    }

    let mut boat_id = find_id_by_name(outputs, &saved.boat_name).unwrap_or_default();
    if boat_id.is_empty() {
        for pat in ["headphones (stone", "stone 358", "headphones (3.0 bt", "headphones ("] {
            if let Some(d) = outputs.iter().find(|d| {
                let nl = d.name.to_lowercase();
                nl.contains(pat) && !nl.contains("realtek") && d.id != intex_id
            }) {
                boat_id = d.id.clone();
                break;
            }
        }
    }

    let mut laptop_id = find_id_by_name(outputs, &saved.laptop_name).unwrap_or_default();
    if laptop_id.is_empty() {
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

struct EngineInner {
    config_path: PathBuf,
    mixer: RwLock<MixerState>,
    selected_ids: RwLock<SelectedIds>,
    selected_names: RwLock<SelectedNames>,
    meters: RwLock<Meters>,
    telemetry: RwLock<Telemetry>,
    ring: Mutex<SharedRing>,
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
            selected_ids: RwLock::new(SelectedIds::default()),
            selected_names: RwLock::new(SelectedNames::default()),
            meters: RwLock::new(Meters::default()),
            telemetry: RwLock::new(Telemetry::default()),
            ring: Mutex::new(SharedRing::new()),
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

    fn load_config(inner: &EngineInner) {
        if !inner.config_path.exists() {
            return;
        }
        let Ok(raw) = fs::read_to_string(&inner.config_path) else {
            return;
        };
        let Ok(parsed) = serde_json::from_str::<PersistedConfig>(&raw) else {
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

        *inner.mixer.write() = m;
        *inner.selected_names.write() = parsed.devices;
    }

    fn save_config_now(inner: &EngineInner) {
        let payload = PersistedConfig {
            mixer: inner.mixer.read().clone(),
            devices: inner.selected_names.read().clone(),
        };
        let Ok(json) = serde_json::to_string_pretty(&payload) else {
            return;
        };
        let tmp = PathBuf::from(format!("{}.tmp", inner.config_path.display()));
        if fs::write(&tmp, json).is_ok() {
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
        let saved_names = inner.selected_names.read().clone();
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

        *inner.selected_ids.write() = picked.clone();
        *inner.selected_names.write() = new_names.clone();
        if persist {
            Self::save_config_now(inner);
        }

        let win_vols = query_selected_win_vols(&picked);
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

        *self.inner.selected_names.write() = SelectedNames {
            input_name: id_to_in.get(ids.input_id.as_str()).unwrap_or(&"").to_string(),
            intex_name: id_to_out.get(ids.intex_id.as_str()).unwrap_or(&"").to_string(),
            boat_name: id_to_out.get(ids.boat_id.as_str()).unwrap_or(&"").to_string(),
            laptop_name: id_to_out.get(ids.laptop_id.as_str()).unwrap_or(&"").to_string(),
        };
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
            m.clone()
        };
        Self::schedule_save(&self.inner);
        updated
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
        let ok = set_device_endpoint_volume(dev_id, volume_pct.clamp(0.0, 100.0), true);
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

    pub fn status_payload(&self) -> StatusPayload {
        let ids = self.inner.selected_ids.read().clone();
        StatusPayload {
            running: self.inner.running.load(Ordering::SeqCst),
            mixer: self.inner.mixer.read().clone(),
            meters: self.inner.meters.read().clone(),
            telemetry: self.inner.telemetry.read().clone(),
            selected_names: self.inner.selected_names.read().clone(),
            win_vols: query_selected_win_vols(&ids),
            selected: ids,
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
        for (role, dev_id) in [
            ("intex", ids.intex_id),
            ("boat", ids.boat_id),
            ("laptop", ids.laptop_id),
        ] {
            if dev_id.is_empty() {
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
    let (def_period, min_period) = client.get_device_period().unwrap_or((100_000, 100_000));
    let desired = WaveFormat::new(32, 32, SampleType::Float, MASTER_RATE as usize, 2, None);

    // 1. Try low-jitter EventsShared + autoconvert directly at 48 kHz float stereo
    if client
        .initialize_client(
            &desired,
            Direction::Capture,
            StreamMode::EventsShared {
                autoconvert: true,
                buffer_duration_hns: min_period.max(100_000),
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
                buffer_duration_hns: def_period.max(200_000),
            },
            None,
        )
        .map_err(|e| format!("{e}"))?;
    Ok((client, mix_fmt, None))
}

fn run_capture_pump(inner: Arc<EngineInner>, device_id: String) {
    ensure_com();
    let mut sig_gen = TestSignalGenerator::new(MASTER_RATE_F64);
    let silence = [[0.0_f32; 2]; PUMP_BLOCK_FRAMES];
    let mut test_buf = [[0.0_f32; 2]; PUMP_BLOCK_FRAMES];
    let mut raw_bytes: Vec<u8> = Vec::with_capacity(48000);
    let mut scratch_stereo: Vec<[f32; 2]> = Vec::with_capacity(2048);
    let mut out_48k: Vec<[f32; 2]> = Vec::with_capacity(2048);

    while !inner.stop_flag.load(Ordering::Relaxed) {
        let mode_now = TestMode::from_str(&inner.mixer.read().test_mode);
        if device_id.is_empty() || mode_now != TestMode::Off {
            inner.telemetry.write().input_status = if device_id.is_empty() {
                "Test Generator Only".to_string()
            } else {
                "Test Tone Active".to_string()
            };
            let mut next_tick = Instant::now();
            let dt = Duration::from_micros(10_000);
            while !inner.stop_flag.load(Ordering::Relaxed) {
                let m = TestMode::from_str(&inner.mixer.read().test_mode);
                if m == TestMode::Off && !device_id.is_empty() {
                    break;
                }
                if m == TestMode::Off {
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

                while !inner.stop_flag.load(Ordering::Relaxed) {
                    if TestMode::from_str(&inner.mixer.read().test_mode) != TestMode::Off {
                        break;
                    }
                    if let Some(ref ev) = h_event {
                        let _ = ev.wait_for_event(15);
                    } else {
                        thread::sleep(Duration::from_millis(10));
                    }

                    let mut got_frames = false;
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
                                inner.telemetry.write().input_status = format!("Input Error ({e})");
                                break;
                            }
                        }
                    }

                    if !got_frames && last_audio_ts.elapsed() > Duration::from_millis(45) {
                        let now = Instant::now();
                        if now.duration_since(silence_pace) >= Duration::from_millis(10) {
                            silence_pace = now;
                            inner.ring.lock().write(&silence);
                            inner.meters.write().in_db = -60.0;
                        }
                    } else if got_frames {
                        silence_pace = Instant::now();
                    }
                }
                let _ = client.stop_stream();
            }
            Err(err) => {
                inner.telemetry.write().input_status = format!("Input Error ({err})");
                thread::sleep(Duration::from_millis(600));
            }
        }
    }
}

fn open_render_client(dev: &Device) -> Result<(AudioClient, WaveFormat, wasapi::Handle), String> {
    let mut client = dev.get_iaudioclient().map_err(|e| format!("{e}"))?;
    let (def_period, _min_period) = client.get_device_period().unwrap_or((100_000, 100_000));
    let desired = WaveFormat::new(32, 32, SampleType::Float, MASTER_RATE as usize, 2, None);

    // Prefer EventsShared with autoconvert so hardware (including 44.1kHz Bluetooth) runs smoothly
    if client
        .initialize_client(
            &desired,
            Direction::Render,
            StreamMode::EventsShared {
                autoconvert: true,
                buffer_duration_hns: def_period.max(100_000),
            },
            None,
        )
        .is_ok()
    {
        let ev = client.set_get_eventhandle().map_err(|e| format!("{e}"))?;
        return Ok((client, desired, ev));
    }

    // Fallback: device mixformat directly without rate autoconvert
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
                buffer_duration_hns: def_period.max(150_000),
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
    let mut dev_id = initial_id;
    let mut block_buf: Vec<[f32; 2]> = vec![[0.0; 2]; 2048];
    let mut byte_buf: Vec<u8> = Vec::with_capacity(2048 * 8);

    while !inner.stop_flag.load(Ordering::Relaxed) {
        let opened = DeviceEnumerator::new()
            .and_then(|e| e.get_device(&dev_id))
            .map_err(|e| format!("{e}"))
            .and_then(|dev| open_render_client(&dev));

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
                    let delay_ms = match role.as_str() {
                        "intex" => params.intex_delay_ms,
                        "boat" => params.boat_delay_ms,
                        _ => params.laptop_delay_ms,
                    };

                    if block_buf.len() != avail {
                        block_buf.resize(avail, [0.0, 0.0]);
                    }

                    let w_now = {
                        let ring = inner.ring.lock();
                        let w = ring.write_idx;
                        let target_lag = (BASE_BUFFER_MS + delay_ms) * MASTER_RATE_F64 / 1000.0;
                        let desired_pos = (w as f64) - target_lag;
                        let mut diff = desired_pos - read_pos;

                        // Snap cleanly without pitch warble when delay slider moves or stream resumes
                        if (delay_ms - prev_delay_ms).abs() > 0.5
                            || diff.abs() > MASTER_RATE_F64 * 0.25
                        {
                            prev_delay_ms = delay_ms;
                            if diff.abs() > 240.0 {
                                read_pos = desired_pos.max(0.0);
                                smoothed_diff = 0.0;
                                fade_in = 0.0;
                                diff = 0.0;
                            }
                        }

                        // 8 ms jitter deadband + ±1500 ppm max clock drift tracking (zero audible pitch bend)
                        smoothed_diff = 0.992 * smoothed_diff + 0.008 * diff;
                        let rate_adj = if smoothed_diff.abs() < 384.0 {
                            1.0
                        } else {
                            1.0 + (smoothed_diff / (MASTER_RATE_F64 * 12.0)).clamp(-0.0015, 0.0015)
                        };
                        let step = (MASTER_RATE_F64 / (out_rate as f64)) * rate_adj;
                        ring.read_interpolated(read_pos, &mut block_buf, step);
                        read_pos += (avail as f64) * step;
                        (w, rate_adj)
                    };

                    let (w_after, rate_adj) = w_now;

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

                    let gr_db = if role == "intex" {
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
                set_output_status(&inner, &role, format!("Reconnecting ({e})"));
                set_output_meter(&inner, &role, -60.0);
                // Interruptible sleep before retrying device ID / re-resolving Bluetooth name
                for _ in 0..8 {
                    if inner.stop_flag.load(Ordering::Relaxed) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                let saved_name = {
                    let n = inner.selected_names.read();
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
        thread::sleep(Duration::from_millis(120));
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
        assert_eq!(m.protection_ceiling_db, -15.0);
        assert_eq!(m.intex_delay_ms, 222.0);
        assert_eq!(m.laptop_delay_ms, 222.0);

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
    fn shared_ring_wraparound_fast_slice_and_interp() {
        let mut ring = SharedRing::new();
        ring.write_idx = (RING_FRAMES - 10) as u64;
        let payload = [[0.5_f32, -0.5_f32]; 25];
        ring.write(&payload);

        let mut out = [[0.0_f32; 2]; 25];
        ring.read_interpolated((RING_FRAMES - 10) as f64, &mut out, 1.0);
        assert!((out[0][0] - 0.5).abs() < 1e-6);
        assert!((out[24][1] + 0.5).abs() < 1e-6);
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
    }
}
