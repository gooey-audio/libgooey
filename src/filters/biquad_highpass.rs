use std::f32::consts::PI;

/// Biquad Highpass Filter - RBJ Audio EQ Cookbook implementation
///
/// Implements a 2nd order highpass filter suitable for percussive tone shaping.
/// Uses Direct Form I processing with coefficient calculation based on the
/// Robert Bristow-Johnson Audio EQ Cookbook.
pub struct BiquadHighpass {
    sample_rate: f32,

    // Normalized coefficients
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,

    // State variables (delay line)
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,

    // Cached parameters for coefficient update optimization
    last_freq: f32,
    last_q: f32,
}

impl BiquadHighpass {
    /// Create a new biquad highpass filter
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
        };
        // Initialize with default params
        filter.set_params(1000.0, 1.0);
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
    /// * `freq` - Cutoff frequency in Hz (clamped to 20-Nyquist)
    /// * `q` - Q factor / resonance (clamped to 0.1-100)
    #[inline]
    pub fn set_params(&mut self, freq: f32, q: f32) {
        if (freq - self.last_freq).abs() < 0.01 && (q - self.last_q).abs() < 0.001 {
            return;
        }

        self.last_freq = freq;
        self.last_q = q;

        self.calculate_coefficients(freq, q);
    }

    fn calculate_coefficients(&mut self, freq: f32, q: f32) {
        let nyquist = self.sample_rate * 0.5;

        let freq = freq.clamp(20.0, nyquist * 0.95);
        let q = q.clamp(0.1, 100.0);

        let omega0 = 2.0 * PI * freq / self.sample_rate;
        let sin_omega = omega0.sin();
        let cos_omega = omega0.cos();

        let alpha = sin_omega / (2.0 * q);

        // RBJ highpass coefficients
        let b0 = (1.0 + cos_omega) / 2.0;
        let b1 = -(1.0 + cos_omega);
        let b2 = (1.0 + cos_omega) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_omega;
        let a2 = 1.0 - alpha;

        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    /// Process a single sample through the filter
    ///
    /// Direct Form I: y[n] = b0*x[n] + b1*x[n-1] + b2*x[n-2] - a1*y[n-1] - a2*y[n-2]
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let output = self.b0 * input + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;

        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        self.y1 = output;

        if output.abs() < 1e-15 {
            return 0.0;
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_biquad_creation() {
        let filter = BiquadHighpass::new(44100.0);
        assert_eq!(filter.sample_rate, 44100.0);
    }

    #[test]
    fn test_biquad_reset() {
        let mut filter = BiquadHighpass::new(44100.0);
        for _ in 0..100 {
            filter.process(1.0);
        }
        filter.reset();
        assert_eq!(filter.x1, 0.0);
        assert_eq!(filter.x2, 0.0);
        assert_eq!(filter.y1, 0.0);
        assert_eq!(filter.y2, 0.0);
    }

    #[test]
    fn test_highpass_attenuates_dc() {
        let mut filter = BiquadHighpass::new(44100.0);
        filter.set_params(1000.0, 1.0);

        let mut output = 0.0;
        for _ in 0..2000 {
            output = filter.process(1.0);
        }
        assert!(output.abs() < 0.1);
    }
}
