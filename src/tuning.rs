use anyhow::{anyhow, Context, Result};
use roxmltree::{Document, Node};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Parsed Dolby DAX3 device tuning configuration for a specific endpoint & profile.
#[derive(Debug, Clone)]
pub struct DeviceTuning {
    pub xml_version: String,
    pub tuning_date: String,
    pub band_20_freq_44100: Vec<f32>,
    pub band_20_freq_48000: Vec<f32>,
    pub presets: HashMap<String, Vec<i32>>,
    pub profiles: HashMap<String, ProfileTuning>,
}

#[derive(Debug, Clone)]
pub struct ProfileTuning {
    pub name: String,
    pub cp: CpTuning,
    pub vlldp: VlldpTuning,
}

#[derive(Debug, Clone)]
pub struct CpTuning {
    pub dialog_enhancer_enable: bool,
    pub dialog_enhancer_amount: f32,
    pub ieq_enable: bool,
    pub ieq_amount: f32,
    pub ieq_target_db: Vec<f32>,
    pub geq_enable: bool,
    pub geq_db: Vec<f32>,
    pub volume_leveler_enable: bool,
    pub volume_leveler_amount: f32,
    pub volume_leveler_in_target_db: f32,
    pub volume_leveler_out_target_db: f32,
    pub surround_decoder_enable: bool,
    pub surround_virtualizer_enable: bool,
    pub surround_boost_db: f32,
    pub pregain_db: f32,
    pub postgain_db: f32,
}

#[derive(Debug, Clone)]
pub struct VlldpTuning {
    pub audio_optimizer_enable: bool,
    pub audio_optimizer_ch0_db: Vec<f32>,
    pub audio_optimizer_ch1_db: Vec<f32>,
    pub speaker_peq_enable: bool,
    pub speaker_peq_filters: Vec<PeqFilterSpec>,
    pub mb_compressor_enable: bool,
    pub mb_compressor_groups: Vec<MbBandGroup>,
    pub regulator_speaker_dist_enable: bool,
    pub regulator_threshold_high_db: Vec<f32>,
    pub regulator_threshold_low_db: Vec<f32>,
    pub postgain_db: f32,
    pub system_gain_db: f32,
}

#[derive(Debug, Clone)]
pub struct PeqFilterSpec {
    pub speaker: usize,
    pub enabled: bool,
    /// Dolby filter type: 1 = peaking EQ, 9 = Butterworth high-pass
    pub filter_type: u32,
    pub f0: f32,
    pub gain_db: f32,
    pub q: f32,
    pub order: u32,
}

#[derive(Debug, Clone)]
pub struct MbBandGroup {
    pub upper_band_idx: usize,
    pub threshold_db: f32,
    /// Fixed-point ratio parameter from XML (scaled by 16384 in DAX3)
    pub ratio_raw: i32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub makeup_db: f32,
}

impl DeviceTuning {
    pub fn from_xml_file(path: impl AsRef<Path>) -> Result<Self> {
        let xml_text = fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read XML: {}", path.as_ref().display()))?;
        Self::from_xml_str(&xml_text)
    }

    pub fn from_xml_str(xml_text: &str) -> Result<Self> {
        let doc = Document::parse(xml_text).context("Failed to parse Dolby tuning XML")?;
        let root = doc.root_element();

        let xml_version = child_attr(root, "xml_version", "value").unwrap_or_default();
        let tuning_date = child_attr(root, "tuning_date", "value").unwrap_or_default();

        let const_node = root
            .children()
            .find(|n| n.has_tag_name("constant"))
            .ok_or_else(|| anyhow!("Missing <constant> section"))?;

        let band_20_freq_44100 = parse_f32_list(
            &child_attr(const_node, "band_20_freq", "fs_44100")
                .ok_or_else(|| anyhow!("Missing band_20_freq fs_44100"))?,
        )?;
        let band_20_freq_48000 = parse_f32_list(
            &child_attr(const_node, "band_20_freq", "fs_48000")
                .ok_or_else(|| anyhow!("Missing band_20_freq fs_48000"))?,
        )?;

        let mut presets = HashMap::new();
        for node in const_node.children().filter(|n| n.is_element()) {
            if let Some(target) = node.attribute("target") {
                presets.insert(node.tag_name().name().to_string(), parse_i32_list(target)?);
            }
        }

        let endpoint = root
            .children()
            .find(|n| {
                n.has_tag_name("endpoint") && n.attribute("type") == Some("internal_speaker")
            })
            .or_else(|| root.children().find(|n| n.has_tag_name("endpoint")))
            .ok_or_else(|| anyhow!("Missing <endpoint> section"))?;

        let mut profiles = HashMap::new();
        for prof_node in endpoint.children().filter(|n| n.has_tag_name("profile")) {
            let Some(name) = prof_node.attribute("type") else {
                continue;
            };
            let cp = parse_cp(prof_node, &presets)?;
            let vlldp = parse_vlldp(prof_node, &presets)?;
            profiles.insert(
                name.to_string(),
                ProfileTuning {
                    name: name.to_string(),
                    cp,
                    vlldp,
                },
            );
        }

        Ok(Self {
            xml_version,
            tuning_date,
            band_20_freq_44100,
            band_20_freq_48000,
            presets,
            profiles,
        })
    }

