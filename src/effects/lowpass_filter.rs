//! Lowpass filter effect with parameter smoothing
//!
//! This module provides a resonant lowpass filter for the global effects chain.
//! Parameters are smoothed to prevent clicks/pops when adjusting cutoff and resonance.

use crate::effects::Effect;
use crate::utils::smoother::SmoothedParam;
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU32, Ordering};

/// Threshold for flushing denormal numbers to zero
const DENORMAL_THRESHOLD: f32 = 1e-15;

/// Internal mutable state for the filter (wrapped in UnsafeCell for interior mutability)
struct FilterState {
    // Smoothed parameters (updated per-sample for click-free changes)
    cutoff_smoothed: SmoothedParam,
    resonance_smoothed: SmoothedParam,

    // Filter state variables
    stage1: f32,
    stage2: f32,
}

/// Two-pole resonant lowpass filter (Moog-style)
/// Provides 12 dB/octave rolloff with resonance control
///
/// This filter uses internal parameter smoothing to prevent audio artifacts
/// when cutoff and resonance are changed during playback.
pub struct LowpassFilterEffect {
    sample_rate: f32,

    // Mutable state wrapped in UnsafeCell for interior mutability
    // SAFETY: This is only accessed from the audio thread during process()
    state: UnsafeCell<FilterState>,

    // Atomic parameters for lock-free updates from control thread
    // Uses bit representation of f32 for atomic operations
    cutoff_target: AtomicU32,
    resonance_target: AtomicU32,
}

// SAFETY: The UnsafeCell is only accessed from a single audio thread
// The AtomicU32 fields are inherently thread-safe
unsafe impl Send for LowpassFilterEffect {}
unsafe impl Sync for LowpassFilterEffect {}

impl LowpassFilterEffect {
    /// Create a new lowpass filter effect
    ///
    /// # Arguments
    /// * `sample_rate` - Audio sample rate in Hz
    /// * `cutoff_freq` - Initial cutoff frequency (20-20000 Hz)
    /// * `resonance` - Initial resonance (0.0-0.95)
    pub fn new(sample_rate: f32, cutoff_freq: f32, resonance: f32) -> Self {
        let cutoff_clamped = cutoff_freq.clamp(20.0, 20000.0);
        let resonance_clamped = resonance.clamp(0.0, 0.95);

        Self {
            sample_rate,
            state: UnsafeCell::new(FilterState {
                // Use 30ms smoothing for filter parameters (longer than typical 15ms for smoother sweeps)
                cutoff_smoothed: SmoothedParam::new(
                    cutoff_clamped,
                    20.0,
                    20000.0,
                    sample_rate,
                    30.0,
                ),
                resonance_smoothed: SmoothedParam::new(
                    resonance_clamped,
                    0.0,
                    0.95,
                    sample_rate,
                    30.0,
                ),
                stage1: 0.0,
                stage2: 0.0,
            }),
            cutoff_target: AtomicU32::new(cutoff_clamped.to_bits()),
            resonance_target: AtomicU32::new(resonance_clamped.to_bits()),
        }
    }

    /// Get a handle to control the filter parameters (lock-free)
    pub fn get_control(&self) -> LowpassFilterControl {
        LowpassFilterControl {
            cutoff_target: &self.cutoff_target as *const AtomicU32,
            resonance_target: &self.resonance_target as *const AtomicU32,
        }
    }

    /// Reset filter state (call when enabling filter or after NaN detection)
    pub fn reset(&self) {
        // SAFETY: Called from main thread when filter is not processing
        let state = unsafe { &mut *self.state.get() };
        state.stage1 = 0.0;
        state.stage2 = 0.0;
    }

    /// Get current cutoff frequency
    pub fn get_cutoff_freq(&self) -> f32 {
        f32::from_bits(self.cutoff_target.load(Ordering::Relaxed))
    }

    /// Get current resonance
    pub fn get_resonance(&self) -> f32 {
        f32::from_bits(self.resonance_target.load(Ordering::Relaxed))
    }

    /// Set cutoff frequency (thread-safe, changes are smoothed)
    pub fn set_cutoff_freq(&self, cutoff_freq: f32) {
        let clamped = cutoff_freq.clamp(20.0, 20000.0);
        self.cutoff_target
            .store(clamped.to_bits(), Ordering::Relaxed);
    }

    /// Set resonance (thread-safe, changes are smoothed)
    pub fn set_resonance(&self, resonance: f32) {
        let clamped = resonance.clamp(0.0, 0.95);
        self.resonance_target
            .store(clamped.to_bits(), Ordering::Relaxed);
    }
}

