/* CLI example for snare drum testing.
Minimal code to start the audio engine and trigger snare drum hits.
*/

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use log::info;
use std::io::{self, Write};

// Import the platform abstraction and audio engine
use gooey::engine::{Engine, EngineOutput};
use gooey::instruments::SnareDrum;
use std::sync::{Arc, Mutex};

// CLI example for snare drum
#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();
    let sample_rate = 44100.0;

    // Create the audio engine
    let mut engine = Engine::new(sample_rate);

    // Add a snare drum instrument
    let snare = SnareDrum::new(sample_rate);
    engine.add_instrument("snare", Box::new(snare));

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

    info!("=== Snare Drum Example ===");
    info!("Press SPACE to trigger snare drum, 'q' to quit");
    #[cfg(feature = "visualization")]
    info!("Waveform visualization enabled");
    info!("");

    // Enable raw mode for immediate key detection
    enable_raw_mode()?;

    // Main input loop (works with or without visualization)
    let result = loop {
        // Update visualization if enabled (no-op if disabled)
        if engine_output.update_visualization() {
            info!("Visualization window closed");
            break Ok(());
        }

        // Poll for key events (non-blocking with short timeout)
        if event::poll(std::time::Duration::from_millis(16))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match code {
                    KeyCode::Char(' ') => {
                        io::stdout().flush().unwrap();
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument("snare");
                        print!("*");
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Char('q') | KeyCode::Esc => {
                        info!("Quitting...");
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
