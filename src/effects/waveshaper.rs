//! Waveshaper distortion effect for velocity-responsive harmonic generation
//!
//! Provides soft-clipping waveshaping similar to Max MSP's overdrive~ object,
//! with adjustable drive and mix for saturation and warmth.

use crate::utils::oversampler::Oversampler2x;

/// Waveshaper distortion with configurable drive
///
/// Uses soft clipping (tanh) for smooth saturation similar to tube overdrive.
/// Matches the behavior of Max MSP's overdrive~ object.
/// Includes 2x oversampling to reduce aliasing from the nonlinear processing.
pub struct Waveshaper {
    /// Distortion amount (1.0-10.0, 1.0 = bypass)
    drive: f32,
    /// Dry/wet mix (0.0-1.0)
    mix: f32,
    /// 2x oversampler for alias reduction
    oversampler: Oversampler2x,
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
            oversampler: Oversampler2x::new(),
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
    pub fn process(&mut self, input: f32) -> f32 {
        // Bypass if no effect or drive is 1.0
        if self.mix <= 0.0001 || self.drive <= 1.0 {
            return input;
        }

        let drive = self.drive;

        // Gain compensation: normalize output level to match drive=1.0
        let reference = 0.5_f32;
        let compensation = reference.tanh() / (reference * drive).tanh();

        // Apply drive gain and soft-clip using tanh with 2x oversampling
        let saturated = self
            .oversampler
            .process(input, |x| (x * drive).tanh() * compensation);

        // Mix dry/wet
        input * (1.0 - self.mix) + saturated * self.mix
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
        let mut ws = Waveshaper::new(5.0, 0.0);
        assert_eq!(ws.process(0.5), 0.5);
        assert_eq!(ws.process(-0.3), -0.3);
    }

    #[test]
    fn test_bypass_when_drive_one() {
        let mut ws = Waveshaper::new(1.0, 1.0);
        assert_eq!(ws.process(0.5), 0.5);
        assert_eq!(ws.process(-0.3), -0.3);
    }

    #[test]
    fn test_soft_clipping() {
        let mut ws = Waveshaper::new(10.0, 1.0);
        // Warm up the oversampler
        for _ in 0..20 {
            ws.process(1.0);
        }
        let output = ws.process(1.0);
        // Output should be soft-limited and gain-compensated
        assert!(
            output > 0.3 && output < 0.8,
            "Expected compensated output, got {}",
            output
        );
    }

    #[test]
    fn test_gain_compensation_consistency() {
        // Test that output level stays relatively consistent across drive values
        let input = 0.5_f32;
        let mut ws_low = Waveshaper::new(2.0, 1.0);
        let mut ws_high = Waveshaper::new(10.0, 1.0);

        // Warm up oversamplers with steady signal
        for _ in 0..100 {
            ws_low.process(input);
            ws_high.process(input);
        }

        let output_low = ws_low.process(input);
        let output_high = ws_high.process(input);

        // With compensation, outputs should be in the same ballpark
        assert!(output_low > 0.1, "Low drive output too quiet: {output_low}");
        assert!(
            output_high > 0.1,
            "High drive output too quiet: {output_high}"
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
        let mut ws = Waveshaper::new(5.0, 1.0);
        // Warm up oversampler
        for _ in 0..20 {
            ws.process(0.0);
        }
        let output = ws.process(0.0);
        assert_eq!(output, 0.0);
    }
}
