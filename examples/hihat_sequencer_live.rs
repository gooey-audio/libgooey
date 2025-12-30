/* Live hi-hat sequencer example.
Demonstrates the sample-accurate callback-based sequencer playing actual audio.
This integrates the Sequencer with Stage in a real-time audio context.
*/

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

// Import the platform abstraction and audio engine
use libgooey::platform::{AudioEngine, AudioOutput, CpalOutput};
use libgooey::sequencer::Sequencer as CallbackSequencer;

// Helper struct to wrap our callback sequencer and pattern
struct SequencerState {
    sequencer: CallbackSequencer,
    pattern: Vec<bool>,
}

impl SequencerState {
    fn new(bpm: f32, sample_rate: f32) -> Self {
        Self {
            sequencer: CallbackSequencer::new(bpm, sample_rate),
            // Simple 8th note pattern: x.x.x.x. (hits on 0, 2, 4, 6)
            pattern: vec![true, false, true, false, true, false, true, false],
        }
    }
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    const SAMPLE_RATE: f32 = 44100.0;
    const INITIAL_BPM: f32 = 120.0;
    
    // Create the audio engine
    let audio_engine = AudioEngine::new(SAMPLE_RATE);
    
    // Create shared sequencer state
    let sequencer_state = Arc::new(Mutex::new(SequencerState::new(INITIAL_BPM, SAMPLE_RATE)));
    let sequencer_state_clone = sequencer_state.clone();
    
    // Start the sequencer
    {
        let mut state = sequencer_state.lock().unwrap();
        state.sequencer.start();
    }
    
    // Create a modified CpalOutput that will integrate our sequencer
    // We'll need to modify the stage's tick to include our sequencer
    let mut cpal_output = CpalOutput::new();
    cpal_output.initialize(SAMPLE_RATE)?;
    
    // Create the stream with stage and audio state
    // Note: The current implementation processes samples in the audio callback
    // We need to integrate our sequencer into that callback
    // For now, let's use a simpler approach with manual polling
    
    println!("=== Live Hi-Hat Sequencer ===");
    println!("BPM: {} | Sample Rate: {} Hz", INITIAL_BPM, SAMPLE_RATE);
    println!();
    println!("Controls:");
    println!("  SPACE  - Start/Stop sequencer");
    println!("  UP     - Increase BPM (+5)");
    println!("  DOWN   - Decrease BPM (-5)");
    println!("  1-8    - Toggle steps in pattern");
    println!("  q/ESC  - Quit");
    println!();
    println!("Pattern: x.x.x.x. (. = silent, x = hit)");
    println!("Sequencer started automatically");
    
    // Create and start the audio stream
    cpal_output.create_stream_with_stage(audio_engine.stage(), audio_engine.audio_state())?;
    cpal_output.start()?;
    
    // We'll manually trigger the stage in sync with our sequencer
    // In a production implementation, this would be inside the audio callback
    std::thread::spawn(move || {
        let mut sample_count = 0u64;
        loop {
            std::thread::sleep(std::time::Duration::from_micros(22)); // ~44100 Hz
            
            let stage = audio_engine.stage();
            let mut state = sequencer_state_clone.lock().unwrap();
            
            // Process one sample through the sequencer
            state.sequencer.tick(|step| {
                // Get the pattern index (loop the pattern)
                let pattern_idx = step % state.pattern.len();
                
                // If this step should trigger, trigger the hihat
                if state.pattern[pattern_idx] {
                    let mut stage_guard = stage.lock().unwrap();
                    stage_guard.trigger_hihat();
                }
            });
            
            sample_count += 1;
        }
    });
    
    // Enable raw mode for immediate key detection
    enable_raw_mode()?;
    
    // Main input loop
    let result = loop {
        // Poll for key events (non-blocking with timeout)
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match code {
                    KeyCode::Char(' ') => {
                        let mut state = sequencer_state.lock().unwrap();
                        if state.sequencer.is_running() {
                            state.sequencer.stop();
                            println!("\n[STOPPED]");
                        } else {
                            state.sequencer.start();
                            println!("\n[STARTED]");
                        }
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Up => {
                        let mut state = sequencer_state.lock().unwrap();
                        let new_bpm = (state.sequencer.bpm + 5.0).min(200.0);
                        state.sequencer.set_bpm(new_bpm);
                        println!("\nBPM: {}", new_bpm);
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Down => {
                        let mut state = sequencer_state.lock().unwrap();
                        let new_bpm = (state.sequencer.bpm - 5.0).max(40.0);
                        state.sequencer.set_bpm(new_bpm);
                        println!("\nBPM: {}", new_bpm);
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Char(c @ '1'..='8') => {
                        let index = (c as usize) - ('1' as usize);
                        let mut state = sequencer_state.lock().unwrap();
                        if index < state.pattern.len() {
                            state.pattern[index] = !state.pattern[index];
                            
                            // Print the pattern
                            let pattern_str: String = state.pattern.iter()
                                .map(|&b| if b { 'x' } else { '.' })
                                .collect();
                            println!("\nPattern: {}", pattern_str);
                            io::stdout().flush().unwrap();
                        }
                    }
                    KeyCode::Char('q') | KeyCode::Esc => {
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

