/* CLI example for sequencer testing.
Demonstrates sample-accurate sequencing with the new Engine.
*/

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

// Import the engine and instruments
use libgooey::engine::{Engine, EngineOutput, Sequencer};
use libgooey::instruments::HiHat;

// CLI example for sequencer
#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;

    // Create the audio engine
    let mut engine = Engine::new(sample_rate);

    // Add a hi-hat instrument
    let hihat = HiHat::new(sample_rate);
    engine.add_instrument("hihat", Box::new(hihat));

    // Create a sequencer with a simple 8-step pattern (16th notes at 120 BPM)
    let pattern = vec![true, false, true, false, true, false, true, false];
    let sequencer = Sequencer::with_pattern(120.0, sample_rate, pattern, "hihat");
    engine.add_sequencer(sequencer);

    // Wrap in Arc<Mutex> for thread-safe access
    let audio_engine = Arc::new(Mutex::new(engine));

    // Create and configure the Engine output
    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;
    engine_output.create_stream_with_engine(audio_engine.clone())?;

    // Start the audio stream
    engine_output.start()?;

    println!("=== Sequencer Example ===");
    println!("Press SPACE to start/stop sequencer");
    println!("Press UP/DOWN to adjust BPM");
    println!("Press 'q' to quit");
    println!("");

    // Enable raw mode for immediate key detection
    enable_raw_mode()?;

    // Main input loop
    let result = loop {
        // Poll for key events (non-blocking with timeout)
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match code {
                    KeyCode::Char(' ') => {
                        io::stdout().flush().unwrap();
                        let mut engine = audio_engine.lock().unwrap();
                        
                        // Toggle the first sequencer
                        if let Some(seq) = engine.sequencer_mut(0) {
                            if seq.is_running() {
                                seq.stop();
                                println!("\rSequencer stopped          ");
                            } else {
                                seq.start();
                                println!("\rSequencer started at {} BPM", seq.bpm());
                            }
                        }
                    }
                    KeyCode::Up => {
                        let mut engine = audio_engine.lock().unwrap();
                        if let Some(seq) = engine.sequencer_mut(0) {
                            let new_bpm = (seq.bpm() + 5.0).min(200.0);
                            seq.set_bpm(new_bpm);
                            println!("\rBPM: {}  ", new_bpm);
                        }
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Down => {
                        let mut engine = audio_engine.lock().unwrap();
                        if let Some(seq) = engine.sequencer_mut(0) {
                            let new_bpm = (seq.bpm() - 5.0).max(60.0);
                            seq.set_bpm(new_bpm);
                            println!("\rBPM: {}  ", new_bpm);
                        }
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
