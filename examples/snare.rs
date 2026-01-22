/* Snare Drum Lab - Interactive CLI for snare drum parameter experimentation.
Supports real-time parameter adjustment, presets, and velocity control.
Also supports MIDI input (if available).
*/

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use std::io::{self, Write};

use gooey::engine::{Engine, EngineOutput, Instrument};
use gooey::instruments::{SnareConfig, SnareDrum};
use std::sync::{Arc, Mutex};

#[cfg(feature = "midi")]
use midir::{MidiInput, MidiInputConnection};
#[cfg(feature = "midi")]
use std::sync::mpsc::{channel, Receiver};

// GM drum note numbers for snare
#[cfg(feature = "midi")]
const SNARE_NOTE: u8 = 38;
#[cfg(feature = "midi")]
const SNARE_NOTE_ALT: u8 = 40;

// Parameter metadata for the UI
struct ParamInfo {
    name: &'static str,
    coarse_step: f32,
    fine_step: f32,
    unit: &'static str,
}

const PARAM_INFO: [ParamInfo; 18] = [
    ParamInfo { name: "frequency", coarse_step: 10.0, fine_step: 2.0, unit: " Hz" },
    ParamInfo { name: "decay", coarse_step: 0.1, fine_step: 0.02, unit: " s" },
    ParamInfo { name: "brightness", coarse_step: 0.1, fine_step: 0.02, unit: "" },
    ParamInfo { name: "volume", coarse_step: 0.1, fine_step: 0.02, unit: "" },
    ParamInfo { name: "tonal", coarse_step: 0.1, fine_step: 0.02, unit: "" },
    ParamInfo { name: "noise", coarse_step: 0.1, fine_step: 0.02, unit: "" },
    ParamInfo { name: "pitch_drop", coarse_step: 0.1, fine_step: 0.02, unit: "" },
    ParamInfo { name: "tonal_decay", coarse_step: 0.05, fine_step: 0.01, unit: " s" },
    ParamInfo { name: "noise_decay", coarse_step: 0.1, fine_step: 0.02, unit: " s" },
    ParamInfo { name: "noise_tail_decay", coarse_step: 0.1, fine_step: 0.02, unit: " s" },
    ParamInfo { name: "noise_color", coarse_step: 50.0, fine_step: 10.0, unit: " Hz" },
    ParamInfo { name: "filter_cutoff", coarse_step: 500.0, fine_step: 100.0, unit: " Hz" },
    ParamInfo { name: "filter_resonance", coarse_step: 0.5, fine_step: 0.1, unit: "" },
    ParamInfo { name: "filter_type", coarse_step: 1.0, fine_step: 1.0, unit: "" },
    ParamInfo { name: "xfade", coarse_step: 0.1, fine_step: 0.02, unit: "" },
    ParamInfo { name: "click_enabled", coarse_step: 1.0, fine_step: 1.0, unit: "" },
    ParamInfo { name: "phase_mod_enabled", coarse_step: 1.0, fine_step: 1.0, unit: "" },
    ParamInfo { name: "phase_mod_amount", coarse_step: 0.1, fine_step: 0.02, unit: "" },
];

// Wrapper to share SnareDrum between audio thread and main thread
struct SharedSnare(Arc<Mutex<SnareDrum>>);

