//! Simplified Tom2 instrument - minimal implementation for A/B testing with Max/MSP
//!
//! This replicates the exact signal flow from a Max patch:
//! - Single envelope controls both pitch and amplitude
//! - Triangle oscillator at 327 Hz base frequency
//! - Envelope: attack to 1.0 in 1ms (curve 0.8), decay to 0.0 in 2000ms (curve -0.83)
//! - Pitch = base_freq * envelope_value
//! - Output = oscillator * envelope_value

use crate::engine::Instrument;
use crate::max_curve::MaxCurveEnvelope;

/// Simplified tom drum for A/B testing with Max/MSP reference
pub struct Tom2 {
    sample_rate: f32,
    base_frequency: f32,
    phase: f32, // Oscillator phase accumulator (0.0 to 1.0)
    envelope: MaxCurveEnvelope,
    is_active: bool,
    trigger_time: f32,
    past_attack: bool, // Track if we've passed the attack phase
}

impl Tom2 {
    /// Create a new Tom2 with default Max patch settings
    pub fn new(sample_rate: f32) -> Self {
        // Create envelope matching Max patch: "1 1 0.8 0 2000 -0.83"
        // Segment 1: go to 1.0 in 1ms with curve 0.8
        // Segment 2: go to 0.0 in 2000ms with curve -0.83
        let envelope = MaxCurveEnvelope::new(vec![
            (1.0, 1.0, 0.8),      // Attack: value=1.0, time=1ms, curve=0.8
            (0.0, 2000.0, -0.83), // Decay: value=0.0, time=2000ms, curve=-0.83
        ]);

        Self {
            sample_rate,
            base_frequency: 327.0,
            phase: 0.0,
            envelope,
            is_active: false,
            trigger_time: 0.0,
            past_attack: false,
        }
    }

    /// Generate naive triangle wave from phase (0.0 to 1.0)
    /// Returns value in range -1.0 to 1.0
    #[inline]
    fn triangle_wave(phase: f32) -> f32 {
        // Triangle: rises from -1 to 1 in first half, falls from 1 to -1 in second half
        let t = phase.fract();
        if t < 0.5 {
            4.0 * t - 1.0 // -1 to 1
        } else {
            3.0 - 4.0 * t // 1 to -1
        }
    }

    /// Create with custom frequency
    pub fn with_frequency(sample_rate: f32, frequency: f32) -> Self {
        let mut tom = Self::new(sample_rate);
        tom.base_frequency = frequency;
        tom
    }

    /// Set the base frequency
    pub fn set_frequency(&mut self, frequency: f32) {
        self.base_frequency = frequency;
    }

    /// Get the base frequency
    pub fn frequency(&self) -> f32 {
        self.base_frequency
    }
}

impl Instrument for Tom2 {
    fn trigger_with_velocity(&mut self, time: f32, _velocity: f32) {
        self.is_active = true;
        self.trigger_time = time;
        self.past_attack = false;
        self.phase = 0.0; // Reset phase on trigger
        self.envelope.trigger(time);
    }

    fn tick(&mut self, current_time: f32) -> f32 {
        if !self.is_active {
            return 0.0;
        }

        // Get envelope value (0.0 to 1.0)
        let env_value = self.envelope.get_value(current_time);

        // Track when we've passed the attack phase (reached near-peak)
        if env_value > 0.9 {
            self.past_attack = true;
        }

        // Pitch modulation: frequency = base_freq * envelope
        let modulated_freq = self.base_frequency * env_value;

        // Threshold cutoffs to prevent sub-audio rumble
        // Use a fade range to avoid pops from abrupt cutoff
        const FADE_START_FREQ: f32 = 40.0; // Start fading at 40 Hz
        const MIN_AUDIBLE_FREQ: f32 = 20.0; // Full cutoff at 20 Hz

        if self.envelope.is_complete()
            || (self.past_attack && modulated_freq < MIN_AUDIBLE_FREQ)
        {
            self.is_active = false;
            return 0.0;
        }

        // Calculate fade factor for smooth cutoff (1.0 at 40Hz, 0.0 at 20Hz)
        let fade_factor = if self.past_attack && modulated_freq < FADE_START_FREQ {
            (modulated_freq - MIN_AUDIBLE_FREQ) / (FADE_START_FREQ - MIN_AUDIBLE_FREQ)
        } else {
            1.0
        };

        // Generate triangle wave directly (no separate oscillator envelope)
        let osc_output = Self::triangle_wave(self.phase);

        // Advance phase based on current frequency
        let phase_increment = modulated_freq / self.sample_rate;
        self.phase += phase_increment;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }

        // VCA: multiply oscillator output by envelope and fade factor
        // This replicates Max's *~ multiplying tri~ output by curve~ signal
        osc_output * env_value * fade_factor
    }

    fn is_active(&self) -> bool {
        self.is_active
    }

    fn as_modulatable(&mut self) -> Option<&mut dyn crate::engine::Modulatable> {
        None
    }
}
