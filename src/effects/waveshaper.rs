//! Waveshaper distortion effect for velocity-responsive harmonic generation
//!
//! Provides asymmetric waveshaping with adjustable drive and mix for rich,
//! complex harmonic content suitable for acoustic-like instrument response.

/// Waveshaper distortion with configurable drive and asymmetry
///
/// Uses a combination of soft clipping (tanh) and polynomial waveshaping
/// for rich harmonic content. Asymmetry adds even harmonics for warmth.
pub struct Waveshaper {
    /// Distortion amount (1.0-10.0)
    drive: f32,
    /// Dry/wet mix (0.0-1.0)
    mix: f32,
    /// Positive bias for even harmonics (0.0-1.0)
    asymmetry: f32,
}

impl Waveshaper {
    /// Create a new waveshaper with the given parameters
    ///
    /// # Arguments
    /// * `drive` - Distortion amount (1.0-10.0, clamped)
    /// * `mix` - Dry/wet mix (0.0-1.0, clamped)
    /// * `asymmetry` - Even harmonic bias (0.0-1.0, clamped)
    pub fn new(drive: f32, mix: f32, asymmetry: f32) -> Self {
        Self {
            drive: drive.clamp(1.0, 10.0),
            mix: mix.clamp(0.0, 1.0),
            asymmetry: asymmetry.clamp(0.0, 1.0),
        }
    }

    /// Create a waveshaper with default settings (minimal distortion)
    pub fn default() -> Self {
        Self::new(1.0, 0.0, 0.3)
    }

    /// Process a single sample through the waveshaper
    ///
    /// The waveshaping algorithm:
    /// 1. Apply drive gain to input
    /// 2. Add asymmetric bias for even harmonics
    /// 3. Soft clip with tanh (smooth saturation)
    /// 4. Add polynomial shaping for complex harmonics
    /// 5. Remove DC offset from asymmetry
    /// 6. Mix with dry signal
    #[inline]
    pub fn process(&self, input: f32) -> f32 {
        // Bypass if no effect
        if self.mix <= 0.0001 {
            return input;
        }

        // Apply drive
        let driven = input * self.drive;

        // Asymmetric bias for even harmonics (adds warmth)
        let biased = driven + self.asymmetry * 0.3 * driven.abs();

        // Soft clip with tanh (smooth saturation)
        let soft_clipped = biased.tanh();

        // Add polynomial shaping for more complex harmonics
        // f(x) = x - (x^3 / 3) gives a smooth S-curve
        let poly_shaped = soft_clipped - (soft_clipped.powi(3) / 3.0);

        // Remove DC offset introduced by asymmetry
        let dc_blocked = poly_shaped - self.asymmetry * 0.1;

        // Mix dry/wet
        input * (1.0 - self.mix) + dc_blocked * self.mix
    }

    /// Set the drive amount (1.0-10.0)
    pub fn set_drive(&mut self, drive: f32) {
        self.drive = drive.clamp(1.0, 10.0);
    }

    /// Get the current drive amount
    pub fn drive(&self) -> f32 {
        self.drive
    }

    /// Set the dry/wet mix (0.0-1.0)
    pub fn set_mix(&mut self, mix: f32) {
        self.mix = mix.clamp(0.0, 1.0);
    }

    /// Get the current mix amount
    pub fn mix(&self) -> f32 {
        self.mix
    }

    /// Set the asymmetry for even harmonics (0.0-1.0)
    pub fn set_asymmetry(&mut self, asymmetry: f32) {
        self.asymmetry = asymmetry.clamp(0.0, 1.0);
    }

    /// Get the current asymmetry amount
    pub fn asymmetry(&self) -> f32 {
        self.asymmetry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bypass_when_mix_zero() {
        let ws = Waveshaper::new(5.0, 0.0, 0.5);
        assert_eq!(ws.process(0.5), 0.5);
        assert_eq!(ws.process(-0.3), -0.3);
    }

    #[test]
    fn test_soft_clipping() {
        let ws = Waveshaper::new(10.0, 1.0, 0.0);
        // High input should be soft-clipped below 1.0
        let output = ws.process(1.0);
        assert!(output < 1.0);
        assert!(output > 0.0);
    }

    #[test]
    fn test_parameter_clamping() {
        let ws = Waveshaper::new(100.0, 5.0, -1.0);
        assert_eq!(ws.drive(), 10.0);
        assert_eq!(ws.mix(), 1.0);
        assert_eq!(ws.asymmetry(), 0.0);
    }

    #[test]
    fn test_zero_input() {
        let ws = Waveshaper::new(5.0, 1.0, 0.5);
        let output = ws.process(0.0);
        // Should be close to zero (small DC offset from asymmetry)
        assert!(output.abs() < 0.1);
    }
}
