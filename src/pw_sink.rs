use crate::dsp::DolbyProcessor;
use anyhow::{Context, Result};
use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use pw::spa::pod::Pod;
use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

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

pub fn run_pipewire_sink(
    processor: DolbyProcessor,
    target_sink: Option<String>,
    sample_rate: u32,
) -> Result<()> {
    pw::init();

    let mainloop =
        pw::main_loop::MainLoopBox::new(None).context("Failed to create PipeWire MainLoop")?;
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

    let mut out_props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Playback",
        *pw::keys::MEDIA_ROLE => "Music",
        *pw::keys::NODE_NAME => "dolby_atmos_rust_out",
        *pw::keys::NODE_DESCRIPTION => "Dolby Atmos Rust Output",
        *pw::keys::AUDIO_CHANNELS => "2",
        "audio.position" => "FL,FR",
        "node.dont-reconnect" => "false",
    };
    if let Some(ref target) = target_sink {
        out_props.insert("target.object", target.as_str());
    }

    let out_stream = pw::stream::StreamBox::new(&core, "dolby-rs-out", out_props)
        .context("Failed to create PipeWire output stream")?;

    let ring_in = Arc::clone(&ring);
    let proc_in = Arc::clone(&proc_state);
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
            let chunk_size = datas[0].chunk().size() as usize;
            let Some(slice) = datas[0].data() else {
                return;
            };
            let n_floats = (chunk_size / std::mem::size_of::<f32>()).min(slice.len() / 4);
            let samples: &mut [f32] = unsafe {
                std::slice::from_raw_parts_mut(slice.as_mut_ptr() as *mut f32, n_floats)
            };
            proc_in.borrow_mut().process_interleaved(samples);
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

    println!(
        "PipeWire virtual sink 'Dolby Atmos Rust (ALC294)' active (rate={} Hz). Ctrl+C to stop.",
        sample_rate
    );

    mainloop.run();
    Ok(())
}
