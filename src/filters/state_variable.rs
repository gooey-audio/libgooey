use std::f32::consts::PI;

/// State Variable Filter - 2nd order resonant filter
///
/// Classic SVF topology using the Chamberlin form that produces lowpass,
/// bandpass, and highpass outputs simultaneously. This implementation
/// provides efficient multi-mode filtering suitable for drum synthesis.
///
/// The filter is particularly useful for snare drum noise shaping,
/// where the bandpass output creates the characteristic "crack" sound.
pub struct StateVariableFilter {
    pub sample_rate: f32,
    pub cutoff_freq: f32,
    pub resonance: f32, // Q factor: 0.5 = no resonance, higher = more resonant

    // Filter state variables
    low: f32,  // Lowpass output (integrator 1)
    band: f32, // Bandpass output (integrator 2)

    // Cached coefficients
    f: f32, // Frequency coefficient
    q: f32, // Damping (1/resonance)
}

impl StateVariableFilter {
    /// Create a new State Variable Filter
    ///
    /// # Arguments
    /// * `sample_rate` - The audio sample rate in Hz
    /// * `cutoff_freq` - Initial cutoff frequency in Hz (20-20000)
    /// * `resonance` - Q factor, 0.5 = no resonance, 2.0+ = sharp resonance
    pub fn new(sample_rate: f32, cutoff_freq: f32, resonance: f32) -> Self {
        let mut filter = Self {
            sample_rate,
            cutoff_freq: cutoff_freq.clamp(20.0, 20000.0),
            resonance: resonance.max(0.5),
            low: 0.0,
            band: 0.0,
            f: 0.0,
            q: 0.0,
        };
        filter.update_coefficients();
        filter
    }

    /// Reset filter state (clear history)
    pub fn reset(&mut self) {
        self.low = 0.0;
        self.band = 0.0;
    }

    /// Update internal coefficients from cutoff and resonance
    fn update_coefficients(&mut self) {
        // Chamberlin SVF coefficients
        // f = 2 * sin(π * cutoff / sample_rate)
        // Using the approximation for stability at high frequencies
        let normalized_freq = (self.cutoff_freq / self.sample_rate).min(0.45);
        self.f = 2.0 * (PI * normalized_freq).sin();
        self.q = 1.0 / self.resonance;
    }

    /// Process a single sample and return bandpass output
    ///
    /// This is the most common use case for drum synthesis (snare crack).
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        // Chamberlin SVF algorithm (2x oversampled for stability)
        for _ in 0..2 {
            self.low = self.low + self.f * self.band;
            let high = input - self.low - self.q * self.band;
            self.band = self.f * high + self.band;
        }

        self.band
    }

    /// Process a single sample and return all three outputs
    ///
    /// Returns (lowpass, bandpass, highpass) tuple
    #[inline]
    pub fn process_all(&mut self, input: f32) -> (f32, f32, f32) {
        // Chamberlin SVF algorithm (2x oversampled for stability)
        let mut high = 0.0;
        for _ in 0..2 {
            self.low = self.low + self.f * self.band;
            high = input - self.low - self.q * self.band;
            self.band = self.f * high + self.band;
        }

        (self.low, self.band, high)
    }

    /// Process a single sample with selectable output mode
    ///
    /// # Arguments
    /// * `input` - Input sample
    /// * `filter_type` - 0=lowpass, 1=bandpass, 2=highpass, 3=notch
    #[inline]
    pub fn process_mode(&mut self, input: f32, filter_type: u8) -> f32 {
        let (low, band, high) = self.process_all(input);

        match filter_type {
            0 => low,        // Lowpass
            1 => band,       // Bandpass
            2 => high,       // Highpass
            3 => low + high, // Notch (reject band)
            _ => band,       // Default to bandpass
        }
    }

    /// Set cutoff frequency in Hz
    pub fn set_cutoff_freq(&mut self, cutoff_freq: f32) {
        let new_cutoff = cutoff_freq.clamp(20.0, 20000.0);
        if (new_cutoff - self.cutoff_freq).abs() > 0.001 {
            self.cutoff_freq = new_cutoff;
            self.update_coefficients();
        }
    }

    /// Set resonance (Q factor)
    /// 0.5 = no resonance, 2.0+ = sharp resonance
    pub fn set_resonance(&mut self, resonance: f32) {
        let new_resonance = resonance.max(0.5);
        if (new_resonance - self.resonance).abs() > 0.001 {
            self.resonance = new_resonance;
            self.update_coefficients();
        }
    }

    /// Set both cutoff and resonance at once (more efficient)
    pub fn set_params(&mut self, cutoff_freq: f32, resonance: f32) {
        self.cutoff_freq = cutoff_freq.clamp(20.0, 20000.0);
        self.resonance = resonance.max(0.5);
        self.update_coefficients();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_svf_creation() {
        let filter = StateVariableFilter::new(44100.0, 1000.0, 1.0);
        assert_eq!(filter.sample_rate, 44100.0);
        assert_eq!(filter.cutoff_freq, 1000.0);
        assert_eq!(filter.resonance, 1.0);
    }

    #[test]
    fn test_svf_clamps_cutoff() {
        let filter = StateVariableFilter::new(44100.0, 30000.0, 1.0);
        assert_eq!(filter.cutoff_freq, 20000.0);

        let filter2 = StateVariableFilter::new(44100.0, 10.0, 1.0);
        assert_eq!(filter2.cutoff_freq, 20.0);
    }

    #[test]
    fn test_svf_reset() {
        let mut filter = StateVariableFilter::new(44100.0, 1000.0, 1.0);
        // Process some samples to build up state
        for _ in 0..100 {
            filter.process(1.0);
        }
        // Reset and verify state is cleared
        filter.reset();
        assert_eq!(filter.low, 0.0);
        assert_eq!(filter.band, 0.0);
    }

    #[test]
    fn test_svf_bandpass_output() {
        let mut filter = StateVariableFilter::new(44100.0, 1000.0, 2.0);
        // Bandpass should attenuate DC input over time
        let mut output = 0.0;
        for _ in 0..1000 {
            output = filter.process(1.0);
        }
        // Bandpass of DC should approach 0
        assert!(output.abs() < 0.1);
    }

    #[test]
    fn test_svf_all_outputs() {
        let mut filter = StateVariableFilter::new(44100.0, 1000.0, 1.0);
        let (low, band, high) = filter.process_all(1.0);
        // First sample should produce some output
        assert!(low != 0.0 || band != 0.0 || high != 0.0);
    }
}
