//! Pink noise generator using the Voss-McCartney algorithm
//!
//! Pink noise has a power spectral density that decreases by 3 dB per octave (1/f).
//! This implementation uses running sums of white noise sources to approximate pink noise.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Pink noise generator with ~1/f frequency spectrum
///
/// Uses the Voss-McCartney algorithm with multiple octave bands
/// to create noise that falls off at approximately 3 dB per octave.
pub struct PinkNoise {
    /// Current sample counter for noise generation
    sample_counter: u64,

    /// Running sums for different octave bands (typically 3-5 bands)
    octave_sums: [f32; 5],

    /// Update counters for each octave band
    update_counters: [u32; 5],
}

impl PinkNoise {
    /// Create a new pink noise generator
    pub fn new() -> Self {
        Self {
            sample_counter: 0,
            octave_sums: [0.0; 5],
            update_counters: [0; 5],
        }
    }

    /// Reset the generator state
    pub fn reset(&mut self) {
        self.sample_counter = 0;
        self.octave_sums = [0.0; 5];
        self.update_counters = [0; 5];
    }

    /// Generate the next pink noise sample (range approximately -1.0 to 1.0)
    pub fn tick(&mut self) -> f32 {
        self.sample_counter = self.sample_counter.wrapping_add(1);

        // Update octave bands at different rates
        // Band 0: every sample
        // Band 1: every 2 samples
        // Band 2: every 4 samples
        // Band 3: every 8 samples
        // Band 4: every 16 samples

        for i in 0..5 {
            let update_rate = 1 << i; // 1, 2, 4, 8, 16
            self.update_counters[i] += 1;

            if self.update_counters[i] >= update_rate {
                self.update_counters[i] = 0;

                // Generate white noise for this band
                let white_noise = self.generate_white_noise(self.sample_counter, i as u64);
                self.octave_sums[i] = white_noise;
            }
        }

        // Sum all octave bands
        let mut output = 0.0;
        for sum in &self.octave_sums {
            output += sum;
        }

        // Normalize (5 bands of white noise, each in range [-1, 1])
        // Scale factor is approximately 0.2 to keep output in reasonable range
        output * 0.2
    }

    /// Generate white noise using hash function
    fn generate_white_noise(&self, seed: u64, offset: u64) -> f32 {
        let mut hasher = DefaultHasher::new();
        (seed.wrapping_add(offset.wrapping_mul(1000000))).hash(&mut hasher);
        let hash = hasher.finish();

        // Convert hash to float in range [-1.0, 1.0]
        let normalized = (hash as f32) / (u64::MAX as f32);
        (normalized * 2.0) - 1.0
    }
}

impl Default for PinkNoise {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pink_noise_generation() {
        let mut pink_noise = PinkNoise::new();

        // Generate samples and check they're in a reasonable range
        for _ in 0..1000 {
            let sample = pink_noise.tick();
            assert!(sample.is_finite(), "Pink noise should be finite");
            assert!(
                sample.abs() < 2.0,
                "Pink noise should be roughly in range [-1, 1]"
            );
        }
    }

    #[test]
    fn test_pink_noise_reset() {
        let mut pink_noise = PinkNoise::new();

        // Generate some samples
        for _ in 0..100 {
            pink_noise.tick();
        }

        // Reset and verify state
        pink_noise.reset();
        assert_eq!(pink_noise.sample_counter, 0);
    }
}
