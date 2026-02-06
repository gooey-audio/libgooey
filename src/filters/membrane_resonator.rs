//! Membrane Resonator - 5-band parallel resonant filter bank
//!
//! Simulates the resonant behavior of a drum membrane using 5 parallel
//! bandpass filters. Based on Max/MSP membrane patch signal flow:
//! input → 5 parallel reson~ filters → summed output
//!
//! This is an audio effect: put sound in, get resonant sound out.
//! The filters will "ring" after excitation, decaying naturally based on Q.

use super::BiquadBandpass;

/// Default membrane filter parameters from Max patch preset 1: (gain, freq_hz, q)
pub const DEFAULT_MEMBRANE_PARAMS: [(f32, f32, f32); 5] = [
    (275.0, 165.0, 376.0),
    (220.0, 228.0, 205.0),
    (79.0, 294.0, 143.0),
    (65.0, 320.0, 129.0),
    (57.0, 326.0, 141.0),
];

/// Membrane Resonator - 5-band parallel resonant filter bank
///
/// Processes audio through 5 parallel bandpass filters to create
/// resonant membrane-like sounds. The filters ring after excitation.
///
/// # Example
/// ```ignore
/// let mut membrane = MembraneResonator::new(44100.0);
/// membrane.set_q_scale(0.01);
/// membrane.set_gain_scale(0.001);
///
/// // In audio callback:
/// let output = membrane.process(input_sample);
/// ```
pub struct MembraneResonator {
    filters: [BiquadBandpass; 5],
    filter_params: [(f32, f32, f32); 5], // (gain, freq, q) for each filter
    q_scale: f32,
    gain_scale: f32,
    ring_level: f32, // Tracks output level for fade detection
}

impl MembraneResonator {
    /// Create a new membrane resonator with default parameters
    ///
    /// # Arguments
    /// * `sample_rate` - Audio sample rate in Hz
    pub fn new(sample_rate: f32) -> Self {
        Self::with_params(sample_rate, DEFAULT_MEMBRANE_PARAMS)
    }

    /// Create a membrane resonator with custom filter parameters
    ///
    /// # Arguments
    /// * `sample_rate` - Audio sample rate in Hz
    /// * `params` - Array of 5 filter parameters: (gain, freq_hz, q)
    pub fn with_params(sample_rate: f32, params: [(f32, f32, f32); 5]) -> Self {
        let filters = [
            BiquadBandpass::new(sample_rate),
            BiquadBandpass::new(sample_rate),
            BiquadBandpass::new(sample_rate),
            BiquadBandpass::new(sample_rate),
            BiquadBandpass::new(sample_rate),
        ];

        let mut resonator = Self {
            filters,
            filter_params: params,
            q_scale: 0.01,    // Default: scales Max Q (100+) to BiquadBandpass range
            gain_scale: 0.0031, // Default: scales Max gain values down
            ring_level: 0.0,
        };

        resonator.update_filters();
        resonator
    }

    /// Reset all filter states (clears the ringing)
    pub fn reset(&mut self) {
        for filter in &mut self.filters {
            filter.reset();
        }
        self.ring_level = 0.0;
    }

    /// Update filter coefficients based on current scaling parameters
    fn update_filters(&mut self) {
        for (i, (gain, freq, q)) in self.filter_params.iter().enumerate() {
            let scaled_q = (q * self.q_scale).clamp(0.1, 100.0);
            let scaled_gain = gain * self.gain_scale;
            self.filters[i].set_params(*freq, scaled_q, scaled_gain);
        }
    }

    /// Set the Q scaling factor
    ///
    /// Q scale maps the raw Max patch Q values to usable BiquadBandpass Q values.
    /// Higher values = more resonance/longer ring time.
    ///
    /// # Arguments
    /// * `scale` - Q scaling factor (clamped to 0.001-1.0, default 0.01)
    pub fn set_q_scale(&mut self, scale: f32) {
        self.q_scale = scale.clamp(0.001, 1.0);
        self.update_filters();
    }

    /// Get the current Q scaling factor
    pub fn q_scale(&self) -> f32 {
        self.q_scale
    }

    /// Set the gain scaling factor
    ///
    /// Gain scale controls the overall output level of the resonators.
    /// Higher values = louder but may cause distortion.
    ///
    /// # Arguments
    /// * `scale` - Gain scaling factor (clamped to 0.0001-0.1, default 0.001)
    pub fn set_gain_scale(&mut self, scale: f32) {
        self.gain_scale = scale.clamp(0.0001, 0.1);
        self.update_filters();
    }

