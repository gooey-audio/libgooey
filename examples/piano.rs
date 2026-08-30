//! Piano Explorer — play chord progressions on a multi-sampled acoustic piano.
//!
//! Run with:
//!   cargo run --example piano --features native,crossterm,bounce
//!
//! With no argument it plays a synthesized stand-in so the UI and voice engine
//! are still demonstrable. To hear a real piano, point it at a pack — and use
//! `--release`, because decoding a few hundred WAV files takes ~2 s optimized
//! but over 30 s in a debug build:
//!
//!   ./scripts/fetch-piano-pack.sh
//!   cargo run --release --example piano --features native,crossterm,bounce -- \
//!       assets/piano/SalamanderGrandPianoV3_44.1khz16bit/SalamanderGrandPianoV3.sfz
//!
//! The pack path may also come from the GOOEY_PIANO_PACK environment variable.
//! Sample data is never vendored into this repository — see
//! docs/multisample-instruments.md for licensing and bring-your-own-pack notes.
//!
//! By default a big pack is thinned to six velocity layers on load. Pass
//! `--layers N` to keep a different number, or `--layers 0` to load the pack
//! exactly as it is — which is what you want for a pack already shrunk by
//! `cargo run --example prepare_piano_pack`.

#[cfg(all(feature = "native", feature = "bounce"))]
use crossterm::{
    cursor,
    event::{
        self, Event, KeyCode, KeyEvent, KeyEventKind, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, Clear, ClearType,
    },
};
#[cfg(all(feature = "native", feature = "bounce"))]
use gooey::engine::{Engine, EngineOutput, Instrument};
#[cfg(all(feature = "native", feature = "bounce"))]
use gooey::frame::StereoFrame;
#[cfg(all(feature = "native", feature = "bounce"))]
use gooey::instruments::multisample::{
    MultiSampleConfig, MultiSampleInstrument, SampleMap, SampleZone, ZoneTrigger,
};
#[cfg(all(feature = "native", feature = "bounce"))]
use gooey::instruments::multisample_pack::{load_sfz, PackLoadOptions};
#[cfg(all(feature = "native", feature = "bounce"))]
use gooey::mixer::StereoSampleBuffer;
#[cfg(all(feature = "native", feature = "bounce"))]
use gooey::music::{
    apply_voicing, available_voicings, midi_to_string, ChordDynamics, Key, NoteName, ScaleType,
    VelocityProfile, VoicingType,
};
#[cfg(all(feature = "native", feature = "bounce"))]
use std::io::{self, Write};
#[cfg(all(feature = "native", feature = "bounce"))]
use std::sync::{Arc, Mutex};
#[cfg(all(feature = "native", feature = "bounce"))]
use std::time::{Duration, Instant};

#[cfg(all(feature = "native", feature = "bounce"))]
const SAMPLE_RATE: f32 = 44_100.0;
#[cfg(all(feature = "native", feature = "bounce"))]
const UI_REFRESH: Duration = Duration::from_millis(80);
/// Home-row keys mapped to scale degrees I..VII.
#[cfg(all(feature = "native", feature = "bounce"))]
const DEGREE_KEYS: [char; 7] = ['a', 's', 'd', 'f', 'g', 'h', 'j'];
#[cfg(all(feature = "native", feature = "bounce"))]
const PRESET_NAMES: [&str; 3] = ["Default", "Soft", "Bright"];

/// The engine owns its instruments, but the UI thread needs to send note-on,
/// note-off, and pedal events. Sharing one `Arc<Mutex<..>>` and forwarding the
/// trait through a newtype is the repo-wide idiom (see `examples/chords.rs`).
#[cfg(all(feature = "native", feature = "bounce"))]
struct SharedPiano(Arc<Mutex<MultiSampleInstrument>>);

