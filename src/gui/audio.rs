//! One stream/worker owner for all panels. No device is required for tests.
use crate::{engine::Engine, StereoFrame};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const BLOCK_SIZE: usize = 256;
pub const SCOPE_SIZE: usize = 2048;

/// Adapter for engines owned by extensions. Fill every frame and return promptly;
/// never wait on GUI locks. Production adapters should not allocate. The built-in
/// legacy Engine is a documented POC exception to allocation-free DSP. The host
/// calls this serially, never from two of its own audio workers.
pub trait BlockRenderer: Send + 'static {
    /// Host-owned monotonic audio time, supplied before every block.
    /// Engines exposing only sample-count rendering can keep this default.
    fn set_time(&mut self, _seconds: f64) {}
    fn render(&mut self, frames: &mut [StereoFrame], sample_rate: f32);
}

/// Mount any safe block-rendering engine without defining another audio host.
pub struct RenderAdapter<F>(pub F);
impl<F> BlockRenderer for RenderAdapter<F>
where
    F: FnMut(&mut [StereoFrame], f32) + Send + 'static,
{
    fn render(&mut self, frames: &mut [StereoFrame], rate: f32) {
        (self.0)(frames, rate);
    }
}

/// Adapter for engines exposing interleaved stereo f32 blocks. Scratch storage
/// is fixed and chunks are bounded even when the caller requests larger blocks.
pub struct InterleavedAdapter<F> {
    render: F,
    scratch: [f32; BLOCK_SIZE * 2],
}
impl<F> InterleavedAdapter<F> {
    pub fn new(render: F) -> Self {
        Self {
            render,
            scratch: [0.0; BLOCK_SIZE * 2],
        }
    }
}
impl<F> BlockRenderer for InterleavedAdapter<F>
where
    F: FnMut(&mut [f32], f32) + Send + 'static,
{
    fn render(&mut self, frames: &mut [StereoFrame], rate: f32) {
        for frames in frames.chunks_mut(BLOCK_SIZE) {
            let scratch = &mut self.scratch[..frames.len() * 2];
            scratch.fill(0.0);
            (self.render)(scratch, rate);
            for (index, frame) in frames.iter_mut().enumerate() {
                *frame = StereoFrame {
                    l: scratch[index * 2],
                    r: scratch[index * 2 + 1],
                };
            }
        }
    }
}

