//! Pink noise generator using filtered white noise.
//!
//! Pink noise has a power spectral density that decreases by 3 dB per octave
//! (1/f). This implementation uses a deterministic white-noise source followed
//! by a sample-rate-aware version of Paul Kellet's economy pink-noise filter.

const REFERENCE_SAMPLE_RATE: f32 = 44_100.0;
const RNG_SEED: u64 = 0x1234_5678_9abc_def0;
const REFERENCE_POLES: [f32; 3] = [0.99765, 0.96300, 0.57000];
const REFERENCE_GAINS: [f32; 3] = [0.0990460, 0.2965164, 1.0526913];
const DIRECT_GAIN: f32 = 0.1848;
const OUTPUT_GAIN: f32 = 0.11;

/// Pink noise generator with an approximately 1/f frequency spectrum.
pub struct PinkNoise {
    rng_state: u64,
    filter_state: [f32; 3],
    poles: [f32; 3],
    gains: [f32; 3],
}

impl PinkNoise {
    /// Create a pink-noise generator for a fixed audio sample rate.
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);
        let rate_ratio = REFERENCE_SAMPLE_RATE / sample_rate;
        let mut poles = [0.0; 3];
        let mut gains = [0.0; 3];

        for i in 0..3 {
            poles[i] = REFERENCE_POLES[i].powf(rate_ratio);

            // Preserve each filter branch's white-noise variance as its pole is
            // moved to maintain the same response at a different sample rate.
            gains[i] = REFERENCE_GAINS[i]
                * ((1.0 - poles[i] * poles[i]) / (1.0 - REFERENCE_POLES[i] * REFERENCE_POLES[i]))
                    .sqrt();
        }

        Self {
            rng_state: RNG_SEED,
            filter_state: [0.0; 3],
            poles,
            gains,
        }
    }

    /// Reset the generator to its initial deterministic sequence.
    pub fn reset(&mut self) {
        self.rng_state = RNG_SEED;
        self.filter_state = [0.0; 3];
    }

    /// Generate the next pink-noise sample.
    #[inline]
    pub fn tick(&mut self) -> f32 {
        let white = self.next_white_sample();

        for i in 0..3 {
            self.filter_state[i] = self.poles[i] * self.filter_state[i] + self.gains[i] * white;
        }

        (self.filter_state.iter().sum::<f32>() + white * DIRECT_GAIN) * OUTPUT_GAIN
    }

    #[inline]
    fn next_white_sample(&mut self) -> f32 {
        // xorshift64*
        let mut x = self.rng_state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng_state = x;
        let hashed = x.wrapping_mul(0x2545_f491_4f6c_dd1d);

        // Use the upper 24 bits so every integer is exactly representable as f32.
        let normalized = (hashed >> 40) as f32 / ((1_u32 << 24) - 1) as f32;
        normalized * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn octave_bin_powers(sample_rate: f32) -> Vec<f64> {
        const BLOCK_SIZE: usize = 4096;
        const BLOCK_COUNT: usize = 64;
        const FREQUENCIES: [f32; 6] = [250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0];

        let mut pink_noise = PinkNoise::new(sample_rate);
        for _ in 0..BLOCK_SIZE {
            pink_noise.tick();
        }

        let mut powers = vec![0.0_f64; FREQUENCIES.len()];
        for _ in 0..BLOCK_COUNT {
            let samples: Vec<f32> = (0..BLOCK_SIZE).map(|_| pink_noise.tick()).collect();

            for (power, frequency) in powers.iter_mut().zip(FREQUENCIES) {
                let bin = (frequency * BLOCK_SIZE as f32 / sample_rate).round();
                let omega = TAU * bin / BLOCK_SIZE as f32;
                let mut real = 0.0_f64;
                let mut imag = 0.0_f64;

                for (index, sample) in samples.iter().enumerate() {
                    let phase = omega * index as f32;
                    real += *sample as f64 * phase.cos() as f64;
                    imag -= *sample as f64 * phase.sin() as f64;
                }

                *power += real * real + imag * imag;
            }
        }

        for power in &mut powers {
            *power /= BLOCK_COUNT as f64;
        }
        powers
    }

    fn spectral_slope_db_per_octave(powers: &[f64]) -> f64 {
        let first_db = 10.0 * powers.first().unwrap().log10();
        let last_db = 10.0 * powers.last().unwrap().log10();
        (last_db - first_db) / (powers.len() - 1) as f64
    }

    #[test]
    fn pink_noise_is_finite_bounded_and_nearly_zero_mean() {
        let mut pink_noise = PinkNoise::new(48_000.0);
        let mut sum = 0.0_f64;

        for _ in 0..200_000 {
            let sample = pink_noise.tick();
            assert!(sample.is_finite(), "pink noise should be finite");
            assert!(sample.abs() < 2.0, "pink noise should remain bounded");
            sum += sample as f64;
        }

        let mean = sum / 200_000.0;
        assert!(mean.abs() < 0.03, "pink-noise mean was {mean}");
    }

    #[test]
    fn reset_restores_the_initial_sequence() {
        let mut pink_noise = PinkNoise::new(44_100.0);
        let initial: Vec<f32> = (0..256).map(|_| pink_noise.tick()).collect();

        for _ in 0..1000 {
            pink_noise.tick();
        }
        pink_noise.reset();

        let after_reset: Vec<f32> = (0..256).map(|_| pink_noise.tick()).collect();
        assert_eq!(initial, after_reset);
    }

    #[test]
    fn spectrum_falls_at_a_consistent_rate_across_sample_rates() {
        let mut slopes = Vec::new();

        for sample_rate in [44_100.0, 48_000.0, 96_000.0] {
            let powers = octave_bin_powers(sample_rate);
            for octave_pair in powers.windows(2) {
                assert!(
                    octave_pair[1] < octave_pair[0],
                    "power did not fall across an octave at {sample_rate} Hz: {powers:?}"
                );
            }

            let slope = spectral_slope_db_per_octave(&powers);
            assert!(
                (-4.5..=-1.5).contains(&slope),
                "unexpected slope at {sample_rate} Hz: {slope:.2} dB/octave; powers={powers:?}"
            );
            slopes.push(slope);
        }

        let min_slope = slopes.iter().copied().fold(f64::INFINITY, f64::min);
        let max_slope = slopes.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        assert!(
            max_slope - min_slope < 0.75,
            "sample-rate slopes diverged: {slopes:?}"
        );
    }
}
