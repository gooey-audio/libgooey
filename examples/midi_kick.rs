/* CLI example for kick drum testing with MIDI input.
   Connect a USB MIDI drum pad to trigger kick drum hits with velocity.

   Run with: cargo run --example midi_kick --features "native,midi"
*/

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use libgooey::engine::{Engine, EngineOutput};
use libgooey::instruments::KickDrum;

mod midi_input;
use midi_input::{drum_notes, velocity_to_float, MidiDrumEvent, MidiHandler};

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

    // Enable visualization (optional)
    #[cfg(feature = "visualization")]
    engine_output.enable_visualization(1200, 400, 2.0)?;

    engine_output.create_stream_with_engine(audio_engine.clone())?;

    // Start the audio stream
    engine_output.start()?;

    // Initialize MIDI input
    println!("=== MIDI Kick Drum Example ===");
    println!("Available MIDI ports: {:?}", MidiHandler::list_ports());

    let midi = match MidiHandler::new() {
        Ok(handler) => {
            println!(
                "MIDI connected! Hit your drum pad (MIDI note {} or {}).",
                drum_notes::KICK,
                drum_notes::KICK_ALT
            );
            Some(handler)
        }
        Err(e) => {
            println!("No MIDI device found: {}", e);
            println!("Connect a MIDI device and restart.");
            None
        }
    };

    println!("Press Ctrl+C to quit");
    println!();

    // Main input loop
    loop {
        // Update visualization if enabled
        if engine_output.update_visualization() {
            println!("\rVisualization window closed");
            break;
        }

        // Poll for MIDI events
        if let Some(ref midi_handler) = midi {
            for event in midi_handler.poll_all() {
                match event {
                    MidiDrumEvent::NoteOn { note, velocity } => {
                        // Check if it's a kick drum note
                        if note == drum_notes::KICK || note == drum_notes::KICK_ALT {
                            let velocity_float = velocity_to_float(velocity);

                            let mut engine = audio_engine.lock().unwrap();
                            engine.trigger_instrument("kick");

                            // Print velocity for testing (will be used when velocity support is added)
                            print!("KICK (vel: {:.0}%) ", velocity_float * 100.0);
                            io::stdout().flush().unwrap();
                        } else {
                            // Print other notes for debugging MIDI mapping
                            print!("[note {}] ", note);
                            io::stdout().flush().unwrap();
                        }
                    }
                    MidiDrumEvent::NoteOff { .. } => {
                        // Drums don't typically use note-off
                    }
                }
            }
        }

        // Small sleep to prevent busy-waiting
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    Ok(())
}

#[cfg(not(feature = "native"))]
fn main() {
    println!("This example requires the 'native' and 'midi' features.");
    println!("Run with: cargo run --example midi_kick --features \"native,midi\"");
}
