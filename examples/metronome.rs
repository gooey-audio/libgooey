//! Minimal interactive CLI for auditioning the monitor click against a beat.
//! Requires a default system audio output.
//!
//! Run with: `cargo run --example metronome --features native,crossterm`

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

/// A backbeat to click against, so the metronome can be judged in context
/// rather than in isolation.
#[cfg(feature = "native")]
fn configure(engine: *mut GooeyEngine) {
    unsafe {
        for step in [0, 4, 8, 12] {
            gooey_engine_sequencer_set_instrument_step(engine, INSTRUMENT_KICK, step, true);
        }
        for step in [4, 12] {
            gooey_engine_sequencer_set_instrument_step(engine, INSTRUMENT_SNARE, step, true);
        }
        for step in [2, 6, 10, 14] {
            gooey_engine_sequencer_set_instrument_step(engine, INSTRUMENT_HIHAT, step, true);
        }
        gooey_engine_set_master_gain(engine, 0.8);
        gooey_engine_sequencer_start(engine);
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
fn division_label(division: u32) -> &'static str {
    match division {
        METRONOME_DIVISION_BAR => "bar",
        METRONOME_DIVISION_QUARTER => "1/4",
        METRONOME_DIVISION_EIGHTH => "1/8",
        METRONOME_DIVISION_SIXTEENTH => "1/16",
        _ => "?",
    }
}

#[cfg(feature = "native")]
fn draw(engine: *mut GooeyEngine, running: bool, bpm: f32) -> io::Result<()> {
    unsafe {
        execute!(io::stdout(), cursor::MoveTo(0, 0), Clear(ClearType::All))?;
        println!("=== Metronome ===\r");
        println!(
            "Transport: {}   BPM: {:.0}   beat: {:.2}\r",
            if running { "PLAY" } else { "STOP" },
            bpm,
            gooey_engine_transport_get_beat_position(engine)
        );
        println!(
            "Click: {}   level: {:.2}   division: {}   accent: {}\r",
            if gooey_engine_get_metronome_enabled(engine) {
                "ON "
            } else {
                "OFF"
            },
            gooey_engine_get_metronome_level(engine),
            division_label(gooey_engine_get_metronome_division(engine)),
            if gooey_engine_get_metronome_accent_enabled(engine) {
                "on"
            } else {
                "off"
            },
        );
        println!(
            "Drums: {}\r",
            if gooey_engine_get_sequencer_triggers_enabled(engine) {
                "ON"
            } else {
                "OFF"
            }
        );
        println!("\rm click on/off  |  d division  |  a accent  |  -/+ level\r");
        println!("k drums on/off  |  [/] tempo  |  space play/stop  |  q quit\r");
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
    configure(engine.lock().unwrap().0);
    // Start with the click audible — this example exists to hear it.
    unsafe { gooey_engine_set_metronome_enabled(engine.lock().unwrap().0, true) };

    let stream = build_stream(engine.clone(), &device, &config, supported.sample_format())?;
    stream.play()?;

    enable_raw_mode()?;
    let mut running = true;
    let mut bpm = 120.0_f32;
    loop {
        draw(engine.lock().unwrap().0, running, bpm)?;
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
                    KeyCode::Char('m') => {
                        let on = gooey_engine_get_metronome_enabled(guard.0);
                        gooey_engine_set_metronome_enabled(guard.0, !on);
                    }
                    KeyCode::Char('a') => {
                        let on = gooey_engine_get_metronome_accent_enabled(guard.0);
                        gooey_engine_set_metronome_accent_enabled(guard.0, !on);
                    }
                    KeyCode::Char('d') => {
                        let next = (gooey_engine_get_metronome_division(guard.0) + 1) % 4;
                        gooey_engine_set_metronome_division(guard.0, next);
                    }
                    KeyCode::Char('-') | KeyCode::Char('_') => {
                        let level = gooey_engine_get_metronome_level(guard.0);
                        gooey_engine_set_metronome_level(guard.0, level - 0.05);
                    }
                    KeyCode::Char('+') | KeyCode::Char('=') => {
                        let level = gooey_engine_get_metronome_level(guard.0);
                        gooey_engine_set_metronome_level(guard.0, level + 0.05);
                    }
                    KeyCode::Char('k') => {
                        let on = gooey_engine_get_sequencer_triggers_enabled(guard.0);
                        gooey_engine_set_sequencer_triggers_enabled(guard.0, !on);
                    }
                    KeyCode::Char('[') => {
                        bpm = (bpm - 5.0).max(40.0);
                        gooey_engine_set_bpm(guard.0, bpm);
                    }
                    KeyCode::Char(']') => {
                        bpm = (bpm + 5.0).min(240.0);
                        gooey_engine_set_bpm(guard.0, bpm);
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
