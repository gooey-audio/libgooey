/* CLI example for snare drum testing.
Minimal code to start the audio engine and trigger snare drum hits.
Supports preset switching (keys 1-6) and keyboard triggering (SPACE).
*/

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::io::{self, Write};

// Import the platform abstraction and audio engine
use gooey::engine::{Engine, EngineOutput};
use gooey::instruments::{SnareConfig, SnareDrum};
use std::sync::{Arc, Mutex};

fn get_preset_config(index: usize) -> (SnareConfig, &'static str) {
    match index {
        0 => (SnareConfig::default(), "Default"),
        1 => (SnareConfig::crispy(), "Crispy"),
        2 => (SnareConfig::deep(), "Deep"),
        3 => (SnareConfig::tight(), "Tight"),
        4 => (SnareConfig::fat(), "Fat"),
        5 => (SnareConfig::ds_snare(), "DS Snare"),
        _ => (SnareConfig::default(), "Default"),
    }
}

// CLI example for snare drum
#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
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

    // Current preset tracking
    let current_preset = Arc::new(Mutex::new(0usize));

    println!("=== Snare Drum Example ===");
    println!("Press SPACE to trigger snare drum, 'q' to quit");
    println!("Press 1-6 to switch presets:");
    println!("  1: Default  2: Crispy  3: Deep  4: Tight  5: Fat  6: DS Snare");
    println!();

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
                    KeyCode::Char('1')
                    | KeyCode::Char('2')
                    | KeyCode::Char('3')
                    | KeyCode::Char('4')
                    | KeyCode::Char('5')
                    | KeyCode::Char('6') => {
                        let preset_idx = match code {
                            KeyCode::Char('1') => 0,
                            KeyCode::Char('2') => 1,
                            KeyCode::Char('3') => 2,
                            KeyCode::Char('4') => 3,
                            KeyCode::Char('5') => 4,
                            KeyCode::Char('6') => 5,
                            _ => 0,
                        };

                        // Update preset
                        *current_preset.lock().unwrap() = preset_idx;
                        let (config, name) = get_preset_config(preset_idx);

                        // Replace snare drum with new config
                        let mut engine = audio_engine.lock().unwrap();
                        let new_snare = SnareDrum::with_config(sample_rate, config);
                        engine.add_instrument("snare", Box::new(new_snare));

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
