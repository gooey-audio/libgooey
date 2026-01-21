//! Soft saturation effect based on musicdsp.org algorithm
//!
//! This implements the soft saturation waveshaper by Bram de Jong (2002).
//! It provides smooth saturation with controllable threshold, useful for
//! adding warmth and harmonics to audio signals.
//!
//! # Smoothing Convention
//!
//! This effect does NOT perform internal parameter smoothing. Smoothing is the
//! responsibility of the caller (see [`crate::utils::SmoothedParam`]).
//!
//! For audio thread hot paths, use [`SoftSaturation::process_with_params`] to
//! pass parameters directly, avoiding atomic operations per sample.
//!
//! # Output Limiting
//!
//! The normalization factor can increase gain at high saturation levels (up to
//! +6dB at threshold=0). To prevent clipping, a soft limiter is applied to the
//! output, clamping to [-1.0, 1.0] with a gentle knee.

use crate::effects::Effect;
use std::sync::atomic::{AtomicU32, Ordering};

/// Soft saturation effect using the musicdsp.org algorithm
///
/// The algorithm operates in three regions based on threshold parameter `a`:
/// - Linear region (x < a): Signal passes through unchanged
/// - Soft curve region (a <= x <= 1): Smooth saturation applied
/// - Clipping region (x > 1): Hard limit at (a+1)/2
///
/// Output is normalized to maintain consistent perceived level, then soft-limited
/// to prevent clipping.
pub struct SoftSaturation {
    /// Threshold parameter 'a' (0-1), stored as f32 bits for atomic access.
    /// Used when processing via the Effect trait. For hot path processing,
    /// use `process_with_params` instead to avoid atomic operations.
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
    ///
    /// Note: For audio thread hot paths, prefer `process_with_params` to avoid
    /// atomic operations on every sample.
    pub fn set_threshold(&self, threshold: f32) {
        self.threshold
            .store(threshold.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// Get the current threshold setting
    pub fn get_threshold(&self) -> f32 {
        f32::from_bits(self.threshold.load(Ordering::Relaxed))
    }

    /// Process a sample with explicit threshold parameter
    ///
    /// This is the preferred method for audio thread hot paths as it avoids
    /// atomic operations. The caller is responsible for smoothing the threshold
    /// parameter to prevent clicks.
    ///
    /// # Arguments
    /// * `input` - Input sample
    /// * `threshold` - Saturation threshold (0-1), typically smoothed by caller
    #[inline]
    pub fn process_with_params(input: f32, threshold: f32) -> f32 {
        Self::saturate(input, threshold)
    }

    /// Core saturation algorithm from musicdsp.org
    ///
    /// Handles negative values symmetrically, applies normalization for
    /// consistent perceived level, and soft-limits output to prevent clipping.
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

        // Normalize output to maintain consistent perceived level
        // At x=1, the soft curve outputs (a+1)/2, so we multiply by 2/(a+1)
        // This can add up to +6dB gain at a=0
        let normalization = 2.0 / (a + 1.0);
        let normalized = sign * saturated * normalization;

        // Soft limit output to prevent clipping from normalization gain
        // Uses tanh-like curve that preserves signal near zero but limits peaks
        Self::soft_limit(normalized)
    }

    /// Soft limiter to prevent clipping
    ///
    /// Uses a polynomial approximation that:
    /// - Is transparent for |x| < 0.8
    /// - Smoothly limits to [-1, 1] for larger values
    /// - Preserves zero crossing and symmetry
    #[inline]
    fn soft_limit(x: f32) -> f32 {
        let abs_x = x.abs();
        if abs_x <= 0.8 {
            // Linear region: pass through unchanged
            x
        } else if abs_x < 1.5 {
            // Soft knee region: cubic curve from 0.8 to ~1.0
            // Maps [0.8, 1.5] -> [0.8, ~0.98]
            let t = (abs_x - 0.8) / 0.7; // 0 to 1 over the knee
            let limited = 0.8 + 0.2 * t * (3.0 - 2.0 * t); // Hermite interpolation
            x.signum() * limited
        } else {
            // Hard limit for extreme values
            x.signum() * 0.98
        }
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
        // Output of 0.4 is below soft limit threshold of 0.8
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

    #[test]
    fn test_output_limiting() {
        // At max saturation with hot signal, output should not exceed 1.0
        let sat = SoftSaturation::new(0.0);
        let output = sat.process(5.0);
        assert!(
            output.abs() <= 1.0,
            "Output should be limited to [-1, 1], got {}",
            output
        );

        // Test negative values too
        let output_neg = sat.process(-5.0);
        assert!(
            output_neg.abs() <= 1.0,
            "Negative output should be limited to [-1, 1], got {}",
            output_neg
        );
    }

    #[test]
    fn test_process_with_params_matches_trait() {
        let sat = SoftSaturation::new(0.3);
        let input = 0.7;

        let trait_output = sat.process(input);
        let direct_output = SoftSaturation::process_with_params(input, 0.3);

        assert!(
            (trait_output - direct_output).abs() < 1e-6,
            "process_with_params should match trait process"
        );
    }
}
