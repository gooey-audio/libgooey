//! Tom2 Example - Minimal test for A/B comparison with Max/MSP patch
//!
//! Press SPACE to trigger the tom sound.
//! Press Q to quit.
//!
//! This implements the exact signal flow from the Max patch:
//! - Envelope: "1 1 0.8 0 2000 -0.83" (attack 1ms, decay 2000ms)
//! - Triangle oscillator at 327 Hz
//! - Pitch = 327 * envelope
//! - Amplitude = oscillator * envelope

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use gooey::engine::{Engine, EngineOutput, Instrument};
use gooey::instruments::Tom2;

// Wrapper to share Tom2 between audio thread and main thread
struct SharedTom2(Arc<Mutex<Tom2>>);

impl Instrument for SharedTom2 {
    fn trigger_with_velocity(&mut self, time: f32, velocity: f32) {
        self.0.lock().unwrap().trigger_with_velocity(time, velocity);
    }

    fn tick(&mut self, current_time: f32) -> f32 {
        self.0.lock().unwrap().tick(current_time)
    }

    fn is_active(&self) -> bool {
        self.0.lock().unwrap().is_active()
    }

    fn as_modulatable(&mut self) -> Option<&mut dyn gooey::engine::Modulatable> {
        None
    }
}

fn render_display(trigger_count: u32) {
    print!("\x1b[2J\x1b[H\x1b[?7l");
    print!("=== Tom2 - Max/MSP A/B Test ===\r\n");
    print!("\r\n");
    print!("Signal flow:\r\n");
    print!("  Envelope: 1ms attack (curve 0.8), 2000ms decay (curve -0.83)\r\n");
    print!("  Oscillator: Triangle @ 327 Hz\r\n");
    print!("  Pitch = 327 * envelope\r\n");
    print!("  Output = oscillator * envelope\r\n");
    print!("\r\n");
    print!("Controls:\r\n");
    print!("  SPACE = trigger tom\r\n");
    print!("  Q     = quit\r\n");
    print!("\r\n");
    print!("Triggers: {}\r\n", trigger_count);
    io::stdout().flush().unwrap();
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;

    // Create Tom2 instrument
    let tom2 = Arc::new(Mutex::new(Tom2::new(sample_rate)));

    // Create shared wrapper for the engine
    let shared_tom2 = SharedTom2(tom2.clone());

    // Create the audio engine
    let mut engine = Engine::new(sample_rate);
    engine.add_instrument("tom2", Box::new(shared_tom2));

    // Wrap engine in Arc<Mutex> for thread-safe access
    let audio_engine = Arc::new(Mutex::new(engine));

    // Enable raw mode for immediate key detection
    enable_raw_mode()?;

    // Create and configure the Engine output
    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;

    #[cfg(feature = "visualization")]
    engine_output.enable_visualization(1200, 400, 2.0)?;

    engine_output.create_stream_with_engine(audio_engine.clone())?;

    // Start the audio stream
    engine_output.start()?;

    // UI state
    let mut trigger_count = 0u32;

    // Initial display
    render_display(trigger_count);

    // Main input loop
    let result = loop {
        #[cfg(feature = "visualization")]
        if engine_output.update_visualization() {
            break Ok(());
        }

        // Poll for key events (non-blocking with short timeout)
        if event::poll(std::time::Duration::from_millis(16))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match code {
                    // Trigger tom2
                    KeyCode::Char(' ') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("tom2", 1.0);
                        trigger_count += 1;
                        render_display(trigger_count);
                    }

                    // Quit
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                        break Ok(());
                    }

                    _ => {}
                }
            }
        }
    };

    // Clean up terminal
    execute!(io::stdout(), Clear(ClearType::All), cursor::MoveTo(0, 0))?;
    disable_raw_mode()?;

    result
}

#[cfg(not(feature = "native"))]
fn main() {
    println!("This example requires the 'native' feature. Run with: cargo run --example tom2 --features native");
}
