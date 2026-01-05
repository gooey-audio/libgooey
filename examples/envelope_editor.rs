/* CLI example demonstrating the envelope editor UI.
This example shows how to use the interactive envelope editor to experiment with
different ADSR envelope shapes and hear the results in real-time.
*/

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::io::{self, Write};

// Import the platform abstraction and audio engine
use libgooey::engine::{Engine, EngineOutput};
use libgooey::envelope::ADSRConfig;
use libgooey::instruments::KickDrum;
use libgooey::visualization::EnvelopeEditor;
use std::sync::{Arc, Mutex};
use std::thread;

// Example for envelope editor with audio feedback
#[cfg(all(feature = "native", feature = "visualization", feature = "crossterm"))]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;

    // Create the audio engine
    let mut engine = Engine::new(sample_rate);

    // Add a kick drum instrument for testing
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

    println!("=== Envelope Editor Example ===");
    println!("Drag the control points to adjust the envelope:");
    println!("  - First point: Attack time");
    println!("  - Second point: Decay time and sustain level");
    println!("  - Third point: Sustain duration (visual only)");
    println!("  - Fourth point: Release time");
    println!("");
    println!("Press SPACE in the terminal to trigger the kick drum");
    println!("Press 'q' in the terminal or ESC in the editor window to quit");
    println!("");

    // Get initial config
    let initial_config = ADSRConfig::default();

    // Create envelope editor in a separate thread
    let editor_config = Arc::new(Mutex::new(initial_config));
    let editor_config_clone = editor_config.clone();

    let editor_thread = thread::spawn(move || {
        let mut editor = EnvelopeEditor::new(800, 600, *editor_config_clone.lock().unwrap())
            .expect("Failed to create envelope editor");

        while !editor.should_close() {
            editor.process_events();
            editor.render();

            // Update the shared config
            let new_config = editor.get_config();
            *editor_config_clone.lock().unwrap() = new_config;

            // Small sleep to prevent busy waiting
            std::thread::sleep(std::time::Duration::from_millis(16)); // ~60 FPS
        }
    });

    // Enable raw mode for immediate key detection
    enable_raw_mode()?;

    // Main input loop
    let result = loop {
        // Poll for key events (non-blocking with timeout)
        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match code {
                    KeyCode::Char(' ') => {
                        io::stdout().flush().unwrap();

                        // Get current envelope config from editor
                        let config = *editor_config.lock().unwrap();

                        // Update the kick drum with the new envelope
                        let mut engine = audio_engine.lock().unwrap();

                        // For kick drum, we'll update its amplitude envelope
                        // Note: This is a simplified example. In a real scenario, you might
                        // want to expose more envelope controls on the instruments.
                        println!("\rTriggering kick with envelope: A={:.3}s D={:.3}s S={:.2} R={:.3}s",
                                 config.attack_time, config.decay_time, config.sustain_level, config.release_time);

                        engine.trigger_instrument("kick");
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

    // Wait for editor thread to finish
    let _ = editor_thread.join();

    result
}

#[cfg(not(all(feature = "native", feature = "visualization", feature = "crossterm")))]
fn main() {
    println!("This example requires the 'native', 'visualization', and 'crossterm' features.");
    println!("Run with: cargo run --example envelope_editor --features native,visualization,crossterm");
}
