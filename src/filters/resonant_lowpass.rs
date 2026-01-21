/// Resonant lowpass filter for instrument use
///
/// Simple one-pole lowpass filter with resonance boost.
/// Designed for use within instruments (non-atomic, mutable API).
pub struct ResonantLowpassFilter {
    pub sample_rate: f32,
    pub cutoff_freq: f32,
    pub resonance: f32,
    filter_state: f32,
    /// Previous output for resonance feedback
    prev_output: f32,
}

impl ResonantLowpassFilter {
    /// Create a new resonant lowpass filter
    ///
    /// # Arguments
    /// * `sample_rate` - Audio sample rate in Hz
    /// * `cutoff_freq` - Cutoff frequency in Hz
    /// * `resonance` - Resonance/Q factor (0.0-10.0, typical 0.5-4.0)
    pub fn new(sample_rate: f32, cutoff_freq: f32, resonance: f32) -> Self {
        Self {
            sample_rate,
            cutoff_freq,
            resonance,
            filter_state: 0.0,
            prev_output: 0.0,
        }
    }

    /// Reset filter state
    pub fn reset(&mut self) {
        self.filter_state = 0.0;
        self.prev_output = 0.0;
    }

    /// Process a single sample through the filter
    pub fn process(&mut self, input: f32) -> f32 {
        // Calculate filter coefficient using exponential formula
        // This gives a smooth, stable response
        let alpha = 1.0 - (-2.0 * std::f32::consts::PI * self.cutoff_freq / self.sample_rate).exp();
        let alpha_clamped = alpha.clamp(0.0, 0.99);

        // Apply resonance feedback from previous output
        // This creates a peak at the cutoff frequency
        let feedback = self.prev_output * self.resonance * 0.1;
        let input_with_feedback = input + feedback;

        // One-pole lowpass filter
        self.filter_state += alpha_clamped * (input_with_feedback - self.filter_state);

        // Apply resonance boost at cutoff frequency
        let resonance_boost = 1.0 + (self.resonance * 0.3);
        let output = self.filter_state * resonance_boost;

        // Store for next iteration
        self.prev_output = output;

        output
    }

    /// Set cutoff frequency
    pub fn set_cutoff_freq(&mut self, cutoff_freq: f32) {
        self.cutoff_freq = cutoff_freq.clamp(20.0, 20000.0);
    }

    /// Set resonance/Q
    pub fn set_resonance(&mut self, resonance: f32) {
        self.resonance = resonance.clamp(0.0, 10.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_creation() {
        let filter = ResonantLowpassFilter::new(44100.0, 1000.0, 2.0);
        assert_eq!(filter.sample_rate, 44100.0);
        assert_eq!(filter.cutoff_freq, 1000.0);
        assert_eq!(filter.resonance, 2.0);
    }

    #[test]
    fn test_filter_reset() {
        let mut filter = ResonantLowpassFilter::new(44100.0, 1000.0, 2.0);

        // Process some samples
        for _ in 0..100 {
            filter.process(1.0);
        }

        // Reset and verify
        filter.reset();
        assert_eq!(filter.filter_state, 0.0);
        assert_eq!(filter.prev_output, 0.0);
    }

    #[test]
    fn test_filter_stability() {
        let mut filter = ResonantLowpassFilter::new(44100.0, 1000.0, 4.0);

        // Process many samples to ensure stability
        for i in 0..44100 {
            let input = (i as f32 * 0.1).sin();
            let output = filter.process(input);
            assert!(output.is_finite(), "Filter output should be finite");
            assert!(output.abs() < 100.0, "Filter should remain stable");
        }
    }
}
