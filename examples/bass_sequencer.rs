/* Bass Sequencer Lab - Interactive CLI for bass sequencer with per-step MIDI notes.
Demonstrates sample-accurate pitch sequencing, presets, and arpeggiator patterns.
*/

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use gooey::engine::{Engine, EngineOutput, Instrument, Modulatable, Sequencer};
use gooey::instruments::{BassConfig, BassSynth};

// Wrapper to share BassSynth between audio thread and main thread
struct SharedBass(Arc<Mutex<BassSynth>>);

impl Instrument for SharedBass {
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
        let hz = midi_to_hz(note);
        let normalized = freq_to_bass_normalized(hz);
        let mut b = self.0.lock().unwrap();
        b.set_frequency(normalized);
        b.snap_params();
    }

    fn set_frequency_normalized(&mut self, value: f32) {
        let mut b = self.0.lock().unwrap();
        b.set_frequency(value);
        b.snap_params();
    }

    fn get_frequency(&self) -> Option<f32> {
        Some(self.0.lock().unwrap().params.frequency.get())
    }

    fn as_modulatable(&mut self) -> Option<&mut dyn Modulatable> {
        None
    }
}

/// MIDI note to name (e.g. 36 -> "C2")
fn note_name(midi: u8) -> String {
    let names = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = (midi / 12) as i32 - 1;
    let name = names[(midi % 12) as usize];
    format!("{}{}", name, octave)
}

/// MIDI note to frequency in Hz
fn midi_to_hz(note: u8) -> f32 {
    440.0 * 2f32.powf((note as f32 - 69.0) / 12.0)
}

/// Normalize frequency to bass range (30-200 Hz)
fn freq_to_bass_normalized(hz: f32) -> f32 {
    ((hz - 30.0) / (200.0 - 30.0)).clamp(0.0, 1.0)
}

// Preset arp patterns (MIDI notes, 255 = rest/no note)
const NONE: u8 = 255;

struct ArpPattern {
    name: &'static str,
    notes: [u8; 16],
}

const ARP_PATTERNS: [ArpPattern; 5] = [
    ArpPattern {
        name: "Octave Bounce",
        notes: [
            36, NONE, 48, NONE, 36, NONE, 48, NONE, 36, NONE, 48, NONE, 36, NONE, 48, NONE,
        ],
    },
    ArpPattern {
        name: "Minor Walk",
        notes: [
            36, 36, 39, 39, 41, 41, 43, 43, 41, 41, 39, 39, 36, 36, 34, 34,
        ],
    },
    ArpPattern {
        name: "Acid Line",
        notes: [
            36, NONE, 36, 48, NONE, 36, 39, NONE, 36, NONE, 48, 36, NONE, 39, 36, NONE,
        ],
    },
    ArpPattern {
        name: "Chromatic",
        notes: [
            36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 47, 46, 45,
        ],
    },
    ArpPattern {
        name: "Root Only",
        notes: [
            36, NONE, NONE, NONE, 36, NONE, NONE, NONE, 36, NONE, NONE, NONE, 36, NONE, NONE, NONE,
        ],
    },
];

struct AppState {
    bpm: f32,
    running: bool,
    selected_step: usize,
    pattern_enabled: [bool; 16],
    step_notes: [u8; 16], // 255 = no note
    current_preset: &'static str,
    current_arp: Option<usize>,
    edit_mode: EditMode,
}

#[derive(PartialEq)]
enum EditMode {
    Steps,    // toggle steps on/off, select steps
    NoteEdit, // adjust MIDI note for selected step
    Params,   // adjust BPM / preset
}

