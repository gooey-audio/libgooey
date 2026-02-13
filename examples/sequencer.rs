/* Sequencer Lab - Interactive CLI for sequencer parameter experimentation.
Demonstrates sample-accurate sequencing with swing, LFO modulation, and filtering.
*/

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use gooey::effects::LowpassFilterEffect;
use gooey::engine::{Engine, EngineOutput, Instrument, Lfo, Modulatable, MusicalDivision, Sequencer};
use gooey::instruments::HiHat;

const LFO_DIVISIONS: [MusicalDivision; 8] = [
    MusicalDivision::FourBars,
    MusicalDivision::TwoBars,
    MusicalDivision::OneBar,
    MusicalDivision::Half,
    MusicalDivision::Quarter,
    MusicalDivision::Eighth,
    MusicalDivision::Sixteenth,
    MusicalDivision::ThirtySecond,
];

fn division_name(div: MusicalDivision) -> &'static str {
    match div {
        MusicalDivision::FourBars => "4 bars",
        MusicalDivision::TwoBars => "2 bars",
        MusicalDivision::OneBar => "1 bar",
        MusicalDivision::Half => "1/2",
        MusicalDivision::Quarter => "1/4",
        MusicalDivision::Eighth => "1/8",
        MusicalDivision::Sixteenth => "1/16",
        MusicalDivision::ThirtySecond => "1/32",
    }
}

fn division_index(div: MusicalDivision) -> usize {
    LFO_DIVISIONS.iter().position(|&d| d == div).unwrap_or(2)
}

// Parameter indices
const PARAM_BPM: usize = 0;
const PARAM_SWING: usize = 1;
const PARAM_DECAY: usize = 2;
const PARAM_CUTOFF: usize = 3;
const PARAM_RESONANCE: usize = 4;
const PARAM_LFO_ENABLED: usize = 5;
const PARAM_LFO_DIVISION: usize = 6;
const PARAM_COUNT: usize = 7;

// Wrapper to share HiHat between audio thread and main thread
struct SharedHiHat(Arc<Mutex<HiHat>>);

impl Instrument for SharedHiHat {
    fn trigger_with_velocity(&mut self, time: f32, velocity: f32) {
        self.0.lock().unwrap().trigger_with_velocity(time, velocity);
    }

    fn tick(&mut self, current_time: f32) -> f32 {
        self.0.lock().unwrap().tick(current_time)
    }

    fn is_active(&self) -> bool {
        self.0.lock().unwrap().is_active()
    }

    fn as_modulatable(&mut self) -> Option<&mut dyn Modulatable> {
        Some(self)
    }
}

impl Modulatable for SharedHiHat {
    fn modulatable_parameters(&self) -> Vec<&'static str> {
        self.0.lock().unwrap().modulatable_parameters()
    }

    fn apply_modulation(&mut self, parameter: &str, value: f32) -> Result<(), String> {
        self.0.lock().unwrap().apply_modulation(parameter, value)
    }

    fn parameter_range(&self, parameter: &str) -> Option<(f32, f32)> {
        self.0.lock().unwrap().parameter_range(parameter)
    }
}

struct SeqState {
    bpm: f32,
    swing: f32,
    decay: f32,
    cutoff: f32,
    resonance: f32,
    lfo_enabled: bool,
    lfo_division_idx: usize,
    running: bool,
}

