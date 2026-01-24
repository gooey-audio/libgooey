/* Tom Drum Lab - Interactive CLI for tom drum parameter experimentation.
Supports real-time parameter adjustment, presets, and velocity control.
*/

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use std::io::{self, Write};

use gooey::engine::{Engine, EngineOutput, Instrument};
use gooey::instruments::{TomConfig, TomDrum};
use std::sync::{Arc, Mutex};

// Parameter metadata for the UI
// All parameters use normalized 0-1 values
struct ParamInfo {
    name: &'static str,
    coarse_step: f32,
    fine_step: f32,
    unit: &'static str,
}

const PARAM_INFO: [ParamInfo; 8] = [
    ParamInfo { name: "frequency", coarse_step: 0.1, fine_step: 0.02, unit: "" },      // 0-1 → 60-300 Hz
    ParamInfo { name: "decay", coarse_step: 0.1, fine_step: 0.02, unit: "" },          // 0-1 → 0.05-2.0s
    ParamInfo { name: "tonal", coarse_step: 0.1, fine_step: 0.02, unit: "" },
    ParamInfo { name: "punch", coarse_step: 0.1, fine_step: 0.02, unit: "" },
    ParamInfo { name: "pitch_drop", coarse_step: 0.1, fine_step: 0.02, unit: "" },
    ParamInfo { name: "volume", coarse_step: 0.1, fine_step: 0.02, unit: "" },
    ParamInfo { name: "amp_decay", coarse_step: 0.1, fine_step: 0.02, unit: "" },      // 0-1 → 0.0-3.0s
    ParamInfo { name: "amp_dcy_crv", coarse_step: 0.1, fine_step: 0.02, unit: "" },    // 0-1 → 0.1-10.0
];

// Wrapper to share TomDrum between audio thread and main thread
struct SharedTom(Arc<Mutex<TomDrum>>);

impl Instrument for SharedTom {
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
        None // We control params directly, not through modulation
    }
}

// Helper functions for parameter access
// All parameters use normalized 0-1 values
fn get_param_value(tom: &TomDrum, index: usize) -> f32 {
    match index {
        0 => tom.params.frequency.get(),
        1 => tom.params.decay.get(),
        2 => tom.params.tonal.get(),
        3 => tom.params.punch.get(),
        4 => tom.params.pitch_drop.get(),
        5 => tom.params.volume.get(),
        6 => tom.params.amp_decay.get(),
        7 => tom.params.amp_decay_curve.get(),
        _ => 0.0,
    }
}

fn get_param_range(_tom: &TomDrum, _index: usize) -> (f32, f32) {
    // All parameters now use normalized 0-1 range
    (0.0, 1.0)
}

fn set_param_value(tom: &mut TomDrum, index: usize, value: f32) {
    match index {
        0 => tom.set_frequency(value),
        1 => tom.set_decay(value),
        2 => tom.set_tonal(value),
        3 => tom.set_punch(value),
        4 => tom.set_pitch_drop(value),
        5 => tom.set_volume(value),
        6 => tom.set_amp_decay(value),
        7 => tom.set_amp_decay_curve(value),
        _ => {}
    }
}

fn adjust_param(tom: &mut TomDrum, index: usize, delta: f32) {
    let current = get_param_value(tom, index);
    let (min, max) = get_param_range(tom, index);
    let new_value = (current + delta).clamp(min, max);
    set_param_value(tom, index, new_value);
}

