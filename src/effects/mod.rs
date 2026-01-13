pub mod limiter;
pub mod lowpass_filter;
pub mod waveshaper;

pub use self::limiter::*;
pub use self::lowpass_filter::*;
pub use self::waveshaper::*;

/// Trait that all global effects must implement
pub trait Effect: Send {
    /// Process a single audio sample through the effect
    fn process(&self, input: f32) -> f32;
}
