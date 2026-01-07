use crate::effects::Effect;
use std::sync::{Arc, Mutex};

/// Two-pole resonant lowpass filter (Moog-style)
/// Provides 12 dB/octave rolloff with resonance control
struct ResonantLowpassFilter {
    sample_rate: f32,
    cutoff_freq: f32,
    resonance: f32,
    // State variables for two-pole filter
    stage1: f32,
    stage2: f32,
}

impl ResonantLowpassFilter {
    fn new(sample_rate: f32, cutoff_freq: f32, resonance: f32) -> Self {
        Self {
            sample_rate,
            cutoff_freq,
            resonance,
            stage1: 0.0,
            stage2: 0.0,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        // Calculate filter coefficient
        let f = 2.0 * (std::f32::consts::PI * self.cutoff_freq / self.sample_rate).sin();

        // Resonance feedback (0.0 = no resonance, 0.95 = high resonance)
        let feedback = self.resonance * 4.0;

        // Apply resonance feedback from second stage
        let input_with_feedback = input - self.stage2 * feedback;

        // First filter stage
        self.stage1 += f * (input_with_feedback - self.stage1);

        // Second filter stage (cascaded for 12 dB/octave)
        self.stage2 += f * (self.stage1 - self.stage2);

        self.stage2
    }

    fn reset(&mut self) {
        self.stage1 = 0.0;
        self.stage2 = 0.0;
    }

    fn set_cutoff_freq(&mut self, cutoff_freq: f32) {
        self.cutoff_freq = cutoff_freq.max(20.0).min(20000.0);
    }

    fn set_resonance(&mut self, resonance: f32) {
        self.resonance = resonance.clamp(0.0, 0.95); // Prevent instability
    }

    fn get_cutoff_freq(&self) -> f32 {
        self.cutoff_freq
    }

    fn get_resonance(&self) -> f32 {
        self.resonance
    }
}

/// Lowpass filter effect for the global effects chain
/// Uses Arc<Mutex<>> to allow mutable access from the Effect trait's immutable process method
pub struct LowpassFilterEffect {
    filter: Arc<Mutex<ResonantLowpassFilter>>,
}

impl LowpassFilterEffect {
    pub fn new(sample_rate: f32, cutoff_freq: f32, resonance: f32) -> Self {
        Self {
            filter: Arc::new(Mutex::new(ResonantLowpassFilter::new(
                sample_rate,
                cutoff_freq,
                resonance,
            ))),
        }
    }

    /// Get a handle to control the filter parameters
    pub fn get_control(&self) -> LowpassFilterControl {
        LowpassFilterControl {
            filter: self.filter.clone(),
        }
    }
}

impl Effect for LowpassFilterEffect {
    fn process(&self, input: f32) -> f32 {
        self.filter.lock().unwrap().process(input)
    }
}

/// Control handle for adjusting lowpass filter parameters
#[derive(Clone)]
pub struct LowpassFilterControl {
    filter: Arc<Mutex<ResonantLowpassFilter>>,
}

impl LowpassFilterControl {
    pub fn set_cutoff_freq(&self, cutoff_freq: f32) {
        self.filter.lock().unwrap().set_cutoff_freq(cutoff_freq);
    }

    pub fn set_resonance(&self, resonance: f32) {
        self.filter.lock().unwrap().set_resonance(resonance);
    }

    pub fn get_cutoff_freq(&self) -> f32 {
        self.filter.lock().unwrap().get_cutoff_freq()
    }

    pub fn get_resonance(&self) -> f32 {
        self.filter.lock().unwrap().get_resonance()
    }
}
