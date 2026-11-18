use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use dolby_rs::dsp::{DolbyProcessor, RuntimeControls};
use dolby_rs::tuning::DeviceTuning;
use dolby_rs::{default_xml_path, pw_sink};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "dolby-rs",
    about = "Experimental Linux PipeWire speaker DSP engine (OpenDolby)"
)]
struct Cli {
    #[arg(long)]
    xml: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Inspect parsed profiles and DSP parameters from the local Dolby XML
    Inspect {
        #[arg(long, default_value = "dynamic")]
        profile: String,
    },
    /// Process a stereo WAV file offline for A/B testing
    ProcessWav {
        input: PathBuf,
        output: PathBuf,
        #[arg(long, default_value = "dynamic")]
        profile: String,
        /// Audition the dry path, with RMS matching if enabled
        #[arg(long)]
        bypass: bool,
        /// Disable approximate RMS A/B alignment (may make processing much louder)
        #[arg(long)]
        no_level_match: bool,
        /// Scale profile-enabled surround contribution (0..2)
        #[arg(long, default_value_t = 1.0)]
        surround: f32,
        /// Scale profile-enabled dialog contribution (0..2)
        #[arg(long, default_value_t = 1.0)]
        dialog: f32,
    },
    /// Run live PipeWire virtual sink ("OpenDolby Speaker")
    Run {
        #[arg(long, default_value = "dynamic")]
        profile: String,
        #[arg(long, default_value_t = 48000)]
        rate: u32,
        #[arg(long)]
        target_sink: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let xml_path = cli.xml.unwrap_or_else(default_xml_path);
    let tuning = DeviceTuning::from_xml_file(&xml_path)?;

    match cli.command {
        Commands::Inspect { profile } => {
            println!(
                "XML version: {} ({})",
                tuning.xml_version, tuning.tuning_date
            );
            let mut names: Vec<&String> = tuning.profiles.keys().collect();
            names.sort();
            println!("Available profiles: {:?}", names);

            let p = tuning
                .profiles
                .get(&profile)
                .ok_or_else(|| anyhow!("Profile '{}' not found", profile))?;
            println!("\n[Profile: {}]", p.name);
            println!("  CP:");
            println!(
                "    ieq_enable={} amount={}",
                p.cp.ieq_enable, p.cp.ieq_amount
            );
            println!(
                "    volume_leveler_enable={} amount={} target={} dB",
                p.cp.volume_leveler_enable,
                p.cp.volume_leveler_amount,
                p.cp.volume_leveler_out_target_db
            );
            println!(
                "    dialog_enhancer_enable={} amount={}",
                p.cp.dialog_enhancer_enable, p.cp.dialog_enhancer_amount
            );
            println!(
                "    surround_decoder={} virtualizer={} boost={} dB",
                p.cp.surround_decoder_enable,
                p.cp.surround_virtualizer_enable,
                p.cp.surround_boost_db
            );
            println!("  VLLDP:");
            println!(
                "    audio_optimizer_enable={} ch0_db={:?}",
                p.vlldp.audio_optimizer_enable, p.vlldp.audio_optimizer_ch0_db
            );
            println!(
                "    speaker_peq_enable={} filters={}",
                p.vlldp.speaker_peq_enable,
                p.vlldp.speaker_peq_filters.len()
            );
            println!(
                "    mb_compressor_enable={} groups={}",
                p.vlldp.mb_compressor_enable,
                p.vlldp.mb_compressor_groups.len()
            );
        }
        Commands::ProcessWav {
            input,
            output,
            profile,
            bypass,
            no_level_match,
            surround,
            dialog,
        } => {
            let mut reader = hound::WavReader::open(&input)
                .with_context(|| format!("Failed to open {}", input.display()))?;
            let spec = reader.spec();
            if spec.channels != 2 {
                return Err(anyhow!("Only stereo WAV files are supported"));
            }

            let mut samples: Vec<f32> = match spec.sample_format {
                hound::SampleFormat::Float => reader
                    .samples::<f32>()
                    .collect::<std::result::Result<_, _>>()?,
                hound::SampleFormat::Int => {
                    let scale = (1_i64 << (spec.bits_per_sample - 1)) as f32;
                    reader
                        .samples::<i32>()
                        .map(|s| s.map(|v| v as f32 / scale))
                        .collect::<std::result::Result<_, _>>()?
                }
            };

            let mut proc = DolbyProcessor::from_tuning(&tuning, &profile, spec.sample_rate)
                .ok_or_else(|| anyhow!("Profile '{}' not found", profile))?;
            proc.set_controls(RuntimeControls {
                bypass,
                level_match: !no_level_match,
                surround_amount: surround,
                dialog_amount: dialog,
            });
            proc.process_interleaved(&mut samples);

            let out_spec = hound::WavSpec {
                channels: 2,
                sample_rate: spec.sample_rate,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            };
            let mut writer = hound::WavWriter::create(&output, out_spec)?;
            for s in samples {
                writer.write_sample(s)?;
            }
            writer.finalize()?;
            println!(
                "Processed {} -> {} (profile={})",
                input.display(),
                output.display(),
                profile
            );
        }
        Commands::Run {
            profile,
            rate,
            target_sink,
        } => {
            let proc = DolbyProcessor::from_tuning(&tuning, &profile, rate)
                .ok_or_else(|| anyhow!("Profile '{}' not found", profile))?;
            pw_sink::run_pipewire_sink(proc, target_sink, rate)?;
        }
    }

    Ok(())
}
