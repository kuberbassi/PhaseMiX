//! Pure Rust zero-allocation DSP filters, EQ, crossover, C1 soft-knee PeakGuard, and test signals.

use std::f64::consts::PI;

#[derive(Clone, Copy, Debug)]
struct BiquadCoeffs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Default for BiquadCoeffs {
    fn default() -> Self {
        Self::bypass()
    }
}

impl BiquadCoeffs {
    fn bypass() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
        }
    }

    fn normalize(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    fn low_shelf(freq_hz: f64, gain_db: f64, fs: f64, q: f64) -> Self {
        if gain_db.abs() < 0.05 {
            return Self::bypass();
        }
        let a = 10.0_f64.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq_hz.clamp(20.0, fs * 0.4) / fs;
        let (sin_w, cos_w) = w0.sin_cos();
        let alpha = sin_w / (2.0 * q);
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        Self::normalize(
            a * ((a + 1.0) - (a - 1.0) * cos_w + two_sqrt_a_alpha),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w),
            a * ((a + 1.0) - (a - 1.0) * cos_w - two_sqrt_a_alpha),
            (a + 1.0) + (a - 1.0) * cos_w + two_sqrt_a_alpha,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos_w),
            (a + 1.0) + (a - 1.0) * cos_w - two_sqrt_a_alpha,
        )
    }

    fn peaking(freq_hz: f64, gain_db: f64, fs: f64, q: f64) -> Self {
        if gain_db.abs() < 0.05 {
            return Self::bypass();
        }
        let a = 10.0_f64.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq_hz.clamp(20.0, fs * 0.45) / fs;
        let (sin_w, cos_w) = w0.sin_cos();
        let alpha = sin_w / (2.0 * q);

        Self::normalize(
            1.0 + alpha * a,
            -2.0 * cos_w,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cos_w,
            1.0 - alpha / a,
        )
    }

    fn high_shelf(freq_hz: f64, gain_db: f64, fs: f64, q: f64) -> Self {
        if gain_db.abs() < 0.05 {
            return Self::bypass();
        }
        let a = 10.0_f64.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq_hz.clamp(200.0, fs * 0.45) / fs;
        let (sin_w, cos_w) = w0.sin_cos();
        let alpha = sin_w / (2.0 * q);
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        Self::normalize(
            a * ((a + 1.0) + (a - 1.0) * cos_w + two_sqrt_a_alpha),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w),
            a * ((a + 1.0) + (a - 1.0) * cos_w - two_sqrt_a_alpha),
            (a + 1.0) - (a - 1.0) * cos_w + two_sqrt_a_alpha,
            2.0 * ((a - 1.0) - (a + 1.0) * cos_w),
            (a + 1.0) - (a - 1.0) * cos_w - two_sqrt_a_alpha,
        )
    }

    fn butterworth_hpf(freq_hz: f64, fs: f64) -> Self {
        let w0 = 2.0 * PI * freq_hz.clamp(20.0, fs * 0.45) / fs;
        let (sin_w, cos_w) = w0.sin_cos();
        let alpha = sin_w / std::f64::consts::SQRT_2;
        Self::normalize(
            (1.0 + cos_w) * 0.5,
            -(1.0 + cos_w),
            (1.0 + cos_w) * 0.5,
            1.0 + alpha,
            -2.0 * cos_w,
            1.0 - alpha,
        )
    }

    fn butterworth_lpf(freq_hz: f64, fs: f64) -> Self {
        let w0 = 2.0 * PI * freq_hz.clamp(100.0, fs * 0.45) / fs;
        let (sin_w, cos_w) = w0.sin_cos();
        let alpha = sin_w / std::f64::consts::SQRT_2;
        Self::normalize(
            (1.0 - cos_w) * 0.5,
            1.0 - cos_w,
            (1.0 - cos_w) * 0.5,
            1.0 + alpha,
            -2.0 * cos_w,
            1.0 - alpha,
        )
    }
}

