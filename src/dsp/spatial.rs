use crate::dsp::biquad::Biquad;

/// Dialog Enhancer: boosts vocal presence (1.2 kHz - 3.5 kHz) on Mid channel.
#[derive(Debug, Clone)]
pub struct DialogEnhancer {
    enabled: bool,
    mid_filter: Biquad,
}

impl DialogEnhancer {
    pub fn new(fs: f32, enabled: bool, amount: f32) -> Self {
        let gain_db = (amount / 10.0).clamp(0.0, 1.0) * 3.5;
        Self {
            enabled: enabled && gain_db > 0.05,
            mid_filter: Biquad::peaking(fs, 2200.0, 0.9, gain_db),
        }
    }

    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        if !self.enabled {
            return (l, r);
        }
        let mid = 0.5 * (l + r);
        let side = 0.5 * (l - r);
        let mid_eq = self.mid_filter.process(mid);
        (mid_eq + side, mid_eq - side)
    }
}

/// Speaker Surround Virtualizer: subtle Mid/Side widening + crossfeed early reflection
/// to widen narrow laptop speaker spacing without phase hollowing.
#[derive(Debug, Clone)]
pub struct SurroundVirtualizer {
    enabled: bool,
    width: f32,
    cross_gain: f32,
    delay_l: Vec<f32>,
    delay_r: Vec<f32>,
    write_idx: usize,
    cross_lp_l: Biquad,
    cross_lp_r: Biquad,
}

impl SurroundVirtualizer {
    pub fn new(fs: f32, enabled: bool, boost_db: f32) -> Self {
        // 0.45 ms inter-aural / speaker crosstalk reflection delay
        let delay_samples = ((fs * 0.00045).round() as usize).max(1);
        let boost_norm = (boost_db / 6.0).clamp(0.0, 1.2);
        let width = 1.0 + 0.28 * boost_norm;
        let cross_gain = 0.14 * boost_norm;

        Self {
            enabled,
            width,
            cross_gain,
            delay_l: vec![0.0; delay_samples],
            delay_r: vec![0.0; delay_samples],
            write_idx: 0,
            cross_lp_l: Biquad::lowpass_q(fs, 3800.0, 0.707),
            cross_lp_r: Biquad::lowpass_q(fs, 3800.0, 0.707),
        }
    }

    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        if !self.enabled {
            return (l, r);
        }

        let mid = 0.5 * (l + r);
        let side = 0.5 * (l - r) * self.width;
        let wl = mid + side;
        let wr = mid - side;

        let delayed_l = self.delay_l[self.write_idx];
        let delayed_r = self.delay_r[self.write_idx];
        self.delay_l[self.write_idx] = wl;
        self.delay_r[self.write_idx] = wr;
        self.write_idx = (self.write_idx + 1) % self.delay_l.len();

        // Inverted phase-delayed crossfeed widens perceived stage on close speakers
        let out_l = wl - self.cross_gain * self.cross_lp_l.process(delayed_r);
        let out_r = wr - self.cross_gain * self.cross_lp_r.process(delayed_l);
        (out_l, out_r)
    }
}