impl Effect for LowpassFilterEffect {
    fn process(&self, input: f32) -> f32 {
        // SAFETY: We use UnsafeCell for interior mutability. This is safe because:
        // 1. The audio thread is the only thread that calls process()
        // 2. Parameter updates via atomics are lock-free and don't conflict
        let state = unsafe { &mut *self.state.get() };

        // Read atomic targets and update smoothers
        let cutoff_target = f32::from_bits(self.cutoff_target.load(Ordering::Relaxed));
        let resonance_target = f32::from_bits(self.resonance_target.load(Ordering::Relaxed));

        state.cutoff_smoothed.set_target(cutoff_target);
        state.resonance_smoothed.set_target(resonance_target);

        // Get smoothed values for this sample
        let cutoff = state.cutoff_smoothed.tick();
        let resonance = state.resonance_smoothed.tick();

        // Limit cutoff to safe range (well below Nyquist to prevent instability)
        // At 44.1kHz, Nyquist is 22050Hz. We limit to ~0.40 * sample_rate for stability.
        let max_cutoff = self.sample_rate * 0.40;
        let safe_cutoff = cutoff.min(max_cutoff);

        // Calculate filter coefficient 
        // Using a simple but stable one-pole coefficient: g = 1 - e^(-2*pi*fc/fs)
        // This is more stable than the sin() or tan() formulations at high frequencies
        let normalized_freq = safe_cutoff / self.sample_rate;
        let g = 1.0 - (-2.0 * std::f32::consts::PI * normalized_freq).exp();
        
        // Clamp g to ensure stability (must be well under 1.0)
        let g = g.clamp(0.0, 0.90);

        // Scale down resonance at high frequencies to prevent instability
        // More aggressive scaling: resonance drops off significantly above 5kHz
        let freq_ratio = (safe_cutoff / 5000.0).min(1.0);
        let resonance_scale = 1.0 - (freq_ratio * freq_ratio * 0.7);
        let effective_resonance = resonance * resonance_scale;

        // Resonance feedback - use a more conservative scale factor
        // Maximum feedback = 0.95 * 0.3 * 3.5 = ~1.0 (just at edge of self-oscillation)
        let feedback = effective_resonance * 3.5;

        // Apply resonance feedback from second stage (with soft clipping to prevent blowup)
        let feedback_signal = state.stage2 * feedback;
        let input_with_feedback = input - feedback_signal.tanh() * feedback.min(1.0);

        // First filter stage (one-pole lowpass)
        state.stage1 += g * (input_with_feedback - state.stage1);

        // Second filter stage (cascaded for 12 dB/octave)
        state.stage2 += g * (state.stage1 - state.stage2);

        // Soft clip the output to prevent any remaining instability from causing harsh distortion
        let output = state.stage2.tanh();

        // Flush denormals to zero to prevent CPU spikes
        if state.stage1.abs() < DENORMAL_THRESHOLD {
            state.stage1 = 0.0;
        }
        if state.stage2.abs() < DENORMAL_THRESHOLD {
            state.stage2 = 0.0;
        }

        // NaN/infinity protection - if filter state becomes invalid, reset it
        if !output.is_finite() {
            state.stage1 = 0.0;
            state.stage2 = 0.0;
            return 0.0;
        }

        output
    }
}

/// Control handle for adjusting lowpass filter parameters (lock-free)
///
/// This handle can be safely used from any thread to adjust filter parameters.
/// Changes are applied smoothly without clicks or pops.
#[derive(Clone, Copy)]
pub struct LowpassFilterControl {
    cutoff_target: *const AtomicU32,
    resonance_target: *const AtomicU32,
}

// SAFETY: The atomic pointers are valid for the lifetime of the LowpassFilterEffect
// and AtomicU32 operations are inherently thread-safe
unsafe impl Send for LowpassFilterControl {}
unsafe impl Sync for LowpassFilterControl {}

impl LowpassFilterControl {
    /// Set cutoff frequency (20-20000 Hz)
    ///
    /// The change will be smoothed over ~30ms to prevent clicks.
    pub fn set_cutoff_freq(&self, cutoff_freq: f32) {
        let clamped = cutoff_freq.clamp(20.0, 20000.0);
        // SAFETY: Pointer is valid for lifetime of LowpassFilterEffect
        unsafe {
            (*self.cutoff_target).store(clamped.to_bits(), Ordering::Relaxed);
        }
    }

