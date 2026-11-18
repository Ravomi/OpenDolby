pub mod biquad;
pub mod dynamics;
pub mod eq;
pub mod level_match;
pub mod spatial;

use crate::dsp::biquad::Biquad;
use crate::dsp::dynamics::{db_to_lin, MultibandCompressor, SafetyLimiter, VolumeLeveler};
use crate::dsp::spatial::{DialogEnhancer, SurroundVirtualizer};
use crate::tuning::{DeviceTuning, ProfileTuning};
use eq::fitted_graphic_eq;
use level_match::LevelMatch;

/// Live controls. Multipliers scale only effects enabled by the selected profile.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeControls {
    pub bypass: bool,
    pub level_match: bool,
    pub surround_amount: f32,
    pub dialog_amount: f32,
}

impl Default for RuntimeControls {
    fn default() -> Self {
        Self {
            bypass: false,
            level_match: true,
            surround_amount: 1.0,
            dialog_amount: 1.0,
        }
    }
}

/// Experimental stereo OEM tuning approximation, not a Dolby Atmos renderer.
/// Processing allocates no memory. Construction/fitting stays off the audio path.
#[derive(Debug, Clone)]
pub struct DolbyProcessor {
    pub profile_name: String,
    pub bypass: bool,
    pregain_lin: f32,
    postgain_lin: f32,
    // CP stage
    volume_leveler: VolumeLeveler,
    dialog_enhancer: DialogEnhancer,
    ieq_l: Vec<Biquad>,
    ieq_r: Vec<Biquad>,
    surround_virtualizer: SurroundVirtualizer,
    // VLLDP stage
    optimizer_l: Vec<Biquad>,
    optimizer_r: Vec<Biquad>,
    speaker_peq_l: Vec<Biquad>,
    speaker_peq_r: Vec<Biquad>,
    mb_compressor: MultibandCompressor,
    limiter: SafetyLimiter,
    controls: RuntimeControls,
    level_match: LevelMatch,
    wet_mix: f32,
    mix_step: f32,
}

impl DolbyProcessor {
    pub fn from_tuning(
        tuning: &DeviceTuning,
        profile_name: &str,
        sample_rate: u32,
    ) -> Option<Self> {
        let profile = tuning.profiles.get(profile_name)?;
        let freqs = tuning.band_freqs(sample_rate);
        Some(Self::new(profile, freqs, sample_rate as f32))
    }

