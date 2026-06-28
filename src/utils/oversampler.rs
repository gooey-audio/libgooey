use halfband::iir::{Downsampler8, Upsampler8};

/// Selects the sample-rate multiplier used around a nonlinear function.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum OversamplingMode {
    /// Process the nonlinear function at the engine sample rate.
    Off = 0,
    /// Process the nonlinear function at twice the engine sample rate.
    X2 = 2,
    /// Process the nonlinear function at four times the engine sample rate.
    #[default]
    X4 = 4,
}

impl OversamplingMode {
    /// Return the nonlinear function's sample-rate multiplier.
    pub const fn factor(self) -> usize {
        match self {
            Self::Off => 1,
            Self::X2 => 2,
            Self::X4 => 4,
        }
    }

    pub(crate) const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Off,
            2 => Self::X2,
            _ => Self::X4,
        }
    }
}

/// Low-latency 2x oversampling for nonlinear audio effects.
///
/// Uses a polyphase IIR half-band filter pair with 94 dB attenuation.
pub struct Oversampler2x {
    upsampler: Upsampler8,
    downsampler: Downsampler8,
}

impl Oversampler2x {
    pub fn new() -> Self {
        Self {
            upsampler: Upsampler8::default(),
            downsampler: Downsampler8::default(),
        }
    }

    /// Process one input sample through a nonlinear function at 2x rate.
    #[inline]
    pub fn process(&mut self, input: f32, mut f: impl FnMut(f32) -> f32) -> f32 {
        let [s0, s1] = self.upsampler.process(input);
        self.downsampler.process(f(s0), f(s1))
    }

    /// Clear all filter history without reallocating coefficients.
    pub fn reset(&mut self) {
        self.upsampler.clear();
        self.downsampler.clear();
    }
}

impl Default for Oversampler2x {
    fn default() -> Self {
        Self::new()
    }
}

/// Low-latency 4x oversampling for nonlinear audio effects.
///
/// Cascades two polyphase IIR half-band filter pairs. The nonlinear closure is
/// evaluated four times per engine-rate input sample.
pub struct Oversampler4x {
    outer_upsampler: Upsampler8,
    inner_upsampler: Upsampler8,
    inner_downsampler: Downsampler8,
    outer_downsampler: Downsampler8,
}

impl Oversampler4x {
    pub fn new() -> Self {
        Self {
            outer_upsampler: Upsampler8::default(),
            inner_upsampler: Upsampler8::default(),
            inner_downsampler: Downsampler8::default(),
            outer_downsampler: Downsampler8::default(),
        }
    }

    /// Process one input sample through a nonlinear function at 4x rate.
    #[inline]
    pub fn process(&mut self, input: f32, mut f: impl FnMut(f32) -> f32) -> f32 {
        let [outer_0, outer_1] = self.outer_upsampler.process(input);

        let [inner_0, inner_1] = self.inner_upsampler.process(outer_0);
        let down_0 = self.inner_downsampler.process(f(inner_0), f(inner_1));

        let [inner_2, inner_3] = self.inner_upsampler.process(outer_1);
        let down_1 = self.inner_downsampler.process(f(inner_2), f(inner_3));

        self.outer_downsampler.process(down_0, down_1)
    }

    /// Clear all filter history without reallocating coefficients.
    pub fn reset(&mut self) {
        self.outer_upsampler.clear();
        self.inner_upsampler.clear();
        self.inner_downsampler.clear();
        self.outer_downsampler.clear();
    }
}

impl Default for Oversampler4x {
    fn default() -> Self {
        Self::new()
    }
}

/// Runtime-selectable oversampling for nonlinear audio effects.
///
/// Changing mode clears both filter paths so stale history is never reused.
/// The default mode is [`OversamplingMode::X4`].
pub struct Oversampler {
    mode: OversamplingMode,
    x2: Oversampler2x,
    x4: Oversampler4x,
}

impl Oversampler {
    pub fn new(mode: OversamplingMode) -> Self {
        Self {
            mode,
            x2: Oversampler2x::new(),
            x4: Oversampler4x::new(),
        }
    }

