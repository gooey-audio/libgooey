/* Spectral Resonator Lab - Interactive CLI for STFT spectral-resonance experimentation.

The resonator breaks the input into spectral partials, emphasizes the bins that align
with a tunable fundamental + its harmonics, and lets that energy ring over time.

A snare pattern excites it with broadband transients so the tuned resonance is clearly
audible. Note: the effect adds ~23 ms of latency (FFT size 1024).
*/

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use gooey::effects::SpectralResonator;
use gooey::engine::{Engine, EngineOutput, Instrument, Sequencer};
use gooey::instruments::SnareDrum;

// Lock-free control handle for the resonator effect.
// SAFETY: SpectralResonator setters take &self and use atomics internally.
// The raw pointer is valid as long as the engine (which owns the Box<dyn Effect>) is alive.
struct ResonatorControl {
    ptr: *const SpectralResonator,
}

unsafe impl Send for ResonatorControl {}

impl ResonatorControl {
    fn set_frequency(&self, v: f32) {
        unsafe { &*self.ptr }.set_frequency(v);
    }
    fn set_resonance(&self, v: f32) {
        unsafe { &*self.ptr }.set_resonance(v);
    }
    fn set_sharpness(&self, v: f32) {
        unsafe { &*self.ptr }.set_sharpness(v);
    }
    fn set_mix(&self, v: f32) {
        unsafe { &*self.ptr }.set_mix(v);
    }
}

// Parameter indices
const PARAM_FREQUENCY: usize = 0;
const PARAM_RESONANCE: usize = 1;
const PARAM_SHARPNESS: usize = 2;
const PARAM_MIX: usize = 3;
const PARAM_BPM: usize = 4;
const PARAM_COUNT: usize = 5;

const MIN_FREQ: f32 = 20.0;
const MAX_FREQ: f32 = 4000.0;

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

struct ResonatorState {
    frequency: f32,
    resonance: f32,
    sharpness: f32,
    mix: f32,
    bpm: f32,
    running: bool,
    enabled: bool,
}

fn make_bar(normalized: f32, width: usize) -> String {
    let filled = (normalized.clamp(0.0, 1.0) * width as f32).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

fn render_display(state: &ResonatorState, selected: usize) {
    print!("\x1b[2J\x1b[H\x1b[?7l");

    print!("=== Spectral Resonator Lab ===\r\n");
    print!("SPACE=start/stop  B=bypass  Q=quit  ↑↓=sel  ←→=adj  []=fine\r\n");
    let status = if state.running { "RUNNING" } else { "STOPPED" };
    let fx_status = if state.enabled { "ON" } else { "BYPASS" };
    print!(
        "Status: {}  Resonator: {}  (latency ~23ms)\r\n",
        status, fx_status
    );
    print!("\r\n");

    // Frequency (log-scaled bar)
    let ind = if selected == PARAM_FREQUENCY {
        ">"
    } else {
        " "
    };
    let freq_norm = (state.frequency / MIN_FREQ).log2() / (MAX_FREQ / MIN_FREQ).log2();
    print!(
        "{} {:<18} [{}] {:>7.1} Hz\r\n",
        ind,
        "frequency",
        make_bar(freq_norm, 10),
        state.frequency
    );

    // Resonance
    let ind = if selected == PARAM_RESONANCE {
        ">"
    } else {
        " "
    };
    print!(
        "{} {:<18} [{}] {:>6.2}\r\n",
        ind,
        "resonance (ring)",
        make_bar(state.resonance, 10),
        state.resonance
    );

    // Sharpness
    let ind = if selected == PARAM_SHARPNESS {
        ">"
    } else {
        " "
    };
    print!(
        "{} {:<18} [{}] {:>6.2}\r\n",
        ind,
        "sharpness",
        make_bar(state.sharpness, 10),
        state.sharpness
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

    // Steady pattern so the resonator is continuously excited.
    let pattern = vec![
        true, false, true, false, true, false, true, false, true, false, true, false, true, false,
        true, false,
    ];
    let sequencer = Sequencer::with_pattern(bpm, sample_rate, pattern, "snare");
    engine.add_sequencer(sequencer);

    engine.set_master_gain(0.7);

    let init = ResonatorState {
        frequency: 220.0,
        resonance: 0.85,
        sharpness: 0.7,
        mix: 0.6,
        bpm,
        running: false,
        enabled: true,
    };

    // Create the resonator and grab a lock-free control handle before moving it in.
    let resonator = Box::new(SpectralResonator::new(
        sample_rate,
        init.frequency,
        init.resonance,
        init.sharpness,
        init.mix,
    ));
    let resonator_control = ResonatorControl {
        ptr: &*resonator as *const SpectralResonator,
    };
    engine.add_global_effect(resonator);

    let audio_engine = Arc::new(Mutex::new(engine));

    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;

    #[cfg(feature = "visualization")]
    engine_output.enable_visualization(1200, 600, 2.0)?;

    engine_output.create_stream_with_engine(audio_engine.clone())?;
    engine_output.start()?;

    let mut state = init;
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
                    KeyCode::Right => {
                        adjust_param(
                            &audio_engine,
                            &resonator_control,
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
                            &resonator_control,
                            &mut state,
                            selected,
                            -1.0,
                            false,
                        );
                        needs_redraw = true;
                    }
                    KeyCode::Char(']') => {
                        adjust_param(
                            &audio_engine,
                            &resonator_control,
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
                            &resonator_control,
                            &mut state,
                            selected,
                            -1.0,
                            true,
                        );
                        needs_redraw = true;
                    }
                    KeyCode::Char('b') | KeyCode::Char('B') => {
                        state.enabled = !state.enabled;
                        if state.enabled {
                            resonator_control.set_mix(state.mix);
                        } else {
                            resonator_control.set_mix(0.0);
                        }
                        needs_redraw = true;
                    }
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

#[cfg(feature = "native")]
fn adjust_param(
    audio_engine: &Arc<Mutex<Engine>>,
    control: &ResonatorControl,
    state: &mut ResonatorState,
    param: usize,
    direction: f32,
    fine: bool,
) {
    match param {
        PARAM_FREQUENCY => {
            // Multiplicative (musical) steps: a semitone coarse, a tenth fine.
            let semitones = if fine { 0.1 } else { 1.0 } * direction;
            state.frequency =
                (state.frequency * 2.0_f32.powf(semitones / 12.0)).clamp(MIN_FREQ, MAX_FREQ);
            control.set_frequency(state.frequency);
        }
        PARAM_RESONANCE => {
            let step = if fine { 0.01 } else { 0.05 };
            state.resonance = (state.resonance + step * direction).clamp(0.0, 1.0);
            control.set_resonance(state.resonance);
        }
        PARAM_SHARPNESS => {
            let step = if fine { 0.01 } else { 0.05 };
            state.sharpness = (state.sharpness + step * direction).clamp(0.0, 1.0);
            control.set_sharpness(state.sharpness);
        }
        PARAM_MIX => {
            let step = if fine { 0.01 } else { 0.05 };
            state.mix = (state.mix + step * direction).clamp(0.0, 1.0);
            if state.enabled {
                control.set_mix(state.mix);
            }
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
