/// Velocity shaping utilities for musical, non-linear velocity response.
///
/// The core curve provides:
/// - More dramatic effect changes at low-to-mid velocities
/// - A soft-knee saturation in the top ~10% where changes taper off
/// - This mimics the physical response of acoustic drums where
///   hitting harder has diminishing returns at the extreme top

/// Shape a raw velocity (0.0-1.0) with a non-linear curve.
///
/// `steepness` controls how aggressive the curve is:
/// - 1.0 = moderate soft-knee (good for decay, amplitude)
/// - 2.0+ = steep low-end sensitivity with strong saturation (good for pitch)
///
/// Returns shaped velocity in 0.0-1.0 range.
#[inline]
pub fn shape_velocity(vel: f32, steepness: f32) -> f32 {
    // Use a soft-clip curve: vel_shaped = 1 - (1 - vel)^(1 + steepness * vel)
    // This creates a curve that:
    // - Is steep at low velocity (large changes per unit velocity)
    // - Saturates at high velocity (soft knee in last ~10-15%)
    // - Always maps 0→0 and 1→1
    let vel = vel.clamp(0.0, 1.0);
    let inv = 1.0 - vel;
    // Variable exponent: higher at low velocity (more dramatic), lower at high velocity
    let exponent = 1.0 + steepness * (1.0 - vel * 0.5);
    1.0 - inv.powf(exponent)
}

/// Shape velocity for pitch-related parameters.
/// Uses a steeper curve since pitch is particularly sensitive on real drums -
/// even small velocity changes should noticeably affect pitch.
#[inline]
pub fn shape_velocity_pitch(vel: f32) -> f32 {
    shape_velocity(vel, 2.0)
}

/// Shape velocity for decay/amplitude parameters.
/// Uses a moderate curve - musical but not as extreme as pitch.
#[inline]
pub fn shape_velocity_default(vel: f32) -> f32 {
    shape_velocity(vel, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_velocity_shape_boundaries() {
        // 0 maps to 0, 1 maps to 1
        assert!((shape_velocity(0.0, 1.0) - 0.0).abs() < 1e-6);
        assert!((shape_velocity(1.0, 1.0) - 1.0).abs() < 1e-6);
        assert!((shape_velocity(0.0, 2.0) - 0.0).abs() < 1e-6);
        assert!((shape_velocity(1.0, 2.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_velocity_shape_monotonic() {
        // Curve should be monotonically increasing
        let mut prev = 0.0;
        for i in 1..=100 {
            let vel = i as f32 / 100.0;
            let shaped = shape_velocity(vel, 1.0);
            assert!(shaped >= prev, "Not monotonic at vel={vel}: {shaped} < {prev}");
            prev = shaped;
        }
    }

    #[test]
    fn test_velocity_top_saturates() {
        // The slope in the top 10% should be less than in the bottom 10%
        let low_slope = shape_velocity(0.1, 1.0) - shape_velocity(0.0, 1.0);
        let high_slope = shape_velocity(1.0, 1.0) - shape_velocity(0.9, 1.0);
        assert!(
            high_slope < low_slope,
            "Top should saturate: high_slope={high_slope} >= low_slope={low_slope}"
        );
    }

    #[test]
    fn test_pitch_steeper_than_default() {
        // Pitch curve should reach higher values at mid-velocity than default
        let mid_default = shape_velocity_default(0.5);
        let mid_pitch = shape_velocity_pitch(0.5);
        assert!(
            mid_pitch > mid_default,
            "Pitch should be steeper: {mid_pitch} <= {mid_default}"
        );
    }
}
