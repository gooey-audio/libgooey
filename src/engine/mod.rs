use std::collections::{HashMap, VecDeque};

#[cfg(feature = "native")]
pub mod engine_output;

#[cfg(feature = "native")]
pub use engine_output::EngineOutput;

/// Trait that all instruments must implement
/// Send is required because instruments are used in the audio thread
pub trait Instrument: Send {
    /// Trigger the instrument at a specific time
    fn trigger(&mut self, time: f32);
    
    /// Generate one sample of audio at the current time
    fn tick(&mut self, current_time: f32) -> f32;
    
    /// Check if the instrument is currently active
    fn is_active(&self) -> bool;
}

/// Minimal audio engine - the primary abstraction for audio generation
pub struct Engine {
    sample_rate: f32,
    instruments: HashMap<String, Box<dyn Instrument>>,
    // Queue of instrument names to trigger on next tick
    trigger_queue: VecDeque<String>,
}

impl Engine {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            instruments: HashMap::new(),
            trigger_queue: VecDeque::new(),
        }
    }

    /// Add an instrument with a unique name
    pub fn add_instrument(&mut self, name: impl Into<String>, instrument: Box<dyn Instrument>) {
        self.instruments.insert(name.into(), instrument);
    }

    /// Queue an instrument to be triggered on the next audio tick
    /// This is thread-safe to call from the main thread
    pub fn trigger_instrument(&mut self, name: &str) {
        self.trigger_queue.push_back(name.to_string());
    }

    /// Generate one sample of audio at the given time
    /// This is called by the audio output on every sample
    pub fn tick(&mut self, current_time: f32) -> f32 {
        // Process trigger queue - trigger instruments with current audio time
        while let Some(name) = self.trigger_queue.pop_front() {
            if let Some(instrument) = self.instruments.get_mut(&name) {
                instrument.trigger(current_time);
            } else {
                eprintln!("Warning: Instrument '{}' not found", name);
            }
        }

        // Sum all instrument outputs
        let mut output = 0.0;

        for instrument in self.instruments.values_mut() {
            output += instrument.tick(current_time);
        }

        // TODO
        // later apply limiter here
        output
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }
}
