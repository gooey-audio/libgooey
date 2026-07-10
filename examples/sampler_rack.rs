//! Interactive sampler-rack CLI. It requires a default system audio output.
//!
//! Run with: `cargo run --example sampler_rack --features native,crossterm`

#[cfg(feature = "native")]
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    FromSample, Sample, SizedSample, Stream, StreamConfig,
};
#[cfg(feature = "native")]
use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
#[cfg(feature = "native")]
use gooey::ffi::*;
#[cfg(feature = "native")]
use std::cell::RefCell;
#[cfg(feature = "native")]
use std::io::{self, Write};
#[cfg(feature = "native")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "native")]
use std::time::Duration;

#[cfg(feature = "native")]
struct FfiEngine(*mut GooeyEngine);

#[cfg(feature = "native")]
unsafe impl Send for FfiEngine {}

#[cfg(feature = "native")]
impl Drop for FfiEngine {
    fn drop(&mut self) {
        unsafe { gooey_engine_free(self.0) }
    }
}

#[cfg(feature = "native")]
fn pad(sample_rate: f32, hz: f32) -> Vec<f32> {
    let frames = (sample_rate * 0.22) as usize;
    (0..frames)
        .map(|i| {
            let t = i as f32 / sample_rate;
            let envelope = (1.0 - i as f32 / frames as f32).powi(2);
            (t * hz * std::f32::consts::TAU).sin() * envelope * 0.7
        })
        .collect()
}

#[cfg(feature = "native")]
fn configure(engine: *mut GooeyEngine, sample_rate: f32) -> u32 {
    unsafe {
        let rack = gooey_engine_sampler_register(engine);
        assert!(rack >= 0, "sampler rack registration failed");
        let rack = rack as u32;
        assert!(gooey_engine_mixer_route_source(
            engine,
            gooey_engine_sampler_get_source_id(engine, rack),
            3,
        ));
        for (slot, hz) in [110.0, 165.0, 220.0, 440.0].into_iter().enumerate() {
            let pcm = pad(sample_rate, hz);
            assert!(gooey_engine_sampler_set_slot_buffer(
                engine,
                rack,
                slot as u32,
                pcm.as_ptr(),
                pcm.len() as u32,
                1,
                sample_rate,
            ));
        }
        for step in 0..16 {
            let enabled = step % 2 == 0;
            assert!(gooey_engine_sampler_set_step(
                engine,
                rack,
                step,
                enabled,
                (step / 2 % 4) as u32,
                0.85,
            ));
        }
        gooey_engine_set_master_gain(engine, 1.0);
        gooey_engine_perf_set_record_mode(engine, PERF_RECORD_MODE_OVERDUB);
        gooey_engine_sequencer_start(engine);
        rack
    }
}

#[cfg(feature = "native")]
thread_local! { static RENDER_BUFFER: RefCell<Vec<f32>> = RefCell::new(vec![0.0; 8192]); }

#[cfg(feature = "native")]
fn render_audio<T: Sample + FromSample<f32>>(
    output: &mut [T],
    channels: usize,
    engine: &Arc<Mutex<FfiEngine>>,
) {
    let frames = output.len() / channels;
    RENDER_BUFFER.with(|cell| {
        let mut input = cell.borrow_mut();
        input.resize(frames * 2, 0.0);
        let guard = engine.lock().unwrap();
        unsafe { gooey_engine_render(guard.0, input.as_mut_ptr(), frames as u32) };
        for (frame_index, frame) in output.chunks_mut(channels).enumerate() {
            let left = input[frame_index * 2];
            let right = input[frame_index * 2 + 1];
            frame[0] = T::from_sample(left);
            if channels > 1 {
                frame[1] = T::from_sample(right);
                for sample in &mut frame[2..] {
                    *sample = T::from_sample(0.5 * (left + right));
                }
            }
        }
    });
}

#[cfg(feature = "native")]
fn make_stream<T: SizedSample + FromSample<f32>>(
    engine: Arc<Mutex<FfiEngine>>,
    device: &cpal::Device,
    config: &StreamConfig,
) -> anyhow::Result<Stream> {
    let channels = config.channels as usize;
    Ok(device.build_output_stream(
        config,
        move |output: &mut [T], _| render_audio(output, channels, &engine),
        |error| eprintln!("audio stream error: {error}"),
        None,
    )?)
}