impl Instrument for SharedSnare {
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
fn get_param_value(snare: &SnareDrum, index: usize) -> f32 {
    match index {
        0 => snare.params.frequency.get(),
        1 => snare.params.decay.get(),
        2 => snare.params.brightness.get(),
        3 => snare.params.volume.get(),
        4 => snare.params.tonal.get(),
        5 => snare.params.noise.get(),
        6 => snare.params.pitch_drop.get(),
        7 => snare.params.tonal_decay.get(),
        8 => snare.params.noise_decay.get(),
        9 => snare.params.noise_tail_decay.get(),
        10 => snare.params.noise_color.get(),
        11 => snare.params.filter_cutoff.get(),
        12 => snare.params.filter_resonance.get(),
        13 => snare.params.filter_type as f32,
        14 => snare.params.xfade.get(),
        15 => if snare.params.click_enabled { 1.0 } else { 0.0 },
        16 => if snare.params.phase_mod_enabled { 1.0 } else { 0.0 },
        17 => snare.params.phase_mod_amount.get(),
        _ => 0.0,
    }
}

fn get_param_range(snare: &SnareDrum, index: usize) -> (f32, f32) {
    match index {
        0 => snare.params.frequency.range(),
        1 => snare.params.decay.range(),
        2 => snare.params.brightness.range(),
        3 => snare.params.volume.range(),
        4 => snare.params.tonal.range(),
        5 => snare.params.noise.range(),
        6 => snare.params.pitch_drop.range(),
        7 => snare.params.tonal_decay.range(),
        8 => snare.params.noise_decay.range(),
        9 => snare.params.noise_tail_decay.range(),
        10 => snare.params.noise_color.range(),
        11 => snare.params.filter_cutoff.range(),
        12 => snare.params.filter_resonance.range(),
        13 => (0.0, 3.0),
        14 => snare.params.xfade.range(),
        15 => (0.0, 1.0),
        16 => (0.0, 1.0),
        17 => snare.params.phase_mod_amount.range(),
        _ => (0.0, 1.0),
    }
}

fn set_param_value(snare: &mut SnareDrum, index: usize, value: f32) {
    match index {
        0 => snare.set_frequency(value),
        1 => snare.set_decay(value),
        2 => snare.set_brightness(value),
        3 => snare.set_volume(value),
        4 => snare.set_tonal(value),
        5 => snare.set_noise(value),
        6 => snare.set_pitch_drop(value),
        7 => snare.set_tonal_decay(value),
        8 => snare.set_noise_decay(value),
        9 => snare.set_noise_tail_decay(value),
        10 => snare.set_noise_color(value),
        11 => snare.set_filter_cutoff(value),
        12 => snare.set_filter_resonance(value),
        13 => snare.set_filter_type(value as u8),
        14 => snare.set_xfade(value),
        15 => snare.set_click_enabled(value > 0.5),
        16 => snare.set_phase_mod_enabled(value > 0.5),
        17 => snare.set_phase_mod_amount(value),
        _ => {}
    }
}

fn adjust_param(snare: &mut SnareDrum, index: usize, delta: f32) {
    let current = get_param_value(snare, index);
    let (min, max) = get_param_range(snare, index);

    // Special handling for boolean and discrete parameters
    let new_value = match index {
        13 => {
            // Filter type: discrete values 0, 1, 2, 3
            let current_int = current as i32;
            let delta_int = if delta > 0.0 { 1 } else { -1 };
            ((current_int + delta_int).max(0).min(3)) as f32
        }
        15 | 16 => {
            // Boolean toggles
            if current > 0.5 { 0.0 } else { 1.0 }
        }
        _ => {
            // Normal continuous parameters
            (current + delta).clamp(min, max)
        }
    };

    set_param_value(snare, index, new_value);
}

// Create a visual bar for normalized value
fn make_bar(normalized: f32, width: usize) -> String {
    let filled = (normalized * width as f32).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

// Get filter type name
fn get_filter_type_name(filter_type: u8) -> &'static str {
    match filter_type {
        0 => "LP",
        1 => "BP",
        2 => "HP",
        3 => "Notch",
        _ => "??",
    }
}

// Render the parameter display
fn render_display(snare: &SnareDrum, selected: usize, trigger_count: u32, velocity: f32, preset_name: &str) {
    // Clear screen, move cursor to home, and disable line wrapping
    print!("\x1b[2J\x1b[H\x1b[?7l");

    print!("=== Snare Drum Lab ===\r\n");
    print!("SPACE=hit Q=quit ↑↓=sel ←→=adj []=fine\r\n");
    print!("Z/X/C/V=vel 25/50/75/100% +/-=adj 1-6=preset\r\n");
    print!("Preset: {}\r\n", preset_name);
    print!("\r\n");

    for (i, info) in PARAM_INFO.iter().enumerate() {
        let value = get_param_value(snare, i);
        let (min, max) = get_param_range(snare, i);

        let indicator = if i == selected { ">" } else { " " };

        // Special formatting for certain parameters
        match i {
            13 => {
                // Filter type - show name instead of number
                let filter_name = get_filter_type_name(value as u8);
                print!("{} {:<18} {:<14} {:>6}\r\n", indicator, info.name, "", filter_name);
            }
            15 | 16 => {
                // Boolean parameters
                let state = if value > 0.5 { "ON " } else { "OFF" };
                print!("{} {:<18} {:<14} {:>6}\r\n", indicator, info.name, "", state);
            }
            _ => {
                // Normal parameters with bars
                let normalized = (value - min) / (max - min);
                let bar = make_bar(normalized, 10);
                print!(
                    "{} {:<18} [{}] {:>6.2}{:<3}\r\n",
                    indicator, info.name, bar, value, info.unit
                );
            }
        }
    }

    print!("\r\n");
    print!("Hits: {} | Vel: {:.0}%", trigger_count, velocity * 100.0);

    // Flush to ensure display updates
    io::stdout().flush().unwrap();
}

#[cfg(feature = "midi")]
struct MidiHandler {
    _connection: MidiInputConnection<()>,
    receiver: Receiver<(u8, u8)>, // (note, velocity)
}

#[cfg(feature = "midi")]
impl MidiHandler {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let midi_in = MidiInput::new("libgooey-snare")?;
        let ports = midi_in.ports();
        if ports.is_empty() {
            return Err("No MIDI input devices found".into());
        }

        let port = &ports[0];
        let port_name = midi_in.port_name(port)?;
        println!("Connecting to MIDI: {}", port_name);

        let (tx, rx) = channel();
        let connection = midi_in.connect(
            port,
            "snare-midi",
            move |_, msg, _| {
                // Note On with velocity > 0
                if msg.len() >= 3 && (msg[0] & 0xF0) == 0x90 && msg[2] > 0 {
                    let _ = tx.send((msg[1], msg[2]));
                }
            },
            (),
        )?;

        Ok(Self {
            _connection: connection,
            receiver: rx,
        })
    }

