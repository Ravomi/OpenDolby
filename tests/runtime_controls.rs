//! Portable synthetic tuning tests. No downloaded OEM files or audio server needed.
use dolby_rs::dsp::{DolbyProcessor, RuntimeControls};
use dolby_rs::tuning::DeviceTuning;

fn tuning() -> DeviceTuning {
    DeviceTuning::from_xml_str(r#"<device_data>
    <constant><band_20_freq fs_44100="47,141,234,328,469,656,844,1031,1313,1688,2250,3000,3750,4688,5813,7125,9000,11025,13875,18777" fs_48000="47,141,234,328,469,656,844,1031,1313,1688,2250,3000,3750,4688,5813,7125,9000,11250,13875,19688" /></constant>
    <endpoint type="internal_speaker"><profile type="test"><tuning-cp>
    <dialog-enhancer-enable value="1"/><dialog-enhancer-amount value="8"/>
    <surround-decoder-enable value="1"/><surround-boost value="96"/>
    <postgain value="96"/>
    </tuning-cp></profile><profile type="off"/></endpoint></device_data>"#).unwrap()
}

fn signal(frames: usize) -> Vec<f32> {
    (0..frames)
        .flat_map(|n| {
            let s = (n as f32 * 2.0 * std::f32::consts::PI * 1800.0 / 48000.0).sin() * 0.05;
            [s, 0.65 * s]
        })
        .collect()
}
fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
}

#[test]
fn matched_bypass_and_wet_have_same_rms_but_different_waveforms() {
    let t = tuning();
    let input = signal(192000);
    let mut wet = DolbyProcessor::from_tuning(&t, "test", 48000).unwrap();
    let mut dry = wet.clone();
    dry.set_controls(RuntimeControls {
        bypass: true,
        ..Default::default()
    });
    let mut a = input.clone();
    let mut b = input.clone();
    wet.process_interleaved(&mut a);
    dry.process_interleaved(&mut b);
    let a = &a[288000..];
    let b = &b[288000..];
    let delta = 20.0 * (rms(a) / rms(b)).log10();
    eprintln!("steady wet / matched dry RMS delta {delta:.4} dB");
    assert!(delta.abs() < 0.2);
    assert!(a.iter().zip(b).any(|(a, b)| (a - b).abs() > 0.001));
}

#[test]
fn block_size_does_not_change_processing_and_live_controls_remain_finite() {
    let t = tuning();
    let input = signal(24000);
    let mut a = input.clone();
    let mut b = input;
    let mut full = DolbyProcessor::from_tuning(&t, "test", 48000).unwrap();
    let mut blocks = full.clone();
    full.process_interleaved(&mut a);
    for block in b.chunks_mut(258) {
        blocks.process_interleaved(block);
    }
    assert_eq!(a, b);
    for i in 0..8 {
        blocks.set_controls(RuntimeControls {
            bypass: i % 2 == 0,
            level_match: i % 3 != 0,
            surround_amount: if i == 7 { f32::NAN } else { i as f32 - 2.0 },
            dialog_amount: if i == 6 {
                f32::INFINITY
            } else {
                i as f32 / 2.0
            },
        });
        blocks.process_interleaved(&mut b);
        assert!(b.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
    }
}

#[test]
fn off_profile_is_flat_even_if_controls_are_maximized() {
    let t = tuning();
    let input = signal(5000);
    let mut out = input.clone();
    let mut flat = DolbyProcessor::from_tuning(&t, "off", 48000).unwrap();
    flat.set_controls(RuntimeControls {
        surround_amount: 2.0,
        dialog_amount: 2.0,
        ..Default::default()
    });
    flat.process_interleaved(&mut out);
    assert_eq!(input, out);
}
