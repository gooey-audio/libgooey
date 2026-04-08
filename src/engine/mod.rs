use crate::effects::{Effect, SoftLimiter};
use crate::utils::SmoothedParam;
use std::collections::{HashMap, VecDeque};

#[cfg(feature = "native")]
pub mod engine_output;

#[cfg(feature = "native")]
pub use engine_output::EngineOutput;

pub mod sequencer;
pub use sequencer::{
    Sequencer, SequencerBlendSetting, SequencerStep, SequencerStepSettings, SequencerTrigger,
};

pub mod lfo;
pub use lfo::{Lfo, LfoSyncMode, MusicalDivision};

// Export WaveformDisplay when both native and visualization features are enabled
#[cfg(all(feature = "native", feature = "visualization"))]
pub use crate::visualization::WaveformDisplay;

/// Trait that all instruments must implement
/// Send is required because instruments are used in the audio thread
pub trait Instrument: Send {
    /// Trigger the instrument at a specific time with velocity
    ///
    /// # Arguments
    /// * `time` - The current audio time in seconds
    /// * `velocity` - Trigger velocity from 0.0 (softest) to 1.0 (hardest)
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32);

    /// Trigger the instrument at full velocity (convenience method)
    fn trigger(&mut self, time: f64) {
        self.trigger_with_velocity(time, 1.0);
    }

    /// Generate one sample of audio at the current time
    fn tick(&mut self, current_time: f64) -> f32;

    /// Check if the instrument is currently active
    fn is_active(&self) -> bool;

    /// Set the instrument's frequency from a MIDI note number (0-127).
    /// Called by the sequencer when a step has a per-step note set.
    /// Default implementation does nothing (instrument is not pitched).
    fn set_midi_note(&mut self, _note: u8) {}

    /// Set the instrument's normalized frequency parameter (0-1).
    /// Used to restore the global frequency after per-step note overrides.
    /// Default implementation does nothing (instrument is not pitched).
    fn set_frequency_normalized(&mut self, _value: f32) {}

    /// Get the current normalized frequency parameter (0-1), if the instrument is pitched.
    /// Used by the sequencer to save/restore the global frequency around per-step note overrides.
    fn get_frequency(&self) -> Option<f32> {
        None
    }

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
    // Queue of (instrument_name, velocity) to trigger on next tick
    trigger_queue: VecDeque<(String, f32)>,
    // Active sequencers
    sequencers: Vec<Sequencer>,
    // LFOs for modulation
    lfos: Vec<Lfo>,
    // Global effects applied to the final output (distinct from per-instrument effects)
    global_effects: Vec<Box<dyn Effect>>,
    // Master gain applied to the summed output before effects
    master_gain: SmoothedParam,
    // Saved global frequency per instrument for restoring after per-step note overrides
    saved_global_freq: HashMap<String, f32>,
}

