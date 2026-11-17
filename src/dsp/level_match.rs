//! Approximate stereo RMS A/B alignment. Not LUFS or a perceptual loudness meter.
//! Compare both continuously running paths at the quieter RMS. Neither path is
//! boosted and a final output limiter remains active in either audition mode.
#[derive(Debug, Clone)]
pub struct LevelMatch {
    dry_power: f64,
    wet_power: f64,
    power_coeff: f64,
    gain_coeff: f64,
    dry_gain: f64,
    wet_gain: f64,
    dry_target: f64,
    wet_target: f64,
    counter: usize,
}

impl LevelMatch {
    pub fn new(fs: f32) -> Self {
        let fs = fs as f64;
        Self {
            dry_power: 0.0,
            wet_power: 0.0,
            power_coeff: (-1.0 / (fs * 0.3)).exp(),
            gain_coeff: (-1.0 / (fs * 0.05)).exp(),
            dry_gain: 1.0,
            wet_gain: 1.0,
            dry_target: 1.0,
            wet_target: 1.0,
            counter: 0,
        }
    }

    pub fn process(
        &mut self,
        dry: (f32, f32),
        wet: (f32, f32),
        enabled: bool,
    ) -> ((f32, f32), (f32, f32)) {
        let power = |x: (f32, f32)| 0.5 * ((x.0 as f64).powi(2) + (x.1 as f64).powi(2));
        self.dry_power += (1.0 - self.power_coeff) * (power(dry) - self.dry_power);
        self.wet_power += (1.0 - self.power_coeff) * (power(wet) - self.wet_power);
        self.counter = (self.counter + 1) % 64;
        if !enabled {
            self.dry_target = 1.0;
            self.wet_target = 1.0;
        } else if self.counter == 0 && self.dry_power > 1e-10 && self.wet_power > 1e-10 {
            self.dry_target = (self.wet_power / self.dry_power).sqrt().min(1.0);
            self.wet_target = (self.dry_power / self.wet_power).sqrt().min(1.0);
        }
        self.dry_gain += (1.0 - self.gain_coeff) * (self.dry_target - self.dry_gain);
        self.wet_gain += (1.0 - self.gain_coeff) * (self.wet_target - self.wet_gain);
        (
            (dry.0 * self.dry_gain as f32, dry.1 * self.dry_gain as f32),
            (wet.0 * self.wet_gain as f32, wet.1 * self.wet_gain as f32),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_both_louder_and_quieter_wet_without_boosting() {
        for factor in [0.1, 0.5, 2.0, 8.0] {
            let mut m = LevelMatch::new(48000.0);
            let mut result = ((0.0, 0.0), (0.0, 0.0));
            for _ in 0..144000 {
                result = m.process((0.1, 0.1), (0.1 * factor, 0.1 * factor), true);
            }
            let ratio = result.0 .0 / result.1 .0;
            assert!((20.0 * ratio.log10()).abs() < 0.1);
            assert!(result.0 .0 <= 0.10001);
            assert!(result.1 .0 <= 0.10001_f32.min(0.1 * factor + 0.00001));
        }
    }
    #[test]
    fn silence_and_toggle_are_finite() {
        let mut m = LevelMatch::new(48000.0);
        for _ in 0..48000 {
            assert_eq!(m.process((0., 0.), (0., 0.), true), ((0., 0.), (0., 0.)));
        }
        for _ in 0..48000 {
            m.process((0.1, 0.1), (0.8, 0.8), true);
        }
        let mut result = ((0., 0.), (0., 0.));
        for _ in 0..48000 {
            result = m.process((0.1, 0.1), (0.8, 0.8), false);
        }
        assert!((result.1 .0 - 0.8).abs() < 0.001);
    }
}