fn render_display(state: &AppState, playhead: usize) {
    print!("\x1b[2J\x1b[H\x1b[?7l");

    print!("=== Bass Sequencer Lab ===\r\n");
    let status = if state.running { "PLAYING" } else { "STOPPED" };
    print!(
        "Status: {}  BPM: {}  Preset: {}\r\n",
        status, state.bpm as u32, state.current_preset
    );

    match state.edit_mode {
        EditMode::Steps => {
            print!("Mode: STEPS | TAB=switch mode  SPACE=play/stop  Q=quit\r\n");
            print!("      ←→=select step  ENTER=toggle step  1-5=arp pattern\r\n");
        }
        EditMode::NoteEdit => {
            print!("Mode: NOTE EDIT | TAB=switch mode  SPACE=play/stop  Q=quit\r\n");
            print!("      ←→=select step  ↑↓=semitone  SHIFT+↑↓=octave  DEL=clear note\r\n");
        }
        EditMode::Params => {
            print!("Mode: PARAMS | TAB=switch mode  SPACE=play/stop  Q=quit\r\n");
            print!("      ←→=BPM  1-4=bass preset  5-9=arp patterns\r\n");
        }
    }

    print!("\r\n");

    // Step grid header
    print!("  Step: ");
    for i in 0..16 {
        if i == playhead && state.running {
            print!(" \x1b[7m{:>2}\x1b[0m", i + 1);
        } else {
            print!(" {:>2}", i + 1);
        }
    }
    print!("\r\n");

    // Step enabled row
    print!("   On:  ");
    for i in 0..16 {
        let marker = if state.pattern_enabled[i] {
            " * "
        } else {
            " . "
        };
        if i == state.selected_step {
            print!("\x1b[4m{}\x1b[0m", marker);
        } else {
            print!("{}", marker);
        }
    }
    print!("\r\n");

    // Note row
    print!("  Note: ");
    for i in 0..16 {
        let label = if state.step_notes[i] == NONE {
            " - ".to_string()
        } else {
            format!("{:>3}", note_name(state.step_notes[i]))
        };
        if i == state.selected_step && state.edit_mode == EditMode::NoteEdit {
            print!("\x1b[1m{}\x1b[0m", label);
        } else {
            print!("{}", label);
        }
    }
    print!("\r\n");

    // Frequency row
    print!("    Hz: ");
    for i in 0..16 {
        if state.step_notes[i] == NONE {
            print!("   ");
        } else {
            let hz = midi_to_hz(state.step_notes[i]);
            if hz >= 100.0 {
                print!("{:>3}", hz as u32);
            } else {
                print!("{:>3}", hz as u32);
            }
        }
    }
    print!("\r\n");

    // Arp pattern info
    print!("\r\n");
    if let Some(idx) = state.current_arp {
        print!("  Arp: {}\r\n", ARP_PATTERNS[idx].name);
    } else {
        print!("  Arp: (custom)\r\n");
    }

    // Selected step detail
    print!("\r\n");
    let step = state.selected_step;
    let enabled = if state.pattern_enabled[step] {
        "ON"
    } else {
        "OFF"
    };
    if state.step_notes[step] == NONE {
        print!(
            "  Step {} | {} | Note: none (uses global freq)\r\n",
            step + 1,
            enabled
        );
    } else {
        let note = state.step_notes[step];
        print!(
            "  Step {} | {} | Note: {} (MIDI {}, {:.1} Hz)\r\n",
            step + 1,
            enabled,
            note_name(note),
            note,
            midi_to_hz(note),
        );
    }

    io::stdout().flush().unwrap();
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;

    let bass = Arc::new(Mutex::new(BassSynth::new(sample_rate)));
    // Start with the acid preset
    bass.lock().unwrap().set_config(BassConfig::acid());

    let mut engine = Engine::new(sample_rate);
    engine.add_instrument("bass", Box::new(SharedBass(bass.clone())));

    let bpm = 120.0;
    engine.set_bpm(bpm);

    // Start with all steps enabled, octave bounce pattern
    let pattern = vec![true; 16];
    let sequencer = Sequencer::with_pattern(bpm, sample_rate, pattern, "bass");
    engine.add_sequencer(sequencer);

    // Apply initial arp pattern notes to sequencer
    if let Some(seq) = engine.sequencer_mut(0) {
        seq.set_note_pattern(&ARP_PATTERNS[0].notes);
    }

    // Set a good initial frequency for when notes aren't overriding
    {
        let mut b = bass.lock().unwrap();
        b.set_frequency(freq_to_bass_normalized(midi_to_hz(36))); // C2
        b.set_volume(0.8);
    }

    engine.set_master_gain(1.0);

    let audio_engine = Arc::new(Mutex::new(engine));

    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;

    #[cfg(feature = "visualization")]
    engine_output.enable_visualization(1200, 400, 2.0)?;

    engine_output.create_stream_with_engine(audio_engine.clone())?;
    engine_output.start()?;

    let mut state = AppState {
        bpm,
        running: false,
        selected_step: 0,
        pattern_enabled: [true; 16],
        step_notes: ARP_PATTERNS[0].notes,
        current_preset: "Acid",
        current_arp: Some(0),
        edit_mode: EditMode::Steps,
    };

    let mut needs_redraw = true;

    execute!(io::stdout(), Clear(ClearType::All), cursor::Hide)?;
    enable_raw_mode()?;

    let result = loop {
        if engine_output.update_visualization() {
            break Ok(());
        }

        // Get current playhead position
        let playhead = {
            let engine = audio_engine.lock().unwrap();
            engine.sequencer(0).map(|s| s.current_step()).unwrap_or(0)
        };

        if needs_redraw || state.running {
            render_display(&state, playhead);
            needs_redraw = false;
        }

        if event::poll(std::time::Duration::from_millis(32))? {
            if let Event::Key(KeyEvent {
                code, modifiers, ..
            }) = event::read()?
            {
                match code {
                    // Mode switching
                    KeyCode::Tab => {
                        state.edit_mode = match state.edit_mode {
                            EditMode::Steps => EditMode::NoteEdit,
                            EditMode::NoteEdit => EditMode::Params,
                            EditMode::Params => EditMode::Steps,
                        };
                        needs_redraw = true;
                    }

                    // Step selection (all modes)
                    KeyCode::Left
                        if state.edit_mode == EditMode::Steps
                            || state.edit_mode == EditMode::NoteEdit =>
                    {
                        state.selected_step = state.selected_step.saturating_sub(1);
                        needs_redraw = true;
                    }
                    KeyCode::Right
                        if state.edit_mode == EditMode::Steps
                            || state.edit_mode == EditMode::NoteEdit =>
                    {
                        state.selected_step = (state.selected_step + 1).min(15);
                        needs_redraw = true;
                    }

                    // Toggle step (Steps mode)
                    KeyCode::Enter if state.edit_mode == EditMode::Steps => {
                        let step = state.selected_step;
                        state.pattern_enabled[step] = !state.pattern_enabled[step];
                        let mut engine = audio_engine.lock().unwrap();
                        if let Some(seq) = engine.sequencer_mut(0) {
                            let current = seq.get_step_enabled(step);
                            seq.set_step_with_velocity(step, !current, 1.0);
                        }
                        needs_redraw = true;
                    }

                    // Note editing (NoteEdit mode)
                    KeyCode::Up if state.edit_mode == EditMode::NoteEdit => {
                        let step = state.selected_step;
                        let semitones = if modifiers.contains(KeyModifiers::SHIFT) {
                            12
                        } else {
                            1
                        };
                        let current = if state.step_notes[step] == NONE {
                            36u8 // default to C2
                        } else {
                            state.step_notes[step]
                        };
                        let new_note = current.saturating_add(semitones).min(72); // max C5
                        state.step_notes[step] = new_note;
                        state.current_arp = None;
                        let mut engine = audio_engine.lock().unwrap();
                        if let Some(seq) = engine.sequencer_mut(0) {
                            seq.set_step_note(step, new_note);
                        }
                        needs_redraw = true;
                    }
                    KeyCode::Down if state.edit_mode == EditMode::NoteEdit => {
                        let step = state.selected_step;
                        let semitones = if modifiers.contains(KeyModifiers::SHIFT) {
                            12
                        } else {
                            1
                        };
                        if state.step_notes[step] != NONE {
                            let new_note = state.step_notes[step].saturating_sub(semitones).max(24); // min C1
                            state.step_notes[step] = new_note;
                            state.current_arp = None;
                            let mut engine = audio_engine.lock().unwrap();
                            if let Some(seq) = engine.sequencer_mut(0) {
                                seq.set_step_note(step, new_note);
                            }
                        }
                        needs_redraw = true;
                    }
                    KeyCode::Delete | KeyCode::Backspace
                        if state.edit_mode == EditMode::NoteEdit =>
                    {
                        let step = state.selected_step;
                        state.step_notes[step] = NONE;
                        state.current_arp = None;
                        let mut engine = audio_engine.lock().unwrap();
                        if let Some(seq) = engine.sequencer_mut(0) {
                            seq.clear_step_note(step);
                        }
                        needs_redraw = true;
                    }

                    // BPM adjustment (Params mode)
                    KeyCode::Left if state.edit_mode == EditMode::Params => {
                        state.bpm = (state.bpm - 5.0).max(60.0);
                        let mut engine = audio_engine.lock().unwrap();
                        engine.set_bpm(state.bpm);
                        if let Some(seq) = engine.sequencer_mut(0) {
                            seq.set_bpm(state.bpm);
                        }
                        needs_redraw = true;
                    }
                    KeyCode::Right if state.edit_mode == EditMode::Params => {
                        state.bpm = (state.bpm + 5.0).min(200.0);
                        let mut engine = audio_engine.lock().unwrap();
                        engine.set_bpm(state.bpm);
                        if let Some(seq) = engine.sequencer_mut(0) {
                            seq.set_bpm(state.bpm);
                        }
                        needs_redraw = true;
                    }

                    // Bass presets (1-4 in Params mode)
                    KeyCode::Char('1') if state.edit_mode == EditMode::Params => {
                        bass.lock().unwrap().set_config(BassConfig::acid());
                        state.current_preset = "Acid";
                        needs_redraw = true;
                    }
                    KeyCode::Char('2') if state.edit_mode == EditMode::Params => {
                        bass.lock().unwrap().set_config(BassConfig::sub());
                        state.current_preset = "Sub";
                        needs_redraw = true;
                    }
                    KeyCode::Char('3') if state.edit_mode == EditMode::Params => {
                        bass.lock().unwrap().set_config(BassConfig::reese());
                        state.current_preset = "Reese";
                        needs_redraw = true;
                    }
                    KeyCode::Char('4') if state.edit_mode == EditMode::Params => {
                        bass.lock().unwrap().set_config(BassConfig::stab());
                        state.current_preset = "Stab";
                        needs_redraw = true;
                    }

                    // Arp patterns (1-5 in Steps mode)
                    KeyCode::Char(c @ '1'..='5') if state.edit_mode == EditMode::Steps => {
                        let idx = (c as u8 - b'1') as usize;
                        apply_arp_pattern(&audio_engine, &mut state, idx);
                        needs_redraw = true;
                    }
                    // Arp patterns (6-9 → patterns 2-5 in Params mode)
                    KeyCode::Char(c @ '6'..='9') if state.edit_mode == EditMode::Params => {
                        let idx = (c as u8 - b'6' + 1) as usize;
                        if idx < ARP_PATTERNS.len() {
                            apply_arp_pattern(&audio_engine, &mut state, idx);
                        }
                        needs_redraw = true;
                    }
                    // Arp pattern 1 in Params mode
                    KeyCode::Char('5') if state.edit_mode == EditMode::Params => {
                        apply_arp_pattern(&audio_engine, &mut state, 0);
                        needs_redraw = true;
                    }

                    // Start/stop
                    KeyCode::Char(' ') => {
                        let mut engine = audio_engine.lock().unwrap();
                        if let Some(seq) = engine.sequencer_mut(0) {
                            if seq.is_running() {
                                seq.stop();
                                state.running = false;
                            } else {
                                seq.start();
                                state.running = true;
                            }
                        }
                        needs_redraw = true;
                    }

                    // Quit
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                        break Ok(());
                    }
                    _ => {}
                }
            }
        }
    };

    print!("\x1b[?7h");
    execute!(io::stdout(), cursor::Show)?;
    disable_raw_mode()?;
    println!("\nQuitting...");

    result
}

fn apply_arp_pattern(audio_engine: &Arc<Mutex<Engine>>, state: &mut AppState, idx: usize) {
    state.step_notes = ARP_PATTERNS[idx].notes;
    state.current_arp = Some(idx);
    // Also enable all steps that have notes
    for i in 0..16 {
        state.pattern_enabled[i] = true;
    }
    let mut engine = audio_engine.lock().unwrap();
    if let Some(seq) = engine.sequencer_mut(0) {
        seq.set_note_pattern(&ARP_PATTERNS[idx].notes);
        seq.set_pattern(vec![true; 16]);
    }
}

#[cfg(not(feature = "native"))]
fn main() {
    println!("This example is only available with the 'native' feature enabled.");
}
