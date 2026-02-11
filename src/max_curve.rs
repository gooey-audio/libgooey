//! Max/MSP curve~ algorithm implementation
//!
//! This module provides an exact implementation of the Max/MSP curve~ object's
//! exponential interpolation algorithm, allowing for accurate reproduction of
//! Max patches in Rust.

/// Calculate curved interpolation using the Max/MSP curve~ algorithm.
///
/// This is the exact formula used by Max/MSP's curve~ object, derived from
/// Emmanuel Jourdan's ej.function source code.
///
/// # Arguments
/// * `progress` - Linear progress through the segment (0.0 to 1.0)
/// * `curve` - Curve parameter (-1.0 to 1.0):
///   - 0.0: Linear interpolation
///   - Positive: Exponential curve (slow start, fast end)
///   - Negative: Logarithmic curve (fast start, slow end)
///
/// # Returns
/// Curved progress value (0.0 to 1.0)
pub fn max_curve(progress: f32, curve: f32) -> f32 {
    let progress = progress.clamp(0.0, 1.0);

    // Linear when curve is near zero
    if curve.abs() < 1e-6 {
        return progress;
    }

    // Calculate exponential factor from curve parameter
    // These magic constants come from the ej.function source
    let hp = ((curve.abs() + 1e-20) * 1.2).powf(0.41) * 0.91;
    let fp = hp / (1.0 - hp);

    // Avoid division by zero for very small fp
    if fp.abs() < 1e-6 {
        return progress;
    }

    // The core exponential curve formula: (e^(fp*x) - 1) / (e^fp - 1)
    let gp = (fp * progress).exp_m1() / fp.exp_m1();

    // Negative curve reverses the shape
    if curve < 0.0 {
        1.0 - max_curve(1.0 - progress, -curve)
    } else {
        gp
    }
}

/// A single segment of a multi-segment envelope
#[derive(Clone, Debug)]
pub struct EnvelopeSegment {
    /// Target value to reach at end of segment
    pub target_value: f32,
    /// Duration in seconds
    pub duration_secs: f32,
    /// Curve parameter (-1.0 to 1.0)
    pub curve: f32,
}

impl EnvelopeSegment {
    pub fn new(target_value: f32, duration_ms: f32, curve: f32) -> Self {
        Self {
            target_value,
            duration_secs: duration_ms / 1000.0,
            curve,
        }
    }
}

/// Multi-segment envelope using Max/MSP curve~ algorithm.
///
/// Accepts segments in the same format as Max's curve~ object:
/// (target_value, time_ms, curve)
#[derive(Clone)]
pub struct MaxCurveEnvelope {
    segments: Vec<EnvelopeSegment>,
    current_segment: usize,
    segment_start_time: f32,
    segment_start_value: f32,
    current_value: f32,
    pub is_active: bool,
    trigger_time: f32,
    initial_value: f32,
}

impl MaxCurveEnvelope {
    /// Create a new envelope from segments.
    ///
    /// Each tuple is (target_value, time_ms, curve).
    /// Example: `vec![(1.0, 1.0, 0.8), (0.0, 2000.0, -0.83)]`
    pub fn new(segments: Vec<(f32, f32, f32)>) -> Self {
        let envelope_segments: Vec<EnvelopeSegment> = segments
            .into_iter()
            .map(|(target, time_ms, curve)| EnvelopeSegment::new(target, time_ms, curve))
            .collect();

        Self {
            segments: envelope_segments,
            current_segment: 0,
            segment_start_time: 0.0,
            segment_start_value: 0.0,
            current_value: 0.0,
            is_active: false,
            trigger_time: 0.0,
            initial_value: 0.0,
        }
    }

    /// Set the initial value before triggering
    pub fn set_initial_value(&mut self, value: f32) {
        self.initial_value = value;
    }

    /// Trigger the envelope at the given time
    pub fn trigger(&mut self, time: f32) {
        self.is_active = true;
        self.trigger_time = time;
        self.current_segment = 0;
        self.segment_start_time = time;
        self.segment_start_value = self.initial_value;
        self.current_value = self.initial_value;
    }

