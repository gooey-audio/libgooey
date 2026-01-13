/* CLI example for sequencer testing.
Demonstrates sample-accurate sequencing with the new Engine.
*/

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use log::info;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

// Import the engine and instruments
use gooey::effects::LowpassFilterEffect;
use gooey::engine::{Engine, EngineOutput, Lfo, MusicalDivision, Sequencer};
use gooey::instruments::HiHat;

// CLI example for sequencer
#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .format(|buf, record| {
            let target = record.target();
            let short_target = target.rsplit("::").next().unwrap_or(target);
            writeln!(buf, "[{:5}] {:15.15} {}", record.level(), short_target, record.args())
        })
        .init();
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

    info!("LFO mapped to hi-hat decay");
    info!("  Synced to: 1 bar (4 beats)");
    info!("  Range: 20ms to 500ms decay time");

    // Add a lowpass filter effect to the engine
    let filter = LowpassFilterEffect::new(sample_rate, 2000.0, 0.3);
    let filter_control = filter.get_control();
    engine.add_global_effect(Box::new(filter));

    info!("Lowpass filter added to output");
    info!("  Initial cutoff: 2000 Hz");
    info!("  Resonance: 0.3");

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

    info!("=== Sequencer + LFO + Filter Example ===");
    info!("Press SPACE to start/stop sequencer");
    info!("Press UP/DOWN to adjust BPM");
    info!("Press LEFT/RIGHT to cycle LFO division");
    info!("Press W/S to adjust filter cutoff frequency");
    info!("Press A/D to adjust filter resonance");
    info!("Press 'q' to quit");
    #[cfg(feature = "visualization")]
    info!("Visualization window shows:");
    #[cfg(feature = "visualization")]
    info!("  Top: Waveform | Bottom: Spectrogram (0-10kHz)");
    info!("");

    // Main input loop
    let result = loop {
        // Update visualization if enabled (no-op if disabled)
        if engine_output.update_visualization() {
            info!("Visualization window closed");
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
                                info!("Sequencer stopped");
                            } else {
                                seq.start();
                                info!("Sequencer started at {} BPM", seq.bpm());
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
                        info!("BPM: {}", new_bpm);
                    }
                    KeyCode::Down => {
                        let mut engine = audio_engine.lock().unwrap();
                        let new_bpm = (engine.bpm() - 5.0).max(60.0);
                        engine.set_bpm(new_bpm);

                        // Also update sequencer BPM
                        if let Some(seq) = engine.sequencer_mut(0) {
                            seq.set_bpm(new_bpm);
                        }
                        info!("BPM: {}", new_bpm);
                    }
                    KeyCode::Right => {
                        let mut engine = audio_engine.lock().unwrap();
                        if let Some(lfo) = engine.lfo_mut(0) {
                            use gooey::engine::LfoSyncMode;
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
                            info!("LFO Division: {} ({:.2} Hz)", div_name, lfo.frequency());
                        }
                    }
                    KeyCode::Left => {
                        let mut engine = audio_engine.lock().unwrap();
                        if let Some(lfo) = engine.lfo_mut(0) {
                            use gooey::engine::LfoSyncMode;
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
                            info!("LFO Division: {} ({:.2} Hz)", div_name, lfo.frequency());
                        }
                    }
                    KeyCode::Char('w') | KeyCode::Char('W') => {
                        let current = filter_control.get_cutoff_freq();
                        let new_cutoff = (current + 200.0).min(20000.0);
                        filter_control.set_cutoff_freq(new_cutoff);
                        info!("Filter Cutoff: {:.0} Hz", new_cutoff);
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        let current = filter_control.get_cutoff_freq();
                        let new_cutoff = (current - 200.0).max(100.0);
                        filter_control.set_cutoff_freq(new_cutoff);
                        info!("Filter Cutoff: {:.0} Hz", new_cutoff);
                    }
                    KeyCode::Char('a') | KeyCode::Char('A') => {
                        let current = filter_control.get_resonance();
                        let new_resonance = (current - 0.05).max(0.0);
                        filter_control.set_resonance(new_resonance);
                        info!("Filter Resonance: {:.2}", new_resonance);
                    }
                    KeyCode::Char('d') | KeyCode::Char('D') => {
                        let current = filter_control.get_resonance();
                        let new_resonance = (current + 0.05).min(0.95);
                        filter_control.set_resonance(new_resonance);
                        info!("Filter Resonance: {:.2}", new_resonance);
                    }
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
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
