/* This example expose parameter to pass generator of sample.
Good starting point for integration of cpal into your application.
*/

use std::io::{self, Write};

// Import the platform abstraction and audio engine
use libgooey::gen::oscillator::Oscillator;
use libgooey::platform::{AudioEngine, AudioOutput, CpalOutput};

// Native binary entry point for the oscillator engine

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    // Create the audio engine
    let audio_engine = AudioEngine::new(44100.0);

    // Configure the stage with an oscillator
    audio_engine.with_stage(|stage| {
        let mut oscillator1 = Oscillator::new(44100.0, 200.0);
        oscillator1.waveform = libgooey::gen::waveform::Waveform::Square;
        stage.add(oscillator1);

        // let mut oscillator2 = Oscillator::new(44100.0, 625.0);
        // oscillator2.waveform = libgooey::gen::waveform::Waveform::Triangle;
        // stage.add(oscillator2);
    });

    // Create and configure the CPAL output
    let mut cpal_output = CpalOutput::new();
    cpal_output.initialize(44100.0)?;
    cpal_output.create_stream_with_stage(audio_engine.stage(), audio_engine.audio_state())?;

    // Start the audio stream
    cpal_output.start()?;

    println!("Press '1' to trigger oscillator, '2' to trigger kick, 'q' to quit");

    // Main input loop
    loop {
        let mut input = String::new();
        io::stdout().flush().unwrap();
        io::stdin().read_line(&mut input).unwrap();

        match input.trim() {
            "1" => {
                println!("Triggering oscillator!");
                audio_engine.trigger_all();
            }
            "2" => {
                println!("Triggering kick!");
                let mut stage = audio_engine.stage_mut();
                stage.trigger_kick();
                // audio_engine.trigger_kick();
            }
            "q" => {
                println!("Quitting...");
                break;
            }
            _ => {
                println!("Press '1' to trigger oscillator, '2' to trigger kick, 'q' to quit");
            }
        }
    }

    Ok(())
}

#[cfg(not(feature = "native"))]
fn main() {
    println!("This binary is only available with the 'native' feature enabled.");
}