    pub fn new(profile: &ProfileTuning, band_freqs: &[f32], fs: f32) -> Self {
        let bypass = profile.name == "off";
        let cp = &profile.cp;
        let vlldp = &profile.vlldp;

        // 1. Fit combined EQ response at band centers. The XML frequencies are
        // nonuniform, so assigning every filter its target dB overcounts neighbors.
        let mut cp_target = vec![0.0; band_freqs.len()];
        if cp.ieq_enable && cp.ieq_amount > 0.0 {
            // Static approximation of the proprietary content-adaptive IEQ.
            let scale = (cp.ieq_amount / 10.0).clamp(0.0, 1.0) * 0.35;
            for (i, db) in cp_target.iter_mut().enumerate() {
                *db += cp.ieq_target_db.get(i).copied().unwrap_or(0.0) * scale;
            }
        }
        if cp.geq_enable {
            for (i, db) in cp_target.iter_mut().enumerate() {
                *db += cp.geq_db.get(i).copied().unwrap_or(0.0);
            }
        }
        let ieq_l = fitted_graphic_eq(fs, band_freqs, &cp_target);
        let ieq_r = ieq_l.clone();

        // 2. VLLDP Audio Optimizer (20-band speaker calibration per channel)
        let (optimizer_l, optimizer_r) = if vlldp.audio_optimizer_enable {
            (
                fitted_graphic_eq(fs, band_freqs, &vlldp.audio_optimizer_ch0_db),
                fitted_graphic_eq(fs, band_freqs, &vlldp.audio_optimizer_ch1_db),
            )
        } else {
            (Vec::new(), Vec::new())
        };

        // 3. VLLDP Speaker PEQ (e.g., 130Hz 8th-order HPF + 4 hardware PEQ notches)
        let mut speaker_peq_l = Vec::new();
        let mut speaker_peq_r = Vec::new();
        if vlldp.speaker_peq_enable {
            for spec in &vlldp.speaker_peq_filters {
                if !spec.enabled {
                    continue;
                }
                let target = if spec.speaker == 0 {
                    &mut speaker_peq_l
                } else {
                    &mut speaker_peq_r
                };
                match spec.filter_type {
                    9 => target.extend(Biquad::butterworth_highpass_cascade(
                        fs, spec.f0, spec.order,
                    )),
                    _ => target.push(Biquad::peaking(fs, spec.f0, spec.q, spec.gain_db)),
                }
            }
        }

        let pregain_lin = db_to_lin(cp.pregain_db);
        let postgain_lin = db_to_lin(cp.postgain_db + vlldp.postgain_db + vlldp.system_gain_db);

        Self {
            profile_name: profile.name.clone(),
            bypass,
            pregain_lin,
            postgain_lin,
            volume_leveler: VolumeLeveler::new(
                fs,
                cp.volume_leveler_enable,
                cp.volume_leveler_amount,
                cp.volume_leveler_out_target_db,
            ),
            dialog_enhancer: DialogEnhancer::new(
                fs,
                cp.dialog_enhancer_enable,
                cp.dialog_enhancer_amount,
            ),
            ieq_l,
            ieq_r,
            surround_virtualizer: SurroundVirtualizer::new(
                fs,
                cp.surround_virtualizer_enable || cp.surround_decoder_enable,
                cp.surround_boost_db,
            ),
            optimizer_l,
            optimizer_r,
            speaker_peq_l,
            speaker_peq_r,
            mb_compressor: MultibandCompressor::new(
                fs,
                vlldp.mb_compressor_enable,
                &vlldp.mb_compressor_groups,
                band_freqs,
            ),
            limiter: SafetyLimiter::new(fs, -0.5),
            controls: RuntimeControls::default(),
            level_match: LevelMatch::new(fs),
            wet_mix: if bypass { 0.0 } else { 1.0 },
            mix_step: 1.0 / (fs * 0.02),
        }
    }

    /// Atomic snapshots from the streaming layer arrive once per audio block.
    pub fn set_controls(&mut self, controls: RuntimeControls) {
        self.controls = controls;
        self.dialog_enhancer.set_amount(controls.dialog_amount);
        self.surround_virtualizer
            .set_amount(controls.surround_amount);
    }

    /// Process stereo interleaved f32 buffer in-place (zero heap allocations).
    #[inline]
    pub fn process_interleaved(&mut self, samples: &mut [f32]) {
        if self.bypass {
            // Profile "off" is genuinely flat (no hidden dynamics or width).
            return;
        }

        for frame in samples.as_chunks_mut::<2>().0 {
            let dry = (
                if frame[0].is_finite() {
                    frame[0].clamp(-16.0, 16.0)
                } else {
                    0.0
                },
                if frame[1].is_finite() {
                    frame[1].clamp(-16.0, 16.0)
                } else {
                    0.0
                },
            );
            let mut l = dry.0 * self.pregain_lin;
            let mut r = dry.1 * self.pregain_lin;

            // CP stage
            (l, r) = self.volume_leveler.process_stereo(l, r);
            (l, r) = self.dialog_enhancer.process_stereo(l, r);
            for bq in &mut self.ieq_l {
                l = bq.process(l);
            }
            for bq in &mut self.ieq_r {
                r = bq.process(r);
            }
            (l, r) = self.surround_virtualizer.process_stereo(l, r);

            // VLLDP stage
            for bq in &mut self.optimizer_l {
                l = bq.process(l);
            }
            for bq in &mut self.optimizer_r {
                r = bq.process(r);
            }
            for bq in &mut self.speaker_peq_l {
                l = bq.process(l);
            }
            for bq in &mut self.speaker_peq_r {
                r = bq.process(r);
            }
            (l, r) = self.mb_compressor.process_stereo(l, r);

            l *= self.postgain_lin;
            r *= self.postgain_lin;

            // Keep the processed path/envelopes warm while auditioning dry.
            // Compare at the quieter RMS and never increase either path's gain.
            let (dry, wet) = self
                .level_match
                .process(dry, (l, r), self.controls.level_match);
            let target_mix = if self.controls.bypass { 0.0 } else { 1.0 };
            self.wet_mix += (target_mix - self.wet_mix).clamp(-self.mix_step, self.mix_step);
            l = dry.0 + self.wet_mix * (wet.0 - dry.0);
            r = dry.1 + self.wet_mix * (wet.1 - dry.1);
            (l, r) = self.limiter.process_stereo(l, r);
            frame[0] = l;
            frame[1] = r;
        }
    }
}