// Create a visual bar for normalized value
fn make_bar(normalized: f32, width: usize) -> String {
    let filled = (normalized * width as f32).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

// Render the parameter display
fn render_display(
    tom: &TomDrum,
    selected: usize,
    trigger_count: u32,
    velocity: f32,
    preset_name: &str,
) {
    // Clear screen, move cursor to home, and disable line wrapping
    print!("\x1b[2J\x1b[H\x1b[?7l");

    print!("=== Tom Drum Lab ===\r\n");
    print!("SPACE=hit Q=quit ↑↓=sel ←→=adj []=fine\r\n");
    print!("Z/X/C/V=vel 25/50/75/100% +/-=adj 1-5=preset\r\n");
    print!("Preset: {}\r\n", preset_name);
    print!("\r\n");

    for (i, info) in PARAM_INFO.iter().enumerate() {
        let value = get_param_value(tom, i);
        let (min, max) = get_param_range(tom, i);
        let normalized = (value - min) / (max - min);
        let bar = make_bar(normalized, 10);

        let indicator = if i == selected { ">" } else { " " };
        print!(
            "{} {:<18} [{}] {:>6.2}{:<3}\r\n",
            indicator, info.name, bar, value, info.unit
        );
    }

    print!("\r\n");
    print!("Hits: {} | Vel: {:.0}%", trigger_count, velocity * 100.0);

    // Flush to ensure display updates
    io::stdout().flush().unwrap();
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;

    // Create shared tom drum that both audio thread and main thread can access
    let tom = Arc::new(Mutex::new(TomDrum::new(sample_rate)));

    // Create the audio engine
    let mut engine = Engine::new(sample_rate);

    // Add the shared tom drum wrapper to the engine
    engine.add_instrument("tom", Box::new(SharedTom(tom.clone())));

    // Wrap engine in Arc<Mutex> for thread-safe access
    let audio_engine = Arc::new(Mutex::new(engine));

    // Create and configure the Engine output
    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;

    // Enable visualization (optional - comment out to disable)
    #[cfg(feature = "visualization")]
    engine_output.enable_visualization(1200, 400, 2.0)?;

    engine_output.create_stream_with_engine(audio_engine.clone())?;

    // Start the audio stream
    engine_output.start()?;

    #[cfg(feature = "visualization")]
    println!("Waveform visualization enabled");

    // UI state
    let mut selected_param: usize = 0;
    let mut trigger_count: u32 = 0;
    let mut current_velocity: f32 = 0.75;
    let mut current_preset = "Default";
    let mut needs_redraw = true;

    // Clear screen and enable raw mode
    execute!(io::stdout(), Clear(ClearType::All), cursor::Hide)?;
    enable_raw_mode()?;

    // Main input loop
    let result = loop {
        // Update visualization if enabled (no-op if disabled)
        if engine_output.update_visualization() {
            println!("\rVisualization window closed");
            break Ok(());
        }

        // Render display if needed
        if needs_redraw {
            let t = tom.lock().unwrap();
            render_display(&t, selected_param, trigger_count, current_velocity, current_preset);
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
                        let mut t = tom.lock().unwrap();
                        adjust_param(&mut t, selected_param, -step);
                        current_preset = "Custom";
                        needs_redraw = true;
                    }
                    KeyCode::Right => {
                        let step = PARAM_INFO[selected_param].coarse_step;
                        let mut t = tom.lock().unwrap();
                        adjust_param(&mut t, selected_param, step);
                        current_preset = "Custom";
                        needs_redraw = true;
                    }

                    // Fine adjustment
                    KeyCode::Char('[') => {
                        let step = PARAM_INFO[selected_param].fine_step;
                        let mut t = tom.lock().unwrap();
                        adjust_param(&mut t, selected_param, -step);
                        current_preset = "Custom";
                        needs_redraw = true;
                    }
                    KeyCode::Char(']') => {
                        let step = PARAM_INFO[selected_param].fine_step;
                        let mut t = tom.lock().unwrap();
                        adjust_param(&mut t, selected_param, step);
                        current_preset = "Custom";
                        needs_redraw = true;
                    }

                    // Presets
                    KeyCode::Char('1') => {
                        let mut t = tom.lock().unwrap();
                        t.set_config(TomConfig::default());
                        current_preset = "Default";
                        needs_redraw = true;
                    }
                    KeyCode::Char('2') => {
                        let mut t = tom.lock().unwrap();
                        t.set_config(TomConfig::high_tom());
                        current_preset = "High Tom";
                        needs_redraw = true;
                    }
                    KeyCode::Char('3') => {
                        let mut t = tom.lock().unwrap();
                        t.set_config(TomConfig::mid_tom());
                        current_preset = "Mid Tom";
                        needs_redraw = true;
                    }
                    KeyCode::Char('4') => {
                        let mut t = tom.lock().unwrap();
                        t.set_config(TomConfig::low_tom());
                        current_preset = "Low Tom";
                        needs_redraw = true;
                    }
                    KeyCode::Char('5') => {
                        let mut t = tom.lock().unwrap();
                        t.set_config(TomConfig::floor_tom());
                        current_preset = "Floor Tom";
                        needs_redraw = true;
                    }

                    // Trigger at current velocity
                    KeyCode::Char(' ') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("tom", current_velocity);
                        trigger_count += 1;
                        needs_redraw = true;
                    }

                    // Velocity-specific triggers
                    KeyCode::Char('z') | KeyCode::Char('Z') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("tom", 0.25);
                        trigger_count += 1;
                        current_velocity = 0.25;
                        needs_redraw = true;
                    }
                    KeyCode::Char('x') | KeyCode::Char('X') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("tom", 0.50);
                        trigger_count += 1;
                        current_velocity = 0.50;
                        needs_redraw = true;
                    }
                    KeyCode::Char('c') | KeyCode::Char('C') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("tom", 0.75);
                        trigger_count += 1;
                        current_velocity = 0.75;
                        needs_redraw = true;
                    }
                    KeyCode::Char('v') | KeyCode::Char('V') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("tom", 1.0);
                        trigger_count += 1;
                        current_velocity = 1.0;
                        needs_redraw = true;
                    }

                    // Adjust default velocity
                    KeyCode::Char('+') | KeyCode::Char('=') => {
                        current_velocity = (current_velocity + 0.05).min(1.0);
                        needs_redraw = true;
                    }
                    KeyCode::Char('-') | KeyCode::Char('_') => {
                        current_velocity = (current_velocity - 0.05).max(0.05);
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

    // Restore terminal to normal mode and re-enable line wrapping
    print!("\x1b[?7h"); // Re-enable line wrapping
    execute!(io::stdout(), cursor::Show)?;
    disable_raw_mode()?;
    println!("\nQuitting...");

    result
}

#[cfg(not(feature = "native"))]
fn main() {
    println!("This example is only available with the 'native' feature enabled.");
}
