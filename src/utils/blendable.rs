//! 2D preset blending utilities for X/Y pad-style interpolation

/// Trait for types that can be linearly interpolated (blended)
///
/// Enables configs to be smoothly blended between presets.
/// Implementations should perform field-by-field linear interpolation.
pub trait Blendable: Clone + Copy {
    /// Linearly interpolate between self and other
    ///
    /// # Arguments
    /// * `other` - The target value to blend towards
    /// * `t` - Blend factor from 0.0 (self) to 1.0 (other)
    ///
    /// # Returns
    /// A new instance with all blendable fields interpolated
    fn lerp(&self, other: &Self, t: f32) -> Self;
}

/// 2D preset blender for X/Y pad-style interpolation
///
/// Stores 4 corner presets and blends between them using bilinear interpolation.
/// The coordinate space is:
/// ```text
///        Y=1
///    TL ---- TR
///     |      |
///     |      |
///    BL ---- BR
///        Y=0
///   X=0      X=1
/// ```
#[derive(Clone, Copy, Debug)]
pub struct PresetBlender<T: Blendable> {
    /// Bottom-left preset (x=0, y=0)
    pub bottom_left: T,
    /// Bottom-right preset (x=1, y=0)
    pub bottom_right: T,
    /// Top-left preset (x=0, y=1)
    pub top_left: T,
    /// Top-right preset (x=1, y=1)
    pub top_right: T,
}

impl<T: Blendable> PresetBlender<T> {
    /// Create a new preset blender with 4 corner presets
    pub fn new(bottom_left: T, bottom_right: T, top_left: T, top_right: T) -> Self {
        Self {
            bottom_left,
            bottom_right,
            top_left,
            top_right,
        }
    }

    /// Create a blender with all corners set to the same preset
    pub fn uniform(preset: T) -> Self {
        Self {
            bottom_left: preset,
            bottom_right: preset,
            top_left: preset,
            top_right: preset,
        }
    }

    /// Perform bilinear interpolation at the given X/Y position
    ///
    /// # Arguments
    /// * `x` - Horizontal position (0.0 = left, 1.0 = right)
    /// * `y` - Vertical position (0.0 = bottom, 1.0 = top)
    ///
    /// # Returns
    /// A blended config interpolated from all 4 corner presets
    pub fn blend(&self, x: f32, y: f32) -> T {
        let x = x.clamp(0.0, 1.0);
        let y = y.clamp(0.0, 1.0);

        // Bilinear interpolation:
        // 1. Interpolate along bottom edge (BL -> BR)
        let bottom = self.bottom_left.lerp(&self.bottom_right, x);

        // 2. Interpolate along top edge (TL -> TR)
        let top = self.top_left.lerp(&self.top_right, x);

        // 3. Interpolate between bottom and top results
        bottom.lerp(&top, y)
    }

    /// Set a corner preset
    pub fn set_bottom_left(&mut self, preset: T) {
        self.bottom_left = preset;
    }

    pub fn set_bottom_right(&mut self, preset: T) {
        self.bottom_right = preset;
    }

    pub fn set_top_left(&mut self, preset: T) {
        self.top_left = preset;
    }

    pub fn set_top_right(&mut self, preset: T) {
        self.top_right = preset;
    }
}

