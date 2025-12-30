/* CLI example demonstrating the sample-accurate sequencer with callbacks.
This shows how to use the basic Sequencer abstraction with any trigger function.
*/

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::io::{self, Write};

// Import the platform abstraction and audio engine
use libgooey::platform::{AudioEngine, AudioOutput, CpalOutput};
use libgooey::sequencer::Sequencer;
use std::sync::{Arc, Mutex};

// CLI example for sequencer
#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    const SAMPLE_RATE: f32 = 44100.0;
    const INITIAL_BPM: f32 = 120.0;
    
    // Create the audio engine
    let audio_engine = AudioEngine::new(SAMPLE_RATE);
    
    // Create a sequencer - wrap it in Arc<Mutex<>> so we can share it with the audio callback
    let sequencer = Arc::new(Mutex::new(Sequencer::new(INITIAL_BPM, SAMPLE_RATE)));
    let sequencer_clone = sequencer.clone();
    
    // Define a simple 8-step pattern (8th notes)
    // true means trigger on that step
    let pattern = Arc::new([true, false, true, false, true, false, true, false]);
    let pattern_clone = pattern.clone();
    
    // Create and configure the CPAL output
    let mut cpal_output = CpalOutput::new();
    cpal_output.initialize(SAMPLE_RATE)?;
    
    // Get references for the audio callback
    let stage = audio_engine.stage();
    let audio_state = audio_engine.audio_state();
    
    // We need to create a custom callback that integrates our sequencer
    // Unfortunately, the current CpalOutput API doesn't support custom callbacks easily,
    // so let's demonstrate the sequencer in a simpler standalone context
    
    // For this demo, let's just show the sequencer logic working in isolation
    println!("=== Sequencer Demo ===");
    println!("This demonstrates the sample-accurate sequencer logic.");
    println!();
    println!("Sequencer configuration:");
    println!("  BPM: {}", INITIAL_BPM);
    println!("  Sample Rate: {}", SAMPLE_RATE);
    println!("  Subdivision: 8th notes");
    println!();
    
    // Calculate timing information
    let seconds_per_8th = (60.0 / INITIAL_BPM) / 2.0;
    let samples_per_8th = seconds_per_8th * SAMPLE_RATE;
    
    println!("Timing calculations:");
    println!("  Seconds per 8th note: {:.4}s", seconds_per_8th);
    println!("  Samples per 8th note: {:.2}", samples_per_8th);
    println!();
    
    // Simulate running the sequencer for a few steps
    println!("Simulating sequencer for 8 steps:");
    println!();
    
    let mut test_sequencer = Sequencer::new(INITIAL_BPM, SAMPLE_RATE);
    test_sequencer.start();
    
    let mut triggered_steps = Vec::new();
    let mut sample_count = 0u64;
    
    // Run for 8 steps (should be approximately 8 * samples_per_8th samples)
    let max_samples = (samples_per_8th * 8.0) as u64 + 100; // Add some buffer
    
    while triggered_steps.len() < 8 && sample_count < max_samples {
        let triggered = test_sequencer.tick(|step| {
            let time = sample_count as f32 / SAMPLE_RATE;
            triggered_steps.push((step, sample_count, time));
            
            // This is where you would call stage.trigger_hihat() or any other trigger function
            println!("Step {}: triggered at sample {} ({:.4}s)", 
                     step, sample_count, time);
        });
        
        sample_count += 1;
    }
    
    println!();
    println!("Sequencer Statistics:");
    println!("  Total samples processed: {}", sample_count);
    println!("  Total steps triggered: {}", triggered_steps.len());
    
    if triggered_steps.len() > 1 {
        println!("  Average samples between steps: {:.2}", 
                 (sample_count as f32) / (triggered_steps.len() as f32));
        println!("  Expected samples between steps: {:.2}", samples_per_8th);
    }
    
    println!();
    println!("Integration Example:");
    println!("To integrate this into your audio callback, you would:");
    println!("1. Create a Sequencer instance");
    println!("2. On each audio buffer sample, call sequencer.tick() with a callback");
    println!("3. In the callback, call stage.trigger_hihat() or other trigger methods");
    println!();
    println!("Example code:");
    println!("  sequencer.tick(|step| {{");
    println!("      if pattern[step % pattern.len()] {{");
    println!("          stage.trigger_hihat();");
    println!("      }}");
    println!("  }});");
    
    Ok(())
}

#[cfg(not(feature = "native"))]
fn main() {
    println!("This example is only available with the 'native' feature enabled.");
}

