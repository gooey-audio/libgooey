/// 2x oversampling for nonlinear audio effects.
///
/// Upsamples input by 2x using linear interpolation, applies the nonlinear
/// function at the higher rate, then downsamples by averaging. This reduces
/// aliasing from waveshaping, saturation, and other nonlinear processing.
pub struct Oversampler2x {
    /// Previous input sample for linear interpolation during upsampling
    prev_input: f32,
}

impl Oversampler2x {
    pub fn new() -> Self {
        Self { prev_input: 0.0 }
    }

    /// Process one input sample through a nonlinear function at 2x rate.
    ///
    /// 1. Upsamples by inserting a linearly interpolated midpoint
    /// 2. Applies `f` to both oversampled values
    /// 3. Downsamples by averaging the two processed samples
    #[inline]
    pub fn process(&mut self, input: f32, mut f: impl FnMut(f32) -> f32) -> f32 {
        // Upsample: create interpolated midpoint between previous and current
        let mid = (self.prev_input + input) * 0.5;
        self.prev_input = input;

        // Process both samples through the nonlinear function
        let y0 = f(mid);
        let y1 = f(input);

        // Downsample by averaging (acts as a simple lowpass)
        (y0 + y1) * 0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oversampler_passthrough_dc() {
        let mut os = Oversampler2x::new();
        // Feed constant DC — output should converge to the same DC value
        let mut last = 0.0;
        for _ in 0..100 {
            last = os.process(1.0, |x| x);
        }
        assert!(
            (last - 1.0).abs() < 0.01,
            "DC passthrough expected ~1.0, got {}",
            last
        );
    }

    #[test]
    fn test_oversampler_with_nonlinearity() {
        let mut os = Oversampler2x::new();
        // Process a sine wave through tanh — should not panic or produce NaN
        let sample_rate = 44100.0_f32;
        let freq = 1000.0;
        for i in 0..44100 {
            let input = (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate).sin();
            let output = os.process(input, |x| (x * 3.0).tanh());
            assert!(!output.is_nan(), "output should not be NaN");
            assert!(
                output.abs() < 2.0,
                "output should be bounded, got {}",
                output
            );
        }
    }
}
