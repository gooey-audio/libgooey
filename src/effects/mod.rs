pub mod compressor;
pub mod delay;
pub mod feedback_waveshaper;
pub mod limiter;
pub mod lowpass_filter;
pub mod reverb;
pub mod saturation;
pub mod tilt_filter;
pub mod waveshaper;

pub use self::compressor::*;
pub use self::delay::*;
pub use self::feedback_waveshaper::*;
pub use self::limiter::*;
pub use self::lowpass_filter::*;
pub use self::reverb::*;
pub use self::saturation::*;
pub use self::tilt_filter::*;
pub use self::waveshaper::*;

use crate::frame::StereoFrame;

/// Trait that all global effects must implement
pub trait Effect: Send {
    /// Process a single audio sample through the effect
    fn process(&self, input: f32) -> f32;

    /// Process one stereo frame (a left/right sample pair) through the effect.
    ///
    /// IMPORTANT: the default implementation calls [`Effect::process`] once per
    /// channel. That is correct ONLY for STATELESS effects (those whose output
    /// depends solely on the current input and immutable parameters, e.g. the
    /// limiters). For any effect with per-sample DSP state (delay lines, filter
    /// memory, envelope followers, smoothers) the default is WRONG: it advances
    /// that single shared state TWICE per frame, interleaving the left and right
    /// channels into one history. Every stateful effect MUST override this with
    /// genuine per-channel state (typically `UnsafeCell<[State; 2]>`).
    fn process_stereo(&self, input: StereoFrame) -> StereoFrame {
        StereoFrame {
            l: self.process(input.l),
            r: self.process(input.r),
        }
    }
}
