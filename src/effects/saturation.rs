//! Tube-style saturation effect for analog warmth
//!
//! This module provides a sophisticated saturation effect that emulates
//! tube saturation characteristics for warm, musical distortion.
//! Uses arctangent-based soft clipping with controllable even harmonic
//! generation for a more analog sound than simple tanh.

use crate::effects::Effect;
use crate::utils::smoother::SmoothedParam;
use std::cell::UnsafeCell;
use std::f32::consts::FRAC_2_PI;
use std::sync::atomic::{AtomicU32, Ordering};

/// Threshold for flushing denormal numbers to zero
const DENORMAL_THRESHOLD: f32 = 1e-15;

/// DC blocker coefficient (R in RC circuit, ~20Hz cutoff at 44.1kHz)
const DC_BLOCKER_COEFF: f32 = 0.995;

/// Internal mutable state for saturation
struct SaturationState {
    // Smoothed parameters
    drive_smoothed: SmoothedParam,
    warmth_smoothed: SmoothedParam,
    mix_smoothed: SmoothedParam,

    // DC blocker state (high-pass to remove DC offset)
    dc_x1: f32,
    dc_y1: f32,
}

/// Tube-style saturation effect
///
/// Provides warm, analog-sounding saturation with:
/// - Soft-knee compression (arctangent-based)
/// - Controllable even harmonic generation (warmth)
/// - Built-in DC blocking
/// - Smooth parameter transitions
pub struct TubeSaturation {
    // Mutable state wrapped in UnsafeCell for interior mutability
    state: UnsafeCell<SaturationState>,

    // Atomic parameters for lock-free updates from control thread
    drive_target: AtomicU32,
    warmth_target: AtomicU32,
    mix_target: AtomicU32,
}

// SAFETY: UnsafeCell only accessed from single audio thread
unsafe impl Send for TubeSaturation {}
unsafe impl Sync for TubeSaturation {}

impl TubeSaturation {
    /// Create a new tube saturation effect
    ///
    /// # Arguments
    /// * `sample_rate` - Audio sample rate in Hz
    /// * `drive` - Initial drive (0.0-1.0)
    /// * `warmth` - Initial warmth/even harmonics (0.0-1.0)
    /// * `mix` - Initial wet/dry mix (0.0-1.0)
    pub fn new(sample_rate: f32, drive: f32, warmth: f32, mix: f32) -> Self {
        let drive_clamped = drive.clamp(0.0, 1.0);
        let warmth_clamped = warmth.clamp(0.0, 1.0);
        let mix_clamped = mix.clamp(0.0, 1.0);

        Self {
            state: UnsafeCell::new(SaturationState {
                drive_smoothed: SmoothedParam::new(drive_clamped, 0.0, 1.0, sample_rate, 30.0),
                warmth_smoothed: SmoothedParam::new(warmth_clamped, 0.0, 1.0, sample_rate, 30.0),
                mix_smoothed: SmoothedParam::new(mix_clamped, 0.0, 1.0, sample_rate, 30.0),
                dc_x1: 0.0,
                dc_y1: 0.0,
            }),
            drive_target: AtomicU32::new(drive_clamped.to_bits()),
            warmth_target: AtomicU32::new(warmth_clamped.to_bits()),
            mix_target: AtomicU32::new(mix_clamped.to_bits()),
        }
    }

    /// Map user drive (0-1) to internal drive (1-8)
    #[inline]
    fn map_drive(user_drive: f32) -> f32 {
        1.0 + user_drive * 7.0
    }

    /// Map user warmth (0-1) to internal bias (0-0.4)
    #[inline]
    fn map_warmth(user_warmth: f32) -> f32 {
        user_warmth * 0.4
    }

    /// Core tube saturation function
    ///
    /// Uses arctangent for softer knee than tanh, with explicit
    /// second harmonic generation for tube-like warmth.
    #[inline]
    fn saturate(input: f32, drive: f32, bias: f32) -> f32 {
        // Apply drive
        let driven = input * drive;

        // Asymmetric bias for even harmonics (tube-like warmth)
        let biased = driven + bias * driven.abs();

        // Soft-knee saturation using scaled arctangent
        // atan(x) * (2/pi) gives smooth saturation with range (-1, 1)
        let soft_sat = biased.atan() * FRAC_2_PI;

        // Add subtle second harmonic (warmth)
        // x^2 * sign(x) generates 2nd harmonic from input
        let second_harmonic = soft_sat.powi(2) * soft_sat.signum() * 0.15;

        // Blend based on bias amount
        soft_sat + second_harmonic * bias
    }

    /// DC blocking high-pass filter
    #[inline]
    fn dc_block(input: f32, x1: &mut f32, y1: &mut f32) -> f32 {
        // y[n] = x[n] - x[n-1] + R * y[n-1]
        let output = input - *x1 + DC_BLOCKER_COEFF * *y1;
        *x1 = input;
        *y1 = if output.abs() < DENORMAL_THRESHOLD {
            0.0
        } else {
            output
        };
        output
    }

    // Parameter setters (thread-safe, called from control thread)

