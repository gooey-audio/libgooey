use std::collections::{HashMap, VecDeque};

#[cfg(feature = "native")]
pub mod engine_output;

#[cfg(feature = "native")]
pub use engine_output::EngineOutput;

pub mod sequencer;
pub use sequencer::Sequencer;

pub mod lfo;
pub use lfo::{Lfo, LfoSyncMode, MusicalDivision};

/// Trait that all instruments must implement
/// Send is required because instruments are used in the audio thread
pub trait Instrument: Send {
    /// Trigger the instrument at a specific time
    fn trigger(&mut self, time: f32);
    
    /// Generate one sample of audio at the current time
    fn tick(&mut self, current_time: f32) -> f32;
    
    /// Check if the instrument is currently active
    fn is_active(&self) -> bool;
    
    /// Try to cast to Modulatable trait object
    /// Override this if the instrument supports modulation
    fn as_modulatable(&mut self) -> Option<&mut dyn Modulatable> {
        None
    }
}

/// Trait for instruments that support parameter modulation
pub trait Modulatable {
    /// Get list of parameter names that can be modulated
    fn modulatable_parameters(&self) -> Vec<&'static str>;
    
    /// Apply a modulation value to a parameter
    /// value is typically -1.0 to 1.0
    /// Returns Ok(()) if parameter exists and was applied, Err otherwise
    fn apply_modulation(&mut self, parameter: &str, value: f32) -> Result<(), String>;
    
    /// Get the range for a parameter (min, max)
    fn parameter_range(&self, parameter: &str) -> Option<(f32, f32)>;
}

/// Minimal audio engine - the primary abstraction for audio generation
pub struct Engine {
    sample_rate: f32,
    bpm: f32, // Global BPM for synced LFOs and sequencers
    instruments: HashMap<String, Box<dyn Instrument>>,
    // Queue of instrument names to trigger on next tick
    trigger_queue: VecDeque<String>,
    // Active sequencers
    sequencers: Vec<Sequencer>,
    // LFOs for modulation
    lfos: Vec<Lfo>,
}

impl Engine {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            bpm: 120.0, // Default BPM
            instruments: HashMap::new(),
            trigger_queue: VecDeque::new(),
            sequencers: Vec::new(),
            lfos: Vec::new(),
        }
    }
    
    /// Set the global BPM and update all synced LFOs
    pub fn set_bpm(&mut self, bpm: f32) {
        self.bpm = bpm;
        // Update all LFOs with the new BPM
        for lfo in &mut self.lfos {
            lfo.set_bpm(bpm);
        }
    }
    
    /// Get the global BPM
    pub fn bpm(&self) -> f32 {
        self.bpm
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

    /// Add an LFO to the engine and return its index
    pub fn add_lfo(&mut self, lfo: Lfo) -> usize {
        self.lfos.push(lfo);
        self.lfos.len() - 1
    }

    /// Get a mutable reference to an LFO by index
    pub fn lfo_mut(&mut self, index: usize) -> Option<&mut Lfo> {
        self.lfos.get_mut(index)
    }

    /// Get a reference to an LFO by index
    pub fn lfo(&self, index: usize) -> Option<&Lfo> {
        self.lfos.get(index)
    }

    /// Map an LFO to modulate a specific instrument parameter
    /// Returns Ok(()) if successful, Err with message if validation fails
    pub fn map_lfo_to_parameter(
        &mut self,
        lfo_index: usize,
        instrument_name: &str,
        parameter: &str,
        amount: f32,
    ) -> Result<(), String> {
        // Validate instrument exists
        let instrument = self.instruments.get_mut(instrument_name)
            .ok_or_else(|| format!("Instrument '{}' not found", instrument_name))?;
        
        // Validate parameter is modulatable
        if let Some(modulatable) = instrument.as_modulatable() {
            if !modulatable.modulatable_parameters().contains(&parameter) {
                return Err(format!(
                    "Parameter '{}' is not modulatable on instrument '{}'. Available: {:?}",
                    parameter, instrument_name, modulatable.modulatable_parameters()
                ));
            }
        } else {
            return Err(format!("Instrument '{}' does not support modulation", instrument_name));
        }
        
        // Set up the mapping
        if let Some(lfo) = self.lfos.get_mut(lfo_index) {
            lfo.target_instrument = instrument_name.to_string();
            lfo.target_parameter = parameter.to_string();
            lfo.amount = amount;
            Ok(())
        } else {
            Err(format!("LFO index {} not found", lfo_index))
        }
    }

    /// Queue an instrument to be triggered on the next audio tick
    /// This is thread-safe to call from the main thread
    pub fn trigger_instrument(&mut self, name: &str) {
        self.trigger_queue.push_back(name.to_string());
    }

    /// Generate one sample of audio at the given time
    /// This is called by the audio output on every sample
    pub fn tick(&mut self, current_time: f32) -> f32 {
        // Process LFOs and apply modulation
        for lfo in &mut self.lfos {
            let lfo_value = lfo.tick();
            
            // Apply modulation if this LFO has a target
            if !lfo.target_instrument.is_empty() && !lfo.target_parameter.is_empty() {
                if let Some(instrument) = self.instruments.get_mut(&lfo.target_instrument) {
                    if let Some(modulatable) = instrument.as_modulatable() {
                        let _ = modulatable.apply_modulation(&lfo.target_parameter, lfo_value);
                    }
                }
            }
        }

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