#[cfg(all(feature = "native", feature = "bounce"))]
impl Instrument for SharedPiano {
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        self.0.lock().unwrap().trigger_with_velocity(time, velocity);
    }

    fn tick(&mut self, current_time: f64) -> f32 {
        self.0.lock().unwrap().tick(current_time)
    }

    /// Forward the stereo override too, or the engine would fall back to the
    /// mono seam and the piano would lose its recorded image.
    fn tick_stereo(&mut self, current_time: f64) -> Option<StereoFrame> {
        self.0.lock().unwrap().tick_stereo(current_time)
    }

    fn is_active(&self) -> bool {
        self.0.lock().unwrap().is_active()
    }

    fn set_midi_note(&mut self, note: u8) {
        self.0.lock().unwrap().set_midi_note(note);
    }
}

// ---------------------------------------------------------------------------
// Pack loading, with a synthesized fallback
// ---------------------------------------------------------------------------

/// Where the map came from, so the UI can tell the user whether they are
/// hearing a real piano or the stand-in.
#[cfg(all(feature = "native", feature = "bounce"))]
struct LoadedMap {
    map: Arc<SampleMap>,
    source: String,
    notes: Vec<String>,
}

#[cfg(all(feature = "native", feature = "bounce"))]
fn resolve_pack_path() -> Option<String> {
    std::env::args()
        .nth(1)
        .filter(|a| !a.starts_with("--"))
        .or_else(|| std::env::var("GOOEY_PIANO_PACK").ok())
        .filter(|path| !path.is_empty())
}

/// Velocity layers to keep, from `--layers N`. Zero means "load the pack as it
/// is", which is what you want for a pack already thinned by
/// `prepare_piano_pack` — otherwise this would throw away layers that were
/// deliberately kept.
#[cfg(all(feature = "native", feature = "bounce"))]
fn resolve_layer_limit() -> Option<usize> {
    let args: Vec<String> = std::env::args().collect();
    let requested = args
        .iter()
        .position(|a| a == "--layers")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<usize>().ok());
    match requested {
        Some(0) => None,
        Some(n) => Some(n),
        None => Some(gooey::instruments::multisample::DEFAULT_VELOCITY_LAYERS),
    }
}

#[cfg(all(feature = "native", feature = "bounce"))]
fn load_map() -> LoadedMap {
    let Some(path) = resolve_pack_path() else {
        return synthesized_map(vec![
            "No pack given — playing a synthesized stand-in.".to_string(),
            "Pass an .sfz path or set GOOEY_PIANO_PACK for the real thing.".to_string(),
            "See docs/multisample-instruments.md.".to_string(),
        ]);
    };

    // Decoding a full piano pack means reading a few hundred WAV files — a
    // couple of seconds in release, and well over half a minute in a debug
    // build. Say so before starting, or the terminal just sits blank and looks
    // hung. This runs before raw mode is enabled, so plain println is fine.
    println!("Loading pack: {path}");
    if cfg!(debug_assertions) {
        println!("  (debug build — decoding is ~15x slower here; add --release if this drags)");
    }
    io::stdout().flush().ok();

    let options = PackLoadOptions::default().with_velocity_layers(resolve_layer_limit());
    let started = Instant::now();
    match load_sfz(&path, &options) {
        Ok(pack) => {
            let map = pack.map.build();
            let elapsed = started.elapsed().as_secs_f32();
            println!("  {} zones in {elapsed:.1}s", pack.zones_loaded);
            let mut notes = vec![format!(
                "{} zones from {} regions, {} velocity layers, loaded in {elapsed:.1}s",
                pack.zones_loaded,
                pack.regions_declared,
                map.velocity_layers()
            )];
            // Surface at most a few warnings; a big pack can produce many.
            notes.extend(pack.warnings.into_iter().take(3));
            LoadedMap {
                map,
                source: path,
                notes,
            }
        }
        Err(error) => synthesized_map(vec![
            format!("Could not load {path}:"),
            error,
            "Falling back to the synthesized stand-in.".to_string(),
        ]),
    }
}

