/* CLI example for hi-hat sequencer testing.
Demonstrates a simple sequencer using time-based triggering.
Note: This is a timing-based approach, not sample-accurate.
*/

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::io::{self, Write};
use std::time::{Duration, Instant};

// Import the platform abstraction and audio engine
use libgooey::platform::{AudioEngine, AudioOutput, CpalOutput};
use libgooey::sequencer;

// Simple sequencer state
// struct SequencerState {
//     bpm: f32,
//     current_step: usize,
//     is_running: bool,
//     pattern: Vec<bool>,
//     last_step_time: Option<Instant>,
// }

// CLI example for hi-hat sequencer
#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    // Create the audio engine

    use libgooey::sequencer::Sequencer;

    let sample_rate = 44100.0;
    let audio_engine = AudioEngine::new(sample_rate);

    let sequencer = Sequencer::new(120.0, sample_rate);
    
    // Create and configure the CPAL output
    let mut cpal_output = CpalOutput::new();
    cpal_output.initialize(44100.0)?;
    cpal_output.create_stream_with_stage(audio_engine.stage(), audio_engine.audio_state())?;
    
    // Start the audio stream
    cpal_output.start()?;
    
    // Define a simple 8-step pattern (8th notes)
    // true means trigger on that step
    let pattern = vec![true, false, true, false, true, false, true, false];
    
    // // Create sequencer state
    // let mut sequencer_state = SequencerState {
    //     bpm: 120.0,
    //     current_step: 0,
    //     is_running: false,
    //     pattern,
    //     last_step_time: None,
    // };
    
    println!("=== Hi-Hat Sequencer Example ===");
    println!("Press SPACE to start/stop sequencer");
    println!("Press UP/DOWN to adjust BPM");
    println!("Press 'q' to quit");
    
    // Enable raw mode for immediate key detection
    enable_raw_mode()?;
    
    // Main input loop
    let result = loop {
        // Check if we need to trigger the next step
        if sequencer_state.is_running {
            let now = Instant::now();
            let should_trigger = if let Some(last_time) = sequencer_state.last_step_time {
                // Calculate 8th note duration
                let seconds_per_8th = (60.0 / sequencer_state.bpm) / 2.0;
                let duration_since_last = now.duration_since(last_time);
                
                duration_since_last.as_secs_f32() >= seconds_per_8th
            } else {
                // First trigger
                true
            };
            
            if should_trigger {
                // Trigger if pattern says so
                let pattern_step = sequencer_state.current_step % sequencer_state.pattern.len();
                if sequencer_state.pattern[pattern_step] {
                    let mut stage = audio_engine.stage_mut();
                    stage.trigger_hihat();
                }
                
                // Advance step and update time
                sequencer_state.current_step += 1;
                sequencer_state.last_step_time = Some(now);
            }
        }
        
        // Poll for key events (non-blocking with short timeout)
        if event::poll(Duration::from_millis(10))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match code {
                    KeyCode::Char(' ') => {
                        // Toggle sequencer
                        sequencer_state.is_running = !sequencer_state.is_running;
                        if sequencer_state.is_running {
                            sequencer_state.current_step = 0; // Reset step when starting
                            sequencer_state.last_step_time = None; // Reset timing
                            println!("\rSequencer started at {} BPM  ", sequencer_state.bpm);
                        } else {
                            println!("\rSequencer stopped            ");
                        }
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Up => {
                        sequencer_state.bpm = (sequencer_state.bpm + 5.0).min(200.0);
                        println!("\rBPM: {}  ", sequencer_state.bpm);
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Down => {
                        sequencer_state.bpm = (sequencer_state.bpm - 5.0).max(60.0);
                        println!("\rBPM: {}  ", sequencer_state.bpm);
                        io::stdout().flush().unwrap();
                    }
                    KeyCode::Char('q') | KeyCode::Esc => {
                        println!("\rQuitting...              ");
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

