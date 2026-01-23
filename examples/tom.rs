/* CLI example for tom drum testing.
Minimal code to start the audio engine and trigger tom drum hits.
Supports preset switching with number keys.
*/

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::io::{self, Write};

// Import the platform abstraction and audio engine
use gooey::engine::{Engine, EngineOutput, Instrument};
use gooey::instruments::{TomConfig, TomDrum};
use std::sync::{Arc, Mutex};

// Wrapper to share TomDrum between audio thread and main thread
struct SharedTom(Arc<Mutex<TomDrum>>);

impl Instrument for SharedTom {
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

// CLI example for tom drum
#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;

    // Create shared tom drum
    let tom = Arc::new(Mutex::new(TomDrum::new(sample_rate)));

    // Create the audio engine
    let mut engine = Engine::new(sample_rate);

    // Add tom drum instrument
    engine.add_instrument("tom", Box::new(SharedTom(tom.clone())));

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

    // UI state
    let mut current_preset = "Mid Tom";
    let mut trigger_count: u32 = 0;
    let mut current_velocity: f32 = 0.75;

    println!("=== Tom Drum Example ===");
    println!("Press SPACE to trigger tom drum");
    println!("Z/X/C/V = velocity 25/50/75/100%");
    println!("");
    println!("Presets:");
    println!("  1 = DS Tom (Max patch style)");
    println!("  2 = High Tom");
    println!("  3 = Mid Tom (default)");
    println!("  4 = Low Tom");
    println!("  5 = Floor Tom");
    println!("");
    println!("Q = quit");
    #[cfg(feature = "visualization")]
    println!("Waveform visualization enabled");
    println!("");
    println!("Current: {} | Hits: 0 | Vel: 75%", current_preset);

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
                    // Trigger at current velocity
                    KeyCode::Char(' ') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("tom", current_velocity);
                        trigger_count += 1;
                        print!("\rCurrent: {} | Hits: {} | Vel: {:.0}%    ",
                               current_preset, trigger_count, current_velocity * 100.0);
                        io::stdout().flush().unwrap();
                    }

                    // Velocity-specific triggers
                    KeyCode::Char('z') | KeyCode::Char('Z') => {
                        current_velocity = 0.25;
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("tom", current_velocity);
                        trigger_count += 1;
                        print!("\rCurrent: {} | Hits: {} | Vel: {:.0}%    ",
                               current_preset, trigger_count, current_velocity * 100.0);
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Char('x') | KeyCode::Char('X') => {
                        current_velocity = 0.50;
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("tom", current_velocity);
                        trigger_count += 1;
                        print!("\rCurrent: {} | Hits: {} | Vel: {:.0}%    ",
                               current_preset, trigger_count, current_velocity * 100.0);
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Char('c') | KeyCode::Char('C') => {
                        current_velocity = 0.75;
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("tom", current_velocity);
                        trigger_count += 1;
                        print!("\rCurrent: {} | Hits: {} | Vel: {:.0}%    ",
                               current_preset, trigger_count, current_velocity * 100.0);
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Char('v') | KeyCode::Char('V') => {
                        current_velocity = 1.0;
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("tom", current_velocity);
                        trigger_count += 1;
                        print!("\rCurrent: {} | Hits: {} | Vel: {:.0}%    ",
                               current_preset, trigger_count, current_velocity * 100.0);
                        io::stdout().flush().unwrap();
                    }

                    // Preset switching
                    KeyCode::Char('1') => {
                        let mut t = tom.lock().unwrap();
                        t.set_config(TomConfig::ds_tom());
                        current_preset = "DS Tom";
                        print!("\rCurrent: {} | Hits: {} | Vel: {:.0}%    ",
                               current_preset, trigger_count, current_velocity * 100.0);
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Char('2') => {
                        let mut t = tom.lock().unwrap();
                        t.set_config(TomConfig::high_tom());
                        current_preset = "High Tom";
                        print!("\rCurrent: {} | Hits: {} | Vel: {:.0}%    ",
                               current_preset, trigger_count, current_velocity * 100.0);
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Char('3') => {
                        let mut t = tom.lock().unwrap();
                        t.set_config(TomConfig::mid_tom());
                        current_preset = "Mid Tom";
                        print!("\rCurrent: {} | Hits: {} | Vel: {:.0}%    ",
                               current_preset, trigger_count, current_velocity * 100.0);
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Char('4') => {
                        let mut t = tom.lock().unwrap();
                        t.set_config(TomConfig::low_tom());
                        current_preset = "Low Tom";
                        print!("\rCurrent: {} | Hits: {} | Vel: {:.0}%    ",
                               current_preset, trigger_count, current_velocity * 100.0);
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Char('5') => {
                        let mut t = tom.lock().unwrap();
                        t.set_config(TomConfig::floor_tom());
                        current_preset = "Floor Tom";
                        print!("\rCurrent: {} | Hits: {} | Vel: {:.0}%    ",
                               current_preset, trigger_count, current_velocity * 100.0);
                        io::stdout().flush().unwrap();
                    }

                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
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