impl Engine {
    pub fn new(sample_rate: f32) -> Self {
        // Initialize with a brick wall limiter as the default global effect
        let mut global_effects: Vec<Box<dyn Effect>> = Vec::new();
        global_effects.push(Box::new(SoftLimiter::new(1.0)));

        Self {
            sample_rate,
            bpm: 120.0, // Default BPM
            instruments: HashMap::new(),
            trigger_queue: VecDeque::new(),
            sequencers: Vec::new(),
            lfos: Vec::new(),
            global_effects,
            // Default of 0.25 provides headroom for mixing multiple instruments
            master_gain: SmoothedParam::new(0.25, 0.0, 2.0, sample_rate, 30.0),
            saved_global_freq: HashMap::new(),
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

    /// Add a global effect to the effects chain
    /// Global effects are applied to the final output after all instruments are mixed
    pub fn add_global_effect(&mut self, effect: Box<dyn Effect>) {
        self.global_effects.push(effect);
    }

    /// Clear all global effects
    pub fn clear_global_effects(&mut self) {
        self.global_effects.clear();
    }

    /// Get the number of global effects
    pub fn global_effect_count(&self) -> usize {
        self.global_effects.len()
    }

    /// Set the master gain level (smoothed to prevent clicks)
    ///
    /// # Arguments
    /// * `gain` - Gain level from 0.0 (silence) to 2.0 (+6dB). Default is 0.7.
    ///
    /// The default of 0.7 provides ~3dB headroom for mixing multiple instruments
    /// without clipping on professional audio interfaces.
    pub fn set_master_gain(&mut self, gain: f32) {
        self.master_gain.set_target(gain);
    }

    /// Get the current master gain target value
    pub fn master_gain(&self) -> f32 {
        self.master_gain.target()
    }

    /// Add an instrument with a unique name
    pub fn add_instrument(&mut self, name: impl Into<String>, instrument: Box<dyn Instrument>) {
        self.instruments.insert(name.into(), instrument);
    }

    /// Get a mutable reference to an instrument by name
    pub fn instrument_mut(&mut self, name: &str) -> Option<&mut Box<dyn Instrument>> {
        self.instruments.get_mut(name)
    }

    /// Get a reference to an instrument by name
    pub fn instrument(&self, name: &str) -> Option<&Box<dyn Instrument>> {
        self.instruments.get(name)
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
        let instrument = self
            .instruments
            .get_mut(instrument_name)
            .ok_or_else(|| format!("Instrument '{}' not found", instrument_name))?;

        // Validate parameter is modulatable
        if let Some(modulatable) = instrument.as_modulatable() {
            if !modulatable.modulatable_parameters().contains(&parameter) {
                return Err(format!(
                    "Parameter '{}' is not modulatable on instrument '{}'. Available: {:?}",
                    parameter,
                    instrument_name,
                    modulatable.modulatable_parameters()
                ));
            }
        } else {
            return Err(format!(
                "Instrument '{}' does not support modulation",
                instrument_name
            ));
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

    /// Queue an instrument to be triggered on the next audio tick at half velocity
    /// This is thread-safe to call from the main thread
    pub fn trigger_instrument(&mut self, name: &str) {
        self.trigger_queue.push_back((name.to_string(), 0.5));
    }

    /// Queue an instrument to be triggered on the next audio tick with specified velocity
    /// This is thread-safe to call from the main thread
    pub fn trigger_instrument_with_velocity(&mut self, name: &str, velocity: f32) {
        self.trigger_queue
            .push_back((name.to_string(), velocity.clamp(0.0, 1.0)));
    }

    /// Generate one sample of audio at the given time
    /// This is called by the audio output on every sample
    pub fn tick(&mut self, current_time: f64) -> f32 {
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

        // Process all sequencers (sample-accurate triggering with velocity and per-step notes)
        for sequencer in &mut self.sequencers {
            if let Some(trigger) = sequencer.tick_with_settings() {
                let instrument_name = trigger.instrument_name;
                let velocity = trigger.velocity;
                let note = trigger.note;

                if let Some(instrument) = self.instruments.get_mut(instrument_name) {
                    if let Some(midi_note) = note {
                        // Save global frequency before overriding (only on first note step)
                        let key = instrument_name.to_string();
                        if !self.saved_global_freq.contains_key(&key) {
                            if let Some(freq) = instrument.get_frequency() {
                                self.saved_global_freq.insert(key, freq);
                            }
                        }
                        instrument.set_midi_note(midi_note);
                    } else if let Some(saved) = self.saved_global_freq.remove(instrument_name) {
                        // Restore global frequency when step has no note
                        instrument.set_frequency_normalized(saved);
                    }
                    instrument.trigger_with_velocity(current_time, velocity);
                }
            }
        }

        // Process trigger queue - trigger instruments with current audio time and velocity
        while let Some((name, velocity)) = self.trigger_queue.pop_front() {
            if let Some(instrument) = self.instruments.get_mut(&name) {
                instrument.trigger_with_velocity(current_time, velocity);
            } else {
                eprintln!("Warning: Instrument '{}' not found", name);
            }
        }

        // Sum all instrument outputs
        let mut output = 0.0;

        for instrument in self.instruments.values_mut() {
            output += instrument.tick(current_time);
        }

        // Apply master gain before effects
        output *= self.master_gain.tick();

        // Apply global effects chain to the final output
        for effect in &self.global_effects {
            output = effect.process(output);
        }

        output
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Prepare the engine for an offline bounce.
    /// Resets all sequencers to beat 0, starts them, and snaps
    /// master gain to its target to avoid smoothing artifacts.
    pub fn prepare_for_bounce(&mut self) {
        for i in 0..self.sequencers.len() {
            self.sequencers[i].reset();
            self.sequencers[i].start();
        }
        for lfo in &mut self.lfos {
            lfo.reset();
        }
        self.master_gain.snap();
        self.trigger_queue.clear();
        self.saved_global_freq.clear();
    }

    /// Stop all sequencers (called after a bounce completes).
    pub fn stop_all_sequencers(&mut self) {
        for seq in &mut self.sequencers {
            seq.stop();
        }
    }
}
