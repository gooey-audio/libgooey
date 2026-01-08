//! Parameter smoothing for click-free audio parameter changes
//!
//! This module provides smoothed parameters for audio synthesis, preventing
//! discontinuities (clicks/pops) when parameters change during playback.

/// Default smoothing time in milliseconds
pub const DEFAULT_SMOOTH_TIME_MS: f32 = 15.0;

/// A smoothed parameter with range constraints
/// 
/// This is the primary type for modulatable instrument parameters.
/// It combines smoothing with min/max range constraints.
#[derive(Clone, Debug)]
pub struct SmoothedParam {
    /// Current smoothed value
    current: f32,
    /// Target value we're smoothing towards
    target: f32,
    /// Smoothing coefficient (0-1, higher = faster)
    coeff: f32,
    /// Whether we've reached the target (optimization)
    settled: bool,
    /// Minimum allowed value
    pub min: f32,
    /// Maximum allowed value
    pub max: f32,
}

impl SmoothedParam {
    /// Create a new smoothed parameter with range
    /// 
    /// # Arguments
    /// * `initial_value` - Starting value (will be clamped to range)
    /// * `min` - Minimum allowed value
    /// * `max` - Maximum allowed value
    /// * `sample_rate` - Audio sample rate in Hz
    /// * `smooth_time_ms` - Smoothing time in milliseconds (5-50ms typical)
    pub fn new(initial_value: f32, min: f32, max: f32, sample_rate: f32, smooth_time_ms: f32) -> Self {
        let coeff = Self::calculate_coeff(sample_rate, smooth_time_ms);
        let clamped = initial_value.clamp(min, max);
        Self {
            current: clamped,
            target: clamped,
            coeff,
            settled: true,
            min,
            max,
        }
    }

    /// Create a 0-1 normalized parameter
    pub fn new_normalized(initial_value: f32, sample_rate: f32) -> Self {
        Self::new(initial_value.clamp(0.0, 1.0), 0.0, 1.0, sample_rate, DEFAULT_SMOOTH_TIME_MS)
    }

    /// Calculate the smoothing coefficient from sample rate and time
    fn calculate_coeff(sample_rate: f32, smooth_time_ms: f32) -> f32 {
        if smooth_time_ms <= 0.0 {
            return 1.0; // Instant (no smoothing)
        }
        let smooth_time_samples = (smooth_time_ms / 1000.0) * sample_rate;
        // One-pole coefficient: we want to reach ~63% of target in smooth_time_samples
        // coeff = 1 - e^(-1/tau) where tau is time constant in samples
        1.0 - (-1.0 / smooth_time_samples).exp()
    }

    /// Set a new target value to smooth towards (clamped to range)
    pub fn set_target(&mut self, target: f32) {
        let clamped = target.clamp(self.min, self.max);
        if (self.target - clamped).abs() > 1e-8 {
            self.target = clamped;
            self.settled = false;
        }
    }

    /// Set value immediately without smoothing (use sparingly)
    pub fn set_immediate(&mut self, value: f32) {
        let clamped = value.clamp(self.min, self.max);
        self.current = clamped;
        self.target = clamped;
        self.settled = true;
    }

    /// Set target from a normalized 0-1 value (maps to parameter range)
    pub fn set_normalized(&mut self, normalized: f32) {
        let value = self.min + normalized.clamp(0.0, 1.0) * (self.max - self.min);
        self.set_target(value);
    }

    /// Set target from a bipolar -1 to 1 value (maps to parameter range)
    /// This is useful for LFO modulation
    pub fn set_bipolar(&mut self, bipolar: f32) {
        let normalized = (bipolar.clamp(-1.0, 1.0) + 1.0) * 0.5;
        self.set_normalized(normalized);
    }

    /// Process one sample, returning the smoothed value
    /// Call this once per audio sample
    #[inline]
    pub fn tick(&mut self) -> f32 {
        if self.settled {
            return self.current;
        }

        // One-pole lowpass: current += coeff * (target - current)
        self.current += self.coeff * (self.target - self.current);

        // Check if we've effectively reached the target
        if (self.current - self.target).abs() < 1e-6 {
            self.current = self.target;
            self.settled = true;
        }

        self.current
    }