    /// Get the current gain scaling factor
    pub fn gain_scale(&self) -> f32 {
        self.gain_scale
    }

    /// Set custom filter parameters
    ///
    /// # Arguments
    /// * `params` - Array of 5 filter parameters: (gain, freq_hz, q)
    pub fn set_filter_params(&mut self, params: [(f32, f32, f32); 5]) {
        self.filter_params = params;
        self.update_filters();
    }

    /// Get the current filter parameters
    pub fn filter_params(&self) -> &[(f32, f32, f32); 5] {
        &self.filter_params
    }

    /// Get the current ring level (smoothed output level)
    ///
    /// Useful for detecting when the resonator has finished ringing.
    /// Values below ~0.001 indicate the resonator is essentially silent.
    pub fn ring_level(&self) -> f32 {
        self.ring_level
    }

    /// Check if the resonator is still audibly ringing
    ///
    /// Returns true if ring_level is above the cutoff threshold (0.0001)
    pub fn is_ringing(&self) -> bool {
        self.ring_level > 0.0001
    }

    /// Get a smooth fade multiplier based on ring level
    ///
    /// Returns 1.0 when ring_level is above fade threshold,
    /// smoothly fades to 0.0 as ring_level approaches cutoff.
    /// This prevents pops when the resonator stops.
    pub fn fade_multiplier(&self) -> f32 {
        const FADE_START: f32 = 0.005; // Start fading at this level
        const FADE_END: f32 = 0.0001;  // Fully silent at this level

        if self.ring_level >= FADE_START {
            1.0
        } else if self.ring_level <= FADE_END {
            0.0
        } else {
            // Smooth fade between thresholds
            (self.ring_level - FADE_END) / (FADE_START - FADE_END)
        }
    }

    /// Process a single sample through all filters
    ///
    /// The input is processed through 5 parallel bandpass filters,
    /// and the outputs are summed. A soft clip (tanh) is applied
    /// to prevent blowup from high resonance.
    ///
    /// # Arguments
    /// * `input` - Input sample
    ///
    /// # Returns
    /// Processed output sample (soft-clipped)
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        // Process through all 5 filters in parallel and sum
        let mut output = 0.0;
        for filter in &mut self.filters {
            output += filter.process(input);
        }

        // Soft clip to prevent blowup
        let clipped = output.tanh();

        // Track ring level with smoothing for fade detection
        self.ring_level = self.ring_level * 0.999 + clipped.abs() * 0.001;

        clipped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_membrane_creation() {
        let membrane = MembraneResonator::new(44100.0);
        assert_eq!(membrane.q_scale(), 0.01);
        assert_eq!(membrane.gain_scale(), 0.001);
    }

    #[test]
    fn test_membrane_reset() {
        let mut membrane = MembraneResonator::new(44100.0);
        // Process some samples to excite
        for _ in 0..1000 {
            membrane.process(0.5);
        }
        assert!(membrane.ring_level() > 0.0);

        // Reset
        membrane.reset();
        assert_eq!(membrane.ring_level(), 0.0);
    }

    #[test]
    fn test_membrane_ringing() {
        let mut membrane = MembraneResonator::new(44100.0);
        membrane.set_gain_scale(0.01); // Higher gain for visible ring

        // Excite with impulse
        membrane.process(1.0);

        // Process with zero input - should still produce output (ringing)
        let mut has_output = false;
        for _ in 0..100 {
            let out = membrane.process(0.0);
            if out.abs() > 0.0001 {
                has_output = true;
                break;
            }
        }
        assert!(has_output, "Membrane should continue ringing after excitation");
    }

    #[test]
    fn test_q_scale_bounds() {
        let mut membrane = MembraneResonator::new(44100.0);

        membrane.set_q_scale(0.0001); // Below min
        assert_eq!(membrane.q_scale(), 0.001);

        membrane.set_q_scale(10.0); // Above max
        assert_eq!(membrane.q_scale(), 1.0);
    }

    #[test]
    fn test_gain_scale_bounds() {
        let mut membrane = MembraneResonator::new(44100.0);

        membrane.set_gain_scale(0.00001); // Below min
        assert_eq!(membrane.gain_scale(), 0.0001);

        membrane.set_gain_scale(1.0); // Above max
        assert_eq!(membrane.gain_scale(), 0.1);
    }
}