    pub fn band_freqs(&self, sample_rate: u32) -> &[f32] {
        if sample_rate == 44100 {
            &self.band_20_freq_44100
        } else {
            &self.band_20_freq_48000
        }
    }
}

fn parse_cp(prof_node: Node, presets: &HashMap<String, Vec<i32>>) -> Result<CpTuning> {
    let Some(cp) = prof_node.children().find(|n| n.has_tag_name("tuning-cp")) else {
        return Ok(CpTuning {
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
            postgain_db: 0.0,
        });
    };

    let ieq_target_db = resolve_band_db(cp, "ieq-bands-set", presets, 20)?;
    let geq_db = resolve_band_db(cp, "graphic-equalizer-bands", presets, 20)?;

    Ok(CpTuning {
        dialog_enhancer_enable: child_bool(cp, "dialog-enhancer-enable", false),
        dialog_enhancer_amount: child_f32(cp, "dialog-enhancer-amount", 0.0),
        ieq_enable: child_bool(cp, "ieq-enable", false),
        ieq_amount: child_f32(cp, "ieq-amount", 0.0),
        ieq_target_db,
        geq_enable: child_bool(cp, "graphic-equalizer-enable", false),
        geq_db,
        volume_leveler_enable: child_bool(cp, "volume-leveler-enable", false),
        volume_leveler_amount: child_f32(cp, "volume-leveler-amount", 0.0),
        volume_leveler_in_target_db: child_f32(cp, "volume-leveler-in-target", -320.0) / 16.0,
        volume_leveler_out_target_db: child_f32(cp, "volume-leveler-out-target", -320.0) / 16.0,
        surround_decoder_enable: child_bool(cp, "surround-decoder-enable", false),
        surround_virtualizer_enable: child_bool(
            cp,
            "output-mode-partial-surround-virtualizer-enable",
            false,
        ),
        surround_boost_db: child_f32(cp, "surround-boost", 0.0) / 16.0,
        pregain_db: child_f32(cp, "pregain", 0.0) / 16.0,
        postgain_db: child_f32(cp, "postgain", 0.0) / 16.0,
    })
}