const DECK_CHUNK_FLOATS: usize = 512;

/// Wraps [`DolbyProcessor`] to perform a 20 ms linear crossfade when hot-swapping presets,
/// avoiding filter-state reset clicks while allocating zero heap memory on the audio path.
pub struct ProcessorDeck {
    active: DolbyProcessor,
    outgoing: Option<DolbyProcessor>,
    retired: Option<DolbyProcessor>,
    fade_pos: usize,
    fade_frames: usize,
}

impl ProcessorDeck {
    pub fn new(active: DolbyProcessor, sample_rate: u32) -> Self {
        let fade_frames = ((sample_rate as usize) / 50).max(1);
        Self {
            active,
            outgoing: None,
            retired: None,
            fade_pos: fade_frames,
            fade_frames,
        }
    }

    /// Initiate a 20 ms crossfade to `next`. Intended to be called from the control timer,
    /// where dropping any retired processor is off the audio callback.
    pub fn swap(&mut self, next: DolbyProcessor) {
        self.retired = None;
        let prev = std::mem::replace(&mut self.active, next);
        self.outgoing = Some(prev);
        self.fade_pos = 0;
    }

    /// Drop any processor that finished crossfading so deallocation stays off the audio path.
    pub fn reap_retired(&mut self) {
        self.retired = None;
    }

    #[inline]
    pub fn process_interleaved(&mut self, controls: RuntimeControls, samples: &mut [f32]) {
        self.active.set_controls(controls);
        let Some(mut old) = self.outgoing.take() else {
            self.active.process_interleaved(samples);
            return;
        };
        old.set_controls(controls);

        for chunk in samples.chunks_mut(DECK_CHUNK_FLOATS) {
            if self.fade_pos >= self.fade_frames {
                self.active.process_interleaved(chunk);
                continue;
            }
            let mut old_buf = [0.0_f32; DECK_CHUNK_FLOATS];
            let n = chunk.len();
            old_buf[..n].copy_from_slice(chunk);
            old.process_interleaved(&mut old_buf[..n]);
            self.active.process_interleaved(chunk);

            let inv_len = 1.0 / (self.fade_frames as f32);
            for (i, frame) in chunk.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                let alpha = ((self.fade_pos as f32) * inv_len).min(1.0);
                frame[0] = old_buf[i * 2] * (1.0 - alpha) + frame[0] * alpha;
                frame[1] = old_buf[i * 2 + 1] * (1.0 - alpha) + frame[1] * alpha;
                if self.fade_pos < self.fade_frames {
                    self.fade_pos += 1;
                }
            }
        }

