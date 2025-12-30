/* CLI example for hi-hat testing.
Minimal code to start the audio engine and trigger hi-hat hits.
*/

use std::io::{self, Write};

// Import the platform abstraction and audio engine
use libgooey::platform::{AudioEngine, AudioOutput, CpalOutput};

// CLI example for hi-hat
#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    // Create the audio engine
    let audio_engine = AudioEngine::new(44100.0);

    // Create and configure the CPAL output
    let mut cpal_output = CpalOutput::new();
    cpal_output.initialize(44100.0)?;
    cpal_output.create_stream_with_stage(audio_engine.stage(), audio_engine.audio_state())?;

    // Start the audio stream
    cpal_output.start()?;

    println!("=== Hi-Hat Example ===");
    println!("Press SPACE to trigger hi-hat, 'q' to quit");

    // Main input loop
    loop {
        let mut input = String::new();
        io::stdout().flush().unwrap();
        io::stdin().read_line(&mut input).unwrap();

        match input.trim() {
            " " | "" => {
                println!("Triggering hi-hat!");
                let mut stage = audio_engine.stage_mut();
                stage.trigger_hihat();
            }
            "q" => {
                println!("Quitting...");
                break;
            }
            _ => {
                println!("Press SPACE to trigger hi-hat, 'q' to quit");
            }
        }
    }

    Ok(())
}

#[cfg(not(feature = "native"))]
fn main() {
    println!("This example is only available with the 'native' feature enabled.");
}