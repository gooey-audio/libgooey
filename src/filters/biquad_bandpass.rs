use std::f32::consts::PI;

/// Biquad Bandpass Filter - RBJ Audio EQ Cookbook implementation
///
/// Implements the "constant-gain bandpass" (gainbpass) filter type matching
/// Max/MSP's `filtercoeff~ gainbpass` + `biquad~` combination.
///
/// The filter uses Direct Form I processing with coefficient calculation
/// based on the Robert Bristow-Johnson Audio EQ Cookbook.
pub struct BiquadBandpass {
    sample_rate: f32,

    // Normalized coefficients
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,

    // State variables (delay line)
    x1: f32, // x[n-1]
    x2: f32, // x[n-2]
    y1: f32, // y[n-1]
    y2: f32, // y[n-2]

    // Cached parameters for coefficient update optimization
    last_freq: f32,
    last_q: f32,
    last_gain: f32,
}

impl BiquadBandpass {
    /// Create a new biquad bandpass filter
    ///
    /// # Arguments
    /// * `sample_rate` - Audio sample rate in Hz
    pub fn new(sample_rate: f32) -> Self {
        let mut filter = Self {
            sample_rate,
            b0: 0.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
            last_freq: -1.0,
            last_q: -1.0,
            last_gain: -1.0,
        };
        // Initialize with default params
        filter.set_params(1000.0, 1.0, 1.0);
        filter
    }

    /// Reset filter state (clear delay line)
    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }

    /// Set filter parameters and recalculate coefficients if changed
    ///
    /// # Arguments
    /// * `freq` - Center frequency in Hz (clamped to 20-Nyquist)
    /// * `q` - Q factor / resonance (clamped to 0.1-100)
    /// * `gain` - Linear gain multiplier
    #[inline]
    pub fn set_params(&mut self, freq: f32, q: f32, gain: f32) {
        // Check if parameters changed (with small tolerance)
        if (freq - self.last_freq).abs() < 0.01
            && (q - self.last_q).abs() < 0.001
            && (gain - self.last_gain).abs() < 0.001
        {
            return;
        }

        self.last_freq = freq;
        self.last_q = q;
        self.last_gain = gain;

        self.calculate_coefficients(freq, q, gain);
    }

    /// Calculate biquad coefficients using RBJ constant-gain bandpass formula
    fn calculate_coefficients(&mut self, freq: f32, q: f32, gain: f32) {
        let nyquist = self.sample_rate * 0.5;

        // Clamp frequency to valid range
        let freq = freq.clamp(20.0, nyquist * 0.95);
        let q = q.clamp(0.1, 100.0);

        // Angular frequency
        let omega0 = 2.0 * PI * freq / self.sample_rate;
        let sin_omega = omega0.sin();
        let cos_omega = omega0.cos();

        // Bandwidth parameter
        let alpha = sin_omega / (2.0 * q);

        // RBJ constant-gain bandpass coefficients
        // This maintains unity gain at the center frequency
        let b0 = q * alpha * gain;
        let b1 = 0.0;
        let b2 = -q * alpha * gain;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_omega;
        let a2 = 1.0 - alpha;

        // Normalize by a0
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    /// Process a single sample through the filter
    ///
    /// Uses Direct Form I: y[n] = b0*x[n] + b1*x[n-1] + b2*x[n-2] - a1*y[n-1] - a2*y[n-2]
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        // Direct Form I difference equation
        let output = self.b0 * input + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;

        // Update delay line
        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        self.y1 = output;

        // Flush denormals
        if output.abs() < 1e-15 {
            return 0.0;
        }

        output
    }

    /// Get current center frequency
    pub fn freq(&self) -> f32 {
        self.last_freq
    }

    /// Get current Q factor
    pub fn q(&self) -> f32 {
        self.last_q
    }

    /// Get current gain
    pub fn gain(&self) -> f32 {
        self.last_gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_biquad_creation() {
        let filter = BiquadBandpass::new(44100.0);
        assert_eq!(filter.sample_rate, 44100.0);
    }

    #[test]
    fn test_biquad_reset() {
        let mut filter = BiquadBandpass::new(44100.0);
        // Process some samples
        for _ in 0..100 {
            filter.process(1.0);
        }
        // Reset
        filter.reset();
        assert_eq!(filter.x1, 0.0);
        assert_eq!(filter.x2, 0.0);
        assert_eq!(filter.y1, 0.0);
        assert_eq!(filter.y2, 0.0);
    }

    #[test]
    fn test_bandpass_attenuates_dc() {
        let mut filter = BiquadBandpass::new(44100.0);
        filter.set_params(1000.0, 1.0, 1.0);

        // Feed DC signal - bandpass should attenuate it
        let mut output = 0.0;
        for _ in 0..1000 {
            output = filter.process(1.0);
        }
        // DC should be heavily attenuated
        assert!(output.abs() < 0.1);
    }

    #[test]
    fn test_coefficient_caching() {
        let mut filter = BiquadBandpass::new(44100.0);
        filter.set_params(1000.0, 1.0, 1.0);
        let b0_first = filter.b0;

        // Same params should not recalculate
        filter.set_params(1000.0, 1.0, 1.0);
        assert_eq!(filter.b0, b0_first);

        // Different params should recalculate
        filter.set_params(2000.0, 1.0, 1.0);
        assert_ne!(filter.b0, b0_first);
    }
}
