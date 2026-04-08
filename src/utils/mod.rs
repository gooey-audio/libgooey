//! Utility modules for audio processing

pub mod blendable;
pub mod oversampler;
pub mod smoother;

pub use blendable::{Blendable, PresetBlender};
pub use oversampler::Oversampler2x;
pub use smoother::{ParamSmoother, SmoothedParam, DEFAULT_SMOOTH_TIME_MS};

/// Convert a normalized tuning value (0.0–1.0) to a frequency multiplier.
///
/// Maps 0.0 → −12 semitones (0.5×), 0.5 → neutral (1.0×), 1.0 → +12 semitones (2.0×).
pub fn tuning_to_multiplier(normalized: f32) -> f32 {
    let semitones = (normalized.clamp(0.0, 1.0) - 0.5) * 24.0;
    2.0_f32.powf(semitones / 12.0)
}