    /// Get the current envelope value at the given time
    pub fn get_value(&mut self, current_time: f32) -> f32 {
        if !self.is_active {
            return self.current_value;
        }

        // Process segments
        loop {
            if self.current_segment >= self.segments.len() {
                // Envelope complete
                self.is_active = false;
                return self.current_value;
            }

            let segment = &self.segments[self.current_segment];
            let elapsed_in_segment = current_time - self.segment_start_time;

            if elapsed_in_segment >= segment.duration_secs {
                // Move to next segment
                self.segment_start_value = segment.target_value;
                self.current_value = segment.target_value;
                self.segment_start_time += segment.duration_secs;
                self.current_segment += 1;
                continue;
            }

            // Calculate progress within segment
            let progress = if segment.duration_secs > 0.0 {
                elapsed_in_segment / segment.duration_secs
            } else {
                1.0
            };

            // Apply Max curve
            let curved_progress = max_curve(progress, segment.curve);

            // Interpolate between start and target
            let range = segment.target_value - self.segment_start_value;
            self.current_value = self.segment_start_value + range * curved_progress;

            return self.current_value;
        }
    }

    /// Check if the envelope has completed all segments
    pub fn is_complete(&self) -> bool {
        !self.is_active && self.current_segment >= self.segments.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_max_curve_linear() {
        // Curve of 0 should be linear
        assert!((max_curve(0.0, 0.0) - 0.0).abs() < 0.001);
        assert!((max_curve(0.5, 0.0) - 0.5).abs() < 0.001);
        assert!((max_curve(1.0, 0.0) - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_max_curve_endpoints() {
        // Endpoints should always be 0 and 1 regardless of curve
        for curve in [-0.9, -0.5, 0.0, 0.5, 0.9] {
            assert!(
                (max_curve(0.0, curve) - 0.0).abs() < 0.001,
                "curve {}: start should be 0",
                curve
            );
            assert!(
                (max_curve(1.0, curve) - 1.0).abs() < 0.001,
                "curve {}: end should be 1",
                curve
            );
        }
    }

    #[test]
    fn test_max_curve_negative_logarithmic() {
        // Negative curve = logarithmic = fast initial change
        // At 50% progress, the value should be > 0.5 (already most of the way there)
        let value = max_curve(0.5, -0.83);
        assert!(
            value > 0.5,
            "Negative curve should be above midpoint at 50%"
        );
    }

    #[test]
    fn test_max_curve_positive_exponential() {
        // Positive curve = exponential = slow initial change
        // At 50% progress, the value should be < 0.5 (still building up)
        let value = max_curve(0.5, 0.83);
        assert!(
            value < 0.5,
            "Positive curve should be below midpoint at 50%"
        );
    }

    #[test]
    fn test_envelope_basic() {
        let mut env = MaxCurveEnvelope::new(vec![
            (1.0, 10.0, 0.0),  // Linear ramp to 1.0 in 10ms
            (0.0, 100.0, 0.0), // Linear ramp to 0.0 in 100ms
        ]);

        env.trigger(0.0);

        // At start
        let v = env.get_value(0.0);
        assert!((v - 0.0).abs() < 0.01, "Start should be 0");

        // Midway through first segment (5ms = 0.005s)
        let v = env.get_value(0.005);
        assert!(
            (v - 0.5).abs() < 0.1,
            "Should be ~0.5 at midpoint of first segment"
        );

        // End of first segment (10ms = 0.01s)
        let v = env.get_value(0.01);
        assert!(
            (v - 1.0).abs() < 0.1,
            "Should be ~1.0 at end of first segment"
        );

        // Midway through second segment (60ms = 0.06s)
        let v = env.get_value(0.06);
        assert!(
            (v - 0.5).abs() < 0.1,
            "Should be ~0.5 at midpoint of second segment"
        );
    }
}