#[cfg(feature = "native")]
fn build_stream(
    engine: Arc<Mutex<FfiEngine>>,
    device: &cpal::Device,
    config: &StreamConfig,
    format: cpal::SampleFormat,
) -> anyhow::Result<Stream> {
    match format {
        cpal::SampleFormat::I8 => make_stream::<i8>(engine, device, config),
        cpal::SampleFormat::I16 => make_stream::<i16>(engine, device, config),
        cpal::SampleFormat::I32 => make_stream::<i32>(engine, device, config),
        cpal::SampleFormat::I64 => make_stream::<i64>(engine, device, config),
        cpal::SampleFormat::U8 => make_stream::<u8>(engine, device, config),
        cpal::SampleFormat::U16 => make_stream::<u16>(engine, device, config),
        cpal::SampleFormat::U32 => make_stream::<u32>(engine, device, config),
        cpal::SampleFormat::U64 => make_stream::<u64>(engine, device, config),
        cpal::SampleFormat::F32 => make_stream::<f32>(engine, device, config),
        cpal::SampleFormat::F64 => make_stream::<f64>(engine, device, config),
        other => Err(anyhow::anyhow!("unsupported output sample format {other}")),
    }
}

#[cfg(feature = "native")]
fn draw(engine: *mut GooeyEngine, running: bool) -> io::Result<()> {
    unsafe {
        execute!(io::stdout(), cursor::MoveTo(0, 0), Clear(ClearType::All))?;
        println!("=== Sampler Rack ===");
        println!(
            "Transport: {}  step: {}  recorded hits: {}",
            if running { "PLAY" } else { "STOP" },
            gooey_engine_sequencer_get_current_step(engine),
            gooey_engine_perf_get_sampler_event_count(engine)
        );
        println!(
            "Sequencer hits: {}",
            if gooey_engine_get_sequencer_triggers_enabled(engine) {
                "ON"
            } else {
                "OFF (transport still runs)"
            }
        );
        println!("\n1–4 trigger pads  |  r record arm  |  c clear recording");
        println!("s toggle sequence hits  |  space play/stop  |  q quit");
        io::stdout().flush()
    }
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("no default audio output device"))?;
    let supported = device.default_output_config()?;
    let config: StreamConfig = supported.clone().into();
    let engine = Arc::new(Mutex::new(FfiEngine(gooey_engine_new(
        config.sample_rate.0 as f32,
    ))));
    let rack = configure(engine.lock().unwrap().0, config.sample_rate.0 as f32);
    let stream = build_stream(engine.clone(), &device, &config, supported.sample_format())?;
    stream.play()?;

    enable_raw_mode()?;
    let mut running = true;
    loop {
        draw(engine.lock().unwrap().0, running)?;
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            if key.kind != event::KeyEventKind::Press {
                continue;
            }
            let guard = engine.lock().unwrap();
            unsafe {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char(' ') => {
                        if running {
                            gooey_engine_sequencer_stop(guard.0);
                        } else {
                            gooey_engine_sequencer_start(guard.0);
                        }
                        running = !running;
                    }
                    KeyCode::Char('r') => {
                        let armed = gooey_engine_perf_is_record_armed(guard.0);
                        gooey_engine_perf_set_record_armed(guard.0, !armed);
                    }
                    KeyCode::Char('c') => gooey_engine_perf_clear_clip(guard.0),
                    KeyCode::Char('s') => {
                        let enabled = gooey_engine_get_sequencer_triggers_enabled(guard.0);
                        gooey_engine_set_sequencer_triggers_enabled(guard.0, !enabled);
                    }
                    KeyCode::Char(ch @ '1'..='4') => {
                        let _ = gooey_engine_sampler_trigger(
                            guard.0,
                            rack,
                            (ch as u8 - b'1') as u32,
                            1.0,
                        );
                    }
                    _ => {}
                }
            }
        }
    }
    disable_raw_mode()?;
    println!("\nBye.");
    Ok(())
}

#[cfg(not(feature = "native"))]
fn main() {
    eprintln!("This example requires --features native,crossterm");
}
