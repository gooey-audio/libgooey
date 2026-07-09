//! Performance Recording Demo — basic CLI for Stage 1 chord clip capture.
//!
//! Plays a simple drum loop, lets you press chord pads (1–7), and record them
//! into a looping clip with punch-out or overdub modes.
//!
//! Run with:
//!   cargo run --example performance_record --features native,crossterm

#[cfg(feature = "native")]
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    FromSample, Sample, SizedSample, Stream, StreamConfig,
};
#[cfg(feature = "native")]
use crossterm::{
    cursor,
    event::{
        self, Event, KeyCode, KeyEvent, KeyEventKind, KeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    },
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
use std::time::{Duration, Instant};

#[cfg(feature = "native")]
const DEFAULT_BPM: f32 = 120.0;
#[cfg(feature = "native")]
const DEGREE_LABELS: [&str; 7] = ["I", "II", "III", "IV", "V", "VI", "VII"];
#[cfg(feature = "native")]
const UI_REFRESH: Duration = Duration::from_millis(100);

/// Snapshot of engine state for UI drawing. Taken under a short lock so the
/// audio thread is not blocked (or forced to output silence) during terminal I/O.
#[cfg(feature = "native")]
struct UiSnapshot {
    armed: bool,
    recording: bool,
    mode: u32,
    event_count: u32,
    length_steps: u32,
    step: i32,
    beat: f64,
    events: Vec<(u32, u32, u32, f32)>, // start, dur, degree, velocity
}

#[cfg(feature = "native")]
struct FfiEngine {
    ptr: *mut GooeyEngine,
}

#[cfg(feature = "native")]
unsafe impl Send for FfiEngine {}

#[cfg(feature = "native")]
impl FfiEngine {
    fn new(sample_rate: f32) -> Self {
        Self {
            ptr: gooey_engine_new(sample_rate),
        }
    }

    fn ptr(&self) -> *mut GooeyEngine {
        self.ptr
    }
}

#[cfg(feature = "native")]
impl Drop for FfiEngine {
    fn drop(&mut self) {
        unsafe {
            gooey_engine_free(self.ptr);
        }
    }
}

#[cfg(feature = "native")]
struct AppState {
    playing: bool,
    holding_degree: Option<u32>,
    root: u32,
    scale: u32, // 0 major, 1 minor
    octave: i32,
    preset: u32,
}

#[cfg(feature = "native")]
fn bool_pattern(steps: &[usize]) -> [bool; 16] {
    let mut pattern = [false; 16];
    for &step in steps {
        pattern[step % 16] = true;
    }
    pattern
}

#[cfg(feature = "native")]
fn configure_engine(engine: *mut GooeyEngine) {
    unsafe {
        gooey_engine_set_bpm(engine, DEFAULT_BPM);
        gooey_engine_set_master_gain(engine, 0.85);

        // Simple four-on-floor so you can hear loop boundaries while recording.
        let kick = bool_pattern(&[0, 4, 8, 12]);
        let snare = bool_pattern(&[4, 12]);
        let hat = bool_pattern(&[0, 2, 4, 6, 8, 10, 12, 14]);
        gooey_engine_sequencer_set_instrument_pattern(engine, INSTRUMENT_KICK, kick.as_ptr());
        gooey_engine_sequencer_set_instrument_pattern(engine, INSTRUMENT_SNARE, snare.as_ptr());
        gooey_engine_sequencer_set_instrument_pattern(engine, INSTRUMENT_HIHAT, hat.as_ptr());
        for step in 0..16u32 {
            gooey_engine_sequencer_set_instrument_step_velocity(
                engine,
                INSTRUMENT_HIHAT,
                step,
                if step % 4 == 0 { 0.5 } else { 0.3 },
            );
        }

        // Default record mode: punch-out (one bar then auto-disarm).
        gooey_engine_perf_set_record_mode(engine, PERF_RECORD_MODE_PUNCH_OUT);
        gooey_engine_perf_set_record_armed(engine, false);

        // Start transport so the first arm can wait for loop start cleanly.
        gooey_engine_sequencer_start(engine);
    }
}

#[cfg(feature = "native")]
fn mode_label(mode: u32) -> &'static str {
    if mode == PERF_RECORD_MODE_OVERDUB {
        "OVERDUB"
    } else {
        "PUNCH-OUT"
    }
}