    /// Set resonance (0.0-0.95)
    ///
    /// The change will be smoothed over ~30ms to prevent clicks.
    pub fn set_resonance(&self, resonance: f32) {
        let clamped = resonance.clamp(0.0, 0.95);
        // SAFETY: Pointer is valid for lifetime of LowpassFilterEffect
        unsafe {
            (*self.resonance_target).store(clamped.to_bits(), Ordering::Relaxed);
        }
    }

    /// Get current cutoff frequency target
    pub fn get_cutoff_freq(&self) -> f32 {
        // SAFETY: Pointer is valid for lifetime of LowpassFilterEffect
        unsafe { f32::from_bits((*self.cutoff_target).load(Ordering::Relaxed)) }
    }

    /// Get current resonance target
    pub fn get_resonance(&self) -> f32 {
        // SAFETY: Pointer is valid for lifetime of LowpassFilterEffect
        unsafe { f32::from_bits((*self.resonance_target).load(Ordering::Relaxed)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_basic_processing() {
        let filter = LowpassFilterEffect::new(44100.0, 1000.0, 0.0);

        // Process some samples
        for _ in 0..1000 {
            let output = filter.process(1.0);
            assert!(output.is_finite(), "Filter output should be finite");
        }
    }

    #[test]
    fn test_filter_parameter_clamping() {
        let filter = LowpassFilterEffect::new(44100.0, 1000.0, 0.5);

        // Test cutoff clamping
        filter.set_cutoff_freq(100000.0);
        assert_eq!(filter.get_cutoff_freq(), 20000.0);

        filter.set_cutoff_freq(1.0);
        assert_eq!(filter.get_cutoff_freq(), 20.0);

        // Test resonance clamping
        filter.set_resonance(2.0);
        assert_eq!(filter.get_resonance(), 0.95);

        filter.set_resonance(-1.0);
        assert_eq!(filter.get_resonance(), 0.0);
    }

    #[test]
    fn test_filter_nan_protection() {
        let filter = LowpassFilterEffect::new(44100.0, 1000.0, 0.5);

        // Feed NaN input
        let output = filter.process(f32::NAN);

        // Filter should reset and return 0 or finite value
        assert!(
            output.is_finite(),
            "Filter should handle NaN input gracefully"
        );
    }

    #[test]
    fn test_filter_stability_at_high_resonance() {
        let filter = LowpassFilterEffect::new(44100.0, 500.0, 0.95);

        // Process many samples at high resonance
        for i in 0..44100 {
            let input = if i < 100 { 1.0 } else { 0.0 }; // Impulse
            let output = filter.process(input);
            assert!(
                output.is_finite() && output.abs() < 100.0,
                "Filter should remain stable at high resonance"
            );
        }
    }

    #[test]
    fn test_filter_control_thread_safety() {
        let filter = LowpassFilterEffect::new(44100.0, 1000.0, 0.5);
        let control = filter.get_control();

        // Simulate parameter changes while processing
        for i in 0..1000 {
            // Change parameters
            control.set_cutoff_freq(200.0 + (i as f32) * 10.0);
            control.set_resonance((i as f32) / 2000.0);

            // Process a sample
            let output = filter.process(1.0);
            assert!(output.is_finite(), "Filter should remain stable during parameter changes");
        }
    }

    #[test]
    fn test_filter_high_frequency_stability() {
        let filter = LowpassFilterEffect::new(44100.0, 20000.0, 0.95);

        // Process at maximum cutoff with high resonance
        // This should NOT produce distortion or instability
        for i in 0..44100 {
            // Use a mix of signals to test stability
            let input = (i as f32 * 0.1).sin() * 0.5;
            let output = filter.process(input);
            
            assert!(
                output.is_finite(),
                "Filter output should be finite at high frequencies"
            );
            assert!(
                output.abs() < 10.0,
                "Filter output should not explode at high frequencies: got {}",
                output
            );
        }
    }

    #[test]
    fn test_filter_sweep_full_range() {
        let filter = LowpassFilterEffect::new(44100.0, 20.0, 0.7);

        // Sweep cutoff from 20Hz to 20kHz while processing
        for i in 0..44100 {
            let t = i as f32 / 44100.0;
            // Logarithmic sweep
            let cutoff = 20.0 * (1000.0_f32).powf(t);
            filter.set_cutoff_freq(cutoff);

            let input = (i as f32 * 0.05).sin();
            let output = filter.process(input);

            assert!(
                output.is_finite() && output.abs() < 10.0,
                "Filter should remain stable during frequency sweep at {} Hz",
                cutoff
            );
        }
    }
}
