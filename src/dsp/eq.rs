//! Fitted graphic EQ approximation, not the proprietary Dolby filter bank.
//!
//! Merely changing Q cannot make a cascade hit a set of band targets. Fit the
//! *combined* response at the actual (nonuniform) XML frequencies instead.
use super::biquad::Biquad;

/// Prototype bandwidth. This is an implementation choice, not an OEM parameter.
pub const GRAPHIC_EQ_Q: f32 = 4.3;

pub fn fitted_graphic_eq(fs: f32, frequencies: &[f32], targets: &[f32]) -> Vec<Biquad> {
    let bands: Vec<_> = frequencies
        .iter()
        .copied()
        .zip(targets.iter().copied())
        .filter(|(f, g)| f.is_finite() && *f > 0.0 && *f < fs * 0.495 && g.is_finite())
        .map(|(f, g)| (f, g.clamp(-24.0, 24.0)))
        .collect();
    let n = bands.len();
    if n == 0 {
        return Vec::new();
    }
    let mut gains = vec![0.0_f32; n];
    // A 1 dB prototype estimates how much each filter affects every band center.
    // All fitting/allocation is control-thread work, never audio callback work.
    let influence: Vec<Vec<f64>> = bands
        .iter()
        .map(|(f, _)| {
            bands
                .iter()
                .map(|(center, _)| {
                    Biquad::peaking(fs, *center, GRAPHIC_EQ_Q, 1.0).response_db(fs, *f) as f64
                })
                .collect()
        })
        .collect();
    for _ in 0..10 {
        let filters: Vec<_> = bands
            .iter()
            .zip(&gains)
            .map(|((f, _), g)| Biquad::peaking(fs, *f, GRAPHIC_EQ_Q, *g))
            .collect();
        let residual: Vec<_> = bands
            .iter()
            .map(|(f, target)| {
                (*target - filters.iter().map(|b| b.response_db(fs, *f)).sum::<f32>()) as f64
            })
            .collect();
        if residual.iter().all(|r| r.abs() < 0.05) {
            break;
        }
        let Some(delta) = solve(influence.clone(), residual) else {
            break;
        };
        for (gain, correction) in gains.iter_mut().zip(delta) {
            *gain = (*gain + 0.8 * correction as f32).clamp(-24.0, 24.0);
        }
    }
    bands
        .iter()
        .zip(gains)
        .map(|((f, _), gain)| Biquad::peaking(fs, *f, GRAPHIC_EQ_Q, gain))
        .collect()
}

fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    for col in 0..b.len() {
        let pivot = (col..b.len()).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[pivot][col].abs() < 1e-8 {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        let (top, rest) = a.split_at_mut(col + 1);
        let pivot_row = &top[col];
        for (row_idx, row_vals) in rest.iter_mut().enumerate() {
            let row = col + 1 + row_idx;
            let factor = row_vals[col] / pivot_row[col];
            for (val, &piv) in row_vals[col..b.len()]
                .iter_mut()
                .zip(&pivot_row[col..b.len()])
            {
                *val -= factor * piv;
            }
            b[row] -= factor * b[col];
        }
    }
    let mut x = vec![0.0; b.len()];
    for row in (0..b.len()).rev() {
        x[row] = (b[row] - (row + 1..b.len()).map(|k| a[row][k] * x[k]).sum::<f64>()) / a[row][row];
    }
    Some(x)
}

#[cfg(test)]
mod tests {
    use super::*;
    const FREQS: [f32; 20] = [
        47., 141., 234., 328., 469., 656., 844., 1031., 1313., 1688., 2250., 3000., 3750., 4688.,
        5813., 7125., 9000., 11250., 13875., 19688.,
    ];
    #[test]
    fn fitted_centers_track_targets_instead_of_summing_boosts() {
        for fs in [44100.0, 48000.0] {
            let targets: Vec<_> = FREQS
                .iter()
                .enumerate()
                .map(|(i, _)| if i < 2 { -8.0 } else { 6.0 })
                .collect();
            let filters = fitted_graphic_eq(fs, &FREQS, &targets);
            let mut old_error = 0.0_f32;
            let mut error = 0.0_f32;
            for (&freq, &target) in FREQS.iter().zip(&targets) {
                let response: f32 = filters.iter().map(|b| b.response_db(fs, freq)).sum();
                let old: f32 = FREQS
                    .iter()
                    .zip(&targets)
                    .map(|(&f, &g)| Biquad::peaking(fs, f, 2.2, g).response_db(fs, freq))
                    .sum();
                error = error.max((response - target).abs());
                old_error = old_error.max((old - target).abs());
            }
            eprintln!("EQ fs={fs}: max center error old={old_error:.2} dB fitted={error:.2} dB");
            assert!(error < 0.25, "max target error {error} dB");
            assert!(error < old_error / 5.0);
        }
    }
}