    /// Set the drive amount (0.0-1.0)
    pub fn set_drive(&self, drive: f32) {
        let clamped = drive.clamp(0.0, 1.0);
        self.drive_target
            .store(clamped.to_bits(), Ordering::Relaxed);
    }

    /// Set the warmth/even harmonics amount (0.0-1.0)
    pub fn set_warmth(&self, warmth: f32) {
        let clamped = warmth.clamp(0.0, 1.0);
        self.warmth_target
            .store(clamped.to_bits(), Ordering::Relaxed);
    }

    /// Set the dry/wet mix (0.0-1.0)
    pub fn set_mix(&self, mix: f32) {
        let clamped = mix.clamp(0.0, 1.0);
        self.mix_target.store(clamped.to_bits(), Ordering::Relaxed);
    }

    // Parameter getters

    /// Get the current drive setting
    pub fn get_drive(&self) -> f32 {
        f32::from_bits(self.drive_target.load(Ordering::Relaxed))
    }

    /// Get the current warmth setting
    pub fn get_warmth(&self) -> f32 {
        f32::from_bits(self.warmth_target.load(Ordering::Relaxed))
    }

    /// Get the current mix setting
    pub fn get_mix(&self) -> f32 {
        f32::from_bits(self.mix_target.load(Ordering::Relaxed))
    }

    /// Reset internal state (DC blocker)
    pub fn reset(&self) {
        let state = unsafe { &mut *self.state.get() };
        state.dc_x1 = 0.0;
        state.dc_y1 = 0.0;
    }
}

impl Effect for TubeSaturation {
    fn process(&self, input: f32) -> f32 {
        // NaN/infinity protection at input
        if !input.is_finite() {
            return 0.0;
        }

        let state = unsafe { &mut *self.state.get() };

        // Read targets and update smoothers
        let drive_target = f32::from_bits(self.drive_target.load(Ordering::Relaxed));
        let warmth_target = f32::from_bits(self.warmth_target.load(Ordering::Relaxed));
        let mix_target = f32::from_bits(self.mix_target.load(Ordering::Relaxed));

        state.drive_smoothed.set_target(drive_target);
        state.warmth_smoothed.set_target(warmth_target);
        state.mix_smoothed.set_target(mix_target);

        // Get smoothed values and map to internal ranges
        let drive = Self::map_drive(state.drive_smoothed.tick());
        let warmth = Self::map_warmth(state.warmth_smoothed.tick());
        let mix = state.mix_smoothed.tick();

        // Early exit if bypassed
        if mix < 0.0001 {
            return input;
        }

        // Apply saturation
        let saturated = Self::saturate(input, drive, warmth);

        // DC blocking (removes offset from asymmetric saturation)
        let dc_blocked = Self::dc_block(saturated, &mut state.dc_x1, &mut state.dc_y1);

        // Mix dry/wet
        let output = input * (1.0 - mix) + dc_blocked * mix;

        // NaN protection at output
        if !output.is_finite() {
            state.dc_x1 = 0.0;
            state.dc_y1 = 0.0;
            return 0.0;
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bypass_when_mix_zero() {
        let sat = TubeSaturation::new(44100.0, 0.5, 0.5, 0.0);
        assert_eq!(sat.process(0.5), 0.5);
        assert_eq!(sat.process(-0.3), -0.3);
    }

    #[test]
    fn test_soft_limiting() {
        let sat = TubeSaturation::new(44100.0, 1.0, 0.0, 1.0);
        // Run a few samples to let smoothers settle
        for _ in 0..1000 {
            sat.process(0.0);
        }
        let output = sat.process(1.0);
        // Output should be soft-limited below 1.0
        assert!(output < 1.0, "Expected output < 1.0, got {}", output);
        assert!(output > 0.3, "Expected output > 0.3, got {}", output);
    }

    #[test]
    fn test_parameter_clamping() {
        let sat = TubeSaturation::new(44100.0, 5.0, -1.0, 2.0);
        assert_eq!(sat.get_drive(), 1.0);
        assert_eq!(sat.get_warmth(), 0.0);
        assert_eq!(sat.get_mix(), 1.0);
    }

    #[test]
    fn test_dc_stability() {
        let sat = TubeSaturation::new(44100.0, 0.5, 1.0, 1.0);
        // Process many samples with asymmetric settings
        // DC blocker should prevent drift
        for _ in 0..44100 {
            let output = sat.process(0.5);
            assert!(output.is_finite());
        }
    }

    #[test]
    fn test_nan_protection() {
        let sat = TubeSaturation::new(44100.0, 0.5, 0.5, 0.5);
        let output = sat.process(f32::NAN);
        assert!(output.is_finite());
        assert_eq!(output, 0.0);
    }

    #[test]
    fn test_parameter_setters() {
        let sat = TubeSaturation::new(44100.0, 0.0, 0.0, 0.0);

        sat.set_drive(0.7);
        sat.set_warmth(0.5);
        sat.set_mix(0.8);

        assert!((sat.get_drive() - 0.7).abs() < 0.001);
        assert!((sat.get_warmth() - 0.5).abs() < 0.001);
        assert!((sat.get_mix() - 0.8).abs() < 0.001);
    }
}
