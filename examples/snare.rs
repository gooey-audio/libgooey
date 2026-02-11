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
use gooey::utils::PresetBlender;
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
// All parameters are now normalized 0-1
struct ParamInfo {
    name: &'static str,
    coarse_step: f32,
    fine_step: f32,
    display_min: f32, // For display scaling
    display_max: f32,
    unit: &'static str,
}

// Parameters in alphabetical order for display
// All parameters display normalized 0-1 values (matching the API)
const PARAM_INFO: [ParamInfo; 19] = [
    ParamInfo {
        name: "amp_decay",
        coarse_step: 0.05,
        fine_step: 0.01,
        display_min: 0.0,
        display_max: 1.0,
        unit: "",
    }, // 0-4.0s
    ParamInfo {
        name: "amp_decay_curve",
        coarse_step: 0.1,
        fine_step: 0.02,
        display_min: 0.0,
        display_max: 1.0,
        unit: "",
    }, // 0.1-10.0
    ParamInfo {
        name: "brightness",
        coarse_step: 0.1,
        fine_step: 0.02,
        display_min: 0.0,
        display_max: 1.0,
        unit: "",
    },
    ParamInfo {
        name: "decay",
        coarse_step: 0.05,
        fine_step: 0.01,
        display_min: 0.0,
        display_max: 1.0,
        unit: "",
    }, // 0.05-3.5s
    ParamInfo {
        name: "filter_cutoff",
        coarse_step: 0.05,
        fine_step: 0.01,
        display_min: 0.0,
        display_max: 1.0,
        unit: "",
    }, // 100-10000 Hz
    ParamInfo {
        name: "filter_resonance",
        coarse_step: 0.1,
        fine_step: 0.02,
        display_min: 0.0,
        display_max: 1.0,
        unit: "",
    }, // 0.5-10.0
    ParamInfo {
        name: "filter_type",
        coarse_step: 1.0,
        fine_step: 1.0,
        display_min: 0.0,
        display_max: 1.0,
        unit: "",
    }, // 0-3 discrete
    ParamInfo {
        name: "frequency",
        coarse_step: 0.05,
        fine_step: 0.01,
        display_min: 0.0,
        display_max: 1.0,
        unit: "",
    }, // 100-600 Hz
    ParamInfo {
        name: "noise",
        coarse_step: 0.1,
        fine_step: 0.02,
        display_min: 0.0,
        display_max: 1.0,
        unit: "",
    },
    ParamInfo {
        name: "noise_decay",
        coarse_step: 0.05,
        fine_step: 0.01,
        display_min: 0.0,
        display_max: 1.0,
        unit: "",
    }, // 0-3.5s
    ParamInfo {
        name: "noise_tail_decay",
        coarse_step: 0.05,
        fine_step: 0.01,
        display_min: 0.0,
        display_max: 1.0,
        unit: "",
    }, // 0-3.5s
    ParamInfo {
        name: "overdrive",
        coarse_step: 0.1,
        fine_step: 0.02,
        display_min: 0.0,
        display_max: 1.0,
        unit: "",
    },
    ParamInfo {
        name: "phase_mod_amount",
        coarse_step: 0.1,
        fine_step: 0.02,
        display_min: 0.0,
        display_max: 1.0,
        unit: "",
    },
    ParamInfo {
        name: "pitch_drop",
        coarse_step: 0.1,
        fine_step: 0.02,
        display_min: 0.0,
        display_max: 1.0,
        unit: "",
    },
    ParamInfo {
        name: "tonal",
        coarse_step: 0.1,
        fine_step: 0.02,
        display_min: 0.0,
        display_max: 1.0,
        unit: "",
    },
    ParamInfo {
        name: "tonal_decay",
        coarse_step: 0.05,
        fine_step: 0.01,
        display_min: 0.0,
        display_max: 1.0,
        unit: "",
    }, // 0-3.5s
    ParamInfo {
        name: "tonal_decay_curve",
        coarse_step: 0.1,
        fine_step: 0.02,
        display_min: 0.0,
        display_max: 1.0,
        unit: "",
    }, // 0.1-10.0
    ParamInfo {
        name: "volume",
        coarse_step: 0.1,
        fine_step: 0.02,
        display_min: 0.0,
        display_max: 1.0,
        unit: "",
    },
    ParamInfo {
        name: "xfade",
        coarse_step: 0.1,
        fine_step: 0.02,
        display_min: 0.0,
        display_max: 1.0,
        unit: "",
    },
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

// Blend mode state
struct BlendState {
    enabled: bool,
    x: f32,
    y: f32,
    blender: PresetBlender<SnareConfig>,
}

impl BlendState {
    fn new() -> Self {
        Self {
            enabled: false,
            x: 0.5,
            y: 0.5,
            blender: PresetBlender::new(
                SnareConfig::tight(), // Bottom-left (0,0)
                SnareConfig::loose(), // Bottom-right (1,0)
                SnareConfig::hiss(),  // Top-left (0,1)
                SnareConfig::smack(), // Top-right (1,1)
            ),
        }
    }
}

// Helper functions for parameter access
// All parameters are normalized 0-1
// Indices match alphabetical PARAM_INFO order
fn get_param_value(snare: &SnareDrum, index: usize) -> f32 {
    match index {
        0 => snare.params.amp_decay.get(),
        1 => snare.params.amp_decay_curve.get(),
        2 => snare.params.brightness.get(),
        3 => snare.params.decay.get(),
        4 => snare.params.filter_cutoff.get(),
        5 => snare.params.filter_resonance.get(),
        6 => snare.params.filter_type as f32 / 3.0, // Normalize to 0-1 for display
        7 => snare.params.frequency.get(),
        8 => snare.params.noise.get(),
        9 => snare.params.noise_decay.get(),
        10 => snare.params.noise_tail_decay.get(),
        11 => snare.params.overdrive.get(),
        12 => snare.params.phase_mod_amount.get(),
        13 => snare.params.pitch_drop.get(),
        14 => snare.params.tonal.get(),
        15 => snare.params.tonal_decay.get(),
        16 => snare.params.tonal_decay_curve.get(),
        17 => snare.params.volume.get(),
        18 => snare.params.xfade.get(),
        _ => 0.0,
    }
}

fn get_param_range(_snare: &SnareDrum, _index: usize) -> (f32, f32) {
    // All parameters are normalized 0-1
    (0.0, 1.0)
}

fn set_param_value(snare: &mut SnareDrum, index: usize, value: f32) {
    match index {
        0 => snare.set_amp_decay(value),
        1 => snare.set_amp_decay_curve(value),
        2 => snare.set_brightness(value),
        3 => snare.set_decay(value),
        4 => snare.set_filter_cutoff(value),
        5 => snare.set_filter_resonance(value),
        6 => snare.set_filter_type((value * 3.0).round() as u8),
        7 => snare.set_frequency(value),
        8 => snare.set_noise(value),
        9 => snare.set_noise_decay(value),
        10 => snare.set_noise_tail_decay(value),
        11 => snare.set_overdrive(value),
        12 => snare.set_phase_mod_amount(value),
        13 => snare.set_pitch_drop(value),
        14 => snare.set_tonal(value),
        15 => snare.set_tonal_decay(value),
        16 => snare.set_tonal_decay_curve(value),
        17 => snare.set_volume(value),
        18 => snare.set_xfade(value),
        _ => {}
    }
}

fn adjust_param(snare: &mut SnareDrum, index: usize, delta: f32) {
    let current = get_param_value(snare, index);
    let (min, max) = get_param_range(snare, index);

    // Special handling for filter_type (discrete parameter, now at index 6)
    let new_value = match index {
        6 => {
            // Filter type: discrete values 0, 1, 2, 3 mapped to 0-1
            let current_int = (current * 3.0).round() as i32;
            let delta_int = if delta > 0.0 { 1 } else { -1 };
            let new_int = (current_int + delta_int).clamp(0, 3);
            new_int as f32 / 3.0
        }
        _ => {
            // Normal continuous parameters (all normalized 0-1)
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
fn render_display(
    snare: &SnareDrum,
    selected: usize,
    trigger_count: u32,
    velocity: f32,
    preset_name: &str,
    blend: &BlendState,
) {
    // Clear screen, move cursor to home, and disable line wrapping
    print!("\x1b[2J\x1b[H\x1b[?7l");

    print!("=== Snare Drum Lab ===\r\n");
    if blend.enabled {
        print!("SPACE=hit Q=quit WASD=blend B=exit blend mode\r\n");
        print!("Z/X/C/V=vel 25/50/75/100%\r\n");
    } else {
        print!("SPACE=hit Q=quit ↑↓=sel ←→=adj []=fine B=blend\r\n");
        print!("Z/X/C/V=vel 25/50/75/100% +/-=adj 1-4=preset\r\n");
    }
    print!("Preset: {}\r\n", preset_name);

    // Show blend X/Y pad visualization when in blend mode
    if blend.enabled {
        print!("\r\n");
        print!("  X/Y Blend Pad (WASD to move)\r\n");
        print!("  ┌──────────────────────┐\r\n");

        // Render 8x8 pad with cursor
        let pad_w = 20;
        let pad_h = 8;
        let cursor_x = (blend.x * (pad_w - 1) as f32).round() as usize;
        let cursor_y = ((1.0 - blend.y) * (pad_h - 1) as f32).round() as usize;

        for row in 0..pad_h {
            print!("  │");
            for col in 0..pad_w {
                if row == cursor_y && col == cursor_x {
                    print!("●");
                } else {
                    // Show corner labels
                    if row == 0 && col == 0 {
                        print!("H"); // Hiss (top-left)
                    } else if row == 0 && col == pad_w - 1 {
                        print!("S"); // Smack (top-right)
                    } else if row == pad_h - 1 && col == 0 {
                        print!("T"); // Tight (bottom-left)
                    } else if row == pad_h - 1 && col == pad_w - 1 {
                        print!("L"); // Loose (bottom-right)
                    } else {
                        print!("·");
                    }
                }
            }
            print!("│\r\n");
        }
        print!("  └──────────────────────┘\r\n");
        print!("  X: {:.2}  Y: {:.2}\r\n", blend.x, blend.y);
    }

    print!("\r\n");

    for (i, info) in PARAM_INFO.iter().enumerate() {
        let value = get_param_value(snare, i);

        let indicator = if i == selected { ">" } else { " " };

        // Special formatting for filter_type
        if i == 12 {
            // Filter type - show name instead of number
            let filter_name = get_filter_type_name(snare.params.filter_type);
            print!(
                "{} {:<18} {:<14} {:>6}\r\n",
                indicator, info.name, "", filter_name
            );
        } else {
            // Normal parameters with bars - show scaled display value
            let display_value = info.display_min + value * (info.display_max - info.display_min);
            let bar = make_bar(value, 10);
            print!(
                "{} {:<18} [{}] {:>6.2}{:<3}\r\n",
                indicator, info.name, bar, display_value, info.unit
            );
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
    let mut current_preset = "Tight";
    let mut needs_redraw = true;
    let mut blend = BlendState::new();

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
            render_display(
                &s,
                selected_param,
                trigger_count,
                current_velocity,
                current_preset,
                &blend,
            );
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

                    // Presets (only when not in blend mode)
                    KeyCode::Char('1') if !blend.enabled => {
                        let mut s = snare.lock().unwrap();
                        s.set_config(SnareConfig::tight());
                        current_preset = "Tight";
                        needs_redraw = true;
                    }
                    KeyCode::Char('2') if !blend.enabled => {
                        let mut s = snare.lock().unwrap();
                        s.set_config(SnareConfig::loose());
                        current_preset = "Loose";
                        needs_redraw = true;
                    }
                    KeyCode::Char('3') if !blend.enabled => {
                        let mut s = snare.lock().unwrap();
                        s.set_config(SnareConfig::hiss());
                        current_preset = "Hiss";
                        needs_redraw = true;
                    }
                    KeyCode::Char('4') if !blend.enabled => {
                        let mut s = snare.lock().unwrap();
                        s.set_config(SnareConfig::smack());
                        current_preset = "Smack";
                        needs_redraw = true;
                    }

                    // Toggle blend mode
                    KeyCode::Char('b') | KeyCode::Char('B') => {
                        blend.enabled = !blend.enabled;
                        if blend.enabled {
                            // Apply current blend position
                            let blended = blend.blender.blend(blend.x, blend.y);
                            let mut s = snare.lock().unwrap();
                            s.set_config(blended);
                            current_preset = "Blended";
                        }
                        needs_redraw = true;
                    }

                    // Blend mode navigation (WASD)
                    KeyCode::Char('w') | KeyCode::Char('W') if blend.enabled => {
                        blend.y = (blend.y + 0.05).min(1.0);
                        let blended = blend.blender.blend(blend.x, blend.y);
                        let mut s = snare.lock().unwrap();
                        s.set_config(blended);
                        current_preset = "Blended";
                        needs_redraw = true;
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') if blend.enabled => {
                        blend.y = (blend.y - 0.05).max(0.0);
                        let blended = blend.blender.blend(blend.x, blend.y);
                        let mut s = snare.lock().unwrap();
                        s.set_config(blended);
                        current_preset = "Blended";
                        needs_redraw = true;
                    }
                    KeyCode::Char('a') | KeyCode::Char('A') if blend.enabled => {
                        blend.x = (blend.x - 0.05).max(0.0);
                        let blended = blend.blender.blend(blend.x, blend.y);
                        let mut s = snare.lock().unwrap();
                        s.set_config(blended);
                        current_preset = "Blended";
                        needs_redraw = true;
                    }
                    KeyCode::Char('d') | KeyCode::Char('D') if blend.enabled => {
                        blend.x = (blend.x + 0.05).min(1.0);
                        let blended = blend.blender.blend(blend.x, blend.y);
                        let mut s = snare.lock().unwrap();
                        s.set_config(blended);
                        current_preset = "Blended";
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
