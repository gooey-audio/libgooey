//! Play a keyboard melody over a sample-accurate I-IV-ii-V chord loop.
//!
//! Run with:
//!     cargo run --example chord_aware_melody --features "native crossterm"

use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat, SizedSample};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyboardEnhancementFlags};
use crossterm::{cursor, execute, terminal};
use gooey::ffi::*;

const BPM: f32 = 120.0;

const PROGRESSION: [(u32, &str); 4] = [
    (0, "I  Cmaj7"),
    (3, "IV Fmaj7"),
    (1, "ii Dm7"),
    (4, "V  G7"),
];

// Chromatic piano layout: white notes on A S D F G H J K, black notes above them.
const NOTE_KEYS: [(char, u32); 13] = [
    ('a', 60),
    ('w', 61),
    ('s', 62),
    ('e', 63),
    ('d', 64),
    ('f', 65),
    ('t', 66),
    ('g', 67),
    ('y', 68),
    ('h', 69),
    ('u', 70),
    ('j', 71),
    ('k', 72),
];

struct FfiEngine {
    ptr: *mut GooeyEngine,
}

// The engine is protected by a mutex and only dereferenced while that lock is held.
unsafe impl Send for FfiEngine {}

impl FfiEngine {
    fn new(sample_rate: f32) -> Result<Self> {
        let ptr = gooey_engine_new(sample_rate);
        if ptr.is_null() {
            bail!("failed to create GooeyEngine");
        }
        Ok(Self { ptr })
    }
}

impl Drop for FfiEngine {
    fn drop(&mut self) {
        unsafe { gooey_engine_free(self.ptr) };
    }
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(
            io::stdout(),
            crossterm::event::PopKeyboardEnhancementFlags,
            cursor::Show
        );
        let _ = terminal::disable_raw_mode();
    }
}

#[derive(Default)]
struct InputState {
    held_key: Option<char>,
    input_note: Option<u32>,
    octave: i32,
    playing: bool,
}

fn main() -> Result<()> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .context("no default audio output device")?;
    let supported = device
        .default_output_config()
        .context("could not read the default audio output configuration")?;
    let sample_rate = supported.sample_rate().0 as f32;
    let config: cpal::StreamConfig = supported.clone().into();

    let mut engine = FfiEngine::new(sample_rate)?;
    prepare_progression(&mut engine, sample_rate)?;
    let engine = Arc::new(Mutex::new(engine));

    let stream = build_output_stream(
        &device,
        &config,
        supported.sample_format(),
        Arc::clone(&engine),
    )?;
    stream.play().context("could not start audio output")?;

    terminal::enable_raw_mode()?;
    let terminal_guard = TerminalGuard;
    execute!(io::stdout(), cursor::Hide)?;
    // Key-up reporting is supported by modern terminals. The demo still works
    // without it: Enter explicitly releases a latched melody note.
    let _ = execute!(
        io::stdout(),
        crossterm::event::PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        )
    );

    let mut state = InputState {
        playing: true,
        ..InputState::default()
    };

    loop {
        draw(&engine, &state)?;

        if !event::poll(Duration::from_millis(25))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };

        if handle_key(key, &engine, &mut state)? {
            break;
        }
    }

    if let Ok(guard) = engine.lock() {
        unsafe { gooey_engine_melody_note_off(guard.ptr) };
    }
    drop(stream);
    drop(terminal_guard);
    println!("Chord-aware melody demo stopped.");
    Ok(())
}

