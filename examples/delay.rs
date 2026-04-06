/* Delay Lab - Interactive CLI for filter delay experimentation.
Demonstrates BPM-synced delay with musical timing divisions and feedback-path lowpass filter.
*/

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use gooey::effects::{DelayEffect, DelayTiming};
use gooey::engine::{Engine, EngineOutput, Instrument, Sequencer};
use gooey::instruments::HiHat;

const DELAY_TIMINGS: [DelayTiming; 9] = [
    DelayTiming::Whole,
    DelayTiming::Half,
    DelayTiming::Quarter,
    DelayTiming::Eighth,
    DelayTiming::Sixteenth,
    DelayTiming::HalfTriplet,
    DelayTiming::QuarterTriplet,
    DelayTiming::EighthTriplet,
    DelayTiming::SixteenthTriplet,
];

fn timing_name(t: DelayTiming) -> &'static str {
    match t {
        DelayTiming::Whole => "1/1",
        DelayTiming::Half => "1/2",
        DelayTiming::Quarter => "1/4",
        DelayTiming::Eighth => "1/8",
        DelayTiming::Sixteenth => "1/16",
        DelayTiming::HalfTriplet => "1/2T",
        DelayTiming::QuarterTriplet => "1/4T",
        DelayTiming::EighthTriplet => "1/8T",
        DelayTiming::SixteenthTriplet => "1/16T",
    }
}

fn timing_index(t: DelayTiming) -> usize {
    DELAY_TIMINGS.iter().position(|&d| d == t).unwrap_or(3)
}

// Lock-free control handle for the delay effect.
// SAFETY: DelayEffect setters take &self and use atomics internally.
// The raw pointer is valid as long as the engine (which owns the Box<dyn Effect>) is alive.
struct DelayControl {
    ptr: *const DelayEffect,
}

unsafe impl Send for DelayControl {}

impl DelayControl {
    fn set_timing(&self, t: DelayTiming) {
        unsafe { &*self.ptr }.set_timing(t);
    }
    fn set_bpm(&self, bpm: f32) {
        unsafe { &*self.ptr }.set_bpm(bpm);
    }
    fn set_feedback(&self, fb: f32) {
        unsafe { &*self.ptr }.set_feedback(fb);
    }
    fn set_mix(&self, mix: f32) {
        unsafe { &*self.ptr }.set_mix(mix);
    }
    fn set_filter_cutoff(&self, cutoff: f32) {
        unsafe { &*self.ptr }.set_filter_cutoff(cutoff);
    }
}

// Parameter indices
const PARAM_BPM: usize = 0;
const PARAM_TIMING: usize = 1;
const PARAM_FEEDBACK: usize = 2;
const PARAM_MIX: usize = 3;
const PARAM_FILTER_CUTOFF: usize = 4;
const PARAM_COUNT: usize = 5;

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

    fn as_modulatable(&mut self) -> Option<&mut dyn gooey::engine::Modulatable> {
        None
    }
}

struct DelayState {
    bpm: f32,
    timing_idx: usize,
    feedback: f32,
    mix: f32,
    filter_cutoff: f32,
    running: bool,
}

fn make_bar(normalized: f32, width: usize) -> String {
    let filled = (normalized * width as f32).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

fn render_display(state: &DelayState, selected: usize) {
    print!("\x1b[2J\x1b[H\x1b[?7l");

    print!("=== Delay Lab ===\r\n");
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

    // Timing division
    let ind = if selected == PARAM_TIMING { ">" } else { " " };
    let timing_norm = state.timing_idx as f32 / (DELAY_TIMINGS.len() - 1) as f32;
    print!(
        "{} {:<18} [{}] {:>6}\r\n",
        ind,
        "timing",
        make_bar(timing_norm, 10),
        timing_name(DELAY_TIMINGS[state.timing_idx])
    );

    // Feedback
    let ind = if selected == PARAM_FEEDBACK { ">" } else { " " };
    let fb_norm = state.feedback / 0.95;
    print!(
        "{} {:<18} [{}] {:>6.2}\r\n",
        ind,
        "feedback",
        make_bar(fb_norm, 10),
        state.feedback
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

    // Filter cutoff
    let ind = if selected == PARAM_FILTER_CUTOFF { ">" } else { " " };
    let cutoff_norm = (state.filter_cutoff - 20.0) / 19980.0;
    print!(
        "{} {:<18} [{}] {:>5.0} Hz\r\n",
        ind,
        "filter cutoff",
        make_bar(cutoff_norm, 10),
        state.filter_cutoff
    );

    io::stdout().flush().unwrap();
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;

    let mut engine = Engine::new(sample_rate);

    let hihat = Arc::new(Mutex::new(HiHat::new(sample_rate)));
    hihat.lock().unwrap().set_decay(0.05);
    engine.add_instrument("hihat", Box::new(SharedHiHat(hihat.clone())));

    let bpm = 120.0;
    engine.set_bpm(bpm);

    // Sparse pattern: hits on steps 0 and 8 so delay echoes are clearly audible
    let pattern = vec![
        true, false, false, false, false, false, false, false, true, false, false, false, false,
        false, false, false,
    ];
    let sequencer = Sequencer::with_pattern(bpm, sample_rate, pattern, "hihat");
    engine.add_sequencer(sequencer);

    engine.set_master_gain(1.0);

    // Create delay and get a lock-free control handle before moving into engine
    let delay = Box::new(DelayEffect::new(
        sample_rate,
        DelayTiming::Eighth,
        bpm,
        0.5,
        0.4,
        8000.0,
    ));
    let delay_control = DelayControl {
        ptr: &*delay as *const DelayEffect,
    };
    engine.add_global_effect(delay);

    let audio_engine = Arc::new(Mutex::new(engine));

    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;

    #[cfg(feature = "visualization")]
    engine_output.enable_visualization(1200, 600, 2.0)?;

    engine_output.create_stream_with_engine(audio_engine.clone())?;
    engine_output.start()?;

    let mut state = DelayState {
        bpm,
        timing_idx: timing_index(DelayTiming::Eighth),
        feedback: 0.5,
        mix: 0.4,
        filter_cutoff: 8000.0,
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
                            &delay_control,
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
                            &delay_control,
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
                            &delay_control,
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
                            &delay_control,
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
    delay_control: &DelayControl,
    state: &mut DelayState,
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
            delay_control.set_bpm(state.bpm);
        }
        PARAM_TIMING => {
            let new_idx = if direction > 0.0 {
                (state.timing_idx + 1).min(DELAY_TIMINGS.len() - 1)
            } else {
                state.timing_idx.saturating_sub(1)
            };
            state.timing_idx = new_idx;
            delay_control.set_timing(DELAY_TIMINGS[new_idx]);
        }
        PARAM_FEEDBACK => {
            let step = if fine { 0.01 } else { 0.05 };
            state.feedback = (state.feedback + step * direction).clamp(0.0, 0.95);
            delay_control.set_feedback(state.feedback);
        }
        PARAM_MIX => {
            let step = if fine { 0.01 } else { 0.05 };
            state.mix = (state.mix + step * direction).clamp(0.0, 1.0);
            delay_control.set_mix(state.mix);
        }
        PARAM_FILTER_CUTOFF => {
            let step = if fine { 50.0 } else { 500.0 };
            state.filter_cutoff = (state.filter_cutoff + step * direction).clamp(20.0, 20000.0);
            delay_control.set_filter_cutoff(state.filter_cutoff);
        }
        _ => {}
    }
}

#[cfg(not(feature = "native"))]
fn main() {
    println!("This example is only available with the 'native' feature enabled.");
}