/// A three-zone plucked tone covering the keyboard in octaves. Not a piano —
/// but it exercises zone selection, velocity layers, pitch shifting, and the
/// sustain pedal, so the example is useful without a 400 MB download. Mirrors
/// the `demo_buffer()` / `demo_loop()` fallbacks in the granulator and loop
/// mixer examples.
#[cfg(all(feature = "native", feature = "bounce"))]
fn synthesized_map(notes: Vec<String>) -> LoadedMap {
    /// Root keys one octave apart, so no note is shifted more than 6 semitones.
    const ROOTS: [u8; 5] = [36, 48, 60, 72, 84];
    /// Two velocity layers: a mellow one and a brighter, harmonically richer one.
    const LAYERS: [(u8, u8, f32); 2] = [(1, 72, 0.35), (73, 127, 1.0)];

    let mut map = SampleMap::new();
    for root in ROOTS {
        for (lovel, hivel, brightness) in LAYERS {
            let buffer = pluck(root, brightness);
            let mut zone = SampleZone::new(buffer, root)
                .with_key_range(root.saturating_sub(6), (root + 6).min(127))
                .with_velocity_range(lovel, hivel);
            zone.envelope = gooey::envelope::ADSRConfig::new(0.002, 0.002, 1.0, 0.35);
            map.push_zone(zone).expect("synthesized zone is valid");
        }
    }

    LoadedMap {
        map: map.build(),
        source: "synthesized stand-in".to_string(),
        notes,
    }
}

/// Additive decaying tone at the pitch of `midi_note`. `brightness` scales how
/// much upper-harmonic content is present, imitating a harder strike.
#[cfg(all(feature = "native", feature = "bounce"))]
fn pluck(midi_note: u8, brightness: f32) -> StereoSampleBuffer {
    let hz = 440.0 * 2.0_f32.powf((midi_note as f32 - 69.0) / 12.0);
    let frames = (SAMPLE_RATE * 4.0) as usize;
    let mut left = Vec::with_capacity(frames);
    let mut right = Vec::with_capacity(frames);

    for i in 0..frames {
        let t = i as f32 / SAMPLE_RATE;
        let mut sample = 0.0;
        for harmonic in 1..=8u32 {
            // Upper partials decay faster, as they do on a real string.
            let decay = (-t * (1.4 + harmonic as f32 * 0.9)).exp();
            let level = brightness.powi(harmonic as i32 - 1) / harmonic as f32;
            sample += (t * hz * harmonic as f32 * std::f32::consts::TAU).sin() * level * decay;
        }
        sample *= 0.22;
        // A touch of detune between channels for a plausible stereo image.
        let spread = (t * 0.6).sin() * 0.04;
        left.push(sample * (1.0 - spread));
        right.push(sample * (1.0 + spread));
    }

    StereoSampleBuffer::from_channels(left, right, SAMPLE_RATE).expect("valid pluck buffer")
}

// ---------------------------------------------------------------------------
// UI state
// ---------------------------------------------------------------------------

#[cfg(all(feature = "native", feature = "bounce"))]
#[derive(Clone, Copy, PartialEq)]
enum ChordLevel {
    Triads,
    Sevenths,
    Ninths,
}

#[cfg(all(feature = "native", feature = "bounce"))]
impl ChordLevel {
    fn label(self) -> &'static str {
        match self {
            ChordLevel::Triads => "Triads",
            ChordLevel::Sevenths => "7ths",
            ChordLevel::Ninths => "9ths",
        }
    }

    fn next(self) -> Self {
        match self {
            ChordLevel::Triads => ChordLevel::Sevenths,
            ChordLevel::Sevenths => ChordLevel::Ninths,
            ChordLevel::Ninths => ChordLevel::Triads,
        }
    }

    fn prev(self) -> Self {
        match self {
            ChordLevel::Triads => ChordLevel::Ninths,
            ChordLevel::Sevenths => ChordLevel::Triads,
            ChordLevel::Ninths => ChordLevel::Sevenths,
        }
    }
}