fn make_bar(normalized: f32, width: usize) -> String {
    let filled = (normalized * width as f32).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

fn render_display(state: &SeqState, selected: usize) {
    print!("\x1b[2J\x1b[H\x1b[?7l");

    print!("=== Sequencer Lab ===\r\n");
    print!("SPACE=start/stop Q=quit ↑↓=sel ←→=adj []=fine\r\n");
    let status = if state.running { "RUNNING" } else { "STOPPED" };
    print!("Status: {}\r\n", status);
    print!("\r\n");

    // BPM
    let ind = if selected == PARAM_BPM { ">" } else { " " };
    let bpm_norm = (state.bpm - 60.0) / 140.0;
    print!(
        "{} {:<18} [{}] {:>6.0} BPM\r\n",
        ind,
        "bpm",
        make_bar(bpm_norm, 10),
        state.bpm
    );

    // Swing (display as 0-100 where 50 = neutral)
    let ind = if selected == PARAM_SWING { ">" } else { " " };
    print!(
        "{} {:<18} [{}] {:>6.0}\r\n",
        ind,
        "swing",
        make_bar(state.swing, 10),
        state.swing * 100.0
    );

    // Decay (0-1 normalized)
    let ind = if selected == PARAM_DECAY { ">" } else { " " };
    print!(
        "{} {:<18} [{}] {:>6.2}\r\n",
        ind,
        "decay",
        make_bar(state.decay, 10),
        state.decay
    );

    // Filter cutoff
    let ind = if selected == PARAM_CUTOFF { ">" } else { " " };
    let cutoff_norm = (state.cutoff - 100.0) / 19900.0;
    print!(
        "{} {:<18} [{}] {:>6.0} Hz\r\n",
        ind,
        "filter cutoff",
        make_bar(cutoff_norm, 10),
        state.cutoff
    );

    // Filter resonance
    let ind = if selected == PARAM_RESONANCE { ">" } else { " " };
    let res_norm = state.resonance / 0.95;
    print!(
        "{} {:<18} [{}] {:>6.2}\r\n",
        ind,
        "filter resonance",
        make_bar(res_norm, 10),
        state.resonance
    );

    // LFO enabled
    let ind = if selected == PARAM_LFO_ENABLED { ">" } else { " " };
    let on_off = if state.lfo_enabled { "ON" } else { "OFF" };
    print!(
        "{} {:<18} [{}] {:>6}\r\n",
        ind,
        "lfo enabled",
        if state.lfo_enabled {
            "██████████"
        } else {
            "░░░░░░░░░░"
        },
        on_off
    );

    // LFO division
    let ind = if selected == PARAM_LFO_DIVISION { ">" } else { " " };
    let div = LFO_DIVISIONS[state.lfo_division_idx];
    let div_norm = state.lfo_division_idx as f32 / 7.0;
    print!(
        "{} {:<18} [{}] {:>6}\r\n",
        ind,
        "lfo division",
        make_bar(div_norm, 10),
        division_name(div)
    );

    io::stdout().flush().unwrap();
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;

    let mut engine = Engine::new(sample_rate);

    let hihat = Arc::new(Mutex::new(HiHat::new(sample_rate)));
    // Start with a tight decay so individual hits are distinct and swing is audible
    hihat.lock().unwrap().set_decay(0.05);
    engine.add_instrument("hihat", Box::new(SharedHiHat(hihat.clone())));

    let bpm = 120.0;
    engine.set_bpm(bpm);

    let pattern = vec![true; 8];
    let sequencer = Sequencer::with_pattern(bpm, sample_rate, pattern, "hihat");
    engine.add_sequencer(sequencer);

    // LFO for hi-hat decay modulation
    let lfo = Lfo::new_synced(MusicalDivision::OneBar, bpm, sample_rate);
    let lfo_index = engine.add_lfo(lfo);
    engine
        .map_lfo_to_parameter(lfo_index, "hihat", "decay", 1.0)
        .expect("Failed to map LFO");

    // Set master gain higher for hi-hat (default 0.25 is too quiet)
    engine.set_master_gain(1.0);

    // Lowpass filter (start above hi-hat's base frequency range)
    // Must Box first, then get_control - raw pointers would dangle if filter moves after get_control
    let filter = Box::new(LowpassFilterEffect::new(sample_rate, 10000.0, 0.3));
    let filter_control = filter.get_control();
    engine.add_global_effect(filter);

    let audio_engine = Arc::new(Mutex::new(engine));

    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;

    #[cfg(feature = "visualization")]
    engine_output.enable_visualization(1200, 600, 2.0)?;

    engine_output.create_stream_with_engine(audio_engine.clone())?;
    engine_output.start()?;

    let mut state = SeqState {
        bpm,
        swing: 0.5,
        decay: 0.05,
        cutoff: 10000.0,
        resonance: 0.3,
        lfo_enabled: true,
        lfo_division_idx: division_index(MusicalDivision::OneBar),
        running: false,
    };
    let mut selected: usize = 0;
    let mut needs_redraw = true;

    execute!(io::stdout(), Clear(ClearType::All), cursor::Hide)?;
    enable_raw_mode()?;

    let result = loop {
        if engine_output.update_visualization() {
            break Ok(());
        }

        if needs_redraw {
            render_display(&state, selected);
            needs_redraw = false;
        }

        if event::poll(std::time::Duration::from_millis(16))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match code {
                    KeyCode::Up => {
                        selected = selected.saturating_sub(1);
                        needs_redraw = true;
                    }
                    KeyCode::Down => {
                        selected = (selected + 1).min(PARAM_COUNT - 1);
                        needs_redraw = true;
                    }

                    // Coarse adjust
                    KeyCode::Right => {
                        adjust_param(
                            &audio_engine,
                            &hihat,
                            &filter_control,
                            &mut state,
                            selected,
                            1.0,
                            false,
                        );
                        needs_redraw = true;
                    }
                    KeyCode::Left => {
                        adjust_param(
                            &audio_engine,
                            &hihat,
                            &filter_control,
                            &mut state,
                            selected,
                            -1.0,
                            false,
                        );
                        needs_redraw = true;
                    }

                    // Fine adjust
                    KeyCode::Char(']') => {
                        adjust_param(
                            &audio_engine,
                            &hihat,
                            &filter_control,
                            &mut state,
                            selected,
                            1.0,
                            true,
                        );
                        needs_redraw = true;
                    }
                    KeyCode::Char('[') => {
                        adjust_param(
                            &audio_engine,
                            &hihat,
                            &filter_control,
                            &mut state,
                            selected,
                            -1.0,
                            true,
                        );
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

fn adjust_param(
    audio_engine: &Arc<Mutex<Engine>>,
    hihat: &Arc<Mutex<HiHat>>,
    filter_control: &gooey::effects::LowpassFilterControl,
    state: &mut SeqState,
    param: usize,
    direction: f32,
    fine: bool,
) {
    match param {
        PARAM_BPM => {
            let step = if fine { 1.0 } else { 5.0 };
            state.bpm = (state.bpm + step * direction).clamp(60.0, 200.0);
            let mut engine = audio_engine.lock().unwrap();
            engine.set_bpm(state.bpm);
            if let Some(seq) = engine.sequencer_mut(0) {
                seq.set_bpm(state.bpm);
            }
        }
        PARAM_SWING => {
            let step = if fine { 0.01 } else { 0.05 };
            state.swing = (state.swing + step * direction).clamp(0.0, 1.0);
            let mut engine = audio_engine.lock().unwrap();
            if let Some(seq) = engine.sequencer_mut(0) {
                seq.set_swing(state.swing);
            }
        }
        PARAM_DECAY => {
            let step = if fine { 0.01 } else { 0.05 };
            state.decay = (state.decay + step * direction).clamp(0.0, 1.0);
            hihat.lock().unwrap().set_decay(state.decay);
        }
        PARAM_CUTOFF => {
            let step = if fine { 50.0 } else { 200.0 };
            state.cutoff = (state.cutoff + step * direction).clamp(100.0, 20000.0);
            filter_control.set_cutoff_freq(state.cutoff);
        }
        PARAM_RESONANCE => {
            let step = if fine { 0.01 } else { 0.05 };
            state.resonance = (state.resonance + step * direction).clamp(0.0, 0.95);
            filter_control.set_resonance(state.resonance);
        }
        PARAM_LFO_ENABLED => {
            state.lfo_enabled = !state.lfo_enabled;
            let mut engine = audio_engine.lock().unwrap();
            if state.lfo_enabled {
                let _ = engine.map_lfo_to_parameter(0, "hihat", "decay", 1.0);
            } else {
                // Clear target so the LFO stops overriding the decay parameter
                if let Some(lfo) = engine.lfo_mut(0) {
                    lfo.target_instrument.clear();
                    lfo.target_parameter.clear();
                }
                // Restore manual decay value
                hihat.lock().unwrap().set_decay(state.decay);
            }
        }
        PARAM_LFO_DIVISION => {
            let new_idx = if direction > 0.0 {
                (state.lfo_division_idx + 1).min(LFO_DIVISIONS.len() - 1)
            } else {
                state.lfo_division_idx.saturating_sub(1)
            };
            state.lfo_division_idx = new_idx;
            let mut engine = audio_engine.lock().unwrap();
            if let Some(lfo) = engine.lfo_mut(0) {
                lfo.set_sync_mode(LFO_DIVISIONS[new_idx]);
            }
        }
        _ => {}
    }
}

#[cfg(not(feature = "native"))]
fn main() {
    println!("This example is only available with the 'native' feature enabled.");
}
