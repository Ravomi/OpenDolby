use std::f32::consts::PI;

/// Direct Form II Transposed Biquad filter (real-time safe, no heap allocations).
#[derive(Debug, Clone, Copy)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Default for Biquad {
    fn default() -> Self {
        Self::identity()
    }
}

impl Biquad {
    pub fn identity() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    /// Analytic magnitude of the quantized coefficients used by the real processor.
    pub fn response_db(&self, fs: f32, frequency: f32) -> f32 {
        let w = 2.0 * std::f64::consts::PI * frequency as f64 / fs as f64;
        let power = |c0: f32, c1: f32, c2: f32| {
            let re = c0 as f64 + c1 as f64 * w.cos() + c2 as f64 * (2.0 * w).cos();
            let im = -c1 as f64 * w.sin() - c2 as f64 * (2.0 * w).sin();
            re * re + im * im
        };
        (10.0
            * (power(self.b0, self.b1, self.b2).max(1e-30)
                / power(1.0, self.a1, self.a2).max(1e-30))
            .log10()) as f32
    }

    #[inline(always)]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    fn from_normalized(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        let inv_a0 = 1.0 / a0;
        Self {
            b0: b0 * inv_a0,
            b1: b1 * inv_a0,
            b2: b2 * inv_a0,
            a1: a1 * inv_a0,
            a2: a2 * inv_a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// Robert Bristow-Johnson Audio EQ Cookbook Peaking EQ.
    pub fn peaking(fs: f32, f0: f32, q: f32, gain_db: f32) -> Self {
        if gain_db.abs() < 1e-4 || f0 <= 0.0 || f0 >= fs * 0.495 {
            return Self::identity();
        }
        let a = 10.0_f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * f0 / fs;
        let alpha = w0.sin() / (2.0 * q.max(0.1));
        let cos_w0 = w0.cos();

        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha / a;
        Self::from_normalized(b0, b1, b2, a0, a1, a2)
    }

    /// 2nd-order High-Pass filter with explicit Q.
    pub fn highpass_q(fs: f32, f0: f32, q: f32) -> Self {
        if f0 <= 0.0 || f0 >= fs * 0.495 {
            return Self::identity();
        }
        let w0 = 2.0 * PI * f0 / fs;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q.max(0.1));

        let b0 = (1.0 + cos_w0) * 0.5;
        let b1 = -(1.0 + cos_w0);
        let b2 = (1.0 + cos_w0) * 0.5;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;
        Self::from_normalized(b0, b1, b2, a0, a1, a2)
    }

    /// 2nd-order Low-Pass filter with explicit Q.
    pub fn lowpass_q(fs: f32, f0: f32, q: f32) -> Self {
        if f0 <= 0.0 || f0 >= fs * 0.495 {
            return Self::identity();
        }
        let w0 = 2.0 * PI * f0 / fs;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q.max(0.1));

        let b0 = (1.0 - cos_w0) * 0.5;
        let b1 = 1.0 - cos_w0;
        let b2 = (1.0 - cos_w0) * 0.5;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;
        Self::from_normalized(b0, b1, b2, a0, a1, a2)
    }

    /// Butterworth Nth-order High-Pass cascade (e.g., order=8 -> 4 biquad stages).
    pub fn butterworth_highpass_cascade(fs: f32, f0: f32, order: u32) -> Vec<Self> {
        let stages = (order / 2).max(1);
        let n = (stages * 2) as f32;
        (0..stages)
            .map(|k| {
                let angle = PI * (2 * k + 1) as f32 / (2.0 * n);
                let q = 1.0 / (2.0 * angle.cos());
                Self::highpass_q(fs, f0, q)
            })
            .collect()
    }
}
