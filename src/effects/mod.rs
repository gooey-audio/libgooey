pub mod compressor;
pub mod delay;
pub mod feedback_waveshaper;
pub mod limiter;
pub mod lowpass_filter;
pub mod saturation;
pub mod waveshaper;

pub use self::compressor::*;
pub use self::delay::*;
pub use self::feedback_waveshaper::*;
pub use self::limiter::*;
pub use self::lowpass_filter::*;
pub use self::saturation::*;
pub use self::waveshaper::*;

/// Trait that all global effects must implement
pub trait Effect: Send {
    /// Process a single audio sample through the effect
    fn process(&self, input: f32) -> f32;
}
