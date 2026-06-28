//! Utility modules for audio processing

pub mod blendable;
pub mod oversampler;
pub mod smoother;

pub use blendable::{Blendable, PresetBlender};
pub use oversampler::{Oversampler, Oversampler2x, Oversampler4x, OversamplingMode};
pub use smoother::{ParamSmoother, SmoothedParam, DEFAULT_SMOOTH_TIME_MS};

/// Convert a normalized tuning value (0.0–1.0) to a frequency multiplier.
///
/// Maps 0.0 → −12 semitones (0.5×), 0.5 → neutral (1.0×), 1.0 → +12 semitones (2.0×).
pub fn tuning_to_multiplier(normalized: f32) -> f32 {
    let semitones = (normalized.clamp(0.0, 1.0) - 0.5) * 24.0;
    2.0_f32.powf(semitones / 12.0)
}

/// 4-point (cubic Catmull-Rom) interpolation between `p1` and `p2`, using the
/// neighbouring samples `p0` and `p3` to estimate the tangents. `t` is the
/// fractional position in `[0, 1]` between `p1` and `p2`.
///
/// Shared by sample-buffer readers (granular + loop playback) so fractional
/// playback positions resolve to a smooth, click-free signal.
#[inline]
pub fn cubic_interpolate(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let a0 = -0.5 * p0 + 1.5 * p1 - 1.5 * p2 + 0.5 * p3;
    let a1 = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
    let a2 = -0.5 * p0 + 0.5 * p2;
    let a3 = p1;
    ((a0 * t + a1) * t + a2) * t + a3
}