/// How chord keys behave, decided once at startup from what the terminal can
/// report.
///
/// Only terminals implementing the kitty keyboard protocol (kitty, foot,
/// WezTerm, Ghostty, recent iTerm2) send key-*release* events. Terminal.app and
/// most others send press only — so on those we cannot know when the key comes
/// up, and hold-to-sustain would leave the chord stuck on forever.
#[cfg(all(feature = "native", feature = "bounce"))]
#[derive(Clone, Copy, PartialEq)]
enum KeyMode {
    /// Press sounds the chord, release damps it.
    Hold,
    /// Press sounds the chord, pressing again damps it.
    Toggle,
}

#[cfg(all(feature = "native", feature = "bounce"))]
impl KeyMode {
    fn label(self) -> &'static str {
        match self {
            KeyMode::Hold => "hold",
            KeyMode::Toggle => "toggle (terminal sends no key-up)",
        }
    }
}

#[cfg(all(feature = "native", feature = "bounce"))]
struct AppState {
    key_mode: KeyMode,
    root_index: usize,
    scale_type: ScaleType,
    voicing_index: usize,
    chord_level: ChordLevel,
    octave: i8,
    velocity: f32,
    preset_index: usize,
    pedal: bool,
    /// Per-voice velocity weighting plus humanizing, so chords are not struck
    /// mechanically. Held across chords so its random sequence keeps advancing.
    dynamics: ChordDynamics,
    /// The velocities the last chord actually used, for the readout.
    last_velocities: Vec<f32>,
    /// Degrees whose key is currently held down, so a chord is released only
    /// when its own key comes up.
    held: [bool; 7],
    /// Last chord triggered per degree, so release hits the same notes even if
    /// the key or voicing changed while it was held.
    sounding: [Vec<u8>; 7],
}

#[cfg(all(feature = "native", feature = "bounce"))]
impl AppState {
    fn new(key_mode: KeyMode) -> Self {
        Self {
            key_mode,
            root_index: 0, // C
            scale_type: ScaleType::Major,
            voicing_index: 0,
            chord_level: ChordLevel::Sevenths,
            octave: 4,
            velocity: 0.75,
            preset_index: 0,
            pedal: false,
            dynamics: ChordDynamics::new(VelocityProfile::MelodyLead, 0.35),
            last_velocities: Vec::new(),
            held: [false; 7],
            sounding: Default::default(),
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
        }
    }

    fn voicings_for(&self, degree: usize) -> Vec<VoicingType> {
        let chords = self.chords();
        available_voicings(&chords[degree.min(chords.len() - 1)].quality)
    }

    fn notes_for(&self, degree: usize) -> Vec<u8> {
        let chords = self.chords();
        let chord = &chords[degree.min(chords.len() - 1)];
        let voicings = available_voicings(&chord.quality);
        let voicing = voicings[self.voicing_index.min(voicings.len() - 1)];
        apply_voicing(chord, voicing, self.octave)
    }
}

