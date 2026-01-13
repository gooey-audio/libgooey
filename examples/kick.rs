/* CLI example for kick drum testing.
Minimal code to start the audio engine and trigger kick drum hits.
Supports both keyboard (SPACE) and MIDI input (if available).
*/

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::io::{self, Write};

use libgooey::engine::{Engine, EngineOutput};
use libgooey::instruments::KickDrum;
use std::sync::{Arc, Mutex};

#[cfg(feature = "midi")]
mod midi_input;

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

    println!("=== Kick Drum Example ===");
    println!("Press SPACE to trigger kick drum, 'q' to quit");

    // Try to initialize MIDI input (optional, fails gracefully)
    #[cfg(feature = "midi")]
    let midi = {
        println!("Available MIDI ports: {:?}", midi_input::MidiHandler::list_ports());
        match midi_input::MidiHandler::new() {
            Ok(handler) => {
                println!(
                    "MIDI connected! Hit drum pad (note {} or {}) to trigger.",
                    midi_input::drum_notes::KICK,
                    midi_input::drum_notes::KICK_ALT
                );
                Some(handler)
            }
            Err(e) => {
                println!("No MIDI device found: {} (using keyboard only)", e);
                None
            }
        }
    };

    #[cfg(feature = "visualization")]
    println!("Waveform visualization enabled");
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
            for event in midi_handler.poll_all() {
                if let midi_input::MidiDrumEvent::NoteOn { note, velocity } = event {
                    if note == midi_input::drum_notes::KICK
                        || note == midi_input::drum_notes::KICK_ALT
                    {
                        let velocity_float = midi_input::velocity_to_float(velocity);
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument("kick");
                        print!("* (vel: {:.0}%) ", velocity_float * 100.0);
                        io::stdout().flush().unwrap();
                    }
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
