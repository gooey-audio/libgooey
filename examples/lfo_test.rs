/* CLI example for testing LFO modulation on all drum instruments.
Demonstrates LFO modulation working on KickDrum, SnareDrum, HiHat, and TomDrum.
*/

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

// Import the engine and instruments
use libgooey::engine::{Engine, EngineOutput, Lfo, MusicalDivision};
use libgooey::instruments::{HiHat, KickDrum, SnareDrum, TomDrum};

// CLI example for LFO testing
#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;

    // Create the audio engine
    let mut engine = Engine::new(sample_rate);
    let bpm = 120.0;
    engine.set_bpm(bpm);

    // Add all drum instruments
    let kick = KickDrum::new(sample_rate);
    engine.add_instrument("kick", Box::new(kick));

    let snare = SnareDrum::new(sample_rate);
    engine.add_instrument("snare", Box::new(snare));

    let hihat = HiHat::new(sample_rate);
    engine.add_instrument("hihat", Box::new(hihat));

    let tom = TomDrum::new(sample_rate);
    engine.add_instrument("tom", Box::new(tom));

    // Create LFOs for each instrument
    // Kick: Modulate pitch_drop with a 1-bar LFO
    let kick_lfo = Lfo::new_synced(MusicalDivision::OneBar, bpm, sample_rate);
    let kick_lfo_idx = engine.add_lfo(kick_lfo);
    engine
        .map_lfo_to_parameter(kick_lfo_idx, "kick", "pitch_drop", 1.0)
        .map_err(|e| anyhow::anyhow!(e))?;

    // Snare: Modulate tonal amount with a 2-bar LFO
    let snare_lfo = Lfo::new_synced(MusicalDivision::TwoBars, bpm, sample_rate);
    let snare_lfo_idx = engine.add_lfo(snare_lfo);
    engine
        .map_lfo_to_parameter(snare_lfo_idx, "snare", "tonal", 1.0)
        .map_err(|e| anyhow::anyhow!(e))?;

    // HiHat: Modulate decay with a half-note LFO
    let hihat_lfo = Lfo::new_synced(MusicalDivision::Half, bpm, sample_rate);
    let hihat_lfo_idx = engine.add_lfo(hihat_lfo);
    engine
        .map_lfo_to_parameter(hihat_lfo_idx, "hihat", "decay", 1.0)
        .map_err(|e| anyhow::anyhow!(e))?;

    // Tom: Modulate frequency with a quarter-note LFO
    let tom_lfo = Lfo::new_synced(MusicalDivision::Quarter, bpm, sample_rate);
    let tom_lfo_idx = engine.add_lfo(tom_lfo);
    engine
        .map_lfo_to_parameter(tom_lfo_idx, "tom", "frequency", 1.0)
        .map_err(|e| anyhow::anyhow!(e))?;

    println!("✓ All instruments and LFOs configured");
    println!("  Kick: pitch_drop modulated by 1-bar LFO");
    println!("  Snare: tonal modulated by 2-bar LFO");
    println!("  HiHat: decay modulated by half-note LFO");
    println!("  Tom: frequency modulated by quarter-note LFO");
    println!();

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

    println!("=== LFO Modulation Test ===");
    println!("Press keys to trigger instruments:");
    println!("  K = Kick (pitch_drop modulated)");
    println!("  S = Snare (tonal modulated)");
    println!("  H = HiHat (decay modulated)");
    println!("  T = Tom (frequency modulated)");
    println!("  Q = Quit");
    #[cfg(feature = "visualization")]
    println!("\nWaveform visualization enabled");
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
                    KeyCode::Char('k') | KeyCode::Char('K') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument("kick");
                        print!("K");
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument("snare");
                        print!("S");
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Char('h') | KeyCode::Char('H') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument("hihat");
                        print!("H");
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Char('t') | KeyCode::Char('T') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument("tom");
                        print!("T");
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
