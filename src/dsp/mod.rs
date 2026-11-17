pub mod biquad;
pub mod dynamics;
pub mod spatial;

use crate::dsp::biquad::Biquad;
use crate::dsp::dynamics::{db_to_lin, MultibandCompressor, SafetyLimiter, VolumeLeveler};
use crate::dsp::spatial::{DialogEnhancer, SurroundVirtualizer};
use crate::tuning::{DeviceTuning, ProfileTuning};

/// Complete real-time safe stereo Dolby DAX3 DSP processor for a selected profile.
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
}

impl DolbyProcessor {
    pub fn from_tuning(tuning: &DeviceTuning, profile_name: &str, sample_rate: u32) -> Option<Self> {
        let profile = tuning.profiles.get(profile_name)?;
        let freqs = tuning.band_freqs(sample_rate);
        Some(Self::new(profile, freqs, sample_rate as f32))
    }

    pub fn new(profile: &ProfileTuning, band_freqs: &[f32], fs: f32) -> Self {
        let bypass = profile.name == "off";
        let cp = &profile.cp;
        let vlldp = &profile.vlldp;

        // 1. Intelligent EQ / Graphic EQ (20-band Peaking cascade, Q ~ 2.2)
        let mut ieq_l = Vec::new();
        let mut ieq_r = Vec::new();
        if cp.ieq_enable && cp.ieq_amount > 0.0 {
            let scale = (cp.ieq_amount / 10.0).clamp(0.0, 1.0) * 0.35;
            for (i, &f0) in band_freqs.iter().enumerate() {
                let db = cp.ieq_target_db.get(i).copied().unwrap_or(0.0) * scale;
                if db.abs() >= 0.1 {
                    ieq_l.push(Biquad::peaking(fs, f0, 2.2, db));
                    ieq_r.push(Biquad::peaking(fs, f0, 2.2, db));
                }
            }
        }
        if cp.geq_enable {
            for (i, &f0) in band_freqs.iter().enumerate() {
                let db = cp.geq_db.get(i).copied().unwrap_or(0.0);
                if db.abs() >= 0.1 {
                    ieq_l.push(Biquad::peaking(fs, f0, 2.2, db));
                    ieq_r.push(Biquad::peaking(fs, f0, 2.2, db));
                }
            }
        }

        // 2. VLLDP Audio Optimizer (20-band speaker calibration per channel)
        let mut optimizer_l = Vec::new();
        let mut optimizer_r = Vec::new();
        if vlldp.audio_optimizer_enable {
            for (i, &f0) in band_freqs.iter().enumerate() {
                let db_l = vlldp.audio_optimizer_ch0_db.get(i).copied().unwrap_or(0.0);
                let db_r = vlldp.audio_optimizer_ch1_db.get(i).copied().unwrap_or(0.0);
                if db_l.abs() >= 0.1 {
                    optimizer_l.push(Biquad::peaking(fs, f0, 2.2, db_l));
                }
                if db_r.abs() >= 0.1 {
                    optimizer_r.push(Biquad::peaking(fs, f0, 2.2, db_r));
                }
            }
        }

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
                        fs,
                        spec.f0,
                        spec.order,
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
        }
    }

    /// Process stereo interleaved f32 buffer in-place (zero heap allocations).
    #[inline]
    pub fn process_interleaved(&mut self, samples: &mut [f32]) {
        if self.bypass {
            return;
        }

        for frame in samples.chunks_exact_mut(2) {
            let mut l = frame[0] * self.pregain_lin;
            let mut r = frame[1] * self.pregain_lin;

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

            (l, r) = self.limiter.process_stereo(l, r);
            frame[0] = l;
            frame[1] = r;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    const LOCAL_XML: &str =
        "/home/waya/Downloads/dolby-research/payload/ext_asus/DEV_0294_SUBSYS_10431693_PCI_SUBSYS_16931043.xml";

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
        let tuning = DeviceTuning::from_xml_file(LOCAL_XML).expect("parse local XML");

        let mut proc_dyn =
            DolbyProcessor::from_tuning(&tuning, "dynamic", 48000).expect("dynamic profile");
        let low = gen_sine(50.0, 48000.0, 4800, 0.25);
        let mut low_out = low.clone();
        proc_dyn.process_interleaved(&mut low_out);
        assert!(low_out.iter().all(|x| x.is_finite()));
        // 130 Hz 8th-order HPF + optimizer (-14 dB at 47 Hz) must strongly attenuate 50 Hz
        assert!(rms(&low_out[2400..]) < rms(&low[2400..]) * 0.2);

        let mut proc_off =
            DolbyProcessor::from_tuning(&tuning, "off", 48000).expect("off profile");
        let mut off_out = low.clone();
        proc_off.process_interleaved(&mut off_out);
        assert_eq!(off_out, low);
    }
}
