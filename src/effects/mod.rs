pub mod compressor;
pub mod delay;
pub mod feedback_waveshaper;
pub mod limiter;
pub mod lowpass_filter;
pub mod reverb;
pub mod saturation;
#[cfg(feature = "spectral")]
pub mod spectral_resonator;
pub mod tilt_filter;
pub mod waveshaper;

pub use self::compressor::*;
pub use self::delay::*;
pub use self::feedback_waveshaper::*;
pub use self::limiter::*;
pub use self::lowpass_filter::*;
pub use self::reverb::*;
pub use self::saturation::*;
#[cfg(feature = "spectral")]
pub use self::spectral_resonator::*;
pub use self::tilt_filter::*;
pub use self::waveshaper::*;

use crate::frame::StereoFrame;

/// Trait that all global effects must implement
pub trait Effect: Send {
    /// Process a single audio sample through the effect (mono path).
    fn process(&self, input: f32) -> f32;

    /// Process one stereo frame (a left/right sample pair) through the effect.
    ///
    /// This is REQUIRED (no default) on purpose. A default that simply calls
    /// [`Effect::process`] once per channel is correct only for STATELESS
    /// effects; for any effect with per-sample DSP state (delay lines, filter
    /// memory, envelope followers, smoothers) it would advance that single
    /// shared state TWICE per frame, interleaving left and right into one
    /// history and silently corrupting the stereo image. Forcing every
    /// implementor to write this method makes that mistake a compile error
    /// rather than a runtime bug.
    ///
    /// - Stateful effects must keep genuine per-channel state (typically
    ///   `UnsafeCell<[State; 2]>`) and run each channel through its own state.
    /// - Stateless effects may simply process each channel independently:
    ///   `StereoFrame { l: self.process(input.l), r: self.process(input.r) }`.
    /// - Wrappers that delegate to an inner [`Effect`] should forward to the
    ///   inner `process_stereo`.
    fn process_stereo(&self, input: StereoFrame) -> StereoFrame;
}
