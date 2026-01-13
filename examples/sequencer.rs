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
use libgooey::effects::LowpassFilterEffect;
use libgooey::engine::{Engine, EngineOutput, Lfo, MusicalDivision, Sequencer, SequencerStep};
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

    // Create a sequencer with varying velocities to demonstrate velocity response
    // Pattern: soft -> medium -> hard -> full (repeating)
    // This lets you hear the velocity differences without a MIDI controller
    let pattern = vec![
        SequencerStep::with_velocity(true, 0.2), // Soft - short decay, less bright
        SequencerStep::with_velocity(false, 0.0), // Rest
        SequencerStep::with_velocity(true, 0.5), // Medium
        SequencerStep::with_velocity(false, 0.0), // Rest
        SequencerStep::with_velocity(true, 0.8), // Hard - longer decay, brighter
        SequencerStep::with_velocity(false, 0.0), // Rest
        SequencerStep::with_velocity(true, 1.0), // Full - maximum decay & brightness
        SequencerStep::with_velocity(false, 0.0), // Rest
    ];
    let sequencer = Sequencer::with_velocity_pattern(bpm, sample_rate, pattern, "hihat");
    engine.add_sequencer(sequencer);

    println!("Pattern velocities: 0.2 (soft) -> 0.5 (med) -> 0.8 (hard) -> 1.0 (full)");

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

    // Add a lowpass filter effect to the engine
    let filter = LowpassFilterEffect::new(sample_rate, 2000.0, 0.3);
    let filter_control = filter.get_control();
    engine.add_global_effect(Box::new(filter));

    println!("✓ Lowpass filter added to output");
    println!("  Initial cutoff: 2000 Hz");
    println!("  Resonance: 0.3");

    // Wrap in Arc<Mutex> for thread-safe access
    let audio_engine = Arc::new(Mutex::new(engine));

    // Enable raw mode for immediate key detection (MUST be before GLFW window creation)
    enable_raw_mode()?;

    // Create and configure the Engine output
    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;

    // Enable visualization (optional - comment out to disable)
    #[cfg(feature = "visualization")]
    engine_output.enable_visualization(1200, 600, 2.0)?;

    engine_output.create_stream_with_engine(audio_engine.clone())?;

    // Start the audio stream
    engine_output.start()?;

    println!("=== Sequencer + LFO + Filter Example ===");
    println!("Press SPACE to start/stop sequencer");
    println!("Press UP/DOWN to adjust BPM");
    println!("Press LEFT/RIGHT to cycle LFO division");
    println!("Press W/S to adjust filter cutoff frequency");
    println!("Press A/D to adjust filter resonance");
    println!("Press 'q' to quit");
    #[cfg(feature = "visualization")]
    println!("\nVisualization window shows:");
    #[cfg(feature = "visualization")]
    println!("  Top: Waveform | Bottom: Spectrogram (0-10kHz)");
    println!("");

    // Main input loop
    let result = loop {
        // Update visualization if enabled (no-op if disabled)
        if engine_output.update_visualization() {
            println!("\rVisualization window closed");
            break Ok(());
        }

        // Poll for key events (non-blocking with short timeout for smooth visualization)
        if event::poll(std::time::Duration::from_millis(16))? {
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
                    KeyCode::Char('w') | KeyCode::Char('W') => {
                        let current = filter_control.get_cutoff_freq();
                        let new_cutoff = (current + 200.0).min(20000.0);
                        filter_control.set_cutoff_freq(new_cutoff);
                        println!("\rFilter Cutoff: {:.0} Hz  ", new_cutoff);
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        let current = filter_control.get_cutoff_freq();
                        let new_cutoff = (current - 200.0).max(100.0);
                        filter_control.set_cutoff_freq(new_cutoff);
                        println!("\rFilter Cutoff: {:.0} Hz  ", new_cutoff);
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Char('a') | KeyCode::Char('A') => {
                        let current = filter_control.get_resonance();
                        let new_resonance = (current - 0.05).max(0.0);
                        filter_control.set_resonance(new_resonance);
                        println!("\rFilter Resonance: {:.2}  ", new_resonance);
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Char('d') | KeyCode::Char('D') => {
                        let current = filter_control.get_resonance();
                        let new_resonance = (current + 0.05).min(0.95);
                        filter_control.set_resonance(new_resonance);
                        println!("\rFilter Resonance: {:.2}  ", new_resonance);
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
