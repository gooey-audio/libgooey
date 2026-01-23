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
struct ParamInfo {
    name: &'static str,
    coarse_step: f32,
    fine_step: f32,
}

// All 7 tom parameters (normalized 0-1)
const PARAM_INFO: [ParamInfo; 7] = [
    ParamInfo { name: "pitch", coarse_step: 0.05, fine_step: 0.01 },           // MIDI 36-84
    ParamInfo { name: "color", coarse_step: 0.1, fine_step: 0.02 },            // band emphasis
    ParamInfo { name: "tone", coarse_step: 0.05, fine_step: 0.01 },            // 200-8000 Hz lowpass
    ParamInfo { name: "bend", coarse_step: 0.1, fine_step: 0.02 },             // pitch envelope amount
    ParamInfo { name: "amp_decay", coarse_step: 0.05, fine_step: 0.01 },       // 0.03-0.8s
    ParamInfo { name: "amp_curve", coarse_step: 0.1, fine_step: 0.02 },        // 0.2-1.5 (<1=punchy)
    ParamInfo { name: "volume", coarse_step: 0.1, fine_step: 0.02 },
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
        None
    }
}

// Get parameter value by index
fn get_param_value(tom: &TomDrum, index: usize) -> f32 {
    match index {
        0 => tom.params.pitch.get(),
        1 => tom.params.color.get(),
        2 => tom.params.tone.get(),
        3 => tom.params.bend.get(),
        4 => tom.params.decay.get(),
        5 => tom.params.decay_curve.get(),
        6 => tom.params.volume.get(),
        _ => 0.0,
    }
}

// Set parameter value by index
fn set_param_value(tom: &mut TomDrum, index: usize, value: f32) {
    match index {
        0 => tom.set_pitch(value),
        1 => tom.set_color(value),
        2 => tom.set_tone(value),
        3 => tom.set_bend(value),
        4 => tom.set_decay(value),
        5 => tom.set_decay_curve(value),
        6 => tom.set_volume(value),
        _ => {}
    }
}

// Adjust parameter by delta
fn adjust_param(tom: &mut TomDrum, index: usize, delta: f32) {
    let current = get_param_value(tom, index);
    let new_value = (current + delta).clamp(0.0, 1.0);
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
fn render_display(tom: &TomDrum, selected: usize, trigger_count: u32, velocity: f32, preset_name: &str) {
    // Clear screen and move cursor home
    print!("\x1b[2J\x1b[H\x1b[?7l");

    print!("=== Tom Drum Lab ===\r\n");
    print!("SPACE=hit Q=quit ↑↓=sel ←→=adj []=fine\r\n");
    print!("Z/X/C/V=vel 25/50/75/100% 1-5=preset\r\n");
    print!("Preset: {}\r\n", preset_name);
    print!("\r\n");

    for (i, info) in PARAM_INFO.iter().enumerate() {
        let value = get_param_value(tom, i);
        let indicator = if i == selected { ">" } else { " " };
        let bar = make_bar(value, 12);
        print!(
            "{} {:<12} [{}] {:>5.2}\r\n",
            indicator, info.name, bar, value
        );
    }

    print!("\r\n");
    print!("Hits: {} | Vel: {:.0}%", trigger_count, velocity * 100.0);

    io::stdout().flush().unwrap();
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;

    // Create shared tom drum
    let tom = Arc::new(Mutex::new(TomDrum::new(sample_rate)));

    // Create the audio engine
    let mut engine = Engine::new(sample_rate);
    engine.add_instrument("tom", Box::new(SharedTom(tom.clone())));

    let audio_engine = Arc::new(Mutex::new(engine));

    // Create and configure the Engine output
    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;

    #[cfg(feature = "visualization")]
    engine_output.enable_visualization(1200, 400, 2.0)?;

    engine_output.create_stream_with_engine(audio_engine.clone())?;
    engine_output.start()?;

    // UI state
    let mut selected_param: usize = 0;
    let mut trigger_count: u32 = 0;
    let mut current_velocity: f32 = 0.75;
    let mut current_preset = "Mid Tom";
    let mut needs_redraw = true;

    // Clear screen and enable raw mode
    execute!(io::stdout(), Clear(ClearType::All), cursor::Hide)?;
    enable_raw_mode()?;

    // Main input loop
    let result = loop {
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

        // Poll for key events
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
                        t.set_config(TomConfig::ds_tom());
                        current_preset = "DS Tom";
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
                        current_velocity = 0.25;
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("tom", current_velocity);
                        trigger_count += 1;
                        needs_redraw = true;
                    }
                    KeyCode::Char('x') | KeyCode::Char('X') => {
                        current_velocity = 0.50;
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("tom", current_velocity);
                        trigger_count += 1;
                        needs_redraw = true;
                    }
                    KeyCode::Char('c') | KeyCode::Char('C') => {
                        current_velocity = 0.75;
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("tom", current_velocity);
                        trigger_count += 1;
                        needs_redraw = true;
                    }
                    KeyCode::Char('v') | KeyCode::Char('V') => {
                        current_velocity = 1.0;
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("tom", current_velocity);
                        trigger_count += 1;
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

    // Restore terminal
    print!("\x1b[?7h");
    execute!(io::stdout(), cursor::Show)?;
    disable_raw_mode()?;
    println!("\nQuitting...");

    result
}

#[cfg(not(feature = "native"))]
fn main() {
    println!("This example is only available with the 'native' feature enabled.");
}
