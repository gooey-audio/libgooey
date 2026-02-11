//! Waveshaper distortion effect for velocity-responsive harmonic generation
//!
//! Provides soft-clipping waveshaping similar to Max MSP's overdrive~ object,
//! with adjustable drive and mix for saturation and warmth.

/// Waveshaper distortion with configurable drive
///
/// Uses soft clipping (tanh) for smooth saturation similar to tube overdrive.
/// Matches the behavior of Max MSP's overdrive~ object.
pub struct Waveshaper {
    /// Distortion amount (1.0-10.0, 1.0 = bypass)
    drive: f32,
    /// Dry/wet mix (0.0-1.0)
    mix: f32,
}

impl Waveshaper {
    /// Create a new waveshaper with the given parameters
    ///
    /// # Arguments
    /// * `drive` - Distortion amount (1.0-10.0, clamped, 1.0 = bypass)
    /// * `mix` - Dry/wet mix (0.0-1.0, clamped)
    pub fn new(drive: f32, mix: f32) -> Self {
        Self {
            drive: drive.clamp(1.0, 10.0),
            mix: mix.clamp(0.0, 1.0),
        }
    }

    /// Create a waveshaper with default settings (bypass)
    pub fn default() -> Self {
        Self::new(1.0, 0.0)
    }

    /// Process a single sample through the waveshaper
    ///
    /// The waveshaping algorithm matches Max MSP's overdrive~:
    /// 1. Apply drive gain to input
    /// 2. Soft clip with tanh (smooth saturation to ±1)
    /// 3. Mix with dry signal
    #[inline]
    pub fn process(&self, input: f32) -> f32 {
        // Bypass if no effect or drive is 1.0
        if self.mix <= 0.0001 || self.drive <= 1.0 {
            return input;
        }

        // Apply drive gain and soft-clip using tanh
        // tanh provides smooth saturation similar to tube/analog overdrive
        let saturated = (input * self.drive).tanh();

        // Gain compensation: normalize output level to match drive=1.0
        // Uses reference level (typical signal amplitude) to calculate
        // how much quieter the output should be to maintain constant loudness
        let reference = 0.5_f32;
        let compensation = reference.tanh() / (reference * self.drive).tanh();
        let compensated = saturated * compensation;

        // Mix dry/wet
        input * (1.0 - self.mix) + compensated * self.mix
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bypass_when_mix_zero() {
        let ws = Waveshaper::new(5.0, 0.0);
        assert_eq!(ws.process(0.5), 0.5);
        assert_eq!(ws.process(-0.3), -0.3);
    }

    #[test]
    fn test_bypass_when_drive_one() {
        let ws = Waveshaper::new(1.0, 1.0);
        assert_eq!(ws.process(0.5), 0.5);
        assert_eq!(ws.process(-0.3), -0.3);
    }

    #[test]
    fn test_soft_clipping() {
        let ws = Waveshaper::new(10.0, 1.0);
        // High drive with gain compensation should produce gain-normalized output
        // The compensation factor at drive=10 is approx tanh(0.5)/tanh(5.0) ≈ 0.46
        let output = ws.process(1.0);
        assert!(output > 0.4 && output < 0.5); // Compensated output near 0.46

        // Lower input should be proportionally less saturated
        let output_low = ws.process(0.1);
        assert!(output_low > 0.0);
        assert!(output_low < output); // Lower input = lower output
    }

    #[test]
    fn test_gain_compensation_consistency() {
        // Test that output level stays relatively consistent across drive values
        let input = 0.5_f32;
        let ws_low = Waveshaper::new(2.0, 1.0);
        let ws_high = Waveshaper::new(10.0, 1.0);

        let output_low = ws_low.process(input);
        let output_high = ws_high.process(input);

        // With compensation, outputs should be similar (within 20% of each other)
        let ratio = output_high / output_low;
        assert!(
            ratio > 0.8 && ratio < 1.2,
            "Outputs differ too much: {output_low} vs {output_high}"
        );
    }

    #[test]
    fn test_parameter_clamping() {
        let ws = Waveshaper::new(100.0, 5.0);
        assert_eq!(ws.drive(), 10.0);
        assert_eq!(ws.mix(), 1.0);
    }

    #[test]
    fn test_zero_input() {
        let ws = Waveshaper::new(5.0, 1.0);
        let output = ws.process(0.0);
        // Should be exactly zero with no asymmetry
        assert_eq!(output, 0.0);
    }
}