#[cfg(feature = "native")]
fn scale_label(scale: u32) -> &'static str {
    if scale == 0 {
        "Major"
    } else {
        "Minor"
    }
}

#[cfg(feature = "native")]
fn root_label(root: u32) -> &'static str {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    NAMES[(root as usize) % 12]
}

#[cfg(feature = "native")]
fn snapshot_ui(engine: *mut GooeyEngine) -> UiSnapshot {
    unsafe {
        let event_count = gooey_engine_perf_get_event_count(engine);
        let show = event_count.min(12);
        let mut events = Vec::with_capacity(show as usize);
        for i in 0..show {
            let mut start = 0u32;
            let mut dur = 0u32;
            let mut degree = 0u32;
            let mut velocity = 0.0f32;
            if gooey_engine_perf_get_event(
                engine,
                i,
                &mut start,
                &mut dur,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut degree,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut velocity,
            ) {
                events.push((start, dur, degree, velocity));
            }
        }
        UiSnapshot {
            armed: gooey_engine_perf_is_record_armed(engine),
            recording: gooey_engine_perf_is_recording(engine),
            mode: gooey_engine_perf_get_record_mode(engine),
            event_count,
            length_steps: gooey_engine_perf_get_length_steps(engine),
            step: gooey_engine_sequencer_get_current_step(engine),
            beat: gooey_engine_sequencer_get_beat_position(engine),
            events,
        }
    }
}

#[cfg(feature = "native")]
fn draw_ui(snap: &UiSnapshot, state: &AppState) -> io::Result<()> {
    // Raw mode: newlines must be \r\n or the cursor only moves down (staircase layout).
    execute!(
        io::stdout(),
        cursor::MoveTo(0, 0),
        Clear(ClearType::FromCursorDown)
    )?;

    print!("=== Performance Recording Demo ===\r\n");
    print!("\r\n");
    print!(
        "Transport: {}   BPM: {:.0}   step: {:>2}/{}   beat: {:.2}\r\n",
        if state.playing { "PLAY" } else { "STOP" },
        DEFAULT_BPM,
        snap.step.max(0),
        snap.length_steps,
        snap.beat
    );
    print!(
        "Key: {} {}   Octave: {}   Preset: pad\r\n",
        root_label(state.root),
        scale_label(state.scale),
        state.octave
    );
    print!("\r\n");

    let rec_status = if snap.recording {
        "RECORDING"
    } else if snap.armed {
        "ARMED (wait for loop)"
    } else {
        "idle"
    };
    print!(
        "Record: {:<22}  mode: {}\r\n",
        rec_status,
        mode_label(snap.mode)
    );
    print!("Clip events: {}\r\n", snap.event_count);
    print!("\r\n");

    print!("Pads: ");
    for (i, label) in DEGREE_LABELS.iter().enumerate() {
        if state.holding_degree == Some(i as u32) {
            print!("[{}] ", label);
        } else {
            print!(" {}  ", label);
        }
    }
    print!("\r\n");
    print!("Keys: 1-7 hold chords\r\n");
    print!("\r\n");

    print!("Recorded events:\r\n");
    if snap.events.is_empty() {
        print!("  (empty)\r\n");
    } else {
        for (i, (start, dur, degree, velocity)) in snap.events.iter().enumerate() {
            let deg = DEGREE_LABELS
                .get(*degree as usize)
                .copied()
                .unwrap_or("?");
            let start_step = *start as f32 / 24.0;
            let dur_steps = *dur as f32 / 24.0;
            print!(
                "  [{i}] {deg:<3}  start~{start_step:.1} steps  gate~{dur_steps:.1}  vel={velocity:.2}\r\n"
            );
        }
        if snap.event_count as usize > snap.events.len() {
            print!(
                "  ... +{} more\r\n",
                snap.event_count as usize - snap.events.len()
            );
        }
    }

    print!("\r\n");
    print!("Controls:\r\n");
    print!("  1-7     chord pads I-VII (hold; release key or press 0)\r\n");
    print!("  0       release held chord\r\n");
    print!("  r       toggle record arm\r\n");
    print!("  m       toggle punch-out / overdub\r\n");
    print!("  c       clear clip\r\n");
    print!("  space   play / stop\r\n");
    print!("  [ ]     root down / up\r\n");
    print!("  s       toggle major / minor\r\n");
    print!("  q       quit\r\n");
    print!("\r\n");
    print!("Tip: arm punch-out, hold chords for one bar, auto-disarm, hear them loop.\r\n");
    io::stdout().flush()?;
    Ok(())
}

