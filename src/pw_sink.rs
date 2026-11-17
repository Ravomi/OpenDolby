use crate::dsp::DolbyProcessor;
use anyhow::{Context, Result};
use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use pw::spa::pod::Pod;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Lock-free SPSC ring buffer for interleaved stereo f32 frames.
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
        let n = data.len().min(avail);
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
        let n = out.len().min(avail);
        for slot in &mut out[..n] {
            *slot = self.buf[t];
            t = (t + 1) % self.cap;
        }
        self.tail.store(t, Ordering::Release);
        n
    }
}

/// Handle for controlling a running PipeWire Dolby virtual sink from another thread.
pub struct EngineHandle {
    pending_proc: Arc<Mutex<Option<DolbyProcessor>>>,
    bypass: Arc<AtomicBool>,
    quit: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl EngineHandle {
    /// Hot-swap the active DSP processor on the next audio callback without blocking.
    pub fn set_processor(&self, processor: DolbyProcessor) {
        if let Ok(mut guard) = self.pending_proc.lock() {
            *guard = Some(processor);
        }
    }

    /// Toggle DSP bypass cleanly without tearing down the PipeWire node.
    pub fn set_bypass(&self, bypass: bool) {
        self.bypass.store(bypass, Ordering::Relaxed);
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
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// Spawn the PipeWire virtual sink on a dedicated thread and return a control handle.
pub fn spawn_pipewire_sink(
    processor: DolbyProcessor,
    target_sink: Option<String>,
    sample_rate: u32,
) -> Result<EngineHandle> {
    let pending_proc = Arc::new(Mutex::new(None));
    let bypass = Arc::new(AtomicBool::new(false));
    let quit = Arc::new(AtomicBool::new(false));

    let pending_c = Arc::clone(&pending_proc);
    let bypass_c = Arc::clone(&bypass);
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
                bypass_c,
                quit_c,
                Some(tx.clone()),
            ) {
                let _ = tx.send(Err(err));
            }
        })
        .context("Failed to spawn PipeWire thread")?;

    rx.recv()
        .context("PipeWire thread exited before initialization")??;

    Ok(EngineHandle {
        pending_proc,
        bypass,
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
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
        None,
    )
}

fn run_pipewire_sink_inner(
    processor: DolbyProcessor,
    target_sink: Option<String>,
    sample_rate: u32,
    pending_proc: Arc<Mutex<Option<DolbyProcessor>>>,
    bypass: Arc<AtomicBool>,
    quit: Arc<AtomicBool>,
    ready_tx: Option<std::sync::mpsc::Sender<Result<()>>>,
) -> Result<()> {
    pw::init();

    let mainloop =
        pw::main_loop::MainLoopRc::new(None).context("Failed to create PipeWire MainLoop")?;
    let context = pw::context::ContextBox::new(mainloop.loop_(), None)
        .context("Failed to create PipeWire Context")?;
    let core = context.connect(None).context("Failed to connect to PipeWire daemon")?;

    // 16384 stereo samples (~170ms max capacity)
    let ring = Arc::new(RefCell::new(RingBuf::new(32768)));
    let proc_state = Arc::new(RefCell::new(processor));

    let in_props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Playback",
        *pw::keys::MEDIA_CLASS => "Audio/Sink",
        *pw::keys::NODE_NAME => "dolby_atmos_rust",
        *pw::keys::NODE_DESCRIPTION => "Dolby Atmos Rust (ALC294)",
        *pw::keys::AUDIO_CHANNELS => "2",
        "audio.position" => "FL,FR",
        "node.virtual" => "true",
    };

    let in_stream = pw::stream::StreamBox::new(&core, "dolby-rs-sink", in_props)
        .context("Failed to create PipeWire virtual sink stream")?;

    let resolved_target = target_sink.or_else(detect_hardware_speaker_sink);

    let mut out_props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Playback",
        *pw::keys::MEDIA_ROLE => "Music",
        *pw::keys::NODE_NAME => "dolby_atmos_rust_out",
        *pw::keys::NODE_DESCRIPTION => "Dolby Atmos Rust Output",
        *pw::keys::AUDIO_CHANNELS => "2",
        "audio.position" => "FL,FR",
        "node.dont-reconnect" => "true",
    };
    if let Some(ref target) = resolved_target {
        out_props.insert("target.object", target.as_str());
    }

    let out_stream = pw::stream::StreamBox::new(&core, "dolby-rs-out", out_props)
        .context("Failed to create PipeWire output stream")?;

    let ring_in = Arc::clone(&ring);
    let proc_in = Arc::clone(&proc_state);
    let pending_in = Arc::clone(&pending_proc);
    let bypass_in = Arc::clone(&bypass);
    let _in_listener = in_stream
        .add_local_listener_with_user_data(())
        .process(move |stream, _| {
            if let Ok(mut guard) = pending_in.try_lock() {
                if let Some(new_proc) = guard.take() {
                    *proc_in.borrow_mut() = new_proc;
                }
            }
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let datas = buffer.datas_mut();
            if datas.is_empty() {
                return;
            }
            let chunk_size = datas[0].chunk().size() as usize;
            let Some(slice) = datas[0].data() else {
                return;
            };
            let n_floats = (chunk_size / std::mem::size_of::<f32>()).min(slice.len() / 4);
            let samples: &mut [f32] = unsafe {
                std::slice::from_raw_parts_mut(slice.as_mut_ptr() as *mut f32, n_floats)
            };
            if !bypass_in.load(Ordering::Relaxed) {
                proc_in.borrow_mut().process_interleaved(samples);
            }
            ring_in.borrow_mut().push_slice(samples);
        })
        .register()
        .context("Failed to register input stream listener")?;

    let ring_out = Arc::clone(&ring);
    let _out_listener = out_stream
        .add_local_listener_with_user_data(())
        .process(move |stream, _| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let datas = buffer.datas_mut();
            if datas.is_empty() {
                return;
            }
            let Some(slice) = datas[0].data() else {
                return;
            };
            let n_frames = (slice.len() / (2 * std::mem::size_of::<f32>())).min(1024);
            let n_floats = n_frames * 2;
            let out: &mut [f32] = unsafe {
                std::slice::from_raw_parts_mut(slice.as_mut_ptr() as *mut f32, n_floats)
            };
            let read = ring_out.borrow_mut().pop_slice(out);
            if read < out.len() {
                out[read..].fill(0.0);
            }
            let chunk = datas[0].chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = (2 * std::mem::size_of::<f32>()) as i32;
            *chunk.size_mut() = (n_floats * std::mem::size_of::<f32>()) as u32;
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
            pw::stream::StreamFlags::AUTOCONNECT
                | pw::stream::StreamFlags::MAP_BUFFERS
                | pw::stream::StreamFlags::RT_PROCESS,
            &mut params,
        )
        .context("Failed to connect virtual sink stream")?;

    out_stream
        .connect(
            spa::utils::Direction::Output,
            None,
            pw::stream::StreamFlags::AUTOCONNECT
                | pw::stream::StreamFlags::MAP_BUFFERS
                | pw::stream::StreamFlags::RT_PROCESS,
            &mut params,
        )
        .context("Failed to connect output stream")?;

    let mainloop_weak = mainloop.downgrade();
    let _quit_timer = mainloop.loop_().add_timer(move |_| {
        if quit.load(Ordering::Relaxed) {
            if let Some(ml) = mainloop_weak.upgrade() {
                ml.quit();
            }
        }
    });
    _quit_timer.update_timer(
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