fn prepare_progression(engine: &mut FfiEngine, sample_rate: f32) -> Result<()> {
    unsafe {
        gooey_engine_set_bpm(engine.ptr, BPM);
        gooey_engine_set_master_gain(engine.ptr, 0.55);
        gooey_engine_melody_set_param(engine.ptr, POLY_PARAM_FILTER_CUTOFF, 0.65);
        gooey_engine_melody_set_param(engine.ptr, POLY_PARAM_AMP_RELEASE, 0.2);
        gooey_engine_melody_set_param(engine.ptr, POLY_PARAM_VOLUME, 0.85);

        gooey_engine_perf_clear_clip(engine.ptr);
        gooey_engine_perf_set_record_mode(engine.ptr, PERF_RECORD_MODE_OVERDUB);
        gooey_engine_perf_set_record_armed(engine.ptr, true);
        gooey_engine_sequencer_start(engine.ptr);

        // One discarded frame lets the recorder observe the transport start.
        render_discarded(engine.ptr, 1);
        let frames_per_beat = (sample_rate * 60.0 / BPM).round() as usize;

        for (degree, _) in PROGRESSION {
            gooey_engine_poly_trigger_chord(
                engine.ptr,
                0,
                SCALE_MAJOR,
                degree,
                VOICING_ROOT_POSITION,
                POLY_PRESET_PAD,
                3,
                0.72,
            );
            render_discarded(engine.ptr, frames_per_beat);
        }

        gooey_engine_poly_release(engine.ptr);
        gooey_engine_perf_set_record_armed(engine.ptr, false);

        if gooey_engine_perf_get_event_count(engine.ptr) != PROGRESSION.len() as u32 {
            bail!("the startup chord loop was not recorded correctly");
        }

        gooey_engine_sequencer_stop(engine.ptr);
        gooey_engine_sequencer_reset(engine.ptr);
        // Let the performance player observe the stopped transport so the
        // following start forces a rescan and triggers the I chord at tick 0.
        render_discarded(engine.ptr, 1);
        gooey_engine_melody_clear_harmony(engine.ptr);
        gooey_engine_sequencer_start(engine.ptr);
    }
    Ok(())
}

unsafe fn render_discarded(engine: *mut GooeyEngine, frames: usize) {
    let mut scratch = vec![0.0; frames * 2];
    gooey_engine_render(engine, scratch.as_mut_ptr(), frames as u32);
}

fn handle_key(
    key: KeyEvent,
    engine: &Arc<Mutex<FfiEngine>>,
    state: &mut InputState,
) -> Result<bool> {
    let pressed = matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat);
    let released = key.kind == KeyEventKind::Release;

    if pressed && key.code == KeyCode::Esc {
        return Ok(true);
    }

    match key.code {
        KeyCode::Char(ch) => {
            let ch = ch.to_ascii_lowercase();
            if pressed && ch == 'q' {
                return Ok(true);
            }

            if let Some(base_note) = note_key(ch) {
                if pressed && key.kind != KeyEventKind::Repeat && state.held_key != Some(ch) {
                    let note = shifted_note(base_note, state.octave);
                    let guard = engine
                        .lock()
                        .map_err(|_| anyhow::anyhow!("engine lock poisoned"))?;
                    unsafe {
                        if state.held_key.is_some() {
                            gooey_engine_melody_update_note(guard.ptr, note);
                        } else {
                            gooey_engine_melody_note_on(guard.ptr, note, 0.9);
                        }
                    }
                    state.held_key = Some(ch);
                    state.input_note = Some(note);
                } else if released && state.held_key == Some(ch) {
                    release_melody(engine, state)?;
                }
                return Ok(false);
            }

            if pressed {
                match ch {
                    'z' => change_octave(engine, state, -1)?,
                    'x' => change_octave(engine, state, 1)?,
                    '[' => change_cutoff(engine, -0.05)?,
                    ']' => change_cutoff(engine, 0.05)?,
                    ' ' => toggle_transport(engine, state)?,
                    _ => {}
                }
            }
        }
        KeyCode::Enter if pressed => release_melody(engine, state)?,
        KeyCode::Up if pressed => change_cutoff(engine, 0.05)?,
        KeyCode::Down if pressed => change_cutoff(engine, -0.05)?,
        _ => {}
    }

    Ok(false)
}

