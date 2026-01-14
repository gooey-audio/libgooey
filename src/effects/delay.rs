//! Delay effect with feedback
//!
//! This module provides a simple delay effect for the global effects chain.
//! Parameters are smoothed to prevent clicks/pops when adjusting time, feedback, and mix.

use crate::effects::Effect;
use crate::utils::smoother::SmoothedParam;
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU32, Ordering};

/// Maximum delay time in seconds
const MAX_DELAY_TIME: f32 = 5.0;

/// Threshold for flushing denormal numbers to zero
const DENORMAL_THRESHOLD: f32 = 1e-15;

/// Internal mutable state for the delay (wrapped in UnsafeCell for interior mutability)
struct DelayState {
    // Circular buffer for delay line
    buffer: Vec<f32>,
    write_index: usize,

    // Smoothed parameters (updated per-sample for click-free changes)
    time_smoothed: SmoothedParam,
    feedback_smoothed: SmoothedParam,
    mix_smoothed: SmoothedParam,
}

/// Simple delay effect with feedback
///
/// Parameters:
/// - Time: Delay time in seconds (0.0 to 5.0s)
/// - Feedback: Amount of delayed signal fed back (0.0 to 0.95)
/// - Mix: Wet/dry mix (0.0 = dry only, 1.0 = wet only)
///
/// This delay uses internal parameter smoothing to prevent audio artifacts
/// when parameters are changed during playback.
pub struct DelayEffect {
    sample_rate: f32,

    // Mutable state wrapped in UnsafeCell for interior mutability
    // SAFETY: This is only accessed from the audio thread during process()
    state: UnsafeCell<DelayState>,

    // Atomic parameters for lock-free updates from control thread
    // Uses bit representation of f32 for atomic operations
    time_target: AtomicU32,
    feedback_target: AtomicU32,
    mix_target: AtomicU32,
}

// SAFETY: The UnsafeCell is only accessed from a single audio thread
// The AtomicU32 fields are inherently thread-safe
unsafe impl Send for DelayEffect {}
unsafe impl Sync for DelayEffect {}

impl DelayEffect {
    /// Create a new delay effect
    ///
    /// # Arguments
    /// * `sample_rate` - Audio sample rate in Hz
    /// * `time` - Initial delay time in seconds (0.0-5.0)
    /// * `feedback` - Initial feedback amount (0.0-0.95)
    /// * `mix` - Initial wet/dry mix (0.0-1.0)
    pub fn new(sample_rate: f32, time: f32, feedback: f32, mix: f32) -> Self {
        let time_clamped = time.clamp(0.0, MAX_DELAY_TIME);
        let feedback_clamped = feedback.clamp(0.0, 0.95);
        let mix_clamped = mix.clamp(0.0, 1.0);

        // Allocate buffer for maximum delay time
        let buffer_size = (sample_rate * MAX_DELAY_TIME) as usize + 1;

        Self {
            sample_rate,
            state: UnsafeCell::new(DelayState {
                buffer: vec![0.0; buffer_size],
                write_index: 0,
                // Use 50ms smoothing for delay time to avoid zipper noise
                time_smoothed: SmoothedParam::new(
                    time_clamped,
                    0.0,
                    MAX_DELAY_TIME,
                    sample_rate,
                    50.0,
                ),
                // Use 30ms smoothing for feedback and mix
                feedback_smoothed: SmoothedParam::new(
                    feedback_clamped,
                    0.0,
                    0.95,
                    sample_rate,
                    30.0,
                ),
                mix_smoothed: SmoothedParam::new(mix_clamped, 0.0, 1.0, sample_rate, 30.0),
            }),
            time_target: AtomicU32::new(time_clamped.to_bits()),
            feedback_target: AtomicU32::new(feedback_clamped.to_bits()),
            mix_target: AtomicU32::new(mix_clamped.to_bits()),
        }
    }

    /// Reset delay state (clear buffer)
    pub fn reset(&self) {
        // SAFETY: Called from main thread when delay is not processing
        let state = unsafe { &mut *self.state.get() };
        state.buffer.fill(0.0);
        state.write_index = 0;
    }

    /// Get current delay time
    pub fn get_time(&self) -> f32 {
        f32::from_bits(self.time_target.load(Ordering::Relaxed))
    }

    /// Get current feedback
    pub fn get_feedback(&self) -> f32 {
        f32::from_bits(self.feedback_target.load(Ordering::Relaxed))
    }

    /// Get current mix
    pub fn get_mix(&self) -> f32 {
        f32::from_bits(self.mix_target.load(Ordering::Relaxed))
    }

    /// Set delay time in seconds (thread-safe, changes are smoothed)
    pub fn set_time(&self, time: f32) {
        let clamped = time.clamp(0.0, MAX_DELAY_TIME);
        self.time_target.store(clamped.to_bits(), Ordering::Relaxed);
    }

    /// Set feedback amount (thread-safe, changes are smoothed)
    pub fn set_feedback(&self, feedback: f32) {
        let clamped = feedback.clamp(0.0, 0.95);
        self.feedback_target
            .store(clamped.to_bits(), Ordering::Relaxed);
    }

    /// Set wet/dry mix (thread-safe, changes are smoothed)
    pub fn set_mix(&self, mix: f32) {
        let clamped = mix.clamp(0.0, 1.0);
        self.mix_target.store(clamped.to_bits(), Ordering::Relaxed);
    }
}

