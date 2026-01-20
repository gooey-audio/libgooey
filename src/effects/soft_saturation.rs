//! Soft saturation effect based on musicdsp.org algorithm
//!
//! This implements the soft saturation waveshaper by Bram de Jong (2002).
//! It provides smooth saturation with controllable threshold, useful for
//! adding warmth and harmonics to audio signals.

use crate::effects::Effect;
use std::sync::atomic::{AtomicU32, Ordering};

/// Soft saturation effect using the musicdsp.org algorithm
///
/// The algorithm operates in three regions based on threshold parameter `a`:
/// - Linear region (x < a): Signal passes through unchanged
/// - Soft curve region (a <= x <= 1): Smooth saturation applied
/// - Clipping region (x > 1): Hard limit at (a+1)/2
///
/// Output is normalized to maintain consistent level across threshold settings.
pub struct SoftSaturation {
    /// Threshold parameter 'a' (0-1), stored as f32 bits for atomic access
    threshold: AtomicU32,
}

// SAFETY: AtomicU32 is inherently thread-safe
unsafe impl Send for SoftSaturation {}
unsafe impl Sync for SoftSaturation {}

impl SoftSaturation {
    /// Create a new soft saturation effect
    ///
    /// # Arguments
    /// * `threshold` - Saturation threshold (0-1). Higher values = less saturation.
    ///   At threshold=1.0, signal passes through unchanged (bypass).
    ///   At threshold=0.0, maximum saturation is applied.
    pub fn new(threshold: f32) -> Self {
        Self {
            threshold: AtomicU32::new(threshold.clamp(0.0, 1.0).to_bits()),
        }
    }

    /// Set the saturation threshold (0-1)
    ///
    /// Higher threshold = less saturation (more linear region)
    /// Lower threshold = more saturation (signal enters soft curve sooner)
    pub fn set_threshold(&self, threshold: f32) {
        self.threshold
            .store(threshold.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// Get the current threshold setting
    pub fn get_threshold(&self) -> f32 {
        f32::from_bits(self.threshold.load(Ordering::Relaxed))
    }

    /// Core saturation algorithm from musicdsp.org
    ///
    /// Handles negative values symmetrically and applies normalization
    /// for consistent output level.
    #[inline]
    fn saturate(x: f32, a: f32) -> f32 {
        // Bypass when threshold is at maximum
        if a >= 1.0 {
            return x;
        }

        let sign = x.signum();
        let abs_x = x.abs();

        let saturated = if abs_x < a {
            // Linear region: signal passes through unchanged
            abs_x
        } else if abs_x <= 1.0 {
            // Soft curve region: a + (x-a)/(1+((x-a)/(1-a))^2)
            let x_minus_a = abs_x - a;
            let one_minus_a = 1.0 - a;
            let ratio = x_minus_a / one_minus_a;
            a + x_minus_a / (1.0 + ratio * ratio)
        } else {
            // Clipping region: hard limit
            (a + 1.0) / 2.0
        };

        // Normalize output to maintain consistent level
        // At x=1, the soft curve outputs (a+1)/2, so we multiply by 2/(a+1)
        let normalization = 2.0 / (a + 1.0);
        sign * saturated * normalization
    }
}

impl Effect for SoftSaturation {
    fn process(&self, input: f32) -> f32 {
        let a = self.get_threshold();
        Self::saturate(input, a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bypass_at_max_threshold() {
        let sat = SoftSaturation::new(1.0);
        assert_eq!(sat.process(0.5), 0.5);
        assert_eq!(sat.process(-0.3), -0.3);
        assert_eq!(sat.process(0.0), 0.0);
    }

    #[test]
    fn test_soft_limiting() {
        let sat = SoftSaturation::new(0.0);
        // With max saturation (a=0), high values should be limited
        let output = sat.process(2.0);
        assert!(output < 2.0, "Saturation should reduce amplitude");
        assert!(output > 0.0, "Output should be positive for positive input");
    }

    #[test]
    fn test_symmetry() {
        let sat = SoftSaturation::new(0.5);
        let positive = sat.process(0.7);
        let negative = sat.process(-0.7);
        assert!(
            (positive + negative).abs() < 1e-6,
            "Should be antisymmetric"
        );
    }

    #[test]
    fn test_linear_region() {
        let sat = SoftSaturation::new(0.5);
        // Values below threshold should be in linear region (scaled by normalization)
        let input = 0.3;
        let output = sat.process(input);
        // With a=0.5, normalization = 2/(0.5+1) = 1.333...
        let expected = input * (2.0 / 1.5);
        assert!(
            (output - expected).abs() < 1e-6,
            "Linear region should scale by normalization factor"
        );
    }

    #[test]
    fn test_parameter_clamping() {
        let sat = SoftSaturation::new(5.0);
        assert_eq!(sat.get_threshold(), 1.0);

        sat.set_threshold(-1.0);
        assert_eq!(sat.get_threshold(), 0.0);
    }
}
