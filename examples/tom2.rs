//! Tom2 Example - Morph oscillator test for A/B comparison with Max/MSP patch
//!
//! Controls:
//! - SPACE = trigger tom
//! - ↑/↓ = select parameter
//! - ←/→ = coarse adjustment (±10)
//! - [/] = fine adjustment (±1)
//! - Z/X/C/V = trigger at 25/50/75/100% velocity
//! - Q = quit

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use gooey::engine::{Engine, EngineOutput, Instrument};
use gooey::instruments::Tom2;

// Parameter metadata for the UI
struct ParamInfo {
    name: &'static str,
    coarse_step: f32,
    fine_step: f32,
    min: f32,
    max: f32,
}

const PARAM_INFO: [ParamInfo; 5] = [
    ParamInfo { name: "tune", coarse_step: 10.0, fine_step: 1.0, min: 0.0, max: 100.0 },
    ParamInfo { name: "bend", coarse_step: 10.0, fine_step: 1.0, min: 0.0, max: 100.0 },
    ParamInfo { name: "tone", coarse_step: 10.0, fine_step: 1.0, min: 0.0, max: 100.0 },
    ParamInfo { name: "color", coarse_step: 10.0, fine_step: 1.0, min: 0.0, max: 100.0 },
    ParamInfo { name: "decay", coarse_step: 10.0, fine_step: 1.0, min: 0.0, max: 100.0 },
];

// Wrapper to share Tom2 between audio thread and main thread
struct SharedTom2(Arc<Mutex<Tom2>>);

