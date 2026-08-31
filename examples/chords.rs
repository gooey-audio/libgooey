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
use std::time::{Duration, Instant};

use gooey::engine::{Engine, EngineOutput, Instrument};
use gooey::instruments::{
    PolyModSource, PolySynth, PolySynthConfig, POLY_MOD_ROUTE_COUNT, POLY_PARAM_AMP_ATTACK,
    POLY_PARAM_AMP_ATTACK_CURVE, POLY_PARAM_AMP_DECAY, POLY_PARAM_AMP_FALL_CURVE,
    POLY_PARAM_AMP_RELEASE, POLY_PARAM_AMP_SUSTAIN, POLY_PARAM_COUNT, POLY_PARAM_DETUNE,
    POLY_PARAM_FILTER_ATTACK, POLY_PARAM_FILTER_ATTACK_CURVE, POLY_PARAM_FILTER_CUTOFF,
    POLY_PARAM_FILTER_DECAY, POLY_PARAM_FILTER_ENV_AMOUNT, POLY_PARAM_FILTER_FALL_CURVE,
    POLY_PARAM_FILTER_RELEASE, POLY_PARAM_FILTER_RESONANCE, POLY_PARAM_FILTER_SUSTAIN,
    POLY_PARAM_OSC_A_LEVEL, POLY_PARAM_OSC_A_WAVEFORM, POLY_PARAM_OSC_B_LEVEL,
    POLY_PARAM_OSC_B_WAVEFORM, POLY_PARAM_PITCH_ATTACK, POLY_PARAM_PITCH_ATTACK_CURVE,
    POLY_PARAM_PITCH_DECAY, POLY_PARAM_PITCH_ENV_AMOUNT, POLY_PARAM_PITCH_FALL_CURVE,
    POLY_PARAM_PITCH_RELEASE, POLY_PARAM_PITCH_SUSTAIN, POLY_PARAM_SATURATION,
    POLY_PARAM_STEREO_WIDTH, POLY_PARAM_VOLUME,
};
use gooey::music::{
    apply_voicing, available_voicings, midi_to_string, Key, NoteName, ScaleType, VoicingType,
};
use gooey::StereoFrame;
use std::sync::{Arc, Mutex};

struct SharedPolySynth(Arc<Mutex<PolySynth>>);