impl Effect for DelayEffect {
    fn process(&self, input: f32) -> f32 {
        // SAFETY: We use UnsafeCell for interior mutability. This is safe because:
        // 1. The audio thread is the only thread that calls process()
        // 2. Parameter updates via atomics are lock-free and don't conflict
        let state = unsafe { &mut *self.state.get() };

        // NaN/infinity protection at input - treat invalid input as silence
        let input = if input.is_finite() { input } else { 0.0 };

        // Read atomic targets and update smoothers
        let time_target = f32::from_bits(self.time_target.load(Ordering::Relaxed));
        let feedback_target = f32::from_bits(self.feedback_target.load(Ordering::Relaxed));
        let mix_target = f32::from_bits(self.mix_target.load(Ordering::Relaxed));

        state.time_smoothed.set_target(time_target);
        state.feedback_smoothed.set_target(feedback_target);
        state.mix_smoothed.set_target(mix_target);

        // Get smoothed values for this sample
        let time = state.time_smoothed.tick();
        let feedback = state.feedback_smoothed.tick();
        let mix = state.mix_smoothed.tick();

        // Calculate delay in samples (with fractional interpolation)
        let delay_samples = time * self.sample_rate;
        let delay_int = delay_samples as usize;
        let delay_frac = delay_samples - delay_int as f32;

        let buffer_len = state.buffer.len();

        // Read from delay line with linear interpolation for smooth time changes
        let read_index_1 = (state.write_index + buffer_len - delay_int) % buffer_len;
        let read_index_2 = (state.write_index + buffer_len - delay_int - 1) % buffer_len;

        let sample_1 = state.buffer[read_index_1];
        let sample_2 = state.buffer[read_index_2];

        // Linear interpolation between adjacent samples
        let delayed_sample = sample_1 * (1.0 - delay_frac) + sample_2 * delay_frac;

        // Write input plus feedback to delay line
        let write_sample = input + delayed_sample * feedback;

        // Flush denormals
        let write_sample = if write_sample.abs() < DENORMAL_THRESHOLD {
            0.0
        } else {
            write_sample
        };

        // NaN protection
        let write_sample = if write_sample.is_finite() {
            write_sample
        } else {
            0.0
        };

        state.buffer[state.write_index] = write_sample;
        state.write_index = (state.write_index + 1) % buffer_len;

        // Mix dry and wet signals
        let output = input * (1.0 - mix) + delayed_sample * mix;

        // Final NaN protection
        if !output.is_finite() {
            return input;
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delay_basic_processing() {
        let delay = DelayEffect::new(44100.0, 0.1, 0.5, 0.5);

        // Process some samples
        for _ in 0..1000 {
            let output = delay.process(1.0);
            assert!(output.is_finite(), "Delay output should be finite");
        }
    }

    #[test]
    fn test_delay_parameter_clamping() {
        let delay = DelayEffect::new(44100.0, 0.5, 0.5, 0.5);

        // Test time clamping
        delay.set_time(10.0);
        assert_eq!(delay.get_time(), 5.0);

        delay.set_time(-1.0);
        assert_eq!(delay.get_time(), 0.0);

        // Test feedback clamping
        delay.set_feedback(2.0);
        assert_eq!(delay.get_feedback(), 0.95);

        delay.set_feedback(-1.0);
        assert_eq!(delay.get_feedback(), 0.0);

        // Test mix clamping
        delay.set_mix(2.0);
        assert_eq!(delay.get_mix(), 1.0);

        delay.set_mix(-1.0);
        assert_eq!(delay.get_mix(), 0.0);
    }

    #[test]
    fn test_delay_zero_time() {
        let delay = DelayEffect::new(44100.0, 0.0, 0.0, 1.0);

        // With zero delay time and full wet, output should equal input
        // (approximately, due to buffer position)
        let output = delay.process(1.0);
        assert!(output.is_finite());
    }

    #[test]
    fn test_delay_feedback_stability() {
        let delay = DelayEffect::new(44100.0, 0.1, 0.95, 0.5);

        // Process many samples with high feedback
        // Should remain stable and not blow up
        for i in 0..44100 {
            let input = if i < 100 { 1.0 } else { 0.0 }; // Impulse
            let output = delay.process(input);
            assert!(
                output.is_finite() && output.abs() < 100.0,
                "Delay should remain stable with high feedback"
            );
        }
    }

    #[test]
    fn test_delay_reset() {
        let delay = DelayEffect::new(44100.0, 0.5, 0.5, 0.5);

        // Fill buffer with samples
        for _ in 0..44100 {
            delay.process(1.0);
        }

        // Reset
        delay.reset();

        // After reset with zero input, output should be minimal
        let output = delay.process(0.0);
        assert!(
            output.abs() < 0.001,
            "After reset, delay should output near-zero"
        );
    }

    #[test]
    fn test_delay_nan_protection() {
        let delay = DelayEffect::new(44100.0, 0.1, 0.5, 0.5);

        // Feed NaN input
        let output = delay.process(f32::NAN);

        // Should handle gracefully
        assert!(
            output.is_finite(),
            "Delay should handle NaN input gracefully"
        );
    }
}
