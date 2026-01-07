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
use libgooey::engine::{Engine, EngineOutput, Lfo, MusicalDivision, Sequencer};
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

    // Set the global BPM
    let bpm = 120.0;
    engine.set_bpm(bpm);

    // Create a sequencer with a simple 8-step pattern (16th notes at 120 BPM)
    let pattern = vec![true, false, true, false, true, false, true, false];
    let sequencer = Sequencer::with_pattern(bpm, sample_rate, pattern, "hihat");
    engine.add_sequencer(sequencer);

    // Add a BPM-synced LFO to modulate the hi-hat decay time
    // Start with 1 bar = one cycle every 4 beats
    let lfo = Lfo::new_synced(MusicalDivision::OneBar, bpm, sample_rate);
    let lfo_index = engine.add_lfo(lfo);

    // Map the LFO to the hi-hat's decay parameter
    // Amount of 1.0 means the LFO will use full modulation range
    engine
        .map_lfo_to_parameter(lfo_index, "hihat", "decay", 1.0)
        .expect("Failed to map LFO to hi-hat decay");

    println!("✓ LFO mapped to hi-hat decay");
    println!("  Synced to: 1 bar (4 beats)");
    println!("  Range: 20ms to 500ms decay time");

    // Wrap in Arc<Mutex> for thread-safe access
    let audio_engine = Arc::new(Mutex::new(engine));

    // Create and configure the Engine output
    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;
    engine_output.create_stream_with_engine(audio_engine.clone())?;

    // Start the audio stream
    engine_output.start()?;

    println!("=== Sequencer + LFO Example ===");
    println!("Press SPACE to start/stop sequencer");
    println!("Press UP/DOWN to adjust BPM");
    println!("Press LEFT/RIGHT to cycle LFO division");
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
                        let new_bpm = (engine.bpm() + 5.0).min(200.0);
                        engine.set_bpm(new_bpm);

                        // Also update sequencer BPM
                        if let Some(seq) = engine.sequencer_mut(0) {
                            seq.set_bpm(new_bpm);
                        }
                        println!("\rBPM: {}  ", new_bpm);
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Down => {
                        let mut engine = audio_engine.lock().unwrap();
                        let new_bpm = (engine.bpm() - 5.0).max(60.0);
                        engine.set_bpm(new_bpm);

                        // Also update sequencer BPM
                        if let Some(seq) = engine.sequencer_mut(0) {
                            seq.set_bpm(new_bpm);
                        }
                        println!("\rBPM: {}  ", new_bpm);
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Right => {
                        let mut engine = audio_engine.lock().unwrap();
                        if let Some(lfo) = engine.lfo_mut(0) {
                            use libgooey::engine::LfoSyncMode;
                            // Cycle to next division
                            let next_division = match lfo.sync_mode() {
                                LfoSyncMode::BpmSync(div) => match div {
                                    MusicalDivision::FourBars => MusicalDivision::TwoBars,
                                    MusicalDivision::TwoBars => MusicalDivision::OneBar,
                                    MusicalDivision::OneBar => MusicalDivision::Half,
                                    MusicalDivision::Half => MusicalDivision::Quarter,
                                    MusicalDivision::Quarter => MusicalDivision::Eighth,
                                    MusicalDivision::Eighth => MusicalDivision::Sixteenth,
                                    MusicalDivision::Sixteenth => MusicalDivision::ThirtySecond,
                                    MusicalDivision::ThirtySecond => MusicalDivision::ThirtySecond, // Stay at fastest
                                },
                                LfoSyncMode::Hz(_) => MusicalDivision::OneBar, // Default to 1 bar if in Hz mode
                            };
                            lfo.set_sync_mode(next_division);
                            let div_name = match next_division {
                                MusicalDivision::FourBars => "4 bars",
                                MusicalDivision::TwoBars => "2 bars",
                                MusicalDivision::OneBar => "1 bar",
                                MusicalDivision::Half => "1/2 note",
                                MusicalDivision::Quarter => "1/4 note",
                                MusicalDivision::Eighth => "1/8 note",
                                MusicalDivision::Sixteenth => "1/16 note",
                                MusicalDivision::ThirtySecond => "1/32 note",
                            };
                            println!("\rLFO Division: {} ({:.2} Hz)  ", div_name, lfo.frequency());
                        }
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Left => {
                        let mut engine = audio_engine.lock().unwrap();
                        if let Some(lfo) = engine.lfo_mut(0) {
                            use libgooey::engine::LfoSyncMode;
                            // Cycle to previous division
                            let prev_division = match lfo.sync_mode() {
                                LfoSyncMode::BpmSync(div) => match div {
                                    MusicalDivision::FourBars => MusicalDivision::FourBars, // Stay at slowest
                                    MusicalDivision::TwoBars => MusicalDivision::FourBars,
                                    MusicalDivision::OneBar => MusicalDivision::TwoBars,
                                    MusicalDivision::Half => MusicalDivision::OneBar,
                                    MusicalDivision::Quarter => MusicalDivision::Half,
                                    MusicalDivision::Eighth => MusicalDivision::Quarter,
                                    MusicalDivision::Sixteenth => MusicalDivision::Eighth,
                                    MusicalDivision::ThirtySecond => MusicalDivision::Sixteenth,
                                },
                                LfoSyncMode::Hz(_) => MusicalDivision::OneBar, // Default to 1 bar if in Hz mode
                            };
                            lfo.set_sync_mode(prev_division);
                            let div_name = match prev_division {
                                MusicalDivision::FourBars => "4 bars",
                                MusicalDivision::TwoBars => "2 bars",
                                MusicalDivision::OneBar => "1 bar",
                                MusicalDivision::Half => "1/2 note",
                                MusicalDivision::Quarter => "1/4 note",
                                MusicalDivision::Eighth => "1/8 note",
                                MusicalDivision::Sixteenth => "1/16 note",
                                MusicalDivision::ThirtySecond => "1/32 note",
                            };
                            println!("\rLFO Division: {} ({:.2} Hz)  ", div_name, lfo.frequency());
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