    /// Get the current smoothed value without advancing
    #[inline]
    pub fn get(&self) -> f32 {
        self.current
    }

    /// Get the target value
    pub fn target(&self) -> f32 {
        self.target
    }

    /// Check if the smoother has reached its target
    pub fn is_settled(&self) -> bool {
        self.settled
    }

    /// Get the parameter range as (min, max)
    pub fn range(&self) -> (f32, f32) {
        (self.min, self.max)
    }

    /// Update the smoothing time
    pub fn set_smooth_time(&mut self, sample_rate: f32, smooth_time_ms: f32) {
        self.coeff = Self::calculate_coeff(sample_rate, smooth_time_ms);
    }

    /// Update sample rate (recalculates coefficient)
    pub fn set_sample_rate(&mut self, sample_rate: f32, smooth_time_ms: f32) {
        self.coeff = Self::calculate_coeff(sample_rate, smooth_time_ms);
    }
}

/// Legacy alias for backwards compatibility
pub type ParamSmoother = SmoothedParam;

impl ParamSmoother {
    /// Legacy constructor (creates unbounded smoother)
    pub fn new_legacy(initial_value: f32, sample_rate: f32, smooth_time_ms: f32) -> Self {
        Self::new(initial_value, f32::MIN, f32::MAX, sample_rate, smooth_time_ms)
    }
    
    /// Get current value (legacy name)
    pub fn current(&self) -> f32 {
        self.get()
    }
}

impl Default for SmoothedParam {
    fn default() -> Self {
        Self::new(0.0, 0.0, 1.0, 44100.0, DEFAULT_SMOOTH_TIME_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_smoother_reaches_target() {
        let mut smoother = SmoothedParam::new(0.0, 0.0, 1.0, 44100.0, 10.0);
        smoother.set_target(1.0);
        
        // Run for 100ms worth of samples (10x the time constant, should definitely settle)
        // One-pole filter reaches 99.995% of target after 10 time constants
        for _ in 0..(44100 / 10) {
            smoother.tick();
        }
        
        assert!((smoother.get() - 1.0).abs() < 0.001, "Expected ~1.0, got {}", smoother.get());
        assert!(smoother.is_settled());
    }

    #[test]
    fn test_immediate_set() {
        let mut smoother = SmoothedParam::new(0.0, 0.0, 1.0, 44100.0, 10.0);
        smoother.set_immediate(1.0);
        
        assert_eq!(smoother.get(), 1.0);
        assert!(smoother.is_settled());
    }

    #[test]
    fn test_range_clamping() {
        let mut smoother = SmoothedParam::new(50.0, 20.0, 200.0, 44100.0, 10.0);
        
        // Test clamping above max
        smoother.set_target(300.0);
        assert_eq!(smoother.target(), 200.0);
        
        // Test clamping below min
        smoother.set_target(10.0);
        assert_eq!(smoother.target(), 20.0);
    }

    #[test]
    fn test_normalized_set() {
        let mut smoother = SmoothedParam::new(50.0, 0.0, 100.0, 44100.0, 10.0);
        
        smoother.set_normalized(0.5);
        assert_eq!(smoother.target(), 50.0);
        
        smoother.set_normalized(0.0);
        assert_eq!(smoother.target(), 0.0);
        
        smoother.set_normalized(1.0);
        assert_eq!(smoother.target(), 100.0);
    }

    #[test]
    fn test_bipolar_set() {
        let mut smoother = SmoothedParam::new(50.0, 0.0, 100.0, 44100.0, 10.0);
        
        smoother.set_bipolar(0.0);  // Center
        assert_eq!(smoother.target(), 50.0);
        
        smoother.set_bipolar(-1.0); // Min
        assert_eq!(smoother.target(), 0.0);
        
        smoother.set_bipolar(1.0);  // Max
        assert_eq!(smoother.target(), 100.0);
    }
}
