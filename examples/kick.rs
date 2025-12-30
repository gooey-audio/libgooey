/* CLI example for kick drum testing.
Minimal code to start the audio engine and trigger kick drum hits.
*/

use std::io::{self, Write};

// Import the platform abstraction and audio engine
use libgooey::engine::{Engine, EngineOutput};
use libgooey::instruments::KickDrum;
use std::sync::{Arc, Mutex};

// CLI example for kick drum
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
    engine_output.create_stream_with_engine(audio_engine.clone())?;

    // Start the audio stream
    engine_output.start()?;

    println!("=== Kick Drum Example ===");
    println!("Press SPACE to trigger kick drum, 'q' to quit");

    // Main input loop
    loop {
        let mut input = String::new();
        io::stdout().flush().unwrap();
        io::stdin().read_line(&mut input).unwrap();

        match input.trim() {
            " " | "" => {
                println!("Triggering kick drum!");
                let mut engine = audio_engine.lock().unwrap();
                engine.trigger_instrument("kick");
            }
            "q" => {
                println!("Quitting...");
                break;
            }
            _ => {
                println!("Press SPACE to trigger kick drum, 'q' to quit");
            }
        }
    }

    Ok(())
}

#[cfg(not(feature = "native"))]
fn main() {
    println!("This example is only available with the 'native' feature enabled.");
}
