use super::Effect;

/// A brick wall limiter that prevents audio signals from exceeding a threshold
pub struct BrickWallLimiter {
    pub threshold: f32,
}

impl BrickWallLimiter {
    pub fn new(threshold: f32) -> Self {
        Self { threshold }
    }
}

impl Effect for BrickWallLimiter {
    /// Apply brick wall limiting to the input signal
    fn process(&self, input: f32) -> f32 {
        if input > self.threshold {
            self.threshold
        } else if input < -self.threshold {
            -self.threshold
        } else {
            input
        }
    }
}

/// A soft limiter using tanh saturation to prevent clipping without
/// introducing hard discontinuities that cause aliasing.
pub struct SoftLimiter {
    pub threshold: f32,
    inv_threshold: f32,
}

impl SoftLimiter {
    pub fn new(threshold: f32) -> Self {
        let threshold = threshold.max(0.001);
        Self {
            threshold,
            inv_threshold: 1.0 / threshold,
        }
    }
}

impl Effect for SoftLimiter {
    fn process(&self, input: f32) -> f32 {
        (input * self.inv_threshold).tanh() * self.threshold
    }
}
