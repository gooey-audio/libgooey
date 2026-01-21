//! Audio effects for the gooey engine
//!
//! This module contains various audio effects that can be applied to signals.
//! Effects can be used as global/master effects via the [`Effect`] trait, or
//! integrated into instrument effect racks for per-instrument processing.
//!
//! # Smoothing Convention
//!
//! **Effects do NOT perform internal parameter smoothing.** This is a deliberate
//! architectural decision to maintain consistency and flexibility:
//!
//! - **Smoothing is the caller's responsibility**: Use [`crate::utils::SmoothedParam`]
//!   in the instrument or engine layer to smooth parameters before passing to effects.
//!
//! - **Why?** The caller knows the audio context (sample rate, desired smooth time)
//!   and can make appropriate decisions. Different use cases may want different
//!   smoothing behavior (or none at all for instant changes).
//!
//! - **Hot path optimization**: For audio thread performance, effects should provide
//!   a method like `process_with_params()` that accepts parameters directly, avoiding
//!   atomic operations per sample. The [`Effect::process`] method reads stored atomic
//!   parameters and is suitable for global effects where the UI thread sets parameters
//!   asynchronously.
//!
//! ## Example
//!
//! ```ignore
//! // In instrument tick() method:
//! let smoothed_threshold = self.params.saturation.get(); // Already smoothed
//! let output = SoftSaturation::process_with_params(input, 1.0 - smoothed_threshold);
//!
//! // For global effects (Effect trait):
//! // UI thread sets parameters via atomic setters
//! effect.set_threshold(new_value); // Atomic store
//! // Audio thread processes via trait
//! let output = effect.process(input); // Atomic load + process
//! ```
//!
//! # Effect Categories
//!
//! - **Dynamics**: [`BrickWallLimiter`] - hard limiting to prevent clipping
//! - **Saturation**: [`TubeSaturation`], [`SoftSaturation`] - harmonic saturation/warmth
//! - **Filters**: [`LowpassFilterEffect`] - frequency shaping
//! - **Time-based**: [`DelayEffect`] - echo/delay effects
//! - **Waveshaping**: [`Waveshaper`] - general waveshaping

pub mod delay;
pub mod limiter;
pub mod lowpass_filter;
pub mod saturation;
pub mod soft_saturation;
pub mod waveshaper;

pub use self::delay::*;
pub use self::limiter::*;
pub use self::lowpass_filter::*;
pub use self::saturation::*;
pub use self::soft_saturation::*;
pub use self::waveshaper::*;

/// Trait that all global effects must implement
///
/// This trait provides a simple interface for processing audio samples through
/// an effect. Parameters are stored internally (typically as atomics) and can
/// be updated from any thread.
///
/// # Smoothing Convention
///
/// Effects implementing this trait do NOT smooth parameters internally.
/// Smoothing is the responsibility of the caller. When using effects in
/// performance-critical instrument code, prefer direct method calls like
/// `process_with_params()` over the trait method to avoid atomic operations.
///
/// # Thread Safety
///
/// The `Send` bound ensures effects can be moved between threads. Effects
/// should use atomic types or other synchronization for any mutable state
/// accessed during processing.
pub trait Effect: Send {
    /// Process a single audio sample through the effect
    ///
    /// This method reads parameters from internal atomic storage, making it
    /// suitable for use in global effect chains where parameters are set
    /// asynchronously from the UI thread.
    ///
    /// For instrument effect racks with already-smoothed parameters, prefer
    /// using direct methods like `process_with_params()` to avoid atomic
    /// operations in the audio hot path.
    fn process(&self, input: f32) -> f32;
}
