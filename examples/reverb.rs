/* Reverb Lab - Interactive CLI for spring reverb experimentation.
Demonstrates the Schroeder/Freeverb-style spring reverb with decay, mix, and damping controls.
*/

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use gooey::effects::SpringReverbEffect;
use gooey::engine::{Engine, EngineOutput, Instrument, Sequencer};
use gooey::instruments::SnareDrum;

// Lock-free control handle for the reverb effect.
// SAFETY: SpringReverbEffect setters take &self and use atomics internally.
// The raw pointer is valid as long as the engine (which owns the Box<dyn Effect>) is alive.
struct ReverbControl {
    ptr: *const SpringReverbEffect,
}

unsafe impl Send for ReverbControl {}

impl ReverbControl {
    fn set_decay(&self, v: f32) {
        unsafe { &*self.ptr }.set_decay(v);
    }
    fn set_mix(&self, v: f32) {
        unsafe { &*self.ptr }.set_mix(v);
    }
    fn set_damping(&self, v: f32) {
        unsafe { &*self.ptr }.set_damping(v);
    }
}

// Parameter indices
const PARAM_DECAY: usize = 0;
const PARAM_MIX: usize = 1;
const PARAM_DAMPING: usize = 2;
const PARAM_BPM: usize = 3;
const PARAM_COUNT: usize = 4;

// Wrapper to share SnareDrum between audio thread and main thread
struct SharedSnare(Arc<Mutex<SnareDrum>>);

impl Instrument for SharedSnare {
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        self.0.lock().unwrap().trigger_with_velocity(time, velocity);
    }

    fn tick(&mut self, current_time: f64) -> f32 {
        self.0.lock().unwrap().tick(current_time)
    }

    fn is_active(&self) -> bool {
        self.0.lock().unwrap().is_active()
    }

    fn as_modulatable(&mut self) -> Option<&mut dyn gooey::engine::Modulatable> {
        None
    }
}

struct ReverbState {
    decay: f32,
    mix: f32,
    damping: f32,
    bpm: f32,
    running: bool,
    enabled: bool,
}

fn make_bar(normalized: f32, width: usize) -> String {
    let filled = (normalized * width as f32).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

fn render_display(state: &ReverbState, selected: usize) {
    print!("\x1b[2J\x1b[H\x1b[?7l");

    print!("=== Reverb Lab ===\r\n");
    print!("SPACE=start/stop  B=bypass  Q=quit  ↑↓=sel  ←→=adj  []=fine\r\n");
    let status = if state.running { "RUNNING" } else { "STOPPED" };
    let reverb_status = if state.enabled { "ON" } else { "BYPASS" };
    print!("Status: {}  Reverb: {}\r\n", status, reverb_status);
    print!("\r\n");

    // Decay
    let ind = if selected == PARAM_DECAY { ">" } else { " " };
    print!(
        "{} {:<18} [{}] {:>6.2}\r\n",
        ind,
        "decay",
        make_bar(state.decay, 10),
        state.decay
    );

    // Mix
    let ind = if selected == PARAM_MIX { ">" } else { " " };
    print!(
        "{} {:<18} [{}] {:>6.2}\r\n",
        ind,
        "mix",
        make_bar(state.mix, 10),
        state.mix
    );

    // Damping
    let ind = if selected == PARAM_DAMPING { ">" } else { " " };
    print!(
        "{} {:<18} [{}] {:>6.2}\r\n",
        ind,
        "damping",
        make_bar(state.damping, 10),
        state.damping
    );

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

    io::stdout().flush().unwrap();
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;

    let mut engine = Engine::new(sample_rate);

    let snare = Arc::new(Mutex::new(SnareDrum::new(sample_rate)));
    engine.add_instrument("snare", Box::new(SharedSnare(snare.clone())));

    let bpm = 120.0;
    engine.set_bpm(bpm);

    // Sparse pattern: hits on 0, 4, 8, 12 so reverb tail is clearly audible
    let pattern = vec![
        true, false, false, false, true, false, false, false, true, false, false, false, true,
        false, false, false,
    ];
    let sequencer = Sequencer::with_pattern(bpm, sample_rate, pattern, "snare");
    engine.add_sequencer(sequencer);

    engine.set_master_gain(0.8);

    // Create reverb and get a lock-free control handle before moving into engine
    let reverb = Box::new(SpringReverbEffect::new(sample_rate, 0.5, 0.4, 0.5));
    let reverb_control = ReverbControl {
        ptr: &*reverb as *const SpringReverbEffect,
    };
    engine.add_global_effect(reverb);

    let audio_engine = Arc::new(Mutex::new(engine));

    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;

    #[cfg(feature = "visualization")]
    engine_output.enable_visualization(1200, 600, 2.0)?;

    engine_output.create_stream_with_engine(audio_engine.clone())?;
    engine_output.start()?;

    let mut state = ReverbState {
        decay: 0.5,
        mix: 0.4,
        damping: 0.5,
        bpm,
        running: false,
        enabled: true,
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
                            &reverb_control,
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
                            &reverb_control,
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
                            &reverb_control,
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
                            &reverb_control,
                            &mut state,
                            selected,
                            -1.0,
                            true,
                        );
                        needs_redraw = true;
                    }

                    // Toggle reverb bypass
                    KeyCode::Char('b') | KeyCode::Char('B') => {
                        state.enabled = !state.enabled;
                        if state.enabled {
                            reverb_control.set_mix(state.mix);
                        } else {
                            reverb_control.set_mix(0.0);
                        }
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
    reverb_control: &ReverbControl,
    state: &mut ReverbState,
    param: usize,
    direction: f32,
    fine: bool,
) {
    match param {
        PARAM_DECAY => {
            let step = if fine { 0.01 } else { 0.05 };
            state.decay = (state.decay + step * direction).clamp(0.0, 1.0);
            reverb_control.set_decay(state.decay);
        }
        PARAM_MIX => {
            let step = if fine { 0.01 } else { 0.05 };
            state.mix = (state.mix + step * direction).clamp(0.0, 1.0);
            if state.enabled {
                reverb_control.set_mix(state.mix);
            }
        }
        PARAM_DAMPING => {
            let step = if fine { 0.01 } else { 0.05 };
            state.damping = (state.damping + step * direction).clamp(0.0, 1.0);
            reverb_control.set_damping(state.damping);
        }
        PARAM_BPM => {
            let step = if fine { 1.0 } else { 5.0 };
            state.bpm = (state.bpm + step * direction).clamp(60.0, 200.0);
            let mut engine = audio_engine.lock().unwrap();
            engine.set_bpm(state.bpm);
            if let Some(seq) = engine.sequencer_mut(0) {
                seq.set_bpm(state.bpm);
            }
        }
        _ => {}
    }
}

#[cfg(not(feature = "native"))]
fn main() {
    println!("This example is only available with the 'native' feature enabled.");
}
