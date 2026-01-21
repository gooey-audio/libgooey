/* CLI example for kick drum testing.
Minimal code to start the audio engine and trigger kick drum hits.
Supports both keyboard (SPACE) and MIDI input (if available).
*/

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::io::{self, Write};

use gooey::engine::{Engine, EngineOutput};
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

fn get_preset_config(index: usize) -> (KickConfig, &'static str) {
    match index {
        0 => (KickConfig::default(), "Default"),
        1 => (KickConfig::punchy(), "Punchy"),
        2 => (KickConfig::deep(), "Deep"),
        3 => (KickConfig::tight(), "Tight"),
        4 => (KickConfig::ds_kick(), "DS Kick"),
        _ => (KickConfig::default(), "Default"),
    }
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;

    // Create the audio engine
    let mut engine = Engine::new(sample_rate);

    // Add a kick drum instrument
    let kick = KickDrum::new(sample_rate);
    engine.add_instrument("kick", Box::new(kick));

    // Wrap in Arc<Mutex> for thread-safe access
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

    // Current preset tracking
    let current_preset = Arc::new(Mutex::new(0usize)); // 0=default, 1=punchy, 2=deep, 3=tight, 4=ds_kick

    println!("=== Kick Drum Example ===");
    println!("Press SPACE to trigger kick drum, 'q' to quit");
    println!("Press 1-5 to switch presets:");
    println!("  1: Default  2: Punchy  3: Deep  4: Tight  5: DS Kick");
    println!();

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

    // Display current preset
    let (_, preset_name) = get_preset_config(0);
    println!("Current preset: {}", preset_name);
    println!();

    // Enable raw mode for immediate key detection
    enable_raw_mode()?;

    // Main input loop (works with or without visualization)
    let result = loop {
        // Update visualization if enabled (no-op if disabled)
        if engine_output.update_visualization() {
            println!("\rVisualization window closed");
            break Ok(());
        }

        // Poll for MIDI events (if available)
        #[cfg(feature = "midi")]
        if let Some(ref midi_handler) = midi {
            while let Ok((note, velocity)) = midi_handler.receiver.try_recv() {
                if note == KICK_NOTE || note == KICK_NOTE_ALT {
                    let mut engine = audio_engine.lock().unwrap();
                    // Convert MIDI velocity (0-127) to normalized (0.0-1.0)
                    let vel_normalized = velocity as f32 / 127.0;
                    // Queue trigger with velocity - Engine will apply correct time in tick()
                    engine.trigger_instrument_with_velocity("kick", vel_normalized);
                    print!("* (vel: {:.0}%) ", vel_normalized * 100.0);
                    io::stdout().flush().unwrap();
                }
            }
        }

        // Poll for key events (non-blocking with short timeout)
        if event::poll(std::time::Duration::from_millis(1))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match code {
                    KeyCode::Char(' ') => {
                        io::stdout().flush().unwrap();
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument("kick");
                        print!("*");
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Char('1') | KeyCode::Char('2') | KeyCode::Char('3')
                    | KeyCode::Char('4') | KeyCode::Char('5') => {
                        let preset_idx = match code {
                            KeyCode::Char('1') => 0,
                            KeyCode::Char('2') => 1,
                            KeyCode::Char('3') => 2,
                            KeyCode::Char('4') => 3,
                            KeyCode::Char('5') => 4,
                            _ => 0,
                        };

                        // Update preset
                        *current_preset.lock().unwrap() = preset_idx;
                        let (config, name) = get_preset_config(preset_idx);

                        // Replace kick drum with new config
                        let mut engine = audio_engine.lock().unwrap();
                        let new_kick = KickDrum::with_config(sample_rate, config);
                        engine.add_instrument("kick", Box::new(new_kick));

                        println!("\rPreset: {}                    ", name);
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Char('q') | KeyCode::Esc => {
                        println!("\rQuitting...           ");
                        break Ok(());
                    }
                    _ => {}
                }
            }
        }
    };

    // Restore terminal to normal mode
    disable_raw_mode()?;

    result
}

#[cfg(not(feature = "native"))]
fn main() {
    println!("This example is only available with the 'native' feature enabled.");
}