#[cfg(all(feature = "native", feature = "bounce"))]
fn preset(index: usize) -> MultiSampleConfig {
    match index {
        1 => MultiSampleConfig::soft(),
        2 => MultiSampleConfig::bright(),
        _ => MultiSampleConfig::default(),
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

#[cfg(all(feature = "native", feature = "bounce"))]
fn make_bar(normalized: f32, width: usize) -> String {
    let filled = ((normalized.clamp(0.0, 1.0) * width as f32).round() as usize).min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

#[cfg(all(feature = "native", feature = "bounce"))]
fn draw_ui(state: &AppState, loaded: &LoadedMap, piano: &Arc<Mutex<MultiSampleInstrument>>) {
    let (voice_count, sounding, layers, key_range) = {
        let guard = piano.lock().unwrap();
        let map = guard.map();
        (
            guard.active_voice_count(),
            guard.sounding_zones(),
            map.velocity_layers(),
            map.key_range(),
        )
    };

    print!("\x1b[2J\x1b[H\x1b[?7l");

    println!("=== Piano Explorer ===\r");
    println!("\r");
    println!("  Pack: {}\r", loaded.source);
    for note in &loaded.notes {
        println!("        {note}\r");
    }
    let range = key_range
        .map(|(lo, hi)| format!("{}..{}", midi_to_string(lo), midi_to_string(hi)))
        .unwrap_or_else(|| "empty".to_string());
    println!(
        "        {} zones · {layers} velocity layers · range {range}\r",
        loaded.map.zone_count()
    );
    println!("\r");

    let verb = match state.key_mode {
        KeyMode::Hold => "hold",
        KeyMode::Toggle => "toggle",
    };
    println!("  a s d f g h j = {verb} chord I..VII    SPACE = sustain pedal\r");
    println!("  <-/-> key   TAB maj/min   [ ] voicing   , . chord level\r");
    println!("  o/k octave   z/x velocity   1-3 preset   q quit\r");
    println!("  v = velocity profile   n/m = humanize down/up\r");
    println!("  chord keys: {}\r", state.key_mode.label());
    println!("\r");

    println!(
        "  Key: {:<8} Octave: {:<3} Level: {:<8} Preset: {:<8} Pedal: {}\r",
        state.key(),
        state.octave,
        state.chord_level.label(),
        PRESET_NAMES[state.preset_index],
        if state.pedal { "DOWN" } else { "up" }
    );
    println!(
        "  Velocity: {} {:.2}\r",
        make_bar(state.velocity, 20),
        state.velocity
    );
    println!(
        "  Profile:  {:<8} Humanize: {} {:.2}\r",
        state.dynamics.profile().to_string(),
        make_bar(state.dynamics.humanize(), 10),
        state.dynamics.humanize()
    );
    println!("\r");

    let chords = state.chords();
    let key = state.key();
    let velocity = ((state.velocity * 127.0).round() as u8).max(1);
    let mut any_unmapped = false;
    for (degree, chord) in chords.iter().enumerate() {
        let marker = if state.held[degree] { ">" } else { " " };
        // A note the pack does not cover is silently dropped by `note_on`, which
        // would otherwise just sound like a missing note. Bracket it instead, so
        // gaps in a pack's range are visible rather than merely audible.
        let names: Vec<String> = state
            .notes_for(degree)
            .iter()
            .map(|&n| {
                let name = midi_to_string(n);
                if loaded
                    .map
                    .select(n, velocity, ZoneTrigger::Attack)
                    .is_some()
                {
                    name
                } else {
                    any_unmapped = true;
                    format!("({name})")
                }
            })
            .collect();
        println!(
            "  {marker} [{}] {:<5} {:<12} {}\r",
            DEGREE_KEYS[degree],
            key.roman_numeral(degree + 1),
            chord.display_name(),
            names.join(" ")
        );
    }
    if any_unmapped {
        println!("      (bracketed) = no zone at this velocity — outside the pack's range\r");
    }
    println!("\r");

    let voicings = state.voicings_for(0);
    let vi = state.voicing_index.min(voicings.len() - 1);
    println!(
        "  Voicing: {} ({}/{})\r",
        voicings[vi],
        vi + 1,
        voicings.len()
    );
    println!("\r");

    // Show the velocities the last chord was actually struck with, lowest voice
    // first. Watching these move is how you tell a profile apart from plain
    // jitter, and how you confirm two presses of the same chord really differ.
    if !state.last_velocities.is_empty() {
        let struck: Vec<String> = state
            .last_velocities
            .iter()
            .map(|v| format!("{v:.2}"))
            .collect();
        println!("  Last struck (low->high): {}\r", struck.join("  "));
        println!("\r");
    }

    // The zone readout is what makes a mapping bug visible rather than merely
    // audible: it shows which recording each sounding note actually picked.
    println!("  Voices: {voice_count}/32\r");
    for (note, zone_index) in sounding.iter().rev().take(6) {
        let detail = loaded
            .map
            .zone(*zone_index)
            .map(|zone| {
                let shift = *note as i32 - zone.root_key as i32;
                format!(
                    "root {} ({shift:+} st)  vel {}..{}",
                    midi_to_string(zone.root_key),
                    zone.lovel,
                    zone.hivel
                )
            })
            .unwrap_or_else(|| "zone gone (map swapped)".to_string());
        println!(
            "    {:<5} zone {zone_index:<4} {detail}\r",
            midi_to_string(*note)
        );
    }

    io::stdout().flush().ok();
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

#[cfg(all(feature = "native", feature = "bounce"))]
fn press_degree(piano: &Arc<Mutex<MultiSampleInstrument>>, state: &mut AppState, degree: usize) {
    if state.held[degree] {
        return;
    }
    let notes = state.notes_for(degree);
    // One velocity per voice rather than one for the whole chord: this is what
    // keeps a sampled chord from sounding like every key was struck by the same
    // machine at the same instant.
    let velocities = state.dynamics.velocities(state.velocity, notes.len());
    let mut guard = piano.lock().unwrap();
    for (&note, &velocity) in notes.iter().zip(velocities.iter()) {
        guard.note_on(note, velocity);
    }
    drop(guard);
    state.last_velocities = velocities;
    state.held[degree] = true;
    state.sounding[degree] = notes;
}

#[cfg(all(feature = "native", feature = "bounce"))]
fn release_degree(piano: &Arc<Mutex<MultiSampleInstrument>>, state: &mut AppState, degree: usize) {
    if !state.held[degree] {
        return;
    }
    let mut guard = piano.lock().unwrap();
    for &note in &state.sounding[degree] {
        guard.note_off(note);
    }
    drop(guard);
    state.held[degree] = false;
    state.sounding[degree].clear();
}

#[cfg(all(feature = "native", feature = "bounce"))]
fn release_all_degrees(piano: &Arc<Mutex<MultiSampleInstrument>>, state: &mut AppState) {
    for degree in 0..7 {
        release_degree(piano, state, degree);
    }
}

#[cfg(all(feature = "native", feature = "bounce"))]
fn main() -> anyhow::Result<()> {
    let loaded = load_map();

    let mut instrument = MultiSampleInstrument::with_map(SAMPLE_RATE, Arc::clone(&loaded.map));
    instrument.snap_params();
    let piano = Arc::new(Mutex::new(instrument));

    let mut engine = Engine::new(SAMPLE_RATE);
    engine.add_instrument("piano", Box::new(SharedPiano(Arc::clone(&piano))));
    engine.set_master_gain(0.9);

    let audio_engine = Arc::new(Mutex::new(engine));
    let mut engine_output = EngineOutput::new();
    engine_output.initialize(SAMPLE_RATE)?;
    engine_output.create_stream_with_engine(audio_engine)?;
    engine_output.start()?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();

    // Hold-to-sustain needs key-*release* events, which only kitty-protocol
    // terminals send. Ask for them, and fall back to toggling the chord when
    // the terminal cannot report them — otherwise a pressed chord would have no
    // way to ever stop, leaving the key dead after its first press.
    let key_mode = match supports_keyboard_enhancement() {
        Ok(true) => {
            let _ = execute!(
                stdout,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
            );
            KeyMode::Hold
        }
        _ => KeyMode::Toggle,
    };
    let mut state = AppState::new(key_mode);

    execute!(stdout, cursor::Hide, Clear(ClearType::All))?;

    let mut last_draw = Instant::now() - UI_REFRESH;

    let result = loop {
        if last_draw.elapsed() >= UI_REFRESH {
            draw_ui(&state, &loaded, &piano);
            last_draw = Instant::now();
        }

        if !event::poll(Duration::from_millis(10))? {
            continue;
        }
        let Event::Key(KeyEvent { code, kind, .. }) = event::read()? else {
            continue;
        };
        let is_press = kind == KeyEventKind::Press;
        let is_release = kind == KeyEventKind::Release;
        if !is_press && !is_release {
            continue;
        }

        let mut redraw = true;
        match code {
            KeyCode::Char(ch) if DEGREE_KEYS.contains(&ch.to_ascii_lowercase()) => {
                let degree = DEGREE_KEYS
                    .iter()
                    .position(|&k| k == ch.to_ascii_lowercase())
                    .unwrap();
                match (state.key_mode, is_press) {
                    // Kitty-protocol terminal: true hold-to-sustain.
                    (KeyMode::Hold, true) => press_degree(&piano, &mut state, degree),
                    (KeyMode::Hold, false) => release_degree(&piano, &mut state, degree),
                    // No key-up available: each press flips the chord.
                    (KeyMode::Toggle, true) => {
                        if state.held[degree] {
                            release_degree(&piano, &mut state, degree);
                        } else {
                            press_degree(&piano, &mut state, degree);
                        }
                    }
                    (KeyMode::Toggle, false) => redraw = false,
                }
            }

            _ if is_release => redraw = false,

            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => break Ok(()),

            KeyCode::Char(' ') => {
                state.pedal = !state.pedal;
                piano.lock().unwrap().set_sustain_pedal(state.pedal);
            }

            KeyCode::Left => {
                release_all_degrees(&piano, &mut state);
                state.root_index = (state.root_index + 11) % 12;
                state.voicing_index = 0;
            }
            KeyCode::Right => {
                release_all_degrees(&piano, &mut state);
                state.root_index = (state.root_index + 1) % 12;
                state.voicing_index = 0;
            }
            KeyCode::Tab => {
                release_all_degrees(&piano, &mut state);
                state.scale_type = match state.scale_type {
                    ScaleType::Major => ScaleType::NaturalMinor,
                    ScaleType::NaturalMinor => ScaleType::Major,
                };
                state.voicing_index = 0;
            }

            KeyCode::Char('[') => {
                let max = state.voicings_for(0).len();
                state.voicing_index = (state.voicing_index + max - 1) % max;
            }
            KeyCode::Char(']') => {
                let max = state.voicings_for(0).len();
                state.voicing_index = (state.voicing_index + 1) % max;
            }

            KeyCode::Char(',') | KeyCode::Char('<') => {
                state.chord_level = state.chord_level.prev();
                state.voicing_index = 0;
            }
            KeyCode::Char('.') | KeyCode::Char('>') => {
                state.chord_level = state.chord_level.next();
                state.voicing_index = 0;
            }

            KeyCode::Char('o') | KeyCode::Char('O') => state.octave = (state.octave - 1).max(1),
            KeyCode::Char('k') | KeyCode::Char('K') => state.octave = (state.octave + 1).min(6),

            KeyCode::Char('v') | KeyCode::Char('V') => {
                let next = state.dynamics.profile().next();
                state.dynamics.set_profile(next);
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                let h = state.dynamics.humanize();
                state.dynamics.set_humanize(h - 0.1);
            }
            KeyCode::Char('m') | KeyCode::Char('M') => {
                let h = state.dynamics.humanize();
                state.dynamics.set_humanize(h + 0.1);
            }

            KeyCode::Char('z') | KeyCode::Char('Z') => {
                state.velocity = (state.velocity - 0.1).clamp(0.05, 1.0)
            }
            KeyCode::Char('x') | KeyCode::Char('X') => {
                state.velocity = (state.velocity + 0.1).clamp(0.05, 1.0)
            }

            KeyCode::Char(ch @ '1'..='3') => {
                state.preset_index = (ch as u8 - b'1') as usize;
                piano.lock().unwrap().set_config(preset(state.preset_index));
            }

            _ => redraw = false,
        }

        if redraw {
            draw_ui(&state, &loaded, &piano);
            last_draw = Instant::now();
        }
    };

    release_all_degrees(&piano, &mut state);
    piano.lock().unwrap().release_all();

    let _ = execute!(stdout, PopKeyboardEnhancementFlags);
    execute!(
        stdout,
        Clear(ClearType::All),
        cursor::MoveTo(0, 0),
        cursor::Show
    )?;
    disable_raw_mode()?;
    println!("Bye.");
    result
}

#[cfg(not(all(feature = "native", feature = "bounce")))]
fn main() {
    eprintln!("This example requires --features native,crossterm,bounce");
}
