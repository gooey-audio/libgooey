/* Chord Explorer - Interactive CLI for browsing keys, chords, and voicings.
Uses a 6-voice poly synth to audition different chord voicings in real time.
*/

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use std::io::{self, Write};

use gooey::engine::{Engine, EngineOutput, Instrument};
use gooey::instruments::{PolySynth, PolySynthConfig};
use gooey::music::{
    apply_voicing, available_voicings, midi_to_string, Key, NoteName, ScaleType, VoicingType,
};
use std::sync::{Arc, Mutex};

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

#[derive(Clone, Copy, PartialEq)]
enum ChordLevel {
    Triads,
    Sevenths,
    Ninths,
    Elevenths,
    Thirteenths,
}

impl ChordLevel {
    fn label(self) -> &'static str {
        match self {
            ChordLevel::Triads => "Triads",
            ChordLevel::Sevenths => "7ths",
            ChordLevel::Ninths => "9ths",
            ChordLevel::Elevenths => "11ths",
            ChordLevel::Thirteenths => "13ths",
        }
    }

    fn next(self) -> Self {
        match self {
            ChordLevel::Triads => ChordLevel::Sevenths,
            ChordLevel::Sevenths => ChordLevel::Ninths,
            ChordLevel::Ninths => ChordLevel::Elevenths,
            ChordLevel::Elevenths => ChordLevel::Thirteenths,
            ChordLevel::Thirteenths => ChordLevel::Triads,
        }
    }

    fn prev(self) -> Self {
        match self {
            ChordLevel::Triads => ChordLevel::Thirteenths,
            ChordLevel::Sevenths => ChordLevel::Triads,
            ChordLevel::Ninths => ChordLevel::Sevenths,
            ChordLevel::Elevenths => ChordLevel::Ninths,
            ChordLevel::Thirteenths => ChordLevel::Elevenths,
        }
    }
}

struct AppState {
    root_index: usize,
    scale_type: ScaleType,
    selected_degree: usize,
    voicing_index: usize,
    chord_level: ChordLevel,
    octave: i8,
    sustaining: bool,
    preset_index: usize,
}

impl AppState {
    fn new() -> Self {
        Self {
            root_index: 0, // C
            scale_type: ScaleType::Major,
            selected_degree: 0,
            voicing_index: 0,
            chord_level: ChordLevel::Triads,
            octave: 4,
            sustaining: false,
            preset_index: 0,
        }
    }

    fn key(&self) -> Key {
        Key::new(NoteName::ALL[self.root_index], self.scale_type)
    }

    fn chords(&self) -> Vec<gooey::music::Chord> {
        let key = self.key();
        match self.chord_level {
            ChordLevel::Triads => key.diatonic_triads(),
            ChordLevel::Sevenths => key.diatonic_sevenths(),
            ChordLevel::Ninths => key.diatonic_ninths(),
            ChordLevel::Elevenths => key.diatonic_elevenths(),
            ChordLevel::Thirteenths => key.diatonic_thirteenths(),
        }
    }

    fn current_voicings(&self) -> Vec<VoicingType> {
        let chords = self.chords();
        let chord = &chords[self.selected_degree];
        available_voicings(&chord.quality)
    }

    fn current_midi_notes(&self) -> Vec<u8> {
        let chords = self.chords();
        let chord = &chords[self.selected_degree];
        let voicings = available_voicings(&chord.quality);
        let voicing = voicings[self.voicing_index.min(voicings.len() - 1)];
        apply_voicing(chord, voicing, self.octave)
    }
}

const PRESET_NAMES: [&str; 5] = ["Default", "Pad", "Pluck", "Keys", "Strings"];

fn get_preset(index: usize) -> PolySynthConfig {
    match index {
        0 => PolySynthConfig::default(),
        1 => PolySynthConfig::pad(),
        2 => PolySynthConfig::pluck(),
        3 => PolySynthConfig::keys(),
        4 => PolySynthConfig::strings(),
        _ => PolySynthConfig::default(),
    }
}

fn draw_ui(state: &AppState) {
    print!("\x1b[2J\x1b[H");

    let key = state.key();
    let chords = state.chords();
    let voicings = state.current_voicings();
    let vi = state.voicing_index.min(voicings.len() - 1);

    println!("=== Chord Explorer ===\r");
    println!("\r");
    println!("  SPACE=play  ENTER=sustain  Q=quit  TAB=maj/min\r");
    println!("  Left/Right=key  Up/Down=chord  [/]=voicing  </>=level\r");
    println!("  P=preset  O/K=octave down/up\r");
    println!("\r");
    println!(
        "  Key: {}    Octave: {}    Level: {}    Preset: {}\r",
        key,
        state.octave,
        state.chord_level.label(),
        PRESET_NAMES[state.preset_index]
    );
    if state.sustaining {
        println!("  [SUSTAINING - press ENTER to release]\r");
    }
    println!("\r");

    // Draw chord list
    for (i, chord) in chords.iter().enumerate() {
        let roman = key.roman_numeral(i + 1);
        let marker = if i == state.selected_degree { ">" } else { " " };

        let notes: Vec<String> = chord
            .note_names()
            .iter()
            .map(|n| format!("{}", n))
            .collect();
        let notes_str = notes.join(" ");

        println!(
            "  {} {:<5} {:<12} [{}]\r",
            marker,
            roman,
            chord.display_name(),
            notes_str,
        );
    }

    println!("\r");

    // Draw voicing info
    let midi_notes = state.current_midi_notes();
    let note_names: Vec<String> = midi_notes.iter().map(|&n| midi_to_string(n)).collect();

    println!(
        "  Voicing: {} ({}/{})\r",
        voicings[vi],
        vi + 1,
        voicings.len()
    );
    println!("  Notes:   {}\r", note_names.join("  "));
    println!("\r");

    // Draw voicing list
    print!("  ");
    for (i, v) in voicings.iter().enumerate() {
        if i == vi {
            print!("[{}]", v);
        } else {
            print!(" {} ", v);
        }
        if i < voicings.len() - 1 {
            print!(" | ");
        }
    }
    println!("\r");

    io::stdout().flush().unwrap();
}