#[cfg(feature = "native")]
fn trigger_degree(engine: *mut GooeyEngine, state: &AppState, degree: u32) {
    unsafe {
        gooey_engine_poly_trigger_chord(
            engine,
            state.root,
            state.scale,
            degree,
            0, // root position
            state.preset,
            state.octave,
            0.9,
        );
    }
}

#[cfg(feature = "native")]
fn release_chord(engine: *mut GooeyEngine) {
    unsafe {
        gooey_engine_poly_release(engine);
    }
}

#[cfg(feature = "native")]
fn build_output_stream(
    engine: Arc<Mutex<FfiEngine>>,
    device: &cpal::Device,
    config: &StreamConfig,
    sample_format: cpal::SampleFormat,
) -> anyhow::Result<Stream> {
    match sample_format {
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
        format => Err(anyhow::anyhow!("unsupported sample format {format}")),
    }
}

#[cfg(feature = "native")]
fn make_stream<T>(
    engine: Arc<Mutex<FfiEngine>>,
    device: &cpal::Device,
    config: &StreamConfig,
) -> anyhow::Result<Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let err_fn = |err| eprintln!("audio stream error: {err}");
    let stream = device.build_output_stream(
        config,
        move |output: &mut [T], _| {
            render_audio(output, channels, &engine);
        },
        err_fn,
        None,
    )?;
    Ok(stream)
}

#[cfg(feature = "native")]
thread_local! {
    static RENDER_BUF: RefCell<Vec<f32>> = RefCell::new(Vec::new());
}

