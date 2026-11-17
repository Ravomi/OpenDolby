use crate::dsp::biquad::Biquad;
use crate::tuning::MbBandGroup;

#[inline]
pub fn db_to_lin(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

#[inline]
pub fn lin_to_db(lin: f32) -> f32 {
    20.0 * lin.max(1e-7).log10()
}

/// 4th-order Linkwitz-Riley crossover pair (2 cascaded Butterworth 2nd-order filters per side).
#[derive(Debug, Clone)]
struct Lr4Crossover {
    lp: [Biquad; 2],
    hp: [Biquad; 2],
}

impl Lr4Crossover {
    fn new(fs: f32, freq: f32) -> Self {
        let q = std::f32::consts::FRAC_1_SQRT_2;
        Self {
            lp: [Biquad::lowpass_q(fs, freq, q), Biquad::lowpass_q(fs, freq, q)],
            hp: [Biquad::highpass_q(fs, freq, q), Biquad::highpass_q(fs, freq, q)],
        }
    }

    #[inline]
    fn split(&mut self, x: f32) -> (f32, f32) {
        let l0 = self.lp[0].process(x);
        let low = self.lp[1].process(l0);
        let h0 = self.hp[0].process(x);
        let high = self.hp[1].process(h0);
        (low, high)
    }
}

#[derive(Debug, Clone)]
struct BandCompressor {
    threshold_db: f32,
    ratio: f32,
    makeup_lin: f32,
    attack_coeff: f32,
    release_coeff: f32,
    env_db: f32,
}

impl BandCompressor {
    fn new(fs: f32, group: &MbBandGroup) -> Self {
        // XML ratio_raw is Q14 fixed point representing 1/ratio or compression slope;
        // clamp to a sensible 1.2..=8.0 compressor ratio.
        let inv_slope = (group.ratio_raw as f32 / 16384.0).clamp(0.125, 0.95);
        let ratio = (1.0 / inv_slope).clamp(1.1, 8.0);
        let attack_s = (group.attack_ms.max(1.0)) * 0.001;
        let release_s = (group.release_ms.max(5.0)) * 0.001;
        Self {
            threshold_db: group.threshold_db,
            ratio,
            makeup_lin: db_to_lin(group.makeup_db),
            attack_coeff: (-1.0 / (fs * attack_s)).exp(),
            release_coeff: (-1.0 / (fs * release_s)).exp(),
            env_db: -96.0,
        }
    }

    #[inline]
    fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        let peak_db = lin_to_db(l.abs().max(r.abs()));
        let coeff = if peak_db > self.env_db {
            self.attack_coeff
        } else {
            self.release_coeff
        };
        self.env_db = coeff * self.env_db + (1.0 - coeff) * peak_db;

        let gain_db = if self.env_db > self.threshold_db {
            let over = self.env_db - self.threshold_db;
            -(over * (1.0 - 1.0 / self.ratio))
        } else {
            0.0
        };
        let g = db_to_lin(gain_db) * self.makeup_lin;
        (l * g, r * g)
    }
}

/// 4-band stereo multiband compressor matching tuning-vlldp <mb-compressor-tuning>.
#[derive(Debug, Clone)]
pub struct MultibandCompressor {
    enabled: bool,
    xover_l: Vec<Lr4Crossover>,
    xover_r: Vec<Lr4Crossover>,
    bands: Vec<BandCompressor>,
}

impl MultibandCompressor {
    pub fn new(fs: f32, enabled: bool, groups: &[MbBandGroup], band_freqs: &[f32]) -> Self {
        if !enabled || groups.len() < 2 {
            return Self {
                enabled: false,
                xover_l: Vec::new(),
                xover_r: Vec::new(),
                bands: Vec::new(),
            };
        }

        let mut xover_l = Vec::with_capacity(groups.len() - 1);
        let mut xover_r = Vec::with_capacity(groups.len() - 1);
        for g in &groups[..groups.len() - 1] {
            let idx = g.upper_band_idx.min(band_freqs.len().saturating_sub(1));
            let freq = band_freqs.get(idx).copied().unwrap_or(1000.0);
            xover_l.push(Lr4Crossover::new(fs, freq));
            xover_r.push(Lr4Crossover::new(fs, freq));
        }

        let bands = groups.iter().map(|g| BandCompressor::new(fs, g)).collect();
        Self {
            enabled: true,
            xover_l,
            xover_r,
            bands,
        }
    }

    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        if !self.enabled || self.bands.is_empty() {
            return (l, r);
        }