fn parse_vlldp(prof_node: Node, presets: &HashMap<String, Vec<i32>>) -> Result<VlldpTuning> {
    let Some(vlldp) = prof_node.children().find(|n| n.has_tag_name("tuning-vlldp")) else {
        return Ok(VlldpTuning {
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
        });
    };

    let opt_bands = vlldp
        .children()
        .find(|n| n.has_tag_name("audio-optimizer-bands"));
    let audio_optimizer_ch0_db = opt_bands
        .map(|n| resolve_band_db(n, "ch_00", presets, 20))
        .transpose()?
        .unwrap_or_else(|| vec![0.0; 20]);
    let audio_optimizer_ch1_db = opt_bands
        .map(|n| resolve_band_db(n, "ch_01", presets, 20))
        .transpose()?
        .unwrap_or_else(|| vec![0.0; 20]);

    let mut speaker_peq_filters = Vec::new();
    if let Some(peq_node) = vlldp
        .children()
        .find(|n| n.has_tag_name("speaker-peq-filters"))
    {
        for f in peq_node.children().filter(|n| n.has_tag_name("filter")) {
            let speaker = f
                .attribute("speaker")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let enabled = f.attribute("enabled").unwrap_or("0") == "1";
            let filter_type = f
                .attribute("type")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            let f0 = f
                .attribute("f0")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1000.0);
            let gain_db = f
                .attribute("gain")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0);
            let q = f.attribute("q").and_then(|v| v.parse().ok()).unwrap_or(1.0);
            let order = f
                .attribute("order")
                .and_then(|v| v.parse().ok())
                .unwrap_or(2);
            speaker_peq_filters.push(PeqFilterSpec {
                speaker,
                enabled,
                filter_type,
                f0,
                gain_db,
                q,
                order,
            });
        }
    }

    let mut mb_compressor_groups = Vec::new();
    if let Some(mb_node) = vlldp
        .children()
        .find(|n| n.has_tag_name("mb-compressor-tuning"))
    {
        for g in mb_node
            .children()
            .filter(|n| n.is_element() && n.tag_name().name().starts_with("band_group_"))
        {
            if let Some(val) = g.attribute("value") {
                let nums = parse_i32_list(val)?;
                if nums.len() >= 6 {
                    mb_compressor_groups.push(MbBandGroup {
                        upper_band_idx: nums[0].max(0) as usize,
                        threshold_db: nums[1] as f32 / 16.0,
                        ratio_raw: nums[2],
                        attack_ms: nums[3] as f32,
                        release_ms: nums[4] as f32,
                        makeup_db: nums[5] as f32 / 16.0,
                    });
                }
            }
        }
    }

    let reg_node = vlldp.children().find(|n| n.has_tag_name("regulator-tuning"));
    let regulator_threshold_high_db = reg_node
        .map(|n| resolve_band_db(n, "threshold_high", presets, 20))
        .transpose()?
        .unwrap_or_else(|| vec![0.0; 20]);
    let regulator_threshold_low_db = reg_node
        .map(|n| resolve_band_db(n, "threshold_low", presets, 20))
        .transpose()?
        .unwrap_or_else(|| vec![-12.0; 20]);

    Ok(VlldpTuning {
        audio_optimizer_enable: child_bool(vlldp, "audio-optimizer-enable", false),
        audio_optimizer_ch0_db,
        audio_optimizer_ch1_db,
        speaker_peq_enable: child_bool(vlldp, "speaker-peq-enable", false),
        speaker_peq_filters,
        mb_compressor_enable: child_bool(vlldp, "mb-compressor-enable", false),
        mb_compressor_groups,
        regulator_speaker_dist_enable: child_bool(vlldp, "regulator-speaker-dist-enable", false),
        regulator_threshold_high_db,
        regulator_threshold_low_db,
        postgain_db: child_f32(vlldp, "postgain", 0.0) / 16.0,
        system_gain_db: child_f32(vlldp, "system-gain", 0.0) / 16.0,
    })
}

fn resolve_band_db(
    parent: Node,
    tag: &str,
    presets: &HashMap<String, Vec<i32>>,
    expected_len: usize,
) -> Result<Vec<f32>> {
    let Some(node) = parent.children().find(|n| n.has_tag_name(tag)) else {
        return Ok(vec![0.0; expected_len]);
    };
    if let Some(val) = node.attribute("value") {
        let ints = parse_i32_list(val)?;
        return Ok(ints.into_iter().map(|v| v as f32 / 16.0).collect());
    }
    if let Some(preset_name) = node.attribute("preset") {
        if let Some(ints) = presets.get(preset_name) {
            return Ok(ints.iter().map(|&v| v as f32 / 16.0).collect());
        }
    }
    Ok(vec![0.0; expected_len])
}

fn child_attr<'a>(parent: Node<'a, 'a>, tag: &str, attr: &str) -> Option<String> {
    parent
        .children()
        .find(|n| n.has_tag_name(tag))
        .and_then(|n| n.attribute(attr))
        .map(str::to_string)
}

fn child_bool(parent: Node, tag: &str, default: bool) -> bool {
    parent
        .children()
        .find(|n| n.has_tag_name(tag))
        .and_then(|n| n.attribute("value"))
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(default)
}

fn child_f32(parent: Node, tag: &str, default: f32) -> f32 {
    parent
        .children()
        .find(|n| n.has_tag_name(tag))
        .and_then(|n| n.attribute("value"))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn parse_i32_list(s: &str) -> Result<Vec<i32>> {
    s.split(',')
        .map(|p| p.trim().parse::<i32>().context("Invalid integer in XML list"))
        .collect()
}

fn parse_f32_list(s: &str) -> Result<Vec<f32>> {
    s.split(',')
        .map(|p| p.trim().parse::<f32>().context("Invalid float in XML list"))
        .collect()
}