#[cfg(feature = "native")]
fn render_audio<T>(output: &mut [T], channels: usize, engine: &Arc<Mutex<FfiEngine>>)
where
    T: Sample + FromSample<f32>,
{
    let frames = output.len() / channels;
    let needed = frames * GOOEY_OUTPUT_CHANNELS as usize;

    // Never allocate on the audio thread after warm-up: reuse a TLS buffer.
    RENDER_BUF.with(|cell| {
        let mut interleaved = cell.borrow_mut();
        if interleaved.len() < needed {
            interleaved.resize(needed, 0.0);
        }

        // Blocking lock is OK if the UI only holds the mutex for microsecond
        // snapshots / key handlers. try_lock+silence was causing buffer-sized pops.
        let engine = engine.lock().unwrap();
        unsafe {
            gooey_engine_render(engine.ptr(), interleaved.as_mut_ptr(), frames as u32);
        }

        for (frame_idx, frame) in output.chunks_mut(channels).enumerate() {
            let l = interleaved[frame_idx * 2];
            let r = interleaved[frame_idx * 2 + 1];
            match frame.len() {
                0 => {}
                1 => frame[0] = T::from_sample(0.5 * (l + r)),
                _ => {
                    frame[0] = T::from_sample(l);
                    frame[1] = T::from_sample(r);
                    if frame.len() > 2 {
                        let mono = T::from_sample(0.5 * (l + r));
                        for sample in &mut frame[2..] {
                            *sample = mono;
                        }
                    }
                }
            }
        }
    });
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("no default output device"))?;
    let supported_config = device.default_output_config()?;
    let config: StreamConfig = supported_config.clone().into();
    let sample_rate = config.sample_rate.0 as f32;

    println!("Output device: {}", device.name()?);
    println!("Sample rate: {sample_rate}");

    let ffi_engine = FfiEngine::new(sample_rate);
    configure_engine(ffi_engine.ptr());
    let engine = Arc::new(Mutex::new(ffi_engine));

    let stream = build_output_stream(
        engine.clone(),
        &device,
        &config,
        supported_config.sample_format(),
    )?;
    stream.play()?;

    let mut state = AppState {
        playing: true,
        holding_degree: None,
        root: 0,  // C
        scale: 0, // major
        octave: 4,
        preset: POLY_PRESET_PAD,
    };

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    // Request key-up events when the terminal supports them (for hold-to-sustain).
    let _ = execute!(
        stdout,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
    );
    execute!(stdout, cursor::Hide)?;

    let mut last_draw = Instant::now() - UI_REFRESH;
    // Warm the audio render buffer so the first callback does not allocate.
    RENDER_BUF.with(|cell| {
        cell.borrow_mut().resize(4096 * 2, 0.0);
    });

    loop {
        // Redraw at a modest rate. Snapshot under a short lock, then print
        // without holding the engine (print was blocking audio → silence pops).
        if last_draw.elapsed() >= UI_REFRESH {
            let snap = {
                let guard = engine.lock().unwrap();
                snapshot_ui(guard.ptr())
            };
            draw_ui(&snap, &state)?;
            last_draw = Instant::now();
        }

        if event::poll(Duration::from_millis(10))? {
            if let Event::Key(KeyEvent { code, kind, .. }) = event::read()? {
                // Ignore key-repeat presses; handle Press and Release.
                let is_press = kind == KeyEventKind::Press;
                let is_release = kind == KeyEventKind::Release;
                if !is_press && !is_release {
                    continue;
                }

                match code {
                    KeyCode::Char('q') | KeyCode::Esc if is_press => break,

                    KeyCode::Char(' ') if is_press => {
                        let guard = engine.lock().unwrap();
                        unsafe {
                            if state.playing {
                                gooey_engine_sequencer_stop(guard.ptr());
                                if state.holding_degree.is_some() {
                                    release_chord(guard.ptr());
                                    state.holding_degree = None;
                                }
                                state.playing = false;
                            } else {
                                gooey_engine_sequencer_start(guard.ptr());
                                state.playing = true;
                            }
                        }
                    }

                    KeyCode::Char('r') if is_press => {
                        let guard = engine.lock().unwrap();
                        unsafe {
                            let armed = gooey_engine_perf_is_record_armed(guard.ptr());
                            gooey_engine_perf_set_record_armed(guard.ptr(), !armed);
                        }
                    }

                    KeyCode::Char('m') if is_press => {
                        let guard = engine.lock().unwrap();
                        unsafe {
                            let mode = gooey_engine_perf_get_record_mode(guard.ptr());
                            let next = if mode == PERF_RECORD_MODE_OVERDUB {
                                PERF_RECORD_MODE_PUNCH_OUT
                            } else {
                                PERF_RECORD_MODE_OVERDUB
                            };
                            gooey_engine_perf_set_record_mode(guard.ptr(), next);
                        }
                    }

                    KeyCode::Char('c') if is_press => {
                        let guard = engine.lock().unwrap();
                        unsafe {
                            gooey_engine_perf_clear_clip(guard.ptr());
                        }
                    }

                    KeyCode::Char('[') if is_press => {
                        state.root = state.root.wrapping_sub(1) % 12;
                    }
                    KeyCode::Char(']') if is_press => {
                        state.root = (state.root + 1) % 12;
                    }
                    KeyCode::Char('s') if is_press => {
                        state.scale = 1 - state.scale;
                    }

                    KeyCode::Char(ch @ '1'..='7') if is_press => {
                        let degree = (ch as u8 - b'1') as u32;
                        if state.holding_degree != Some(degree) {
                            let guard = engine.lock().unwrap();
                            if state.holding_degree.is_some() {
                                release_chord(guard.ptr());
                            }
                            trigger_degree(guard.ptr(), &state, degree);
                            state.holding_degree = Some(degree);
                        }
                    }

                    KeyCode::Char(ch @ '1'..='7') if is_release => {
                        let degree = (ch as u8 - b'1') as u32;
                        if state.holding_degree == Some(degree) {
                            let guard = engine.lock().unwrap();
                            release_chord(guard.ptr());
                            state.holding_degree = None;
                        }
                    }

                    // Fallback when the terminal does not report key-up.
                    KeyCode::Char('0') | KeyCode::Enter if is_press => {
                        if state.holding_degree.is_some() {
                            let guard = engine.lock().unwrap();
                            release_chord(guard.ptr());
                            state.holding_degree = None;
                        }
                    }

                    _ => {}
                }
            }
        }
    }

    // Release any held chord on exit.
    {
        let guard = engine.lock().unwrap();
        if state.holding_degree.is_some() {
            release_chord(guard.ptr());
        }
        unsafe {
            gooey_engine_sequencer_stop(guard.ptr());
        }
    }

    let _ = execute!(stdout, PopKeyboardEnhancementFlags);
    execute!(stdout, cursor::Show)?;
    disable_raw_mode()?;
    println!("\nBye.");
    Ok(())
}

#[cfg(not(feature = "native"))]
fn main() {
    eprintln!("This example requires --features native,crossterm");
}
