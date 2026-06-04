use std::f32::consts::PI;

/// Stable two-pole resonant low-pass filter for instrument use.
///
/// Uses a topology-preserving-transform state-variable filter and exposes the
/// low-pass output. Designed for use within instruments (non-atomic, mutable
/// API).
pub struct ResonantLowpassFilter {
    pub sample_rate: f32,
    pub cutoff_freq: f32,
    pub resonance: f32,
    g: f32,
    r: f32,
    h: f32,
    ic1eq: f32,
    ic2eq: f32,
}

impl ResonantLowpassFilter {
    /// Create a new resonant low-pass filter.
    ///
    /// # Arguments
    /// * `sample_rate` - Audio sample rate in Hz
    /// * `cutoff_freq` - Cutoff frequency in Hz
    /// * `resonance` - Resonance/Q factor (0.5-10.0)
    pub fn new(sample_rate: f32, cutoff_freq: f32, resonance: f32) -> Self {
        let mut filter = Self {
            sample_rate,
            cutoff_freq: cutoff_freq.clamp(20.0, 20_000.0),
            resonance: resonance.clamp(0.5, 10.0),
            g: 0.0,
            r: 0.0,
            h: 0.0,
            ic1eq: 0.0,
            ic2eq: 0.0,
        };
        filter.update_coefficients();
        filter
    }

    /// Reset filter state.
    pub fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }

    /// Process a single sample through the low-pass output.
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let v1 = (self.g * (input - self.ic2eq) + self.ic1eq) * self.h;
        let v2 = self.ic2eq + self.g * v1;

        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;

        if v2.abs() < 1e-15 {
            0.0
        } else {
            v2
        }
    }

    /// Set cutoff frequency.
    pub fn set_cutoff_freq(&mut self, cutoff_freq: f32) {
        let cutoff_freq = cutoff_freq.clamp(20.0, 20_000.0);
        if (cutoff_freq - self.cutoff_freq).abs() > 0.001 {
            self.cutoff_freq = cutoff_freq;
            self.update_coefficients();
        }
    }

    /// Set resonance/Q.
    pub fn set_resonance(&mut self, resonance: f32) {
        let resonance = resonance.clamp(0.5, 10.0);
        if (resonance - self.resonance).abs() > 0.001 {
            self.resonance = resonance;
            self.update_coefficients();
        }
    }

    /// Set cutoff and resonance together, recalculating coefficients once.
    pub fn set_params(&mut self, cutoff_freq: f32, resonance: f32) {
        let cutoff_freq = cutoff_freq.clamp(20.0, 20_000.0);
        let resonance = resonance.clamp(0.5, 10.0);

        if (cutoff_freq - self.cutoff_freq).abs() > 0.001
            || (resonance - self.resonance).abs() > 0.001
        {
            self.cutoff_freq = cutoff_freq;
            self.resonance = resonance;
            self.update_coefficients();
        }
    }

    fn update_coefficients(&mut self) {
        let sample_rate = self.sample_rate.max(1.0);
        let cutoff = self.cutoff_freq.clamp(20.0, sample_rate * 0.45);
        let q = self.resonance.clamp(0.5, 10.0);

        self.g = (PI * cutoff / sample_rate).tan();
        self.r = 1.0 / q;
        self.h = 1.0 / (1.0 + self.r * self.g + self.g * self.g);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn response_rms(sample_rate: f32, cutoff: f32, q: f32, frequency: f32) -> f32 {
        let mut filter = ResonantLowpassFilter::new(sample_rate, cutoff, q);
        let sample_count = sample_rate as usize;
        let mut sum_squares = 0.0_f64;

        for index in 0..sample_count {
            let input = (TAU * frequency * index as f32 / sample_rate).sin();
            let output = filter.process(input);
            if index >= sample_count / 2 {
                sum_squares += (output * output) as f64;
            }
        }

        (sum_squares / (sample_count / 2) as f64).sqrt() as f32
    }

    #[test]
    fn creation_clamps_parameters_to_stable_ranges() {
        let filter = ResonantLowpassFilter::new(44_100.0, 30_000.0, 0.0);
        assert_eq!(filter.sample_rate, 44_100.0);
        assert_eq!(filter.cutoff_freq, 20_000.0);
        assert_eq!(filter.resonance, 0.5);
    }

    #[test]
    fn reset_clears_filter_history() {
        let mut filter = ResonantLowpassFilter::new(44_100.0, 1000.0, 2.0);
        for _ in 0..100 {
            filter.process(1.0);
        }

        filter.reset();
        assert_eq!(filter.ic1eq, 0.0);
        assert_eq!(filter.ic2eq, 0.0);
    }

    #[test]
    fn lowpass_attenuates_frequencies_above_cutoff() {
        let low = response_rms(48_000.0, 1000.0, 0.707, 100.0);
        let high = response_rms(48_000.0, 1000.0, 0.707, 8000.0);
        assert!(low > high * 10.0, "low={low}, high={high}");
    }

    #[test]
    fn resonance_boosts_response_near_cutoff() {
        let low_q = response_rms(48_000.0, 1000.0, 0.5, 1000.0);
        let high_q = response_rms(48_000.0, 1000.0, 4.0, 1000.0);
        assert!(high_q > low_q * 4.0, "low_q={low_q}, high_q={high_q}");
    }

    #[test]
    fn remains_stable_at_extreme_settings_and_sample_rates() {
        for sample_rate in [44_100.0, 48_000.0, 96_000.0] {
            for cutoff in [20.0, 1000.0, 20_000.0] {
                for resonance in [0.5, 5.0, 10.0] {
                    let mut filter = ResonantLowpassFilter::new(sample_rate, cutoff, resonance);

                    for index in 0..sample_rate as usize {
                        let input = (index as f32 * 0.1).sin();
                        let output = filter.process(input);
                        assert!(
                            output.is_finite() && output.abs() < 100.0,
                            "unstable at sample_rate={sample_rate}, cutoff={cutoff}, resonance={resonance}: {output}"
                        );
                    }
                }
            }
        }
    }
}