fn release_melody(engine: &Arc<Mutex<FfiEngine>>, state: &mut InputState) -> Result<()> {
    let guard = engine
        .lock()
        .map_err(|_| anyhow::anyhow!("engine lock poisoned"))?;
    unsafe { gooey_engine_melody_note_off(guard.ptr) };
    state.held_key = None;
    state.input_note = None;
    Ok(())
}

fn change_octave(
    engine: &Arc<Mutex<FfiEngine>>,
    state: &mut InputState,
    change: i32,
) -> Result<()> {
    state.octave = (state.octave + change).clamp(-3, 3);
    let Some(key) = state.held_key else {
        return Ok(());
    };
    let Some(base_note) = note_key(key) else {
        return Ok(());
    };
    let note = shifted_note(base_note, state.octave);
    let guard = engine
        .lock()
        .map_err(|_| anyhow::anyhow!("engine lock poisoned"))?;
    unsafe {
        gooey_engine_melody_update_note(guard.ptr, note);
    }
    state.input_note = Some(note);
    Ok(())
}

fn change_cutoff(engine: &Arc<Mutex<FfiEngine>>, change: f32) -> Result<()> {
    let guard = engine
        .lock()
        .map_err(|_| anyhow::anyhow!("engine lock poisoned"))?;
    unsafe {
        let current = gooey_engine_melody_get_param(guard.ptr, POLY_PARAM_FILTER_CUTOFF);
        if current.is_finite() {
            gooey_engine_melody_set_param(
                guard.ptr,
                POLY_PARAM_FILTER_CUTOFF,
                (current + change).clamp(0.0, 1.0),
            );
        }
    }
    Ok(())
}

fn toggle_transport(engine: &Arc<Mutex<FfiEngine>>, state: &mut InputState) -> Result<()> {
    let guard = engine
        .lock()
        .map_err(|_| anyhow::anyhow!("engine lock poisoned"))?;
    unsafe {
        if state.playing {
            gooey_engine_sequencer_stop(guard.ptr);
        } else {
            gooey_engine_sequencer_start(guard.ptr);
        }
    }
    state.playing = !state.playing;
    Ok(())
}

fn note_key(ch: char) -> Option<u32> {
    NOTE_KEYS
        .iter()
        .find_map(|(key, note)| (*key == ch).then_some(*note))
}

fn shifted_note(note: u32, octave: i32) -> u32 {
    (note as i32 + octave * 12).clamp(0, 127) as u32
}

fn draw(engine: &Arc<Mutex<FfiEngine>>, state: &InputState) -> Result<()> {
    let guard = engine
        .lock()
        .map_err(|_| anyhow::anyhow!("engine lock poisoned"))?;
    let (beat, output_note, cutoff) = unsafe {
        (
            gooey_engine_sequencer_get_beat_position(guard.ptr),
            gooey_engine_melody_get_note(guard.ptr),
            gooey_engine_melody_get_param(guard.ptr, POLY_PARAM_FILTER_CUTOFF),
        )
    };
    drop(guard);

    let chord_index = beat.floor() as usize % PROGRESSION.len();
    let chord = PROGRESSION[chord_index].1;
    let input = state
        .input_note
        .map(note_name)
        .unwrap_or_else(|| "--".to_string());
    let output = if output_note >= 0 {
        note_name(output_note as u32)
    } else {
        "--".to_string()
    };
    let transport = if state.playing { "playing" } else { "paused " };
    let cutoff_bars = (cutoff * 20.0).round() as usize;

    let mut stdout = io::stdout();
    execute!(
        stdout,
        cursor::MoveTo(0, 0),
        terminal::Clear(terminal::ClearType::All)
    )?;
    writeln!(stdout, "Chord-aware melody — I · IV · ii · V")?;
    writeln!(stdout)?;
    writeln!(
        stdout,
        "Transport: {transport}   Beat: {:>4.2}   Chord: {chord}",
        beat
    )?;
    writeln!(
        stdout,
        "Input: {:<4}  Quantized output: {:<4}  Octave shift: {:+}",
        input, output, state.octave
    )?;
    writeln!(
        stdout,
        "Lead cutoff: [{}{}]",
        "#".repeat(cutoff_bars),
        "-".repeat(20 - cutoff_bars)
    )?;
    writeln!(stdout)?;
    writeln!(stdout, "Black keys:   W E   T Y U")?;
    writeln!(stdout, "White keys: A S D F G H J K")?;
    writeln!(stdout)?;
    writeln!(stdout, "Z/X octave  ·  ↑/↓ or [/] cutoff  ·  Enter release")?;
    writeln!(stdout, "Space play/pause  ·  Q or Esc quit")?;
    stdout.flush()?;
    Ok(())
}