/// Macro to implement Blendable for a struct with all f32 fields
///
/// # Usage
/// ```ignore
/// impl_blendable!(KickConfig {
///     frequency,
///     punch_amount,
///     sub_amount,
///     // ... all f32 fields
/// });
/// ```
#[macro_export]
macro_rules! impl_blendable {
    ($type:ty { $($field:ident),* $(,)? }) => {
        impl $crate::utils::Blendable for $type {
            fn lerp(&self, other: &Self, t: f32) -> Self {
                let t = t.clamp(0.0, 1.0);
                let inv_t = 1.0 - t;
                Self {
                    $($field: self.$field * inv_t + other.$field * t,)*
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq)]
    struct TestConfig {
        a: f32,
        b: f32,
    }

    impl Blendable for TestConfig {
        fn lerp(&self, other: &Self, t: f32) -> Self {
            let t = t.clamp(0.0, 1.0);
            let inv_t = 1.0 - t;
            Self {
                a: self.a * inv_t + other.a * t,
                b: self.b * inv_t + other.b * t,
            }
        }
    }

    #[test]
    fn test_lerp_at_zero() {
        let a = TestConfig { a: 0.0, b: 10.0 };
        let b = TestConfig { a: 1.0, b: 20.0 };
        let result = a.lerp(&b, 0.0);
        assert_eq!(result.a, 0.0);
        assert_eq!(result.b, 10.0);
    }

    #[test]
    fn test_lerp_at_one() {
        let a = TestConfig { a: 0.0, b: 10.0 };
        let b = TestConfig { a: 1.0, b: 20.0 };
        let result = a.lerp(&b, 1.0);
        assert_eq!(result.a, 1.0);
        assert_eq!(result.b, 20.0);
    }

    #[test]
    fn test_lerp_at_half() {
        let a = TestConfig { a: 0.0, b: 10.0 };
        let b = TestConfig { a: 1.0, b: 20.0 };
        let result = a.lerp(&b, 0.5);
        assert_eq!(result.a, 0.5);
        assert_eq!(result.b, 15.0);
    }

    #[test]
    fn test_blend_at_corners() {
        let bl = TestConfig { a: 0.0, b: 0.0 };
        let br = TestConfig { a: 1.0, b: 0.0 };
        let tl = TestConfig { a: 0.0, b: 1.0 };
        let tr = TestConfig { a: 1.0, b: 1.0 };

        let blender = PresetBlender::new(bl, br, tl, tr);

        // Bottom-left corner
        let result = blender.blend(0.0, 0.0);
        assert_eq!(result.a, 0.0);
        assert_eq!(result.b, 0.0);

        // Bottom-right corner
        let result = blender.blend(1.0, 0.0);
        assert_eq!(result.a, 1.0);
        assert_eq!(result.b, 0.0);

        // Top-left corner
        let result = blender.blend(0.0, 1.0);
        assert_eq!(result.a, 0.0);
        assert_eq!(result.b, 1.0);

        // Top-right corner
        let result = blender.blend(1.0, 1.0);
        assert_eq!(result.a, 1.0);
        assert_eq!(result.b, 1.0);
    }

    #[test]
    fn test_blend_at_center() {
        let bl = TestConfig { a: 0.0, b: 0.0 };
        let br = TestConfig { a: 1.0, b: 0.0 };
        let tl = TestConfig { a: 0.0, b: 1.0 };
        let tr = TestConfig { a: 1.0, b: 1.0 };

        let blender = PresetBlender::new(bl, br, tl, tr);
        let result = blender.blend(0.5, 0.5);

        assert_eq!(result.a, 0.5);
        assert_eq!(result.b, 0.5);
    }

    #[test]
    fn test_blend_clamping() {
        let bl = TestConfig { a: 0.0, b: 0.0 };
        let br = TestConfig { a: 1.0, b: 0.0 };
        let tl = TestConfig { a: 0.0, b: 1.0 };
        let tr = TestConfig { a: 1.0, b: 1.0 };

        let blender = PresetBlender::new(bl, br, tl, tr);

        // Values outside 0-1 should be clamped
        let result = blender.blend(-0.5, 1.5);
        assert_eq!(result.a, 0.0);
        assert_eq!(result.b, 1.0);
    }

    #[test]
    fn test_uniform_blender() {
        let preset = TestConfig { a: 0.5, b: 0.75 };
        let blender = PresetBlender::uniform(preset);

        // Should return same value at any position
        let result = blender.blend(0.3, 0.7);
        assert_eq!(result.a, 0.5);
        assert_eq!(result.b, 0.75);
    }
}