pub struct EngineRenderer {
    pub engine: Arc<Mutex<Engine>>,
    pub health: Arc<AudioHealth>,
    time: f64,
}
impl EngineRenderer {
    pub fn new(engine: Arc<Mutex<Engine>>, health: Arc<AudioHealth>) -> Self {
        Self {
            engine,
            health,
            time: 0.0,
        }
    }
}
impl BlockRenderer for EngineRenderer {
    fn set_time(&mut self, seconds: f64) {
        self.time = seconds;
    }
    fn render(&mut self, frames: &mut [StereoFrame], sample_rate: f32) {
        if let Ok(mut engine) = self.engine.try_lock() {
            for frame in frames {
                *frame = engine.tick_stereo(self.time);
                self.time += 1.0 / sample_rate as f64;
            }
        } else {
            frames.fill(StereoFrame::default());
            self.health.contention.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[derive(Default)]
pub struct AudioHealth {
    pub frames: AtomicU64,
    pub blocks: AtomicU64,
    pub non_finite: AtomicU64,
    pub contention: AtomicU64,
    pub overruns: AtomicU64,
    pub max_block_us: AtomicU64,
    pub device_errors: AtomicU64,
    pub control_errors: AtomicU64,
    pub render_failures: AtomicU64,
}

/// Bounded, single-writer stereo history. Snapshots can contain a stale frame,
/// deliberately avoiding a lock between the GUI and audio callback.
pub struct Telemetry {
    left: Box<[AtomicU32]>,
    right: Box<[AtomicU32]>,
    cursor: AtomicUsize,
    pub health: Arc<AudioHealth>,
}
impl Default for Telemetry {
    fn default() -> Self {
        let channel = || (0..SCOPE_SIZE).map(|_| AtomicU32::new(0)).collect();
        Self {
            left: channel(),
            right: channel(),
            cursor: AtomicUsize::new(0),
            health: Arc::default(),
        }
    }
}
impl Telemetry {
    fn clear_scope(&self) {
        self.cursor.store(0, Ordering::Release);
    }
    pub fn publish(&self, frames: &mut [StereoFrame]) {
        let mut cursor = self.cursor.load(Ordering::Relaxed);
        for frame in frames.iter_mut() {
            for sample in [&mut frame.l, &mut frame.r] {
                if !sample.is_finite() {
                    *sample = 0.0;
                    self.health.non_finite.fetch_add(1, Ordering::Relaxed);
                }
            }
            self.left[cursor % SCOPE_SIZE].store(frame.l.to_bits(), Ordering::Relaxed);
            self.right[cursor % SCOPE_SIZE].store(frame.r.to_bits(), Ordering::Relaxed);
            cursor = cursor.wrapping_add(1);
        }
        self.cursor.store(cursor, Ordering::Release);
        self.health.blocks.fetch_add(1, Ordering::Relaxed);
        self.health
            .frames
            .fetch_add(frames.len() as u64, Ordering::Relaxed);
    }
    pub fn snapshot(&self) -> Vec<StereoFrame> {
        let end = self.cursor.load(Ordering::Acquire);
        (end.wrapping_sub(end.min(SCOPE_SIZE))..end)
            .map(|i| StereoFrame {
                l: f32::from_bits(self.left[i % SCOPE_SIZE].load(Ordering::Relaxed)),
                r: f32::from_bits(self.right[i % SCOPE_SIZE].load(Ordering::Relaxed)),
            })
            .collect()
    }
}

pub(super) fn render_block(
    renderer: &mut dyn BlockRenderer,
    frames: &mut [StereoFrame],
    rate: f32,
    telemetry: &Telemetry,
) {
    let start = Instant::now();
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        renderer.set_time(telemetry.health.frames.load(Ordering::Relaxed) as f64 / rate as f64);
        renderer.render(frames, rate);
    }))
    .is_err()
    {
        frames.fill(StereoFrame::default());
        telemetry
            .health
            .render_failures
            .fetch_add(1, Ordering::Relaxed);
    }
    telemetry.publish(frames);
    let elapsed = start.elapsed().as_micros() as u64;
    telemetry
        .health
        .max_block_us
        .fetch_max(elapsed, Ordering::Relaxed);
    if elapsed as f64 > frames.len() as f64 / rate as f64 * 1e6 {
        telemetry.health.overruns.fetch_add(1, Ordering::Relaxed);
    }
}