fn note_name(note: u32) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!("{}{}", NAMES[(note % 12) as usize], note as i32 / 12 - 1)
}

fn build_output_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: SampleFormat,
    engine: Arc<Mutex<FfiEngine>>,
) -> Result<cpal::Stream> {
    match format {
        SampleFormat::F32 => make_stream::<f32>(device, config, engine),
        SampleFormat::F64 => make_stream::<f64>(device, config, engine),
        SampleFormat::I8 => make_stream::<i8>(device, config, engine),
        SampleFormat::I16 => make_stream::<i16>(device, config, engine),
        SampleFormat::I32 => make_stream::<i32>(device, config, engine),
        SampleFormat::I64 => make_stream::<i64>(device, config, engine),
        SampleFormat::U8 => make_stream::<u8>(device, config, engine),
        SampleFormat::U16 => make_stream::<u16>(device, config, engine),
        SampleFormat::U32 => make_stream::<u32>(device, config, engine),
        SampleFormat::U64 => make_stream::<u64>(device, config, engine),
        other => bail!("unsupported output sample format: {other:?}"),
    }
}

fn make_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    engine: Arc<Mutex<FfiEngine>>,
) -> Result<cpal::Stream>
where
    T: SizedSample + Sample + cpal::FromSample<f32>,
{
    let channels = config.channels as usize;
    let mut stereo = Vec::new();
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| render_audio(output, channels, &engine, &mut stereo),
            |error| eprintln!("audio stream error: {error}"),
            None,
        )
        .context("could not build audio output stream")
}

fn render_audio<T>(
    output: &mut [T],
    channels: usize,
    engine: &Arc<Mutex<FfiEngine>>,
    stereo: &mut Vec<f32>,
) where
    T: Sample + cpal::FromSample<f32>,
{
    let frames = output.len() / channels;
    stereo.resize(frames * 2, 0.0);

    if let Ok(guard) = engine.try_lock() {
        unsafe { gooey_engine_render(guard.ptr, stereo.as_mut_ptr(), frames as u32) };
    } else {
        stereo.fill(0.0);
    }

    for (frame_index, frame) in output.chunks_exact_mut(channels).enumerate() {
        let left = stereo[frame_index * 2];
        let right = stereo[frame_index * 2 + 1];
        for (channel, sample) in frame.iter_mut().enumerate() {
            *sample = T::from_sample(if channel == 0 { left } else { right });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_progression_records_and_replays_four_chords() {
        let mut engine = FfiEngine::new(48_000.0).unwrap();
        prepare_progression(&mut engine, 48_000.0).unwrap();

        unsafe {
            assert_eq!(gooey_engine_perf_get_event_count(engine.ptr), 4);
            assert!(!gooey_engine_melody_has_harmony(engine.ptr));
            render_discarded(engine.ptr, 64);
            assert!(gooey_engine_melody_has_harmony(engine.ptr));
        }
    }

    #[test]
    fn keyboard_layout_is_chromatic() {
        let notes: Vec<u32> = NOTE_KEYS.iter().map(|(_, note)| *note).collect();
        assert_eq!(notes, (60..=72).collect::<Vec<_>>());
    }
}