impl Instrument for SharedTom2 {
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

// Helper functions for parameter access
fn get_param_value(tom: &Tom2, index: usize) -> f32 {
    match index {
        0 => tom.tune(),
        1 => tom.bend(),
        2 => tom.tone(),
        3 => tom.color(),
        4 => tom.decay(),
        _ => 0.0,
    }
}

fn set_param_value(tom: &mut Tom2, index: usize, value: f32) {
    match index {
        0 => tom.set_tune(value),
        1 => tom.set_bend(value),
        2 => tom.set_tone(value),
        3 => tom.set_color(value),
        4 => tom.set_decay(value),
        _ => {}
    }
}

fn adjust_param(tom: &mut Tom2, index: usize, delta: f32) {
    let info = &PARAM_INFO[index];
    let current = get_param_value(tom, index);
    let new_value = (current + delta).clamp(info.min, info.max);
    set_param_value(tom, index, new_value);
}

// Create a visual bar for parameter value
fn make_bar(value: f32, min: f32, max: f32, width: usize) -> String {
    let normalized = (value - min) / (max - min);
    let filled = (normalized * width as f32).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

fn render_display(tom: &Tom2, selected: usize, trigger_count: u32, velocity: f32) {
    // Clear screen, move cursor to home, and disable line wrapping
    print!("\x1b[2J\x1b[H\x1b[?7l");

    print!("=== Tom2 - Morph Oscillator Test ===\r\n");
    print!("SPACE=hit Q=quit ↑↓=sel ←→=adj []=fine T=tri\r\n");
    print!("Z/X/C/V=vel 25/50/75/100%\r\n");
    print!("\r\n");

    for (i, info) in PARAM_INFO.iter().enumerate() {
        let value = get_param_value(tom, i);
        let bar = make_bar(value, info.min, info.max, 10);

        let indicator = if i == selected { ">" } else { " " };

        // Show frequency in Hz for tune parameter, ms for decay
        if i == 0 {
            let freq = tom.frequency();
            print!(
                "{} {:<10} [{}] {:>6.1} ({:.0} Hz)\r\n",
                indicator, info.name, bar, value, freq
            );
        } else if i == 4 {
            let decay_ms = tom.decay_ms();
            print!(
                "{} {:<10} [{}] {:>6.1} ({:.0} ms)\r\n",
                indicator, info.name, bar, value, decay_ms
            );
        } else {
            print!(
                "{} {:<10} [{}] {:>6.1}\r\n",
                indicator, info.name, bar, value
            );
        }
    }

    print!("\r\n");
    let tri_state = if tom.triangle_enabled() { "ON" } else { "OFF" };
    print!("Hits: {} | Vel: {:.0}% | Triangle: {}\r\n", trigger_count, velocity * 100.0, tri_state);
    print!("\r\n");
    print!("Ch1: sine+fixed190Hz | Ch2: triangle | Ch3: (empty)\r\n");
    print!("tone=0: Ch1 only | tone=50: Ch2 only | tone=100: silent\r\n");

    io::stdout().flush().unwrap();
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;

    // Create Tom2 instrument
    let tom2 = Arc::new(Mutex::new(Tom2::new(sample_rate)));

    // Create shared wrapper for the engine
    let shared_tom2 = SharedTom2(tom2.clone());

    // Create the audio engine
    let mut engine = Engine::new(sample_rate);
    engine.add_instrument("tom2", Box::new(shared_tom2));

    // Wrap engine in Arc<Mutex> for thread-safe access
    let audio_engine = Arc::new(Mutex::new(engine));

    // Create and configure the Engine output
    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;

    #[cfg(feature = "visualization")]
    engine_output.enable_visualization(1200, 400, 2.0)?;

    engine_output.create_stream_with_engine(audio_engine.clone())?;

    // Start the audio stream
    engine_output.start()?;

    // UI state
    let mut selected_param: usize = 0;
    let mut trigger_count: u32 = 0;
    let mut current_velocity: f32 = 0.75;
    let mut needs_redraw = true;

    // Clear screen and enable raw mode
    execute!(io::stdout(), Clear(ClearType::All), cursor::Hide)?;
    enable_raw_mode()?;

    // Main input loop
    let result = loop {
        #[cfg(feature = "visualization")]
        if engine_output.update_visualization() {
            break Ok(());
        }

        // Render display if needed
        if needs_redraw {
            let t = tom2.lock().unwrap();
            render_display(&t, selected_param, trigger_count, current_velocity);
            needs_redraw = false;
        }

        // Poll for key events (non-blocking with short timeout)
        if event::poll(std::time::Duration::from_millis(16))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match code {
                    // Parameter navigation
                    KeyCode::Up => {
                        selected_param = selected_param.saturating_sub(1);
                        needs_redraw = true;
                    }
                    KeyCode::Down => {
                        selected_param = (selected_param + 1).min(PARAM_INFO.len() - 1);
                        needs_redraw = true;
                    }

                    // Coarse adjustment
                    KeyCode::Left => {
                        let step = PARAM_INFO[selected_param].coarse_step;
                        let mut t = tom2.lock().unwrap();
                        adjust_param(&mut t, selected_param, -step);
                        needs_redraw = true;
                    }
                    KeyCode::Right => {
                        let step = PARAM_INFO[selected_param].coarse_step;
                        let mut t = tom2.lock().unwrap();
                        adjust_param(&mut t, selected_param, step);
                        needs_redraw = true;
                    }

                    // Fine adjustment
                    KeyCode::Char('[') => {
                        let step = PARAM_INFO[selected_param].fine_step;
                        let mut t = tom2.lock().unwrap();
                        adjust_param(&mut t, selected_param, -step);
                        needs_redraw = true;
                    }
                    KeyCode::Char(']') => {
                        let step = PARAM_INFO[selected_param].fine_step;
                        let mut t = tom2.lock().unwrap();
                        adjust_param(&mut t, selected_param, step);
                        needs_redraw = true;
                    }

                    // Trigger at current velocity
                    KeyCode::Char(' ') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("tom2", current_velocity);
                        trigger_count += 1;
                        needs_redraw = true;
                    }

                    // Velocity-specific triggers
                    KeyCode::Char('z') | KeyCode::Char('Z') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("tom2", 0.25);
                        trigger_count += 1;
                        current_velocity = 0.25;
                        needs_redraw = true;
                    }
                    KeyCode::Char('x') | KeyCode::Char('X') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("tom2", 0.50);
                        trigger_count += 1;
                        current_velocity = 0.50;
                        needs_redraw = true;
                    }
                    KeyCode::Char('c') | KeyCode::Char('C') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("tom2", 0.75);
                        trigger_count += 1;
                        current_velocity = 0.75;
                        needs_redraw = true;
                    }
                    KeyCode::Char('v') | KeyCode::Char('V') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("tom2", 1.0);
                        trigger_count += 1;
                        current_velocity = 1.0;
                        needs_redraw = true;
                    }

                    // Toggle triangle oscillator
                    KeyCode::Char('t') | KeyCode::Char('T') => {
                        let mut t = tom2.lock().unwrap();
                        let enabled = !t.triangle_enabled();
                        t.set_triangle_enabled(enabled);
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

    // Restore terminal to normal mode
    print!("\x1b[?7h"); // Re-enable line wrapping
    execute!(io::stdout(), cursor::Show)?;
    disable_raw_mode()?;
    println!("\nQuitting...");

    result
}

#[cfg(not(feature = "native"))]
fn main() {
    println!("This example requires the 'native' feature. Run with: cargo run --example tom2 --features native");
}