    /// Process one input sample at the selected oversampling rate.
    #[inline]
    pub fn process(&mut self, input: f32, f: impl FnMut(f32) -> f32) -> f32 {
        match self.mode {
            OversamplingMode::Off => {
                let mut f = f;
                f(input)
            }
            OversamplingMode::X2 => self.x2.process(input, f),
            OversamplingMode::X4 => self.x4.process(input, f),
        }
    }

    /// Change the oversampling rate and clear all filter history.
    ///
    /// Switching an active signal can create a discontinuity, so clean A/B
    /// comparisons should change mode while stopped or silent.
    pub fn set_mode(&mut self, mode: OversamplingMode) {
        if self.mode != mode {
            self.mode = mode;
            self.reset();
        }
    }

    pub fn mode(&self) -> OversamplingMode {
        self.mode
    }

    /// Clear all filter history without changing the selected mode.
    pub fn reset(&mut self) {
        self.x2.reset();
        self.x4.reset();
    }
}

impl Default for Oversampler {
    fn default() -> Self {
        Self::new(OversamplingMode::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;

    const TEST_SAMPLE_RATE: f32 = 48_000.0;
    const TEST_FREQUENCY: f32 = 10_000.0;
    const TEST_DRIVE: f32 = 10.0;
    const TEST_SAMPLES: usize = 4_800;
    const WARMUP_SAMPLES: usize = 1_024;

    struct Oversampler16x {
        outer: Oversampler4x,
        inner: Oversampler4x,
    }

    impl Oversampler16x {
        fn new() -> Self {
            Self {
                outer: Oversampler4x::new(),
                inner: Oversampler4x::new(),
            }
        }

        fn process(&mut self, input: f32, mut f: impl FnMut(f32) -> f32) -> f32 {
            let outer = &mut self.outer;
            let inner = &mut self.inner;
            outer.process(input, |x| inner.process(x, &mut f))
        }
    }

    fn test_input(sample: usize) -> f32 {
        (std::f32::consts::TAU * TEST_FREQUENCY * sample as f32 / TEST_SAMPLE_RATE).sin() * 0.8
    }

    fn render_base_rate() -> Vec<f32> {
        (WARMUP_SAMPLES..WARMUP_SAMPLES + TEST_SAMPLES)
            .map(|i| (test_input(i) * TEST_DRIVE).tanh())
            .collect()
    }

    fn render_4x() -> Vec<f32> {
        let mut oversampler = Oversampler4x::new();
        (0..WARMUP_SAMPLES + TEST_SAMPLES)
            .filter_map(|i| {
                let output = oversampler.process(test_input(i), |x| (x * TEST_DRIVE).tanh());
                (i >= WARMUP_SAMPLES).then_some(output)
            })
            .collect()
    }

    fn render_16x_reference() -> Vec<f32> {
        let mut oversampler = Oversampler16x::new();
        (0..WARMUP_SAMPLES + TEST_SAMPLES)
            .filter_map(|i| {
                let output = oversampler.process(test_input(i), |x| (x * TEST_DRIVE).tanh());
                (i >= WARMUP_SAMPLES).then_some(output)
            })
            .collect()
    }

    fn bin_power(samples: &[f32], frequency: f32) -> f64 {
        let phase_step = TAU * frequency as f64 / TEST_SAMPLE_RATE as f64;
        let (real, imag) =
            samples
                .iter()
                .enumerate()
                .fold((0.0_f64, 0.0_f64), |(real, imag), (i, &x)| {
                    let phase = phase_step * i as f64;
                    (real + x as f64 * phase.cos(), imag - x as f64 * phase.sin())
                });
        real * real + imag * imag
    }

    fn combined_power(samples: &[f32], frequencies: &[f32]) -> f64 {
        frequencies
            .iter()
            .map(|&frequency| bin_power(samples, frequency))
            .sum()
    }

    fn magnitude_vector(samples: &[f32], frequencies: &[f32]) -> Vec<f64> {
        frequencies
            .iter()
            .map(|&frequency| bin_power(samples, frequency).sqrt())
            .collect()
    }

    fn squared_vector_error(actual: &[f64], reference: &[f64]) -> f64 {
        actual
            .iter()
            .zip(reference)
            .map(|(actual, reference)| (actual - reference).powi(2))
            .sum()
    }

    #[test]
    fn test_oversampler_passthrough_dc() {
        let mut os = Oversampler2x::new();
        // Feed constant DC - output should converge to the same DC value.
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
        // Process a sine wave through tanh - should not panic or produce NaN.
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

    #[test]
    fn test_oversampler_4x_passthrough_dc() {
        let mut os = Oversampler4x::new();
        let mut last = 0.0;
        for _ in 0..200 {
            last = os.process(1.0, |x| x);
        }
        assert!(
            (last - 1.0).abs() < 0.01,
            "DC passthrough expected ~1.0, got {last}"
        );
    }

    #[test]
    fn test_oversampler_4x_reset_matches_fresh_instance() {
        let mut reset = Oversampler4x::new();
        for i in 0..1000 {
            reset.process((i as f32 * 0.1).sin(), |x| (x * 5.0).tanh());
        }
        reset.reset();

        let mut fresh = Oversampler4x::new();
        for i in 0..100 {
            let input = (i as f32 * 0.27).sin();
            assert_eq!(
                reset.process(input, |x| (x * 5.0).tanh()),
                fresh.process(input, |x| (x * 5.0).tanh())
            );
        }
    }

    #[test]
    fn test_selectable_oversampler_defaults_to_4x() {
        let oversampler = Oversampler::default();
        assert_eq!(oversampler.mode(), OversamplingMode::X4);
        assert_eq!(oversampler.mode().factor(), 4);
    }

    #[test]
    fn test_selectable_oversampler_off_is_exact() {
        let mut oversampler = Oversampler::new(OversamplingMode::Off);
        assert_eq!(oversampler.process(0.25, |x| x * 2.0), 0.5);
    }

    #[test]
    fn test_selectable_oversampler_mode_change_resets_filter_history() {
        let mut switched = Oversampler::new(OversamplingMode::X4);
        for i in 0..1000 {
            switched.process((i as f32 * 0.1).sin(), |x| (x * 5.0).tanh());
        }
        switched.set_mode(OversamplingMode::X2);

        let mut fresh = Oversampler::new(OversamplingMode::X2);
        for i in 0..100 {
            let input = (i as f32 * 0.27).sin();
            assert_eq!(
                switched.process(input, |x| (x * 5.0).tanh()),
                fresh.process(input, |x| (x * 5.0).tanh())
            );
        }
    }

    #[test]
    fn test_oversampler_4x_reduces_known_tanh_aliases_by_20_db() {
        let base_rate = render_base_rate();
        let oversampled = render_4x();
        let alias_frequencies = [2_000.0, 18_000.0, 22_000.0];

        let base_alias_power = combined_power(&base_rate, &alias_frequencies);
        let oversampled_alias_power = combined_power(&oversampled, &alias_frequencies);
        let reduction_db = 10.0 * (base_alias_power / oversampled_alias_power).log10();

        assert!(
            reduction_db >= 20.0,
            "expected at least 20 dB alias reduction, measured {reduction_db:.2} dB"
        );

        let base_fundamental = bin_power(&base_rate, TEST_FREQUENCY).sqrt();
        let oversampled_fundamental = bin_power(&oversampled, TEST_FREQUENCY).sqrt();
        let fundamental_change_db = 20.0 * (oversampled_fundamental / base_fundamental).log10();
        assert!(
            fundamental_change_db.abs() < 1.0,
            "fundamental changed by {fundamental_change_db:.2} dB"
        );
    }

    #[test]
    fn test_oversampler_4x_is_closer_to_16x_spectral_reference_than_base_rate() {
        let base_rate = render_base_rate();
        let oversampled = render_4x();
        let reference = render_16x_reference();
        let measured_frequencies = [2_000.0, 10_000.0, 18_000.0, 22_000.0];

        let base_magnitudes = magnitude_vector(&base_rate, &measured_frequencies);
        let oversampled_magnitudes = magnitude_vector(&oversampled, &measured_frequencies);
        let reference_magnitudes = magnitude_vector(&reference, &measured_frequencies);

        let base_error = squared_vector_error(&base_magnitudes, &reference_magnitudes);
        let oversampled_error =
            squared_vector_error(&oversampled_magnitudes, &reference_magnitudes);

        assert!(
            oversampled_error < base_error * 0.1,
            "4x spectral error ({oversampled_error:.3}) should be at least 10x lower than base-rate error ({base_error:.3})"
        );
    }
}