        let mut rem_l = l;
        let mut rem_r = r;
        let mut sum_l = 0.0;
        let mut sum_r = 0.0;

        for i in 0..self.xover_l.len() {
            let (low_l, high_l) = self.xover_l[i].split(rem_l);
            let (low_r, high_r) = self.xover_r[i].split(rem_r);
            let (cl, cr) = self.bands[i].process_stereo(low_l, low_r);
            sum_l += cl;
            sum_r += cr;
            rem_l = high_l;
            rem_r = high_r;
        }

        let last = self.bands.len() - 1;
        let (cl, cr) = self.bands[last].process_stereo(rem_l, rem_r);
        (sum_l + cl, sum_r + cr)
    }
}

/// Dolby Volume Leveler (RMS loudness normalization toward out_target_db).
#[derive(Debug, Clone)]
pub struct VolumeLeveler {
    enabled: bool,
    amount: f32,
    target_db: f32,
    rms_sq: f32,
    rms_coeff: f32,
    gain_smooth: f32,
    gain_coeff: f32,
}

impl VolumeLeveler {
    pub fn new(fs: f32, enabled: bool, amount: f32, target_db: f32) -> Self {
        Self {
            enabled: enabled && amount > 0.0,
            amount: (amount / 10.0).clamp(0.0, 1.0),
            target_db,
            rms_sq: 1e-4,
            rms_coeff: (-1.0 / (fs * 0.35)).exp(),
            gain_smooth: 1.0,
            gain_coeff: (-1.0 / (fs * 0.15)).exp(),
        }
    }

    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        if !self.enabled {
            return (l, r);
        }

        let energy = 0.5 * (l * l + r * r);
        self.rms_sq = self.rms_coeff * self.rms_sq + (1.0 - self.rms_coeff) * energy;
        let rms_db = 10.0 * self.rms_sq.max(1e-8).log10();

        // Only ride gain when signal is above noise floor (-55 dBFS).
        let desired_gain_db = if rms_db > -55.0 {
            ((self.target_db - rms_db) * self.amount * 0.65).clamp(-9.0, 9.0)
        } else {
            0.0
        };
        let desired_lin = db_to_lin(desired_gain_db);
        self.gain_smooth = self.gain_coeff * self.gain_smooth + (1.0 - self.gain_coeff) * desired_lin;

        (l * self.gain_smooth, r * self.gain_smooth)
    }
}

/// Fast-attack output peak limiter to protect small laptop speakers from clipping.
#[derive(Debug, Clone)]
pub struct SafetyLimiter {
    ceiling_lin: f32,
    env: f32,
    attack_coeff: f32,
    release_coeff: f32,
}

impl SafetyLimiter {
    pub fn new(fs: f32, ceiling_db: f32) -> Self {
        Self {
            ceiling_lin: db_to_lin(ceiling_db),
            env: 0.0,
            attack_coeff: (-1.0 / (fs * 0.0005)).exp(),
            release_coeff: (-1.0 / (fs * 0.080)).exp(),
        }
    }

    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        let peak = l.abs().max(r.abs());
        let coeff = if peak > self.env {
            self.attack_coeff
        } else {
            self.release_coeff
        };
        self.env = coeff * self.env + (1.0 - coeff) * peak;

        let gain = if self.env > self.ceiling_lin {
            self.ceiling_lin / self.env
        } else {
            1.0
        };
        (
            (l * gain).clamp(-self.ceiling_lin, self.ceiling_lin),
            (r * gain).clamp(-self.ceiling_lin, self.ceiling_lin),
        )
    }
}