/// Owns either one CPAL stream or one silent worker. Drop/stop joins the worker
/// before another renderer is started. Device failures are returned to the UI.
pub struct GuiAudio {
    pub telemetry: Arc<Telemetry>,
    pub sample_rate: f32,
    pub status: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    #[cfg(feature = "native")]
    stream: Option<cpal::Stream>,
}
impl Default for GuiAudio {
    fn default() -> Self {
        Self {
            telemetry: Arc::default(),
            sample_rate: 44_100.0,
            status: "Stopped".into(),
            stop: Arc::default(),
            worker: None,
            #[cfg(feature = "native")]
            stream: None,
        }
    }
}
impl GuiAudio {
    pub fn stop(&mut self) {
        #[cfg(feature = "native")]
        {
            self.stream = None;
        }
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                self.telemetry
                    .health
                    .render_failures
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        self.status = "Stopped".into();
    }
    /// Query the device rate before constructing rate-dependent DSP.
    pub fn preferred_rate(silent: bool) -> f32 {
        #[cfg(feature = "native")]
        if !silent {
            use cpal::traits::{DeviceTrait, HostTrait};
            if let Some(device) = cpal::default_host().default_output_device() {
                if let Ok(config) = device.default_output_config() {
                    return config.sample_rate().0 as f32;
                }
            }
        }
        let _ = silent;
        44_100.0
    }
    pub fn start(&mut self, renderer: Box<dyn BlockRenderer>, silent: bool) -> anyhow::Result<()> {
        self.stop();
        self.telemetry.clear_scope();
        anyhow::ensure!(
            self.sample_rate.is_finite()
                && self.sample_rate >= 8000.0
                && self.sample_rate <= 192_000.0,
            "Invalid GUI audio sample rate"
        );
        self.stop = Arc::new(AtomicBool::new(false));
        #[cfg(feature = "native")]
        if !silent {
            return self.start_native(renderer);
        }
        let _ = silent;
        let stop = Arc::clone(&self.stop);
        let telemetry = Arc::clone(&self.telemetry);
        let rate = self.sample_rate;
        self.worker = Some(
            std::thread::Builder::new()
                .name("omni-silent-render".into())
                .spawn(move || {
                    let mut renderer = renderer;
                    let mut block = [StereoFrame::default(); BLOCK_SIZE];
                    let period = Duration::from_secs_f64(BLOCK_SIZE as f64 / rate as f64);
                    while !stop.load(Ordering::Acquire) {
                        let start = Instant::now();
                        render_block(renderer.as_mut(), &mut block, rate, &telemetry);
                        std::thread::sleep(period.saturating_sub(start.elapsed()));
                    }
                })?,
        );
        self.status = "Silent render (no audio device)".into();
        Ok(())
    }
    #[cfg(feature = "native")]
    fn start_native(&mut self, renderer: Box<dyn BlockRenderer>) -> anyhow::Result<()> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let device = cpal::default_host()
            .default_output_device()
            .ok_or_else(|| anyhow::anyhow!("No output device; select Silent render"))?;
        let supported = device.default_output_config()?;
        let config: cpal::StreamConfig = supported.clone().into();
        anyhow::ensure!(
            config.sample_rate.0 as f32 == self.sample_rate,
            "Device sample rate changed; restart audio"
        );
        let telemetry = Arc::clone(&self.telemetry);
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => native_stream::<f32>(&device, &config, renderer, telemetry)?,
            cpal::SampleFormat::F64 => native_stream::<f64>(&device, &config, renderer, telemetry)?,
            cpal::SampleFormat::I8 => native_stream::<i8>(&device, &config, renderer, telemetry)?,
            cpal::SampleFormat::I16 => native_stream::<i16>(&device, &config, renderer, telemetry)?,
            cpal::SampleFormat::I32 => native_stream::<i32>(&device, &config, renderer, telemetry)?,
            cpal::SampleFormat::I64 => native_stream::<i64>(&device, &config, renderer, telemetry)?,
            cpal::SampleFormat::U8 => native_stream::<u8>(&device, &config, renderer, telemetry)?,
            cpal::SampleFormat::U16 => native_stream::<u16>(&device, &config, renderer, telemetry)?,
            cpal::SampleFormat::U32 => native_stream::<u32>(&device, &config, renderer, telemetry)?,
            cpal::SampleFormat::U64 => native_stream::<u64>(&device, &config, renderer, telemetry)?,
            format => anyhow::bail!("Unsupported audio format {format:?}; select Silent render"),
        };
        stream.play()?;
        self.stream = Some(stream);
        self.status = format!(
            "Native · {} Hz · {} channels",
            config.sample_rate.0, config.channels
        );
        Ok(())
    }
}
impl Drop for GuiAudio {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn channel_sample(frame: StereoFrame, channel: usize, channels: usize) -> f32 {
    if channels == 1 {
        (frame.l + frame.r) * 0.5
    } else {
        match channel {
            0 => frame.l,
            1 => frame.r,
            _ => 0.0,
        }
    }
}

#[cfg(feature = "native")]
fn native_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut renderer: Box<dyn BlockRenderer>,
    telemetry: Arc<Telemetry>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    use cpal::traits::DeviceTrait;
    let rate = config.sample_rate.0 as f32;
    let channels = config.channels as usize;
    let errors = Arc::clone(&telemetry);
    let mut scratch = [StereoFrame::default(); BLOCK_SIZE];
    device.build_output_stream(
        config,
        move |output: &mut [T], _| {
            for chunk in output.chunks_mut(BLOCK_SIZE * channels) {
                let count = chunk.len() / channels;
                render_block(renderer.as_mut(), &mut scratch[..count], rate, &telemetry);
                for (i, frame) in chunk.chunks_mut(channels).enumerate() {
                    for (channel, sample) in frame.iter_mut().enumerate() {
                        *sample = T::from_sample(
                            channel_sample(scratch[i], channel, channels).clamp(-1.0, 1.0),
                        );
                    }
                }
            }
        },
        move |_| {
            errors.health.device_errors.fetch_add(1, Ordering::Relaxed);
        },
        None,
    )
}

/// Device-free accelerated rendering used by the smoke harness and extensions.
pub fn exercise(renderer: &mut dyn BlockRenderer, rate: f32, blocks: usize) -> Arc<Telemetry> {
    let telemetry = Arc::new(Telemetry::default());
    let mut scratch = [StereoFrame::default(); BLOCK_SIZE];
    for _ in 0..blocks {
        render_block(renderer, &mut scratch, rate, &telemetry);
    }
    telemetry
}
