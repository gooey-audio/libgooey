//! Reusable mappings from normalized macro controls to DSP values.

/// A piecewise-linear shaping curve over normalized input and output values.
#[derive(Clone, Copy, Debug)]
pub struct MacroCurve {
    points: &'static [(f32, f32)],
}

impl MacroCurve {
    pub const fn new(points: &'static [(f32, f32)]) -> Self {
        Self { points }
    }

    pub fn eval(&self, input: f32) -> f32 {
        let Some(&(first_x, first_y)) = self.points.first() else {
            return input.clamp(0.0, 1.0);
        };
        let input = if input.is_finite() {
            input.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if input <= first_x {
            return first_y;
        }

        for pair in self.points.windows(2) {
            let (x0, y0) = pair[0];
            let (x1, y1) = pair[1];
            if input <= x1 {
                let width = x1 - x0;
                if width <= f32::EPSILON {
                    return y1;
                }
                let t = (input - x0) / width;
                return y0 + (y1 - y0) * t;
            }
        }

        self.points.last().map(|point| point.1).unwrap_or(input)
    }
}

/// Scaling used after a macro curve has shaped a normalized value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MacroScale {
    Linear,
    Log,
}

/// A complete normalized-control mapping with output bounds.
#[derive(Clone, Copy, Debug)]
pub struct MacroTarget {
    pub curve: MacroCurve,
    pub min: f32,
    pub max: f32,
    pub scale: MacroScale,
}

impl MacroTarget {
    pub const fn new(curve: MacroCurve, min: f32, max: f32, scale: MacroScale) -> Self {
        Self {
            curve,
            min,
            max,
            scale,
        }
    }

    pub fn value(&self, input: f32) -> f32 {
        let shaped = self.curve.eval(input).clamp(0.0, 1.0);
        match self.scale {
            MacroScale::Linear => self.min + shaped * (self.max - self.min),
            MacroScale::Log if self.min > 0.0 && self.max > 0.0 => {
                self.min * (self.max / self.min).powf(shaped)
            }
            MacroScale::Log => self.min + shaped * (self.max - self.min),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURVE: MacroCurve = MacroCurve::new(&[(0.0, 0.0), (0.25, 0.1), (1.0, 1.0)]);

    #[test]
    fn eval_hits_breakpoints_exactly() {
        assert_eq!(CURVE.eval(0.0), 0.0);
        assert_eq!(CURVE.eval(0.25), 0.1);
        assert_eq!(CURVE.eval(1.0), 1.0);
    }

    #[test]
    fn eval_interpolates_midpoints() {
        assert!((CURVE.eval(0.125) - 0.05).abs() < 1.0e-6);
        assert!((CURVE.eval(0.625) - 0.55).abs() < 1.0e-6);
    }

    #[test]
    fn log_scale_midpoint_is_geometric_mean() {
        let target = MacroTarget::new(
            MacroCurve::new(&[(0.0, 0.0), (1.0, 1.0)]),
            10.0,
            1_000.0,
            MacroScale::Log,
        );
        assert!((target.value(0.5) - 100.0).abs() < 1.0e-4);
    }

    #[test]
    fn eval_clamps_outside_unit_range() {
        assert_eq!(CURVE.eval(-1.0), 0.0);
        assert_eq!(CURVE.eval(2.0), 1.0);
    }
}