/// Stateful stereo biquad filter (Direct Form II Transposed, cache-keyed to avoid click resets).
#[derive(Clone, Debug)]
struct StereoBiquad {
    enabled: bool,
    last_key: i32,
    coeffs: BiquadCoeffs,
    z1: [f64; 2],
    z2: [f64; 2],
}

impl Default for StereoBiquad {
    fn default() -> Self {
        Self {
            enabled: false,
            last_key: i32::MIN,
            coeffs: BiquadCoeffs::bypass(),
            z1: [0.0; 2],
            z2: [0.0; 2],
        }
    }
}

impl StereoBiquad {
    fn configure(&mut self, key: i32, active: bool, build: impl FnOnce() -> BiquadCoeffs) {
        if key == self.last_key {
            return;
        }
        self.last_key = key;
        self.enabled = active;
        if active {
            self.coeffs = build();
        } else {
            self.coeffs = BiquadCoeffs::bypass();
            self.z1 = [0.0; 2];
            self.z2 = [0.0; 2];
        }
    }

    #[inline]
    fn process_in_place(&mut self, block: &mut [[f32; 2]]) {
        if !self.enabled {
            return;
        }
        let c = self.coeffs;
        for frame in block.iter_mut() {
            for ch in 0..2 {
                let x = frame[ch] as f64;
                let y = c.b0 * x + self.z1[ch];
                self.z1[ch] = c.b1 * x - c.a1 * y + self.z2[ch];
                self.z2[ch] = c.b2 * x - c.a2 * y;
                frame[ch] = y as f32;
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct ThreeBandEq {
    fs: f64,
    bass: StereoBiquad,
    mid: StereoBiquad,
    treble: StereoBiquad,
}

impl ThreeBandEq {
    pub fn new(fs: f64) -> Self {
        Self {
            fs,
            bass: StereoBiquad::default(),
            mid: StereoBiquad::default(),
            treble: StereoBiquad::default(),
        }
    }

    pub fn process_in_place(&mut self, block: &mut [[f32; 2]], bass_db: f64, mid_db: f64, treble_db: f64) {
        let b = bass_db.clamp(-12.0, 12.0);
        let m = mid_db.clamp(-12.0, 12.0);
        let t = treble_db.clamp(-12.0, 12.0);

        self.bass.configure((b * 100.0).round() as i32, b.abs() >= 0.05, || {
            BiquadCoeffs::low_shelf(110.0, b, self.fs, 0.707)
        });
        self.mid.configure((m * 100.0).round() as i32, m.abs() >= 0.05, || {
            BiquadCoeffs::peaking(1000.0, m, self.fs, 0.75)
        });
        self.treble.configure((t * 100.0).round() as i32, t.abs() >= 0.05, || {
            BiquadCoeffs::high_shelf(4500.0, t, self.fs, 0.707)
        });

        self.bass.process_in_place(block);
        self.mid.process_in_place(block);
        self.treble.process_in_place(block);
    }
}

#[inline]
fn sanitize_sample(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else if v.is_nan() {
        0.0
    } else if v.is_sign_positive() {
        1.0
    } else {
        -1.0
    }
}

#[inline]
fn soft_limit_sample(v: f32) -> f32 {
    if v.abs() <= 0.96 {
        v
    } else {
        (v * 0.95).tanh()
    }
}

/// Smooth ear-safe output peak limiter with C1 soft-knee first-block transient catch.
#[derive(Clone, Debug)]
pub struct PeakGuard {
    env: f64,
    pub last_gr_db: f64,
}

impl PeakGuard {
    pub fn new() -> Self {
        Self {
            env: 1.0,
            last_gr_db: 0.0,
        }
    }

    pub fn process_in_place(&mut self, block: &mut [[f32; 2]], enabled: bool, ceiling_db: f64) {
        if !enabled || block.is_empty() {
            self.env = (self.env + 0.25).min(1.0);
            self.last_gr_db = 0.0;
            for frame in block.iter_mut() {
                frame[0] = soft_limit_sample(sanitize_sample(frame[0]));
                frame[1] = soft_limit_sample(sanitize_sample(frame[1]));
            }
            return;
        }

        let ceiling_lin = 10.0_f64.powf(ceiling_db.clamp(-30.0, -0.5) / 20.0).clamp(0.05, 0.98);
        let mut peak = 0.0_f64;
        for frame in block.iter_mut() {
            frame[0] = sanitize_sample(frame[0]);
            frame[1] = sanitize_sample(frame[1]);
            peak = peak.max(frame[0].abs() as f64).max(frame[1].abs() as f64);
        }

        let target_gain = if peak > ceiling_lin {
            (ceiling_lin / peak).min(1.0)
        } else {
            1.0
        };
        let alpha = if target_gain < self.env { 0.9 } else { 0.14 };
        let next_env = (1.0 - alpha) * self.env + alpha * target_gain;
        let block_len = block.len();
        let inv_span = if block_len > 1 {
            1.0 / ((block_len - 1) as f64)
        } else {
            1.0
        };
        let env_start = self.env;
        self.env = next_env;

        self.last_gr_db = if next_env < 0.995 {
            (20.0 * next_env.max(1e-4).log10() * 10.0).round() / 10.0
        } else {
            0.0
        };

        let knee = (ceiling_lin * 0.85) as f32;
        let headroom = ((ceiling_lin - knee as f64).max(1e-4)) as f32;
        let ceil_f32 = ceiling_lin as f32;

        for (idx, frame) in block.iter_mut().enumerate() {
            let t = if block_len > 1 {
                (idx as f64) * inv_span
            } else {
                1.0
            };
            let g = (env_start + (next_env - env_start) * t) as f32;
            for ch in 0..2 {
                let mut y = frame[ch] * g;
                let ay = y.abs();
                if ay > ceil_f32 || ay > knee {
                    let mag = knee + headroom * ((ay - knee) / headroom).tanh();
                    y = y.signum() * mag;
                }
                frame[ch] = soft_limit_sample(y);
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct PrimaryParams {
    pub master_vol: f64,
    pub vol: f64,
    pub width_pct: f64,
    pub balance: f64,
    pub bass_db: f64,
    pub mid_db: f64,
    pub treble_db: f64,
    pub mute: bool,
    pub protection_enabled: bool,
    pub protection_ceiling_db: f64,
}

pub struct PrimaryProcessor {
    eq: ThreeBandEq,
    pub guard: PeakGuard,
    prev_gain: f64,
}

impl PrimaryProcessor {
    pub fn new(fs: f64) -> Self {
        Self {
            eq: ThreeBandEq::new(fs),
            guard: PeakGuard::new(),
            prev_gain: 0.0,
        }
    }

    pub fn process_in_place(&mut self, block: &mut [[f32; 2]], p: &PrimaryParams) {
        let n = block.len();
        if n == 0 {
            return;
        }
        let target_gain = if p.mute {
            0.0
        } else {
            (p.master_vol / 100.0) * (p.vol / 100.0)
        };

        if target_gain == 0.0 && self.prev_gain == 0.0 {
            self.guard.last_gr_db = 0.0;
            for f in block.iter_mut() {
                *f = [0.0, 0.0];
            }
            return;
        }

        let width = (p.width_pct / 100.0) as f32;
        if (width - 1.0).abs() > 1e-3 {
            for f in block.iter_mut() {
                let mid = 0.5 * (f[0] + f[1]);
                let side = 0.5 * (f[0] - f[1]) * width;
                f[0] = mid + side;
                f[1] = mid - side;
            }
        }

        self.eq.process_in_place(block, p.bass_db, p.mid_db, p.treble_db);

        let bal = p.balance.clamp(-100.0, 100.0) as f32;
        if bal < -0.5 {
            let atten = 1.0 + bal / 100.0;
            for f in block.iter_mut() {
                f[1] *= atten;
            }
        } else if bal > 0.5 {
            let atten = 1.0 - bal / 100.0;
            for f in block.iter_mut() {
                f[0] *= atten;
            }
        }

        let g0 = self.prev_gain;
        let g1 = target_gain;
        let denom = (n.max(1)) as f64;
        for (i, f) in block.iter_mut().enumerate() {
            let g = (g0 + (g1 - g0) * ((i + 1) as f64 / denom)) as f32;
            f[0] *= g;
            f[1] *= g;
        }
        self.prev_gain = g1;
        self.guard
            .process_in_place(block, p.protection_enabled, p.protection_ceiling_db);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuxMode {
    Stereo,
    MonoCenter,
    VocalMid,
    SideAmbience,
}

impl AuxMode {
    pub fn from_str(s: &str) -> Self {
        match s {
            "mono_center" => Self::MonoCenter,
            "vocal_mid" => Self::VocalMid,
            "side_ambience" => Self::SideAmbience,
            _ => Self::Stereo,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AuxParams {
    pub master_vol: f64,
    pub vol: f64,
    pub mode: AuxMode,
    pub hpf_hz: f64,
    pub lpf_hz: f64,
    pub bass_db: f64,
    pub mid_db: f64,
    pub treble_db: f64,
    pub invert: bool,
    pub mute: bool,
    pub protection_enabled: bool,
    pub protection_ceiling_db: f64,
}

pub struct AuxChannelProcessor {
    fs: f64,
    hpf: StereoBiquad,
    lpf: StereoBiquad,
    vocal: StereoBiquad,
    eq: ThreeBandEq,
    pub guard: PeakGuard,
    prev_gain: f64,
}

impl AuxChannelProcessor {
    pub fn new(fs: f64) -> Self {
        Self {
            fs,
            hpf: StereoBiquad::default(),
            lpf: StereoBiquad::default(),
            vocal: StereoBiquad::default(),
            eq: ThreeBandEq::new(fs),
            guard: PeakGuard::new(),
            prev_gain: 0.0,
        }
    }

    pub fn process_in_place(&mut self, block: &mut [[f32; 2]], p: &AuxParams) {
        let n = block.len();
        if n == 0 {
            return;
        }
        let target_gain = if p.mute {
            0.0
        } else {
            (p.master_vol / 100.0) * (p.vol / 100.0)
        };

        if target_gain == 0.0 && self.prev_gain == 0.0 {
            self.guard.last_gr_db = 0.0;
            for f in block.iter_mut() {
                *f = [0.0, 0.0];
            }
            return;
        }

        match p.mode {
            AuxMode::MonoCenter | AuxMode::VocalMid => {
                for f in block.iter_mut() {
                    let c = 0.5 * (f[0] + f[1]);
                    f[0] = c;
                    f[1] = c;
                }
            }
            AuxMode::SideAmbience => {
                for f in block.iter_mut() {
                    let s = 0.5 * (f[0] - f[1]);
                    f[0] = s;
                    f[1] = -s;
                }
            }
            AuxMode::Stereo => {}
        }

        let vocal_active = p.mode == AuxMode::VocalMid;
        self.vocal.configure(if vocal_active { 1 } else { 0 }, vocal_active, || {
            BiquadCoeffs::peaking(1800.0, 3.5, self.fs, 0.75)
        });
        self.vocal.process_in_place(block);

        let hpf_active = p.hpf_hz > 20.0;
        self.hpf.configure((p.hpf_hz * 10.0).round() as i32, hpf_active, || {
            BiquadCoeffs::butterworth_hpf(p.hpf_hz, self.fs)
        });
        self.hpf.process_in_place(block);

        let lpf_active = p.lpf_hz < 17500.0;
        self.lpf.configure((p.lpf_hz * 10.0).round() as i32, lpf_active, || {
            BiquadCoeffs::butterworth_lpf(p.lpf_hz, self.fs)
        });
        self.lpf.process_in_place(block);

        self.eq.process_in_place(block, p.bass_db, p.mid_db, p.treble_db);

        let inv_sign = if p.invert { -1.0_f64 } else { 1.0_f64 };
        let g0 = self.prev_gain;
        let g1 = target_gain;
        let denom = (n.max(1)) as f64;
        for (i, f) in block.iter_mut().enumerate() {
            let g = ((g0 + (g1 - g0) * ((i + 1) as f64 / denom)) * inv_sign) as f32;
            f[0] *= g;
            f[1] *= g;
        }
        self.prev_gain = g1;
        self.guard
            .process_in_place(block, p.protection_enabled, p.protection_ceiling_db);
    }
}

/// Instantaneous transient-weighted RMS+Peak meter in dBFS floored at -60 dB.
pub fn block_db(block: &[[f32; 2]]) -> f64 {
    if block.is_empty() {
        return -60.0;
    }
    let mut sum_sq = 0.0_f64;
    let mut peak = 0.0_f64;
    for f in block {
        let a0 = f[0] as f64;
        let a1 = f[1] as f64;
        sum_sq += a0 * a0 + a1 * a1;
        peak = peak.max(a0.abs()).max(a1.abs());
    }
    let rms = (sum_sq / ((block.len() * 2) as f64)).sqrt();
    let level = 0.65 * rms + 0.35 * peak;
    if level <= 1e-4 {
        -60.0
    } else {
        (20.0 * level.log10()).clamp(-60.0, 0.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestMode {
    Off,
    Click,
    Chirp,
    Lr,
}

impl TestMode {
    pub fn from_str(s: &str) -> Self {
        match s {
            "click" => Self::Click,
            "chirp" => Self::Chirp,
            "lr" => Self::Lr,
            _ => Self::Off,
        }
    }
}

pub struct TestSignalGenerator {
    fs: f64,
    sample_idx: u64,
}

impl TestSignalGenerator {
    pub fn new(fs: f64) -> Self {
        Self { fs, sample_idx: 0 }
    }

    pub fn generate(&mut self, mode: TestMode, out: &mut [[f32; 2]]) {
        for f in out.iter_mut() {
            let t = (self.sample_idx as f64) / self.fs;
            self.sample_idx = self.sample_idx.wrapping_add(1);
            match mode {
                TestMode::Off => *f = [0.0, 0.0],
                TestMode::Click => {
                    let phase = t % 0.75;
                    if phase < 0.012 {
                        let env = (-phase * 420.0).exp();
                        let s = (env
                            * (0.55 * (2.0 * PI * 1600.0 * phase).sin()
                                + 0.35 * (2.0 * PI * 450.0 * phase).sin())) as f32;
                        *f = [s, s];
                    } else {
                        *f = [0.0, 0.0];
                    }
                }
                TestMode::Chirp => {
                    let cycle = t % 3.0;
                    if cycle < 1.5 {
                        let env = (PI * (cycle / 1.5).clamp(0.0, 1.0)).sin();
                        let s = (0.35 * env * (2.0 * PI * 65.0 * cycle).sin()) as f32;
                        *f = [s, s];
                    } else {
                        let mt = cycle - 1.5;
                        let env = (PI * (mt / 1.5).clamp(0.0, 1.0)).sin();
                        let s = (0.22
                            * env
                            * ((2.0 * PI * 700.0 * mt).sin()
                                + 0.7 * (2.0 * PI * 1400.0 * mt).sin()
                                + 0.4 * (2.0 * PI * 2100.0 * mt).sin())) as f32;
                        *f = [s, s];
                    }
                }
                TestMode::Lr => {
                    let cycle = t % 2.4;
                    let (l_amp, r_amp, freq, lt) = if cycle < 0.42 {
                        (0.32, 0.0, 523.25, cycle)
                    } else if (0.6..1.02).contains(&cycle) {
                        (0.25, 0.25, 783.99, cycle - 0.6)
                    } else if (1.2..1.62).contains(&cycle) {
                        (0.0, 0.32, 659.25, cycle - 1.2)
                    } else {
                        *f = [0.0, 0.0];
                        continue;
                    };
                    let env = (PI * (lt / 0.42).clamp(0.0, 1.0)).sin();
                    let tone = env * (2.0 * PI * freq * lt).sin();
                    *f = [(tone * l_amp) as f32, (tone * r_amp) as f32];
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_guard_catches_nan_and_enforces_ceiling() {
        let mut pg = PeakGuard::new();
        let mut block = [[f32::NAN, f32::INFINITY], [10.0, -10.0]];
        pg.process_in_place(&mut block, true, -15.0);
        let ceiling_lin = 10.0_f32.powf(-15.0 / 20.0);
        for f in &block {
            assert!(f[0].is_finite() && f[1].is_finite());
            assert!(f[0].abs() <= ceiling_lin + 1e-4, "L overshoot {}", f[0]);
            assert!(f[1].abs() <= ceiling_lin + 1e-4, "R overshoot {}", f[1]);
        }
    }

    #[test]
    fn primary_silent_fast_path_and_processing() {
        let mut proc = PrimaryProcessor::new(48000.0);
        let mut sig = [[0.2_f32, -0.15_f32]; 240];
        let muted_params = PrimaryParams {
            master_vol: 60.0,
            vol: 100.0,
            width_pct: 120.0,
            balance: 0.0,
            bass_db: 4.0,
            mid_db: -2.0,
            treble_db: 2.0,
            mute: true,
            protection_enabled: true,
            protection_ceiling_db: -15.0,
        };
        proc.process_in_place(&mut sig, &muted_params);
        assert!(sig.iter().all(|f| f[0] == 0.0 && f[1] == 0.0));

        let mut live = [[0.2_f32, -0.15_f32]; 240];
        let mut p = muted_params;
        p.mute = false;
        proc.process_in_place(&mut live, &p);
        let peak = live.iter().fold(0.0_f32, |m, f| m.max(f[0].abs()).max(f[1].abs()));
        assert!(peak > 0.01 && live.iter().all(|f| f[0].is_finite() && f[1].is_finite()));
    }

    #[test]
    fn aux_eq_invert_and_transient_block_db() {
        let mut aux = AuxChannelProcessor::new(48000.0);
        let mut a = [[0.15_f32, 0.15_f32]; 240];
        let p = AuxParams {
            master_vol: 60.0,
            vol: 80.0,
            mode: AuxMode::VocalMid,
            hpf_hz: 260.0,
            lpf_hz: 14000.0,
            bass_db: -3.0,
            mid_db: 4.0,
            treble_db: 1.5,
            invert: false,
            mute: false,
            protection_enabled: true,
            protection_ceiling_db: -15.0,
        };
        aux.process_in_place(&mut a, &p);
        assert!(a[239][0].is_finite());

        let mut imp = [[0.0_f32, 0.0_f32]; 480];
        for frame in &mut imp[..8] {
            *frame = [0.8, 0.8];
        }
        assert!(block_db(&imp) > -22.0);
        assert_eq!(block_db(&[[0.0, 0.0]; 128]), -60.0);
    }

    #[test]
    fn test_generator_click_produces_signal() {
        let mut sig_gen = TestSignalGenerator::new(48000.0);
        let mut buf = [[0.0_f32; 2]; 480];
        sig_gen.generate(TestMode::Click, &mut buf);
        let max_amp = buf.iter().fold(0.0_f32, |m, f| m.max(f[0].abs()));
        assert!(max_amp > 0.2);
    }
}
