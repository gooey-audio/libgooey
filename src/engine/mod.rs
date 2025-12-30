use std::collections::{HashMap, VecDeque};

#[cfg(feature = "native")]
pub mod engine_output;

#[cfg(feature = "native")]
pub use engine_output::EngineOutput;

pub mod sequencer;
pub use sequencer::Sequencer;

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
    // Active sequencers
    sequencers: Vec<Sequencer>,
}

impl Engine {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            instruments: HashMap::new(),
            trigger_queue: VecDeque::new(),
            sequencers: Vec::new(),
        }
    }

    /// Add an instrument with a unique name
    pub fn add_instrument(&mut self, name: impl Into<String>, instrument: Box<dyn Instrument>) {
        self.instruments.insert(name.into(), instrument);
    }

    /// Add a sequencer to the engine
    pub fn add_sequencer(&mut self, sequencer: Sequencer) {
        self.sequencers.push(sequencer);
    }

    /// Get a mutable reference to a sequencer by index
    pub fn sequencer_mut(&mut self, index: usize) -> Option<&mut Sequencer> {
        self.sequencers.get_mut(index)
    }

    /// Get a reference to a sequencer by index
    pub fn sequencer(&self, index: usize) -> Option<&Sequencer> {
        self.sequencers.get(index)
    }

    /// Get the number of sequencers
    pub fn sequencer_count(&self) -> usize {
        self.sequencers.len()
    }

    /// Queue an instrument to be triggered on the next audio tick
    /// This is thread-safe to call from the main thread
    pub fn trigger_instrument(&mut self, name: &str) {
        self.trigger_queue.push_back(name.to_string());
    }

    /// Generate one sample of audio at the given time
    /// This is called by the audio output on every sample
    pub fn tick(&mut self, current_time: f32) -> f32 {
        // Process all sequencers (sample-accurate triggering)
        for sequencer in &mut self.sequencers {
            if let Some(instrument_name) = sequencer.tick() {
                // Sequencer says to trigger this instrument
                if let Some(instrument) = self.instruments.get_mut(instrument_name) {
                    instrument.trigger(current_time);
                }
            }
        }

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