fn play_chord(synth: &Arc<Mutex<PolySynth>>, notes: &[u8], sustain: bool) {
    let mut s = synth.lock().unwrap();
    for &note in notes {
        s.trigger_note(note, 0.8);
    }
    if !sustain {
        // Release immediately so the envelope plays attack -> decay -> release
        // rather than holding at the sustain level forever
        s.release_all();
    }
}

fn release_all(synth: &Arc<Mutex<PolySynth>>) {
    let mut s = synth.lock().unwrap();
    s.release_all();
}

fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;

    // Create poly synth
    let synth = Arc::new(Mutex::new(PolySynth::new(sample_rate)));

    // Create engine
    let mut engine = Engine::new(sample_rate);
    let shared_synth = SharedPolySynth(Arc::clone(&synth));
    engine.add_instrument("poly", Box::new(shared_synth));
    engine.set_master_gain(0.8);

    let audio_engine = Arc::new(Mutex::new(engine));

    // Start audio output
    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;
    engine_output.create_stream_with_engine(audio_engine)?;
    engine_output.start()?;

    // App state
    let mut state = AppState::new();

    // Terminal setup
    enable_raw_mode()?;
    execute!(io::stdout(), cursor::Hide, Clear(ClearType::All))?;

    draw_ui(&state);

    loop {
        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match code {
                    KeyCode::Char('q') | KeyCode::Char('Q') => break,

                    // Key root selection
                    KeyCode::Left => {
                        state.root_index = (state.root_index + 11) % 12;
                        state.voicing_index = 0;
                        if state.sustaining {
                            release_all(&synth);
                            state.sustaining = false;
                        }
                        draw_ui(&state);
                    }
                    KeyCode::Right => {
                        state.root_index = (state.root_index + 1) % 12;
                        state.voicing_index = 0;
                        if state.sustaining {
                            release_all(&synth);
                            state.sustaining = false;
                        }
                        draw_ui(&state);
                    }

                    // Major/Minor toggle
                    KeyCode::Tab => {
                        state.scale_type = match state.scale_type {
                            ScaleType::Major => ScaleType::NaturalMinor,
                            ScaleType::NaturalMinor => ScaleType::Major,
                        };
                        state.voicing_index = 0;
                        if state.sustaining {
                            release_all(&synth);
                            state.sustaining = false;
                        }
                        draw_ui(&state);
                    }

                    // Chord selection
                    KeyCode::Up => {
                        state.selected_degree = (state.selected_degree + 6) % 7;
                        state.voicing_index =
                            state.voicing_index.min(state.current_voicings().len() - 1);
                        draw_ui(&state);
                    }
                    KeyCode::Down => {
                        state.selected_degree = (state.selected_degree + 1) % 7;
                        state.voicing_index =
                            state.voicing_index.min(state.current_voicings().len() - 1);
                        draw_ui(&state);
                    }

                    // Voicing selection
                    KeyCode::Char('[') => {
                        let max = state.current_voicings().len();
                        state.voicing_index = (state.voicing_index + max - 1) % max;
                        draw_ui(&state);
                    }
                    KeyCode::Char(']') => {
                        let max = state.current_voicings().len();
                        state.voicing_index = (state.voicing_index + 1) % max;
                        draw_ui(&state);
                    }

                    // Chord level
                    KeyCode::Char('<') | KeyCode::Char(',') => {
                        state.chord_level = state.chord_level.prev();
                        state.voicing_index = 0;
                        draw_ui(&state);
                    }
                    KeyCode::Char('>') | KeyCode::Char('.') => {
                        state.chord_level = state.chord_level.next();
                        state.voicing_index = 0;
                        draw_ui(&state);
                    }

                    // Octave
                    KeyCode::Char('o') | KeyCode::Char('O') => {
                        state.octave = (state.octave - 1).max(2);
                        draw_ui(&state);
                    }
                    KeyCode::Char('k') | KeyCode::Char('K') => {
                        state.octave = (state.octave + 1).min(6);
                        draw_ui(&state);
                    }

                    // Preset cycling
                    KeyCode::Char('p') | KeyCode::Char('P') => {
                        state.preset_index = (state.preset_index + 1) % PRESET_NAMES.len();
                        let config = get_preset(state.preset_index);
                        synth.lock().unwrap().set_config(config);
                        draw_ui(&state);
                    }

                    // Play chord (one-shot)
                    KeyCode::Char(' ') => {
                        if state.sustaining {
                            release_all(&synth);
                            state.sustaining = false;
                        }
                        let notes = state.current_midi_notes();
                        play_chord(&synth, &notes, false);
                        draw_ui(&state);
                    }

                    // Sustain toggle
                    KeyCode::Enter => {
                        if state.sustaining {
                            release_all(&synth);
                            state.sustaining = false;
                        } else {
                            let notes = state.current_midi_notes();
                            play_chord(&synth, &notes, true);
                            state.sustaining = true;
                        }
                        draw_ui(&state);
                    }

                    _ => {}
                }
            }
        }
    }

    // Cleanup
    execute!(io::stdout(), cursor::Show)?;
    disable_raw_mode()?;
    println!();

    Ok(())
}
