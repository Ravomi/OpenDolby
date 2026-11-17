use crate::dsp::biquad::Biquad;

/// Dialog Enhancer: boosts vocal presence (1.2 kHz - 3.5 kHz) on Mid channel.
#[derive(Debug, Clone)]
pub struct DialogEnhancer {
    enabled: bool,
    mid_filter: Biquad,
    amount: f32,
    target_amount: f32,
    smoothing: f32,
}

impl DialogEnhancer {
    pub fn new(fs: f32, enabled: bool, amount: f32) -> Self {
        let gain_db = (amount / 10.0).clamp(0.0, 1.0) * 3.5;
        Self {
            enabled: enabled && gain_db > 0.05,
            mid_filter: Biquad::peaking(fs, 2200.0, 0.9, gain_db),
            amount: 1.0,
            target_amount: 1.0,
            smoothing: 1.0 - (-1.0 / (fs * 0.02)).exp(),
        }
    }

    /// 0 disables this effect, 1 uses the profile, 2 doubles its contribution.
    /// It does not force on an effect disabled in the XML profile.
    pub fn set_amount(&mut self, amount: f32) {
        self.target_amount = if amount.is_finite() {
            amount.clamp(0.0, 2.0)
        } else {
            1.0
        };
    }

    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        if !self.enabled {
            return (l, r);
        }
        let mid = 0.5 * (l + r);
        let side = 0.5 * (l - r);
        self.amount += self.smoothing * (self.target_amount - self.amount);
        let filtered = self.mid_filter.process(mid);
        let mid_eq = mid + self.amount * (filtered - mid);
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
    amount: f32,
    target_amount: f32,
    smoothing: f32,
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
            amount: 1.0,
            target_amount: 1.0,
            smoothing: 1.0 - (-1.0 / (fs * 0.02)).exp(),
        }
    }

    /// Live strength multiplier, smoothed over 20 ms to avoid zipper noise.
    pub fn set_amount(&mut self, amount: f32) {
        self.target_amount = if amount.is_finite() {
            amount.clamp(0.0, 2.0)
        } else {
            1.0
        };
    }

    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        if !self.enabled {
            return (l, r);
        }

        let mid = 0.5 * (l + r);
        self.amount += self.smoothing * (self.target_amount - self.amount);
        let side = 0.5 * (l - r) * (1.0 + (self.width - 1.0) * self.amount);
        let wl = mid + side;
        let wr = mid - side;

        let delayed_l = self.delay_l[self.write_idx];
        let delayed_r = self.delay_r[self.write_idx];
        self.delay_l[self.write_idx] = wl;
        self.delay_r[self.write_idx] = wr;
        self.write_idx = (self.write_idx + 1) % self.delay_l.len();

        // Inverted phase-delayed crossfeed widens perceived stage on close speakers
        let out_l = wl - self.amount * self.cross_gain * self.cross_lp_l.process(delayed_r);
        let out_r = wr - self.amount * self.cross_gain * self.cross_lp_r.process(delayed_l);
        (out_l, out_r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_strength_zero_is_dry_and_disabled_profiles_stay_disabled() {
        let mut dialog = DialogEnhancer::new(48000.0, true, 5.0);
        let mut surround = SurroundVirtualizer::new(48000.0, true, 6.0);
        dialog.set_amount(0.0);
        surround.set_amount(0.0);
        let mut de = 0.0_f32;
        let mut se = 0.0_f32;
        for n in 0..24000 {
            let x = 0.1 * (n as f32 * 0.3).sin();
            let (l, r) = dialog.process_stereo(x, x * 0.7);
            let (sl, sr) = surround.process_stereo(x, x * 0.7);
            if n > 12000 {
                de = de.max((l - x).abs()).max((r - x * 0.7).abs());
                se = se.max((sl - x).abs()).max((sr - x * 0.7).abs());
            }
        }
        assert!(de < 1e-5 && se < 1e-5, "de={de} se={se}");
        let mut disabled = SurroundVirtualizer::new(48000.0, false, 6.0);
        disabled.set_amount(2.0);
        assert_eq!(disabled.process_stereo(0.1, 0.2), (0.1, 0.2));
    }
    #[test]
    fn increasing_strength_changes_output() {
        let mut soft = SurroundVirtualizer::new(48000.0, true, 6.0);
        let mut strong = soft.clone();
        soft.set_amount(0.0);
        strong.set_amount(2.0);
        let mut error = 0.0;
        for n in 0..12000 {
            let x = 0.1 * (n as f32 * 0.1).sin();
            let a = soft.process_stereo(x, -0.3 * x);
            let b = strong.process_stereo(x, -0.3 * x);
            if n > 6000 {
                error += (a.0 - b.0).abs() + (a.1 - b.1).abs();
            }
        }
        assert!(error > 1.0);
    }
}