    fn list_ports() -> Vec<String> {
        MidiInput::new("list")
            .map(|m| {
                m.ports()
                    .iter()
                    .filter_map(|p| m.port_name(p).ok())
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;

    // Create shared snare drum that both audio thread and main thread can access
    let snare = Arc::new(Mutex::new(SnareDrum::new(sample_rate)));

    // Create the audio engine
    let mut engine = Engine::new(sample_rate);

    // Add the shared snare drum wrapper to the engine
    engine.add_instrument("snare", Box::new(SharedSnare(snare.clone())));

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

    // Try to initialize MIDI input (optional, fails gracefully)
    #[cfg(feature = "midi")]
    let midi = {
        println!("Available MIDI ports: {:?}", MidiHandler::list_ports());
        match MidiHandler::new() {
            Ok(handler) => {
                println!(
                    "MIDI connected! Hit drum pad (note {} or {}).",
                    SNARE_NOTE, SNARE_NOTE_ALT
                );
                Some(handler)
            }
            Err(e) => {
                println!("No MIDI device: {} (keyboard only)", e);
                None
            }
        }
    };

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
            let s = snare.lock().unwrap();
            render_display(&s, selected_param, trigger_count, current_velocity, current_preset);
            needs_redraw = false;
        }

        // Poll for MIDI events (if available)
        #[cfg(feature = "midi")]
        if let Some(ref midi_handler) = midi {
            while let Ok((note, velocity)) = midi_handler.receiver.try_recv() {
                if note == SNARE_NOTE || note == SNARE_NOTE_ALT {
                    let mut engine = audio_engine.lock().unwrap();
                    // Convert MIDI velocity (0-127) to normalized (0.0-1.0)
                    let vel_normalized = velocity as f32 / 127.0;
                    engine.trigger_instrument_with_velocity("snare", vel_normalized);
                    trigger_count += 1;
                    current_velocity = vel_normalized;
                    needs_redraw = true;
                }
            }
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
                        let mut s = snare.lock().unwrap();
                        adjust_param(&mut s, selected_param, -step);
                        current_preset = "Custom";
                        needs_redraw = true;
                    }
                    KeyCode::Right => {
                        let step = PARAM_INFO[selected_param].coarse_step;
                        let mut s = snare.lock().unwrap();
                        adjust_param(&mut s, selected_param, step);
                        current_preset = "Custom";
                        needs_redraw = true;
                    }

                    // Fine adjustment
                    KeyCode::Char('[') => {
                        let step = PARAM_INFO[selected_param].fine_step;
                        let mut s = snare.lock().unwrap();
                        adjust_param(&mut s, selected_param, -step);
                        current_preset = "Custom";
                        needs_redraw = true;
                    }
                    KeyCode::Char(']') => {
                        let step = PARAM_INFO[selected_param].fine_step;
                        let mut s = snare.lock().unwrap();
                        adjust_param(&mut s, selected_param, step);
                        current_preset = "Custom";
                        needs_redraw = true;
                    }

                    // Presets
                    KeyCode::Char('1') => {
                        let mut s = snare.lock().unwrap();
                        s.set_config(SnareConfig::default());
                        current_preset = "Default";
                        needs_redraw = true;
                    }
                    KeyCode::Char('2') => {
                        let mut s = snare.lock().unwrap();
                        s.set_config(SnareConfig::crispy());
                        current_preset = "Crispy";
                        needs_redraw = true;
                    }
                    KeyCode::Char('3') => {
                        let mut s = snare.lock().unwrap();
                        s.set_config(SnareConfig::deep());
                        current_preset = "Deep";
                        needs_redraw = true;
                    }
                    KeyCode::Char('4') => {
                        let mut s = snare.lock().unwrap();
                        s.set_config(SnareConfig::tight());
                        current_preset = "Tight";
                        needs_redraw = true;
                    }
                    KeyCode::Char('5') => {
                        let mut s = snare.lock().unwrap();
                        s.set_config(SnareConfig::fat());
                        current_preset = "Fat";
                        needs_redraw = true;
                    }
                    KeyCode::Char('6') => {
                        let mut s = snare.lock().unwrap();
                        s.set_config(SnareConfig::ds_snare());
                        current_preset = "DS Snare";
                        needs_redraw = true;
                    }

                    // Trigger at current velocity
                    KeyCode::Char(' ') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("snare", current_velocity);
                        trigger_count += 1;
                        needs_redraw = true;
                    }

                    // Velocity-specific triggers
                    KeyCode::Char('z') | KeyCode::Char('Z') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("snare", 0.25);
                        trigger_count += 1;
                        current_velocity = 0.25;
                        needs_redraw = true;
                    }
                    KeyCode::Char('x') | KeyCode::Char('X') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("snare", 0.50);
                        trigger_count += 1;
                        current_velocity = 0.50;
                        needs_redraw = true;
                    }
                    KeyCode::Char('c') | KeyCode::Char('C') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("snare", 0.75);
                        trigger_count += 1;
                        current_velocity = 0.75;
                        needs_redraw = true;
                    }
                    KeyCode::Char('v') | KeyCode::Char('V') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("snare", 1.0);
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
