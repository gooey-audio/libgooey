/* Hi-Hat Lab - Interactive CLI for hi-hat parameter experimentation.
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
use gooey::instruments::{HiHatConfig, HiHat};
use std::sync::{Arc, Mutex};

// Parameter metadata for the UI
// All parameters use normalized 0-1 values
// Parameters in alphabetical order for display
struct ParamInfo {
    name: &'static str,
    coarse_step: f32,
    fine_step: f32,
    unit: &'static str,
}

const PARAM_INFO: [ParamInfo; 6] = [
    ParamInfo { name: "amp_decay", coarse_step: 0.1, fine_step: 0.02, unit: "" },      // 0-1 -> 0.0-4.0s
    ParamInfo { name: "amp_dcy_crv", coarse_step: 0.1, fine_step: 0.02, unit: "" },    // 0-1 -> 0.1-10.0
    ParamInfo { name: "decay", coarse_step: 0.1, fine_step: 0.02, unit: "" },          // 0-1 -> 0.005-0.4s
    ParamInfo { name: "filter", coarse_step: 0.1, fine_step: 0.02, unit: "" },
    ParamInfo { name: "frequency", coarse_step: 0.1, fine_step: 0.02, unit: "" },      // 0-1 -> 4000-16000 Hz
    ParamInfo { name: "volume", coarse_step: 0.1, fine_step: 0.02, unit: "" },
];

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
        None // We control params directly, not through modulation
    }
}

// Helper functions for parameter access
// All parameters use normalized 0-1 values
// Indices match alphabetical PARAM_INFO order
fn get_param_value(hihat: &HiHat, index: usize) -> f32 {
    match index {
        0 => hihat.params.amp_decay.get(),
        1 => hihat.params.amp_decay_curve.get(),
        2 => hihat.params.decay.get(),
        3 => hihat.params.filter.get(),
        4 => hihat.params.frequency.get(),
        5 => hihat.params.volume.get(),
        _ => 0.0,
    }
}

fn get_param_range(_hihat: &HiHat, _index: usize) -> (f32, f32) {
    // All parameters now use normalized 0-1 range
    (0.0, 1.0)
}

fn set_param_value(hihat: &mut HiHat, index: usize, value: f32) {
    match index {
        0 => hihat.set_amp_decay(value),
        1 => hihat.set_amp_decay_curve(value),
        2 => hihat.set_decay(value),
        3 => hihat.set_filter(value),
        4 => hihat.set_frequency(value),
        5 => hihat.set_volume(value),
        _ => {}
    }
}

fn adjust_param(hihat: &mut HiHat, index: usize, delta: f32) {
    let current = get_param_value(hihat, index);
    let (min, max) = get_param_range(hihat, index);
    let new_value = (current + delta).clamp(min, max);
    set_param_value(hihat, index, new_value);
}

// Create a visual bar for normalized value
fn make_bar(normalized: f32, width: usize) -> String {
    let filled = (normalized * width as f32).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    format!("{}{}", "=".repeat(filled), "-".repeat(empty))
}

// Render the parameter display
fn render_display(
    hihat: &HiHat,
    selected: usize,
    trigger_count: u32,
    velocity: f32,
    preset_name: &str,
    is_open: bool,
) {
    // Clear screen, move cursor to home, and disable line wrapping
    print!("\x1b[2J\x1b[H\x1b[?7l");

    print!("=== Hi-Hat Lab ===\r\n");
    print!("SPACE=hit Q=quit arrows=sel/adj []=fine O=open/close\r\n");
    print!("1-4=presets V/B=velocity\r\n");
    print!("\r\n");

    // Display preset and velocity info
    print!(
        "Preset: {} | Vel: {:.0}% | Hits: {} | Mode: {}\r\n",
        preset_name,
        velocity * 100.0,
        trigger_count,
        if is_open { "OPEN" } else { "CLOSED" }
    );
    print!("\r\n");

    // Display parameters
    for (i, info) in PARAM_INFO.iter().enumerate() {
        let value = get_param_value(hihat, i);
        let bar = make_bar(value, 10);

        // Selection indicator
        let indicator = if i == selected { ">" } else { " " };

        print!(
            "{} {:<12} [{}] {:>6.2}{}\r\n",
            indicator, info.name, bar, value, info.unit
        );
    }

    print!("\r\n");
    io::stdout().flush().unwrap();
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;

    // Create hi-hat with default config
    let hihat = Arc::new(Mutex::new(HiHat::with_config(
        sample_rate,
        HiHatConfig::closed_default(),
    )));

    // Create shared wrapper for the engine
    let shared_hihat = SharedHiHat(hihat.clone());

    // Create the audio engine
    let mut engine = Engine::new(sample_rate);
    engine.add_instrument("hihat", Box::new(shared_hihat));

    // Wrap engine in Arc<Mutex> for thread-safe access
    let audio_engine = Arc::new(Mutex::new(engine));

    // Enable raw mode for immediate key detection
    enable_raw_mode()?;

    // Create and configure the Engine output
    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;

    #[cfg(feature = "visualization")]
    engine_output.enable_visualization(1200, 400, 2.0)?;

    engine_output.create_stream_with_engine(audio_engine.clone())?;

    // Start the audio stream
    engine_output.start()?;

    // UI state
    let mut selected_param = 0;
    let mut trigger_count = 0u32;
    let mut velocity = 0.8f32;
    let mut preset_name = "closed_default";
    let mut is_open = false;

    // Initial display
    {
        let hihat_lock = hihat.lock().unwrap();
        render_display(&hihat_lock, selected_param, trigger_count, velocity, preset_name, is_open);
    }

    // Main input loop
    let result = loop {
        #[cfg(feature = "visualization")]
        if engine_output.update_visualization() {
            break Ok(());
        }

        // Poll for key events (non-blocking with short timeout)
        if event::poll(std::time::Duration::from_millis(16))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                let mut needs_redraw = true;

                match code {
                    // Trigger hi-hat
                    KeyCode::Char(' ') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("hihat", velocity);
                        trigger_count += 1;
                    }

                    // Quit
                    KeyCode::Char('q') | KeyCode::Esc => {
                        break Ok(());
                    }

                    // Toggle open/closed
                    KeyCode::Char('o') | KeyCode::Char('O') => {
                        is_open = !is_open;
                        let mut hihat_lock = hihat.lock().unwrap();
                        hihat_lock.set_open(is_open);
                        preset_name = if is_open { "open" } else { "closed" };
                    }

                    // Parameter navigation
                    KeyCode::Up => {
                        if selected_param > 0 {
                            selected_param -= 1;
                        }
                    }
                    KeyCode::Down => {
                        if selected_param < PARAM_INFO.len() - 1 {
                            selected_param += 1;
                        }
                    }

                    // Parameter adjustment (coarse)
                    KeyCode::Left => {
                        let mut hihat_lock = hihat.lock().unwrap();
                        adjust_param(&mut hihat_lock, selected_param, -PARAM_INFO[selected_param].coarse_step);
                    }
                    KeyCode::Right => {
                        let mut hihat_lock = hihat.lock().unwrap();
                        adjust_param(&mut hihat_lock, selected_param, PARAM_INFO[selected_param].coarse_step);
                    }

                    // Parameter adjustment (fine)
                    KeyCode::Char('[') => {
                        let mut hihat_lock = hihat.lock().unwrap();
                        adjust_param(&mut hihat_lock, selected_param, -PARAM_INFO[selected_param].fine_step);
                    }
                    KeyCode::Char(']') => {
                        let mut hihat_lock = hihat.lock().unwrap();
                        adjust_param(&mut hihat_lock, selected_param, PARAM_INFO[selected_param].fine_step);
                    }

                    // Velocity control
                    KeyCode::Char('v') | KeyCode::Char('V') => {
                        velocity = (velocity - 0.1).max(0.1);
                    }
                    KeyCode::Char('b') | KeyCode::Char('B') => {
                        velocity = (velocity + 0.1).min(1.0);
                    }

                    // Presets
                    KeyCode::Char('1') => {
                        let mut hihat_lock = hihat.lock().unwrap();
                        hihat_lock.set_config(HiHatConfig::closed_default());
                        is_open = false;
                        preset_name = "closed_default";
                    }
                    KeyCode::Char('2') => {
                        let mut hihat_lock = hihat.lock().unwrap();
                        hihat_lock.set_config(HiHatConfig::closed_tight());
                        is_open = false;
                        preset_name = "closed_tight";
                    }
                    KeyCode::Char('3') => {
                        let mut hihat_lock = hihat.lock().unwrap();
                        hihat_lock.set_config(HiHatConfig::open_default());
                        is_open = true;
                        preset_name = "open_default";
                    }
                    KeyCode::Char('4') => {
                        let mut hihat_lock = hihat.lock().unwrap();
                        hihat_lock.set_config(HiHatConfig::open_bright());
                        is_open = true;
                        preset_name = "open_bright";
                    }

                    _ => {
                        needs_redraw = false;
                    }
                }

                if needs_redraw {
                    let hihat_lock = hihat.lock().unwrap();
                    render_display(&hihat_lock, selected_param, trigger_count, velocity, preset_name, is_open);
                }
            }
        }
    };

    // Clean up terminal
    execute!(io::stdout(), Clear(ClearType::All), cursor::MoveTo(0, 0))?;
    disable_raw_mode()?;

    result
}

#[cfg(not(feature = "native"))]
fn main() {
    println!("This example is only available with the 'native' feature enabled.");
}
