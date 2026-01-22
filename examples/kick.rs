/* Kick Drum Lab - Interactive CLI for kick drum parameter experimentation.
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
use gooey::instruments::{KickConfig, KickDrum};
use std::sync::{Arc, Mutex};

#[cfg(feature = "midi")]
use midir::{MidiInput, MidiInputConnection};
#[cfg(feature = "midi")]
use std::sync::mpsc::{channel, Receiver};

// GM drum note numbers for kick
#[cfg(feature = "midi")]
const KICK_NOTE: u8 = 36;
#[cfg(feature = "midi")]
const KICK_NOTE_ALT: u8 = 35;

// Parameter metadata for the UI
struct ParamInfo {
    name: &'static str,
    coarse_step: f32,
    fine_step: f32,
    unit: &'static str,
}

const PARAM_INFO: [ParamInfo; 19] = [
    ParamInfo { name: "frequency", coarse_step: 5.0, fine_step: 1.0, unit: " Hz" },
    ParamInfo { name: "punch", coarse_step: 0.1, fine_step: 0.02, unit: "" },
    ParamInfo { name: "sub", coarse_step: 0.1, fine_step: 0.02, unit: "" },
    ParamInfo { name: "click", coarse_step: 0.1, fine_step: 0.02, unit: "" },
    ParamInfo { name: "snap", coarse_step: 0.1, fine_step: 0.02, unit: "" },
    ParamInfo { name: "decay", coarse_step: 0.1, fine_step: 0.02, unit: " s" },
    ParamInfo { name: "pitch_envelope", coarse_step: 0.1, fine_step: 0.02, unit: "" },
    ParamInfo { name: "pitch_curve", coarse_step: 0.5, fine_step: 0.1, unit: "" },
    ParamInfo { name: "volume", coarse_step: 0.1, fine_step: 0.02, unit: "" },
    ParamInfo { name: "pitch_start_ratio", coarse_step: 0.5, fine_step: 0.1, unit: "" },
    ParamInfo { name: "phase_mod_amount", coarse_step: 0.1, fine_step: 0.02, unit: "" },
    ParamInfo { name: "noise_amount", coarse_step: 0.1, fine_step: 0.02, unit: "" },
    ParamInfo { name: "noise_cutoff", coarse_step: 100.0, fine_step: 20.0, unit: " Hz" },
    ParamInfo { name: "noise_resonance", coarse_step: 0.5, fine_step: 0.1, unit: "" },
    ParamInfo { name: "overdrive", coarse_step: 0.1, fine_step: 0.02, unit: "" },
    ParamInfo { name: "amp_attack", coarse_step: 0.01, fine_step: 0.001, unit: " s" },
    ParamInfo { name: "amp_decay", coarse_step: 0.1, fine_step: 0.02, unit: " s" },
    ParamInfo { name: "amp_attack_curve", coarse_step: 0.5, fine_step: 0.1, unit: "" },
    ParamInfo { name: "amp_decay_curve", coarse_step: 0.5, fine_step: 0.1, unit: "" },
];

// Wrapper to share KickDrum between audio thread and main thread
struct SharedKick(Arc<Mutex<KickDrum>>);

impl Instrument for SharedKick {
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
fn get_param_value(kick: &KickDrum, index: usize) -> f32 {
    match index {
        0 => kick.params.frequency.get(),
        1 => kick.params.punch.get(),
        2 => kick.params.sub.get(),
        3 => kick.params.click.get(),
        4 => kick.params.snap.get(),
        5 => kick.params.decay.get(),
        6 => kick.params.pitch_envelope.get(),
        7 => kick.params.pitch_curve.get(),
        8 => kick.params.volume.get(),
        9 => kick.params.pitch_start_ratio.get(),
        10 => kick.params.phase_mod_amount.get(),
        11 => kick.params.noise_amount.get(),
        12 => kick.params.noise_cutoff.get(),
        13 => kick.params.noise_resonance.get(),
        14 => kick.params.overdrive.get(),
        15 => kick.params.amp_attack.get(),
        16 => kick.params.amp_decay.get(),
        17 => kick.params.amp_attack_curve.get(),
        18 => kick.params.amp_decay_curve.get(),
        _ => 0.0,
    }
}

fn get_param_range(kick: &KickDrum, index: usize) -> (f32, f32) {
    match index {
        0 => kick.params.frequency.range(),
        1 => kick.params.punch.range(),
        2 => kick.params.sub.range(),
        3 => kick.params.click.range(),
        4 => kick.params.snap.range(),
        5 => kick.params.decay.range(),
        6 => kick.params.pitch_envelope.range(),
        7 => kick.params.pitch_curve.range(),
        8 => kick.params.volume.range(),
        9 => kick.params.pitch_start_ratio.range(),
        10 => kick.params.phase_mod_amount.range(),
        11 => kick.params.noise_amount.range(),
        12 => kick.params.noise_cutoff.range(),
        13 => kick.params.noise_resonance.range(),
        14 => kick.params.overdrive.range(),
        15 => kick.params.amp_attack.range(),
        16 => kick.params.amp_decay.range(),
        17 => kick.params.amp_attack_curve.range(),
        18 => kick.params.amp_decay_curve.range(),
        _ => (0.0, 1.0),
    }
}

fn set_param_value(kick: &mut KickDrum, index: usize, value: f32) {
    match index {
        0 => kick.set_frequency(value),
        1 => kick.set_punch(value),
        2 => kick.set_sub(value),
        3 => kick.set_click(value),
        4 => kick.set_snap(value),
        5 => kick.set_decay(value),
        6 => kick.set_pitch_envelope(value),
        7 => kick.set_pitch_curve(value),
        8 => kick.set_volume(value),
        9 => kick.set_pitch_start_ratio(value),
        10 => kick.set_phase_mod_amount(value),
        11 => kick.set_noise_amount(value),
        12 => kick.set_noise_cutoff(value),
        13 => kick.set_noise_resonance(value),
        14 => kick.set_overdrive(value),
        15 => kick.set_amp_attack(value),
        16 => kick.set_amp_decay(value),
        17 => kick.set_amp_attack_curve(value),
        18 => kick.set_amp_decay_curve(value),
        _ => {}
    }
}

fn adjust_param(kick: &mut KickDrum, index: usize, delta: f32) {
    let current = get_param_value(kick, index);
    let (min, max) = get_param_range(kick, index);
    let new_value = (current + delta).clamp(min, max);
    set_param_value(kick, index, new_value);
}

// Create a visual bar for normalized value
fn make_bar(normalized: f32, width: usize) -> String {
    let filled = (normalized * width as f32).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

// Render the parameter display
fn render_display(kick: &KickDrum, selected: usize, trigger_count: u32, velocity: f32, preset_name: &str, phase_mod_enabled: bool) {
    // Clear screen, move cursor to home, and disable line wrapping
    print!("\x1b[2J\x1b[H\x1b[?7l");

    print!("=== Kick Drum Lab ===\r\n");
    print!("SPACE=hit Q=quit ↑↓=sel ←→=adj []=fine P=phase {}\r\n", if phase_mod_enabled { "ON " } else { "OFF" });
    print!("Z/X/C/V=vel 25/50/75/100% +/-=adj 1-5=preset\r\n");
    print!("Preset: {}\r\n", preset_name);
    print!("\r\n");

    for (i, info) in PARAM_INFO.iter().enumerate() {
        let value = get_param_value(kick, i);
        let (min, max) = get_param_range(kick, i);
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

#[cfg(feature = "midi")]
struct MidiHandler {
    _connection: MidiInputConnection<()>,
    receiver: Receiver<(u8, u8)>, // (note, velocity)
}

#[cfg(feature = "midi")]
impl MidiHandler {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let midi_in = MidiInput::new("libgooey-kick")?;
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
            "kick-midi",
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

    // Create shared kick drum that both audio thread and main thread can access
    let kick = Arc::new(Mutex::new(KickDrum::new(sample_rate)));

    // Create the audio engine
    let mut engine = Engine::new(sample_rate);

    // Add the shared kick drum wrapper to the engine
    engine.add_instrument("kick", Box::new(SharedKick(kick.clone())));

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
                    KICK_NOTE, KICK_NOTE_ALT
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
    let mut phase_mod_enabled = false;
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
            let k = kick.lock().unwrap();
            render_display(&k, selected_param, trigger_count, current_velocity, current_preset, phase_mod_enabled);
            needs_redraw = false;
        }

        // Poll for MIDI events (if available)
        #[cfg(feature = "midi")]
        if let Some(ref midi_handler) = midi {
            while let Ok((note, velocity)) = midi_handler.receiver.try_recv() {
                if note == KICK_NOTE || note == KICK_NOTE_ALT {
                    let mut engine = audio_engine.lock().unwrap();
                    // Convert MIDI velocity (0-127) to normalized (0.0-1.0)
                    let vel_normalized = velocity as f32 / 127.0;
                    engine.trigger_instrument_with_velocity("kick", vel_normalized);
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
                        let mut k = kick.lock().unwrap();
                        adjust_param(&mut k, selected_param, -step);
                        current_preset = "Custom";
                        needs_redraw = true;
                    }
                    KeyCode::Right => {
                        let step = PARAM_INFO[selected_param].coarse_step;
                        let mut k = kick.lock().unwrap();
                        adjust_param(&mut k, selected_param, step);
                        current_preset = "Custom";
                        needs_redraw = true;
                    }

                    // Fine adjustment
                    KeyCode::Char('[') => {
                        let step = PARAM_INFO[selected_param].fine_step;
                        let mut k = kick.lock().unwrap();
                        adjust_param(&mut k, selected_param, -step);
                        current_preset = "Custom";
                        needs_redraw = true;
                    }
                    KeyCode::Char(']') => {
                        let step = PARAM_INFO[selected_param].fine_step;
                        let mut k = kick.lock().unwrap();
                        adjust_param(&mut k, selected_param, step);
                        current_preset = "Custom";
                        needs_redraw = true;
                    }

                    // Toggle phase modulation
                    KeyCode::Char('p') | KeyCode::Char('P') => {
                        phase_mod_enabled = !phase_mod_enabled;
                        let mut k = kick.lock().unwrap();
                        k.set_phase_mod_enabled(phase_mod_enabled);
                        needs_redraw = true;
                    }

                    // Presets
                    KeyCode::Char('1') => {
                        let mut k = kick.lock().unwrap();
                        k.set_config(KickConfig::default());
                        current_preset = "Default";
                        phase_mod_enabled = false;
                        needs_redraw = true;
                    }
                    KeyCode::Char('2') => {
                        let mut k = kick.lock().unwrap();
                        k.set_config(KickConfig::punchy());
                        current_preset = "Punchy";
                        phase_mod_enabled = false;
                        needs_redraw = true;
                    }
                    KeyCode::Char('3') => {
                        let mut k = kick.lock().unwrap();
                        k.set_config(KickConfig::deep());
                        current_preset = "Deep";
                        phase_mod_enabled = false;
                        needs_redraw = true;
                    }
                    KeyCode::Char('4') => {
                        let mut k = kick.lock().unwrap();
                        k.set_config(KickConfig::tight());
                        current_preset = "Tight";
                        phase_mod_enabled = false;
                        needs_redraw = true;
                    }
                    KeyCode::Char('5') => {
                        let mut k = kick.lock().unwrap();
                        k.set_config(KickConfig::ds_kick());
                        current_preset = "DS Kick";
                        phase_mod_enabled = true; // DS Kick uses phase mod
                        needs_redraw = true;
                    }

                    // Trigger at current velocity
                    KeyCode::Char(' ') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("kick", current_velocity);
                        trigger_count += 1;
                        needs_redraw = true;
                    }

                    // Velocity-specific triggers
                    KeyCode::Char('z') | KeyCode::Char('Z') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("kick", 0.25);
                        trigger_count += 1;
                        current_velocity = 0.25;
                        needs_redraw = true;
                    }
                    KeyCode::Char('x') | KeyCode::Char('X') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("kick", 0.50);
                        trigger_count += 1;
                        current_velocity = 0.50;
                        needs_redraw = true;
                    }
                    KeyCode::Char('c') | KeyCode::Char('C') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("kick", 0.75);
                        trigger_count += 1;
                        current_velocity = 0.75;
                        needs_redraw = true;
                    }
                    KeyCode::Char('v') | KeyCode::Char('V') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("kick", 1.0);
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