impl Instrument for SharedPolySynth {
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        self.0.lock().unwrap().trigger_with_velocity(time, velocity);
    }

    fn tick(&mut self, current_time: f64) -> f32 {
        self.0.lock().unwrap().tick(current_time)
    }

    fn tick_stereo(&mut self, current_time: f64) -> Option<StereoFrame> {
        Some(self.0.lock().unwrap().tick_frame(current_time))
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

#[derive(Clone, Copy)]
struct Control {
    id: u32,
    label: &'static str,
}

const OSC_CONTROLS: &[Control] = &[
    Control {
        id: POLY_PARAM_OSC_A_WAVEFORM,
        label: "Osc A waveform",
    },
    Control {
        id: POLY_PARAM_OSC_A_LEVEL,
        label: "Osc A level",
    },
    Control {
        id: POLY_PARAM_OSC_B_WAVEFORM,
        label: "Osc B waveform",
    },
    Control {
        id: POLY_PARAM_OSC_B_LEVEL,
        label: "Osc B level",
    },
    Control {
        id: POLY_PARAM_DETUNE,
        label: "Detune",
    },
    Control {
        id: POLY_PARAM_STEREO_WIDTH,
        label: "Stereo width",
    },
];

const AMP_CONTROLS: &[Control] = &[
    Control {
        id: POLY_PARAM_AMP_ATTACK,
        label: "Attack",
    },
    Control {
        id: POLY_PARAM_AMP_DECAY,
        label: "Decay",
    },
    Control {
        id: POLY_PARAM_AMP_SUSTAIN,
        label: "Sustain",
    },
    Control {
        id: POLY_PARAM_AMP_RELEASE,
        label: "Release",
    },
    Control {
        id: POLY_PARAM_AMP_ATTACK_CURVE,
        label: "Attack curve",
    },
    Control {
        id: POLY_PARAM_AMP_FALL_CURVE,
        label: "Fall curve",
    },
];

const PITCH_CONTROLS: &[Control] = &[
    Control {
        id: POLY_PARAM_PITCH_ENV_AMOUNT,
        label: "Amount (-24..+24 st)",
    },
    Control {
        id: POLY_PARAM_PITCH_ATTACK,
        label: "Attack",
    },
    Control {
        id: POLY_PARAM_PITCH_DECAY,
        label: "Decay",
    },
    Control {
        id: POLY_PARAM_PITCH_SUSTAIN,
        label: "Sustain",
    },
    Control {
        id: POLY_PARAM_PITCH_RELEASE,
        label: "Release",
    },
    Control {
        id: POLY_PARAM_PITCH_ATTACK_CURVE,
        label: "Attack curve",
    },
    Control {
        id: POLY_PARAM_PITCH_FALL_CURVE,
        label: "Fall curve",
    },
];

const FILTER_CONTROLS: &[Control] = &[
    Control {
        id: POLY_PARAM_FILTER_CUTOFF,
        label: "Cutoff",
    },
    Control {
        id: POLY_PARAM_FILTER_RESONANCE,
        label: "Resonance",
    },
    Control {
        id: POLY_PARAM_FILTER_ENV_AMOUNT,
        label: "Envelope amount",
    },
    Control {
        id: POLY_PARAM_FILTER_ATTACK,
        label: "Attack",
    },
    Control {
        id: POLY_PARAM_FILTER_DECAY,
        label: "Decay",
    },
    Control {
        id: POLY_PARAM_FILTER_SUSTAIN,
        label: "Sustain",
    },
    Control {
        id: POLY_PARAM_FILTER_RELEASE,
        label: "Release",
    },
    Control {
        id: POLY_PARAM_FILTER_ATTACK_CURVE,
        label: "Attack curve",
    },
    Control {
        id: POLY_PARAM_FILTER_FALL_CURVE,
        label: "Fall curve",
    },
];

const EXPRESSION_CONTROLS: &[Control] = &[
    Control {
        id: POLY_PARAM_SATURATION,
        label: "Saturation",
    },
    Control {
        id: POLY_PARAM_VOLUME,
        label: "Volume",
    },
];

const PAGE_NAMES: [&str; 6] = ["Osc", "Amp", "Pitch", "Filter", "Expression", "Matrix"];

#[derive(Clone, Copy)]
enum RouteField {
    Enabled,
    Source,
    Destination,
    Depth,
    Curve,
    KeyScale,
}

impl RouteField {
    fn next(self) -> Self {
        match self {
            Self::Enabled => Self::Source,
            Self::Source => Self::Destination,
            Self::Destination => Self::Depth,
            Self::Depth => Self::Curve,
            Self::Curve => Self::KeyScale,
            Self::KeyScale => Self::Enabled,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Source => "source",
            Self::Destination => "destination",
            Self::Depth => "depth",
            Self::Curve => "curve",
            Self::KeyScale => "key scale",
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
    velocity: f32,
    page: usize,
    control_index: usize,
    route_field: RouteField,
    one_shot_release_at: Option<Instant>,
    presets: [PolySynthConfig; 5],
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
            velocity: 0.8,
            page: 0,
            control_index: 0,
            route_field: RouteField::Depth,
            one_shot_release_at: None,
            presets: [
                PolySynthConfig::default(),
                PolySynthConfig::pad(),
                PolySynthConfig::pluck(),
                PolySynthConfig::keys(),
                PolySynthConfig::strings(),
            ],
        }
    }

    fn controls(&self) -> &'static [Control] {
        match self.page {
            0 => OSC_CONTROLS,
            1 => AMP_CONTROLS,
            2 => PITCH_CONTROLS,
            3 => FILTER_CONTROLS,
            4 => EXPRESSION_CONTROLS,
            _ => &[],
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

fn draw_ui(state: &AppState, synth: &PolySynth) {
    print!("\x1b[2J\x1b[H");

    let key = state.key();
    let chords = state.chords();
    let voicings = state.current_voicings();
    let vi = state.voicing_index.min(voicings.len() - 1);

    println!("=== Chord Explorer ===\r");
    println!("\r");
    println!("  SPACE=play  ENTER=sustain  Q=quit  TAB=maj/min\r");
    println!("  Left/Right=key  Up/Down=chord  [/]=voicing  </>=level\r");
    println!("  P=preset  O/K=octave  V/B=velocity  1..6=editor page\r");
    println!("  W/S=select  A/D=edit  F=matrix field\r");
    println!("\r");
    println!(
        "  Key: {}    Octave: {}    Velocity: {:.2}    Level: {}    Preset: {}\r",
        key,
        state.octave,
        state.velocity,
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

    println!(
        "  Editor: {}  [{}]\r",
        PAGE_NAMES[state.page],
        PAGE_NAMES.join(" | ")
    );
    if state.page < 5 {
        for (index, control) in state.controls().iter().enumerate() {
            let marker = if index == state.control_index {
                ">"
            } else {
                " "
            };
            let value = synth.param(control.id).unwrap_or(f32::NAN);
            let filled = (value.clamp(0.0, 1.0) * 20.0).round() as usize;
            println!(
                "  {marker} {:<24} [{:<20}] {:.3}\r",
                control.label,
                "#".repeat(filled),
                value
            );
        }
    } else {
        println!("  Editing field: {}\r", state.route_field.label());
        for slot in 0..POLY_MOD_ROUTE_COUNT {
            let marker = if slot == state.control_index {
                ">"
            } else {
                " "
            };
            if let Some(route) = synth.mod_route(slot) {
                let source = match route.source {
                    PolyModSource::Velocity => "velocity",
                    PolyModSource::KeyPosition => "key",
                };
                println!(
                    "  {marker} {} {:<8} -> param {:>2}  depth {:+.2} curve {:.2} key {:+.2}\r",
                    if route.enabled { "on " } else { "off" },
                    source,
                    route.destination,
                    route.depth,
                    route.curve,
                    route.key_scale,
                );
            }
        }
    }
    println!("\r");

    io::stdout().flush().unwrap();
}

fn redraw(state: &AppState, synth: &Arc<Mutex<PolySynth>>) {
    draw_ui(state, &synth.lock().unwrap());
}

fn play_chord(synth: &Arc<Mutex<PolySynth>>, notes: &[u8], velocity: f32) {
    let mut s = synth.lock().unwrap();
    for &note in notes {
        s.trigger_note(note, velocity);
    }
}

fn release_all(synth: &Arc<Mutex<PolySynth>>) {
    let mut s = synth.lock().unwrap();
    s.release_all();
}

fn change_selection(state: &mut AppState, direction: i32) {
    let count = if state.page < 5 {
        state.controls().len()
    } else {
        POLY_MOD_ROUTE_COUNT
    };
    state.control_index =
        (state.control_index as i32 + direction).rem_euclid(count as i32) as usize;
}

fn adjust_editor(state: &mut AppState, synth: &mut PolySynth, direction: f32) {
    if state.page < 5 {
        let control = state.controls()[state.control_index];
        let value = synth.param(control.id).unwrap_or(0.5) + direction * 0.025;
        synth.set_param(control.id, value);
        state.presets[state.preset_index].set_param(control.id, value);
        return;
    }

    let Some(mut route) = synth.mod_route(state.control_index) else {
        return;
    };
    match state.route_field {
        RouteField::Enabled => route.enabled = !route.enabled,
        RouteField::Source => {
            route.source = match route.source {
                PolyModSource::Velocity => PolyModSource::KeyPosition,
                PolyModSource::KeyPosition => PolyModSource::Velocity,
            }
        }
        RouteField::Destination => {
            route.destination = (route.destination as i32 + direction.signum() as i32)
                .rem_euclid(POLY_PARAM_COUNT as i32) as u32;
        }
        RouteField::Depth => route.depth = (route.depth + direction * 0.05).clamp(-1.0, 1.0),
        RouteField::Curve => route.curve = (route.curve + direction * 0.05).clamp(0.0, 1.0),
        RouteField::KeyScale => {
            route.key_scale = (route.key_scale + direction * 0.05).clamp(-1.0, 1.0)
        }
    }
    synth.set_mod_route(state.control_index, route);
    state.presets[state.preset_index].set_mod_route(state.control_index, route);
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

    redraw(&state, &synth);

    loop {
        if state
            .one_shot_release_at
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            release_all(&synth);
            state.one_shot_release_at = None;
        }
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
                        redraw(&state, &synth);
                    }
                    KeyCode::Right => {
                        state.root_index = (state.root_index + 1) % 12;
                        state.voicing_index = 0;
                        if state.sustaining {
                            release_all(&synth);
                            state.sustaining = false;
                        }
                        redraw(&state, &synth);
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
                        redraw(&state, &synth);
                    }

                    // Chord selection
                    KeyCode::Up => {
                        state.selected_degree = (state.selected_degree + 6) % 7;
                        state.voicing_index =
                            state.voicing_index.min(state.current_voicings().len() - 1);
                        redraw(&state, &synth);
                    }
                    KeyCode::Down => {
                        state.selected_degree = (state.selected_degree + 1) % 7;
                        state.voicing_index =
                            state.voicing_index.min(state.current_voicings().len() - 1);
                        redraw(&state, &synth);
                    }

                    // Voicing selection
                    KeyCode::Char('[') => {
                        let max = state.current_voicings().len();
                        state.voicing_index = (state.voicing_index + max - 1) % max;
                        redraw(&state, &synth);
                    }
                    KeyCode::Char(']') => {
                        let max = state.current_voicings().len();
                        state.voicing_index = (state.voicing_index + 1) % max;
                        redraw(&state, &synth);
                    }

                    // Chord level
                    KeyCode::Char('<') | KeyCode::Char(',') => {
                        state.chord_level = state.chord_level.prev();
                        state.voicing_index = 0;
                        redraw(&state, &synth);
                    }
                    KeyCode::Char('>') | KeyCode::Char('.') => {
                        state.chord_level = state.chord_level.next();
                        state.voicing_index = 0;
                        redraw(&state, &synth);
                    }

                    // Octave
                    KeyCode::Char('o') | KeyCode::Char('O') => {
                        state.octave = (state.octave - 1).max(2);
                        redraw(&state, &synth);
                    }
                    KeyCode::Char('k') | KeyCode::Char('K') => {
                        state.octave = (state.octave + 1).min(6);
                        redraw(&state, &synth);
                    }

                    // Strike velocity for comparing expression curves.
                    KeyCode::Char('v') | KeyCode::Char('V') => {
                        state.velocity = (state.velocity - 0.05).max(0.05);
                        redraw(&state, &synth);
                    }
                    KeyCode::Char('b') | KeyCode::Char('B') => {
                        state.velocity = (state.velocity + 0.05).min(1.0);
                        redraw(&state, &synth);
                    }

                    // Editor page and selected control.
                    KeyCode::Char(page @ '1'..='6') => {
                        state.page = page.to_digit(10).unwrap() as usize - 1;
                        state.control_index = 0;
                        redraw(&state, &synth);
                    }
                    KeyCode::Char('w') | KeyCode::Char('W') => {
                        change_selection(&mut state, -1);
                        redraw(&state, &synth);
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        change_selection(&mut state, 1);
                        redraw(&state, &synth);
                    }
                    KeyCode::Char('a') | KeyCode::Char('A') => {
                        adjust_editor(&mut state, &mut synth.lock().unwrap(), -1.0);
                        redraw(&state, &synth);
                    }
                    KeyCode::Char('d') | KeyCode::Char('D') => {
                        adjust_editor(&mut state, &mut synth.lock().unwrap(), 1.0);
                        redraw(&state, &synth);
                    }
                    KeyCode::Char('f') | KeyCode::Char('F') if state.page == 5 => {
                        state.route_field = state.route_field.next();
                        redraw(&state, &synth);
                    }

                    // Preset cycling
                    KeyCode::Char('p') | KeyCode::Char('P') => {
                        state.preset_index = (state.preset_index + 1) % PRESET_NAMES.len();
                        let config = state.presets[state.preset_index];
                        synth.lock().unwrap().set_config(config);
                        redraw(&state, &synth);
                    }

                    // Play chord (one-shot)
                    KeyCode::Char(' ') => {
                        if state.sustaining || state.one_shot_release_at.is_some() {
                            release_all(&synth);
                            state.sustaining = false;
                        }
                        let notes = state.current_midi_notes();
                        play_chord(&synth, &notes, state.velocity);
                        state.one_shot_release_at =
                            Some(Instant::now() + Duration::from_millis(350));
                        redraw(&state, &synth);
                    }

                    // Sustain toggle
                    KeyCode::Enter => {
                        if state.sustaining {
                            release_all(&synth);
                            state.sustaining = false;
                        } else {
                            if state.one_shot_release_at.take().is_some() {
                                release_all(&synth);
                            }
                            let notes = state.current_midi_notes();
                            play_chord(&synth, &notes, state.velocity);
                            state.sustaining = true;
                        }
                        redraw(&state, &synth);
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
