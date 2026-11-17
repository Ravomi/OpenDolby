use crate::dsp::{DolbyProcessor, RuntimeControls};
use anyhow::{anyhow, Context, Result};
use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use pw::spa::pod::Pod;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const VIRTUAL_SINK_NAME: &str = "dolby_atmos_rust";
const VIRTUAL_OUT_NAME: &str = "dolby_atmos_rust_out";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

/// Bounded ring buffer for interleaved stereo f32 frames (serialized via Rc<RefCell> on the PipeWire mainloop).
struct RingBuf {
    buf: Vec<f32>,
    cap: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
}

impl RingBuf {
    fn new(cap: usize) -> Self {
        Self {
            buf: vec![0.0; cap],
            cap,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    fn push_slice(&mut self, data: &[f32]) {
        let mut h = self.head.load(Ordering::Relaxed);
        let t = self.tail.load(Ordering::Acquire);
        let avail = self.cap - 1 - ((h + self.cap - t) % self.cap);
        let n = (data.len().min(avail) / 2) * 2;
        for &s in &data[..n] {
            self.buf[h] = s;
            h = (h + 1) % self.cap;
        }
        self.head.store(h, Ordering::Release);
    }

    fn pop_slice(&mut self, out: &mut [f32]) -> usize {
        let h = self.head.load(Ordering::Acquire);
        let mut t = self.tail.load(Ordering::Relaxed);
        let avail = (h + self.cap - t) % self.cap;
        let n = (out.len().min(avail) / 2) * 2;
        for slot in &mut out[..n] {
            *slot = self.buf[t];
            t = (t + 1) % self.cap;
        }
        self.tail.store(t, Ordering::Release);
        n
    }
}

/// Lock-free atomic live controls shared between the UI/control thread and the PipeWire mainloop.
struct ControlAtomics {
    bypass: AtomicBool,
    level_match: AtomicBool,
    surround_amount: AtomicU32,
    dialog_amount: AtomicU32,
}

impl ControlAtomics {
    fn new() -> Self {
        Self {
            bypass: AtomicBool::new(false),
            level_match: AtomicBool::new(true),
            surround_amount: AtomicU32::new(1.0_f32.to_bits()),
            dialog_amount: AtomicU32::new(1.0_f32.to_bits()),
        }
    }

    fn snapshot(&self) -> RuntimeControls {
        RuntimeControls {
            bypass: self.bypass.load(Ordering::Relaxed),
            level_match: self.level_match.load(Ordering::Relaxed),
            surround_amount: f32::from_bits(self.surround_amount.load(Ordering::Relaxed)),
            dialog_amount: f32::from_bits(self.dialog_amount.load(Ordering::Relaxed)),
        }
    }
}

/// Handle for controlling a running PipeWire Dolby virtual sink from another thread.
pub struct EngineHandle {
    pending_proc: Arc<Mutex<Option<DolbyProcessor>>>,
    controls: Arc<ControlAtomics>,
    quit: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl EngineHandle {
    /// Hot-swap the active DSP processor on the PipeWire mainloop timer without blocking audio.
    pub fn set_processor(&self, processor: DolbyProcessor) {
        if let Ok(mut guard) = self.pending_proc.lock() {
            *guard = Some(processor);
        }
    }

    /// Toggle DSP bypass cleanly without tearing down the PipeWire node.
    pub fn set_bypass(&self, bypass: bool) {
        self.controls.bypass.store(bypass, Ordering::Relaxed);
    }

    /// Toggle loudness/level matching during bypass/processing.
    pub fn set_level_match(&self, level_match: bool) {
        self.controls
            .level_match
            .store(level_match, Ordering::Relaxed);
    }

    /// Set surround amount (0.0..=1.0+).
    pub fn set_surround_amount(&self, amount: f32) {
        self.controls
            .surround_amount
            .store(amount.to_bits(), Ordering::Relaxed);
    }

    /// Set dialog enhancement amount (0.0..=1.0+).
    pub fn set_dialog_amount(&self, amount: f32) {
        self.controls
            .dialog_amount
            .store(amount.to_bits(), Ordering::Relaxed);
    }

    /// Request the PipeWire main loop to quit and wait for the audio thread to exit.
    pub fn stop(&mut self) {
        self.quit.store(true, Ordering::Relaxed);
        if let Some(handle) = self.join.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Detect the hardware ALSA Speaker sink node.name so dolby_atmos_rust_out never loops back to itself.
pub fn detect_hardware_speaker_sink() -> Option<String> {
    let out = std::process::Command::new("pw-cli")
        .args(["ls", "Node"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if line.contains("node.name") && line.contains("Speaker__sink") {
            let mut parts = line.split('"');
            let _ = parts.next();
            if let Some(name) = parts.next() {
                if !name.is_empty() && name != VIRTUAL_SINK_NAME && name != VIRTUAL_OUT_NAME {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

fn select_output_frames(requested: u64, max_frames: usize) -> usize {
    if requested > 0 {
        (requested as usize).min(max_frames)
    } else {
        max_frames
    }
}

fn validate_target_sink(target: Option<String>) -> Result<String> {
    let target = target.ok_or_else(|| {
        anyhow!("Failed to resolve hardware speaker sink target; refusing to start without a valid target")
    })?;
    if target == VIRTUAL_SINK_NAME || target == VIRTUAL_OUT_NAME {
        return Err(anyhow!(
            "Refusing to target virtual sink '{}' (would cause self-loop)",
            target
        ));
    }
    Ok(target)
}

/// Spawn the PipeWire virtual sink on a dedicated thread and return a control handle.
pub fn spawn_pipewire_sink(
    processor: DolbyProcessor,
    target_sink: Option<String>,
    sample_rate: u32,
) -> Result<EngineHandle> {
    let pending_proc = Arc::new(Mutex::new(None));
    let controls = Arc::new(ControlAtomics::new());
    let quit = Arc::new(AtomicBool::new(false));

    let pending_c = Arc::clone(&pending_proc);
    let controls_c = Arc::clone(&controls);
    let quit_c = Arc::clone(&quit);

    let (tx, rx) = std::sync::mpsc::channel::<Result<()>>();
    let join = thread::Builder::new()
        .name("dolby-pw-sink".into())
        .spawn(move || {
            if let Err(err) = run_pipewire_sink_inner(
                processor,
                target_sink,
                sample_rate,
                pending_c,
                controls_c,
                quit_c,
                Some(tx.clone()),
            ) {
                let _ = tx.send(Err(err));
            }
        })
        .context("Failed to spawn PipeWire thread")?;

    match rx.recv_timeout(STARTUP_TIMEOUT) {
        Ok(res) => res?,
        Err(RecvTimeoutError::Timeout) => {
            quit.store(true, Ordering::Relaxed);
            return Err(anyhow!(
                "Timed out after {:?} waiting for PipeWire virtual sink initialization",
                STARTUP_TIMEOUT
            ));
        }
        Err(RecvTimeoutError::Disconnected) => {
            return Err(anyhow!("PipeWire thread exited before initialization"));
        }
    }

    Ok(EngineHandle {
        pending_proc,
        controls,
        quit,
        join: Some(join),
    })
}

pub fn run_pipewire_sink(
    processor: DolbyProcessor,
    target_sink: Option<String>,
    sample_rate: u32,
) -> Result<()> {
    run_pipewire_sink_inner(
        processor,
        target_sink,
        sample_rate,
        Arc::new(Mutex::new(None)),
        Arc::new(ControlAtomics::new()),
        Arc::new(AtomicBool::new(false)),
        None,
    )
}

fn run_pipewire_sink_inner(
    processor: DolbyProcessor,
    target_sink: Option<String>,
    sample_rate: u32,
    pending_proc: Arc<Mutex<Option<DolbyProcessor>>>,
    controls: Arc<ControlAtomics>,
    quit: Arc<AtomicBool>,
    ready_tx: Option<std::sync::mpsc::Sender<Result<()>>>,
) -> Result<()> {
    if sample_rate != 44100 && sample_rate != 48000 {
        return Err(anyhow!(
            "Unsupported sample rate {} Hz (supported: 44100, 48000)",
            sample_rate
        ));
    }
    let resolved_target = validate_target_sink(target_sink.or_else(detect_hardware_speaker_sink))?;

    pw::init();

    let mainloop =
        pw::main_loop::MainLoopRc::new(None).context("Failed to create PipeWire MainLoop")?;
    let context = pw::context::ContextBox::new(mainloop.loop_(), None)
        .context("Failed to create PipeWire Context")?;
    let core = context
        .connect(None)
        .context("Failed to connect to PipeWire daemon")?;

    // Non-hard-RT: stream callbacks are serialized on the PipeWire mainloop thread
    // (RT_PROCESS is intentionally omitted), so Rc<RefCell<...>> is sound.
    // 16384 stereo frames = 32768 f32 samples (~341ms @ 48kHz).
    let ring = Rc::new(RefCell::new(RingBuf::new(32768)));
    let proc_state = Rc::new(RefCell::new(processor));

    let in_props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Playback",
        *pw::keys::MEDIA_CLASS => "Audio/Sink",
        *pw::keys::NODE_NAME => VIRTUAL_SINK_NAME,
        *pw::keys::NODE_DESCRIPTION => "Dolby Atmos Rust (ALC294)",
        "application.name" => "OpenDolby",
        "application.icon-name" => "opendolby-app",
        "media.icon-name" => "opendolby-app",
        *pw::keys::AUDIO_CHANNELS => "2",
        "audio.position" => "FL,FR",
        "node.virtual" => "true",
    };

    let in_stream = pw::stream::StreamBox::new(&core, "dolby-rs-sink", in_props)
        .context("Failed to create PipeWire virtual sink stream")?;

    let mut out_props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Playback",
        *pw::keys::MEDIA_ROLE => "Music",
        *pw::keys::NODE_NAME => VIRTUAL_OUT_NAME,
        *pw::keys::NODE_DESCRIPTION => "OpenDolby Output",
        "application.name" => "OpenDolby",
        "application.icon-name" => "opendolby-app",
        "media.name" => "Speaker DSP",
        "media.icon-name" => "opendolby-app",
        *pw::keys::AUDIO_CHANNELS => "2",
        "audio.position" => "FL,FR",
        "node.dont-reconnect" => "true",
    };
    out_props.insert("target.object", resolved_target.as_str());

    let out_stream = pw::stream::StreamBox::new(&core, "dolby-rs-out", out_props)
        .context("Failed to create PipeWire output stream")?;

    let ring_in = Rc::clone(&ring);
    let proc_in = Rc::clone(&proc_state);
    let controls_in = Arc::clone(&controls);
    let _in_listener = in_stream
        .add_local_listener_with_user_data(())
        .process(move |stream, _| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let datas = buffer.datas_mut();
            if datas.is_empty() {
                return;
            }
            let chunk = datas[0].chunk();
            let offset = chunk.offset() as usize;
            let size = chunk.size() as usize;
            let Some(slice) = datas[0].data() else {
                return;
            };
            if offset >= slice.len() || size == 0 {
                return;
            }
            let valid_bytes = size.min(slice.len() - offset);
            let frame_bytes = 2 * std::mem::size_of::<f32>();
            let n_frames = valid_bytes / frame_bytes;
            if n_frames == 0 {
                return;
            }
            let n_floats = n_frames * 2;
            let byte_slice = &mut slice[offset..offset + n_frames * frame_bytes];
            let live_controls = controls_in.snapshot();

            let mut proc = proc_in.borrow_mut();
            proc.set_controls(live_controls);

            if (byte_slice.as_ptr() as usize) % std::mem::align_of::<f32>() == 0 {
                let samples: &mut [f32] = unsafe {
                    std::slice::from_raw_parts_mut(byte_slice.as_mut_ptr() as *mut f32, n_floats)
                };
                proc.process_interleaved(samples);
                ring_in.borrow_mut().push_slice(samples);
            } else {
                let mut scratch = vec![0.0_f32; n_floats];
                for (dst, src) in scratch.iter_mut().zip(byte_slice.chunks_exact(4)) {
                    *dst = f32::from_le_bytes([src[0], src[1], src[2], src[3]]);
                }
                proc.process_interleaved(&mut scratch);
                ring_in.borrow_mut().push_slice(&scratch);
            }
        })
        .register()
        .context("Failed to register input stream listener")?;

    let ring_out = Rc::clone(&ring);
    let _out_listener = out_stream
        .add_local_listener_with_user_data(())
        .process(move |stream, _| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let req_frames = buffer.requested();
            let datas = buffer.datas_mut();
            if datas.is_empty() {
                return;
            }
            let Some(slice) = datas[0].data() else {
                return;
            };
            let frame_bytes = 2 * std::mem::size_of::<f32>();
            let max_frames = slice.len() / frame_bytes;
            let n_frames = select_output_frames(req_frames, max_frames);
            if n_frames == 0 {
                return;
            }
            let n_floats = n_frames * 2;
            if (slice.as_ptr() as usize) % std::mem::align_of::<f32>() == 0 {
                let out: &mut [f32] = unsafe {
                    std::slice::from_raw_parts_mut(slice.as_mut_ptr() as *mut f32, n_floats)
                };
                let read = ring_out.borrow_mut().pop_slice(out);
                if read < out.len() {
                    out[read..].fill(0.0);
                }
            } else {
                let mut scratch = vec![0.0_f32; n_floats];
                let read = ring_out.borrow_mut().pop_slice(&mut scratch);
                if read < scratch.len() {
                    scratch[read..].fill(0.0);
                }
                for (dst, src) in slice[..n_frames * frame_bytes]
                    .chunks_exact_mut(4)
                    .zip(scratch.iter())
                {
                    dst.copy_from_slice(&src.to_le_bytes());
                }
            }
            let chunk = datas[0].chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = frame_bytes as i32;
            *chunk.size_mut() = (n_frames * frame_bytes) as u32;
        })
        .register()
        .context("Failed to register output stream listener")?;

    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    audio_info.set_rate(sample_rate);
    audio_info.set_channels(2);

    let pod_bytes = pw::spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(pw::spa::pod::Object {
            type_: pw::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
            id: pw::spa::param::ParamType::EnumFormat.as_raw(),
            properties: audio_info.into(),
        }),
    )
    .context("Failed to serialize audio format pod")?
    .0
    .into_inner();

    let mut params = [Pod::from_bytes(&pod_bytes).unwrap()];

    in_stream
        .connect(
            spa::utils::Direction::Input,
            None,
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .context("Failed to connect virtual sink stream")?;

    out_stream
        .connect(
            spa::utils::Direction::Output,
            None,
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .context("Failed to connect output stream")?;

    let mainloop_weak = mainloop.downgrade();
    let proc_timer = Rc::clone(&proc_state);
    let _control_timer = mainloop.loop_().add_timer(move |_| {
        if let Ok(mut guard) = pending_proc.try_lock() {
            if let Some(new_proc) = guard.take() {
                *proc_timer.borrow_mut() = new_proc;
            }
        }
        if quit.load(Ordering::Relaxed) {
            if let Some(ml) = mainloop_weak.upgrade() {
                ml.quit();
            }
        }
    });
    _control_timer.update_timer(
        Some(Duration::from_millis(50)),
        Some(Duration::from_millis(50)),
    );

    if let Some(tx) = ready_tx {
        let _ = tx.send(Ok(()));
    } else {
        println!(
            "PipeWire virtual sink 'Dolby Atmos Rust (ALC294)' active (rate={} Hz). Ctrl+C to stop.",
            sample_rate
        );
    }

    mainloop.run();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buf_push_pop_and_wrap() {
        let mut ring = RingBuf::new(8); // max 7 floats -> 3 stereo frames (6 floats)
        ring.push_slice(&[1.0, 2.0, 3.0, 4.0]);
        let mut out = [0.0; 2];
        assert_eq!(ring.pop_slice(&mut out), 2);
        assert_eq!(out, [1.0, 2.0]);

        // Wrap around & overflow preserves whole stereo frames (no split L/R)
        ring.push_slice(&[5.0, 6.0, 7.0, 8.0, 9.0, 10.0]);
        let mut out2 = [0.0; 8];
        let n = ring.pop_slice(&mut out2);
        assert_eq!(n, 6);
        assert_eq!(&out2[..n], &[3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
    }

    #[test]
    fn select_output_frames_respects_requested_and_capacity() {
        assert_eq!(select_output_frames(0, 2048), 2048);
        assert_eq!(select_output_frames(1536, 2048), 1536);
        assert_eq!(select_output_frames(4096, 2048), 2048);
        assert_eq!(select_output_frames(512, 2048), 512);
    }

    #[test]
    fn control_atomics_defaults_and_updates() {
        let ctrl = ControlAtomics::new();
        let snap = ctrl.snapshot();
        assert!(!snap.bypass);
        assert!(snap.level_match);
        assert!((snap.surround_amount - 1.0).abs() < 1e-6);
        assert!((snap.dialog_amount - 1.0).abs() < 1e-6);

        ctrl.bypass.store(true, Ordering::Relaxed);
        ctrl.level_match.store(false, Ordering::Relaxed);
        ctrl.surround_amount
            .store(0.42_f32.to_bits(), Ordering::Relaxed);
        ctrl.dialog_amount
            .store(1.35_f32.to_bits(), Ordering::Relaxed);

        let updated = ctrl.snapshot();
        assert!(updated.bypass);
        assert!(!updated.level_match);
        assert!((updated.surround_amount - 0.42).abs() < 1e-6);
        assert!((updated.dialog_amount - 1.35).abs() < 1e-6);
    }

    #[test]
    fn validate_target_sink_fails_closed() {
        assert!(validate_target_sink(None).is_err());
        assert!(validate_target_sink(Some(VIRTUAL_SINK_NAME.to_string())).is_err());
        assert!(validate_target_sink(Some(VIRTUAL_OUT_NAME.to_string())).is_err());
        assert_eq!(
            validate_target_sink(Some(
                "alsa_output.pci-0000_00_1f.3.Speaker__sink".to_string()
            ))
            .unwrap(),
            "alsa_output.pci-0000_00_1f.3.Speaker__sink"
        );
    }
}