        if self.fade_pos >= self.fade_frames {
            self.retired = Some(old);
        } else {
            self.outgoing = Some(old);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn gen_sine(freq: f32, fs: f32, frames: usize, amp: f32) -> Vec<f32> {
        let mut out = Vec::with_capacity(frames * 2);
        for n in 0..frames {
            let s = amp * (2.0 * PI * freq * n as f32 / fs).sin();
            out.push(s);
            out.push(s);
        }
        out
    }

    fn rms(buf: &[f32]) -> f32 {
        let sum: f32 = buf.iter().map(|x| x * x).sum();
        (sum / buf.len() as f32).sqrt()
    }

    #[test]
    fn test_local_xml_and_dsp_hpf_and_bypass() {
        let xml_path = crate::default_xml_path();
        if !xml_path.exists() {
            eprintln!(
                "skipping test_local_xml_and_dsp_hpf_and_bypass: {:?} not found",
                xml_path
            );
            return;
        }
        let tuning = DeviceTuning::from_xml_file(&xml_path).expect("parse local XML");

        let mut proc_dyn =
            DolbyProcessor::from_tuning(&tuning, "dynamic", 48000).expect("dynamic profile");
        let low = gen_sine(50.0, 48000.0, 4800, 0.25);
        let mut low_out = low.clone();
        proc_dyn.process_interleaved(&mut low_out);
        assert!(low_out.iter().all(|x| x.is_finite()));
        // 130 Hz 8th-order HPF + optimizer (-14 dB at 47 Hz) must strongly attenuate 50 Hz
        assert!(rms(&low_out[2400..]) < rms(&low[2400..]) * 0.2);

        let mut proc_off = DolbyProcessor::from_tuning(&tuning, "off", 48000).expect("off profile");
        let mut off_out = low.clone();
        proc_off.process_interleaved(&mut off_out);
        assert_eq!(off_out, low);
    }

    #[test]
    fn processor_deck_crossfades_preset_swap_without_step_click() {
        use crate::tuning::{CpTuning, ProfileTuning, VlldpTuning};

        let make_profile = |name: &str, postgain_db: f32| ProfileTuning {
            name: name.to_string(),
            cp: CpTuning {
                dialog_enhancer_enable: false,
                dialog_enhancer_amount: 0.0,
                ieq_enable: false,
                ieq_amount: 0.0,
                ieq_target_db: vec![0.0; 20],
                geq_enable: false,
                geq_db: vec![0.0; 20],
                volume_leveler_enable: false,
                volume_leveler_amount: 0.0,
                volume_leveler_in_target_db: -20.0,
                volume_leveler_out_target_db: -20.0,
                surround_decoder_enable: false,
                surround_virtualizer_enable: false,
                surround_boost_db: 0.0,
                pregain_db: 0.0,
                postgain_db,
            },
            vlldp: VlldpTuning {
                audio_optimizer_enable: false,
                audio_optimizer_ch0_db: vec![0.0; 20],
                audio_optimizer_ch1_db: vec![0.0; 20],
                speaker_peq_enable: false,
                speaker_peq_filters: Vec::new(),
                mb_compressor_enable: false,
                mb_compressor_groups: Vec::new(),
                regulator_speaker_dist_enable: false,
                regulator_threshold_high_db: vec![0.0; 20],
                regulator_threshold_low_db: vec![-12.0; 20],
                postgain_db: 0.0,
                system_gain_db: 0.0,
            },
        };

        let freqs = [100.0_f32; 20];
        let controls = RuntimeControls {
            level_match: false,
            ..RuntimeControls::default()
        };
        let proc_quiet = DolbyProcessor::new(&make_profile("quiet", -18.0), &freqs, 48_000.0);
        let proc_off = DolbyProcessor::new(&make_profile("off", 0.0), &freqs, 48_000.0);

        let mut deck = ProcessorDeck::new(proc_quiet, 48_000);
        // Constant DC-like test signal of 0.4 to detect instantaneous step discontinuities
        let mut pre = vec![0.4_f32; 512];
        deck.process_interleaved(controls, &mut pre);
        let last_before_swap = pre[pre.len() - 2];

        deck.swap(proc_off);
        let mut fade_buf = vec![0.4_f32; 2400]; // 1200 stereo frames (> 960-frame 20ms crossfade)
        deck.process_interleaved(controls, &mut fade_buf);

        // First sample after swap must start at outgoing level (alpha = 0), not jump by ~0.35
        assert!((fade_buf[0] - last_before_swap).abs() < 0.01);
        // Sample-to-sample delta across the entire crossfade must remain tiny
        let max_step = fade_buf
            .as_chunks::<2>()
            .0
            .iter()
            .map(|f| f[0])
            .collect::<Vec<_>>()
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0_f32, f32::max);
        assert!(max_step < 0.005, "unexpected step jump: {max_step}");
        // After 960 frames, output must have settled cleanly onto incoming "off" (0.4)
        assert!((fade_buf[fade_buf.len() - 2] - 0.4).abs() < 1e-5);
        deck.reap_retired();
    }
}
