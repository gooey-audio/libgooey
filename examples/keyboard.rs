/* Poly Synth Keyboard - Play the poly synth from your computer keyboard.

Mimics a piano layout on the home row and above. Each key press triggers a
note. Because most terminals don't deliver reliable key-release events, each
note is auto-released after a fixed gate time (adjust with , / .) so sounds
don't hang forever; SPACE also force-releases everything. Same key pressed
twice simply layers a second voice (voice stealing kicks in beyond 6 voices).
*/

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gooey::engine::{Engine, EngineOutput, Instrument};
use gooey::instruments::{PolySynth, PolySynthConfig};

struct SharedPolySynth(Arc<Mutex<PolySynth>>);

impl Instrument for SharedPolySynth {
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        self.0.lock().unwrap().trigger_with_velocity(time, velocity);
    }

    fn tick(&mut self, current_time: f64) -> f32 {
        self.0.lock().unwrap().tick(current_time)
    }

    fn is_active(&self) -> bool {
        self.0.lock().unwrap().is_active()
    }

    fn set_midi_note(&mut self, note: u8) {
        self.0.lock().unwrap().set_midi_note(note);
    }
}

/// Map a character to a MIDI note, relative to `base_octave`.
/// Layout (white keys on home row, black keys on row above):
///   w   e       t   y   u
///  a s d f  g  h j k  l  ;
/// starts at C at `base_octave`.
fn key_to_midi(c: char, base_octave: i32) -> Option<u8> {
    let base = (base_octave + 1) * 12; // MIDI note number for C at octave
    let semitone = match c {
        'a' => 0,  // C
        'w' => 1,  // C#
        's' => 2,  // D
        'e' => 3,  // D#
        'd' => 4,  // E
        'f' => 5,  // F
        't' => 6,  // F#
        'g' => 7,  // G
        'y' => 8,  // G#
        'h' => 9,  // A
        'u' => 10, // A#
        'j' => 11, // B
        'k' => 12, // C (next octave)
        'o' => 13, // C#
        'l' => 14, // D
        'p' => 15, // D#
        ';' => 16, // E
        _ => return None,
    };
    let n = base + semitone;
    if (0..=127).contains(&n) {
        Some(n as u8)
    } else {
        None
    }
}

const PRESET_NAMES: [&str; 5] = ["Default", "Pad", "Pluck", "Keys", "Strings"];

fn get_preset(i: usize) -> PolySynthConfig {
    match i {
        1 => PolySynthConfig::pad(),
        2 => PolySynthConfig::pluck(),
        3 => PolySynthConfig::keys(),
        4 => PolySynthConfig::strings(),
        _ => PolySynthConfig::default(),
    }
}

struct State {
    octave: i32,
    preset: usize,
    tuning: f32,
    overdrive: f32,
    gate_ms: u64,
    last_note: Option<u8>,
}

fn draw(state: &State) {
    print!("\x1b[2J\x1b[H");
    println!("=== Poly Synth Keyboard ===\r");
    println!("\r");
    println!("  Play:    a s d f g h j k l ;   (white keys)\r");
    println!("           w e   t y u   o p     (black keys)\r");
    println!("  Octave:  [ / ]    (currently {})\r", state.octave);
    println!(
        "  Preset:  1-5      (currently {})\r",
        PRESET_NAMES[state.preset]
    );
    println!(
        "  Tuning:  - / =    ({:+.2} semitones)\r",
        (state.tuning - 0.5) * 24.0
    );
    println!("  Drive:   z / x    ({:.2})\r", state.overdrive);
    println!("  Gate:    , / .    ({} ms)\r", state.gate_ms);
    println!("  SPACE:   release all notes\r");
    println!("  Q:       quit\r");
    println!("\r");
    if let Some(n) = state.last_note {
        println!("  Last note: MIDI {}\r", n);
    } else {
        println!("  Last note: -\r");
    }
    io::stdout().flush().unwrap();
}

fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;

    let synth = Arc::new(Mutex::new(PolySynth::new(sample_rate)));

    let mut engine = Engine::new(sample_rate);
    engine.add_instrument("poly", Box::new(SharedPolySynth(Arc::clone(&synth))));
    engine.set_master_gain(0.8);

    let audio_engine = Arc::new(Mutex::new(engine));

    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;
    engine_output.create_stream_with_engine(audio_engine)?;
    engine_output.start()?;

    let mut state = State {
        octave: 4,
        preset: 0,
        tuning: 0.5,
        overdrive: 0.0,
        gate_ms: 500,
        last_note: None,
    };

    // Notes sounding under the auto-gate: (midi note, time triggered).
    let mut sounding: Vec<(u8, Instant)> = Vec::new();

    enable_raw_mode()?;
    execute!(io::stdout(), cursor::Hide, Clear(ClearType::All))?;
    draw(&state);

    loop {
        // Auto-release any notes whose gate time has elapsed. event::poll below
        // wakes at least every 50ms, so this runs regularly even when idle.
        let gate = Duration::from_millis(state.gate_ms);
        let now = Instant::now();
        let mut i = 0;
        while i < sounding.len() {
            if now.duration_since(sounding[i].1) >= gate {
                let (note, _) = sounding.remove(i);
                synth.lock().unwrap().release_note(note);
            } else {
                i += 1;
            }
        }

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(KeyEvent { code, kind, .. }) = event::read()? {
                // Ignore release events entirely — some terminals report them
                // when enhanced keyboard flags are enabled, but most don't.
                if kind == KeyEventKind::Release {
                    continue;
                }

                match code {
                    KeyCode::Char('q') | KeyCode::Char('Q') => break,

                    KeyCode::Char(' ') => {
                        synth.lock().unwrap().release_all();
                        sounding.clear();
                        state.last_note = None;
                        draw(&state);
                    }

                    KeyCode::Char('[') => {
                        state.octave = (state.octave - 1).max(0);
                        draw(&state);
                    }
                    KeyCode::Char(']') => {
                        state.octave = (state.octave + 1).min(8);
                        draw(&state);
                    }

                    KeyCode::Char('1') => {
                        state.preset = 0;
                        synth.lock().unwrap().set_config(get_preset(0));
                        draw(&state);
                    }
                    KeyCode::Char('2') => {
                        state.preset = 1;
                        synth.lock().unwrap().set_config(get_preset(1));
                        draw(&state);
                    }
                    KeyCode::Char('3') => {
                        state.preset = 2;
                        synth.lock().unwrap().set_config(get_preset(2));
                        draw(&state);
                    }
                    KeyCode::Char('4') => {
                        state.preset = 3;
                        synth.lock().unwrap().set_config(get_preset(3));
                        draw(&state);
                    }
                    KeyCode::Char('5') => {
                        state.preset = 4;
                        synth.lock().unwrap().set_config(get_preset(4));
                        draw(&state);
                    }

                    KeyCode::Char('-') => {
                        state.tuning = (state.tuning - 0.04167).max(0.0); // ~1 semitone
                        synth.lock().unwrap().set_tuning(state.tuning);
                        draw(&state);
                    }
                    KeyCode::Char('=') => {
                        state.tuning = (state.tuning + 0.04167).min(1.0);
                        synth.lock().unwrap().set_tuning(state.tuning);
                        draw(&state);
                    }

                    KeyCode::Char('z') | KeyCode::Char('Z') => {
                        state.overdrive = (state.overdrive - 0.1).max(0.0);
                        synth.lock().unwrap().set_overdrive(state.overdrive);
                        draw(&state);
                    }
                    KeyCode::Char('x') | KeyCode::Char('X') => {
                        state.overdrive = (state.overdrive + 0.1).min(1.0);
                        synth.lock().unwrap().set_overdrive(state.overdrive);
                        draw(&state);
                    }

                    KeyCode::Char(',') => {
                        state.gate_ms = state.gate_ms.saturating_sub(50).max(50);
                        draw(&state);
                    }
                    KeyCode::Char('.') => {
                        state.gate_ms = (state.gate_ms + 50).min(4000);
                        draw(&state);
                    }

                    KeyCode::Char(c) => {
                        if let Some(note) = key_to_midi(c, state.octave) {
                            synth.lock().unwrap().trigger_note(note, 0.9);
                            sounding.push((note, Instant::now()));
                            state.last_note = Some(note);
                            draw(&state);
                        }
                    }

                    _ => {}
                }
            }
        }
    }

    synth.lock().unwrap().release_all();
    execute!(io::stdout(), cursor::Show)?;
    disable_raw_mode()?;
    println!();

    Ok(())
}
