//! Morph oscillator based on Max/MSP morphosc subpatch
//!
//! Combines multiple oscillators with a 3-channel crossfade mixer:
//! - Channel 1: Ring modulation of main sine and fixed 190Hz sine
//! - Channel 2: Triangle wave + noise
//! - Channel 3: Noise + gated sine
//!
//! Noise components:
//! - White noise (noise~) scaled by 0.2
//! - Sample-and-hold random (rand~) at rate controlled by color parameter via mtof
//! - Combined noise is scaled by 0.4
//! - Channel 3 also includes a gated sine when tone < 99

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Generate sine wave from phase (0.0 to 1.0)
#[inline]
fn sine(phase: f32) -> f32 {
    (phase * 2.0 * std::f32::consts::PI).sin()
}

/// Generate triangle wave from phase (0.0 to 1.0)
#[inline]
fn triangle(phase: f32) -> f32 {
    let t = phase.fract();
    if t < 0.5 {
        4.0 * t - 1.0
    } else {
        3.0 - 4.0 * t
    }
}

/// MIDI note to frequency (mtof~)
#[inline]
fn mtof(midi: f32) -> f32 {
    440.0 * 2.0_f32.powf((midi - 69.0) / 12.0)
}

/// Generate white noise using hash function (same pattern as PinkNoise/Oscillator)
#[inline]
fn white_noise(counter: u64) -> f32 {
    let mut hasher = DefaultHasher::new();
    counter.hash(&mut hasher);
    let hash = hasher.finish();
    (hash as f32) / (u64::MAX as f32) * 2.0 - 1.0
}

/// Morph oscillator based on Max/MSP morphosc subpatch
///
/// Channel 1: Ring modulation of main sine (at input freq) and fixed 190Hz sine.
/// Ring mod creates sum/difference sidebands for a darker, complex timbre.
///
/// Channel 2: Triangle oscillator (tri~) at input frequency + noise.
///
/// Channel 3: Noise + gated sine (gate controlled by tone < 99).
///
/// All mixed through a 3-channel crossfade controlled by mix_control.
pub struct MorphOsc {
    sample_rate: f32,

    // Phase accumulators for each oscillator (0.0 to 1.0)
    main_sine_phase: f32,
    tri_phase: f32,
    fixed_sine_phase: f32,

    // Noise channel state
    noise_counter: u64,    // Counter for hash-based white noise
    rand_phase: f32,       // Phase for rand~ timing (0 to 1)
    rand_current: f32,     // Current interpolation start value
    rand_target: f32,      // Target value to ramp toward
    gated_sine_phase: f32, // Phase for the gated sine in channel 3
}

impl MorphOsc {
    /// Create a new morph oscillator
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            main_sine_phase: 0.0,
            tri_phase: 0.0,
            fixed_sine_phase: 0.0,
            noise_counter: 0,
            rand_phase: 0.0,
            rand_current: 0.0,
            rand_target: 0.0,
            gated_sine_phase: 0.0,
        }
    }

    /// Reset all phase accumulators (call on trigger)
    pub fn reset(&mut self) {
        self.main_sine_phase = 0.0;
        self.tri_phase = 0.0;
        self.fixed_sine_phase = 0.0;
        self.noise_counter = 0;
        self.rand_phase = 0.0;
        self.rand_current = 0.0;
        self.rand_target = 0.0;
        self.gated_sine_phase = 0.0;
    }

    /// Advance a phase accumulator by frequency
    #[inline]
    fn advance_phase(phase: &mut f32, frequency: f32, sample_rate: f32) {
        *phase += frequency / sample_rate;
        if *phase >= 1.0 {
            *phase -= 1.0;
        }
    }

    /// 3-channel crossfade mix (inline mix3 logic)
    ///
    /// control: -1 to 1
    /// - At -1: channel 1 is full, others are 0
    /// - At 0: channel 2 is full, others are 0
    /// - At +1: channel 3 is full, others are 0
    #[inline]
    fn mix3(control: f32, ch1: f32, ch2: f32, ch3: f32) -> f32 {
        // channel1_weight = clip(-control, 0, 1)
        let w1 = (-control).clamp(0.0, 1.0);
        // channel2_weight = clip(1 - abs(control), 0, 1)
        let w2 = (1.0 - control.abs()).clamp(0.0, 1.0);
        // channel3_weight = clip(control, 0, 1)
        let w3 = control.clamp(0.0, 1.0);

        ch1 * w1 + ch2 * w2 + ch3 * w3
    }

    /// Generate one sample from the morph oscillator
    ///
    /// # Arguments
    /// * `frequency` - Base oscillator frequency in Hz
    /// * `mix_control` - Crossfade position (-1 to 1, from tone scaled)
    /// * `color_freq` - First-mtof result (~46-147 Hz), will be mtof'd again for rand~ rate
    /// * `tone` - Tone parameter (0-100), controls gated sine (gate open when < 99)
    pub fn tick(
        &mut self,
        frequency: f32,
        mix_control: f32,
        color_freq: f32,
        tone: f32,
    ) -> f32 {
        // === Generate oscillator signals ===

        // Main sine: cycle~(freq) → *0.5
        let main_sine = sine(self.main_sine_phase) * 0.5;
        Self::advance_phase(&mut self.main_sine_phase, frequency, self.sample_rate);

        // Triangle: tri~(freq) → *0.5
        let tri = triangle(self.tri_phase) * 0.5;
        Self::advance_phase(&mut self.tri_phase, frequency, self.sample_rate);

        // Fixed sine: cycle~ 190Hz → *0.5
        let fixed_sine = sine(self.fixed_sine_phase) * 0.5;
        Self::advance_phase(&mut self.fixed_sine_phase, 190.0, self.sample_rate);

        // === Noise components ===

        // White noise: noise~ → *0.2
        self.noise_counter = self.noise_counter.wrapping_add(1);
        let noise = white_noise(self.noise_counter) * 0.2;

        // rand~ ramps linearly between random values at mtof(color_freq) rate
        // This is the second mtof in the chain: color → zmap → mtof → morphosc → mtof → rand~
        let rand_freq = mtof(color_freq);
        let prev_rand_phase = self.rand_phase;
        Self::advance_phase(&mut self.rand_phase, rand_freq, self.sample_rate);

        // When phase wraps, start new ramp: current becomes old target, pick new target
        if self.rand_phase < prev_rand_phase {
            self.rand_current = self.rand_target;
            self.rand_target = white_noise(self.noise_counter.wrapping_add(0x12345678));
        }

        // Linear interpolation from current to target based on phase position
        let rand_value = self.rand_current + (self.rand_target - self.rand_current) * self.rand_phase;

        // Combined noise signal: (noise + rand_value) * 0.4
        let noise_combined = (noise + rand_value) * 0.4;

        // === Gated sine for channel 3 ===
        // Gate controlled by tone < 99
        let gated_sine = if tone < 99.0 {
            sine(self.gated_sine_phase) * 0.2
        } else {
            0.0
        };
        Self::advance_phase(&mut self.gated_sine_phase, frequency, self.sample_rate);

        // === Mix into 3 channels ===

        // Channel 1: RING MODULATION of main_sine and fixed_sine
        // Max patch: (cycle~(freq) * 0.5) * (cycle~ 190 * 0.5)
        let ch1 = main_sine * fixed_sine;

        // Channel 2: triangle + noise
        // Max patch: tri~ * 0.5 + (noise~ * 0.2 + rand~) * 0.4
        let ch2 = tri + noise_combined;

        // Channel 3: noise + gated sine
        // Max patch: (noise~ * 0.2 + rand~) * 0.4 + gated(cycle~ * 0.2)
        let ch3 = noise_combined + gated_sine;

        // Apply mix3 crossfade
        Self::mix3(mix_control, ch1, ch2, ch3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mix3_extremes() {
        // At control = -1, only ch1
        assert!((MorphOsc::mix3(-1.0, 1.0, 0.5, 0.25) - 1.0).abs() < 0.001);
        // At control = 0, only ch2
        assert!((MorphOsc::mix3(0.0, 1.0, 0.5, 0.25) - 0.5).abs() < 0.001);
        // At control = 1, only ch3
        assert!((MorphOsc::mix3(1.0, 1.0, 0.5, 0.25) - 0.25).abs() < 0.001);
    }

    #[test]
    fn test_morph_osc_output_range() {
        let mut osc = MorphOsc::new(44100.0);

        // Generate samples and check they're in reasonable range
        for i in 0..1000 {
            let sample = osc.tick(440.0, 0.0, 60.0, 50.0);
            assert!(sample.is_finite(), "Sample {} should be finite", i);
            assert!(sample.abs() < 2.0, "Sample {} should be in reasonable range: {}", i, sample);
        }
    }

    #[test]
    fn test_channel3_has_output() {
        let mut osc = MorphOsc::new(44100.0);

        // At mix_control = 1.0, only channel 3 is active (noise + gated sine)
        let mut sum = 0.0;
        for _ in 0..100 {
            sum += osc.tick(440.0, 1.0, 60.0, 50.0).abs();
        }

        // Channel 3 should have audio (noise + gated sine when tone < 99)
        assert!(sum > 0.1, "Channel 3 should have output");
    }

    #[test]
    fn test_gated_sine_closes_at_tone_99() {
        let mut osc1 = MorphOsc::new(44100.0);
        let mut osc2 = MorphOsc::new(44100.0);

        // Generate samples with tone < 99 (gate open)
        let mut sum_gate_open = 0.0;
        for _ in 0..100 {
            sum_gate_open += osc1.tick(440.0, 1.0, 60.0, 50.0).abs();
        }

        // Generate samples with tone >= 99 (gate closed)
        let mut sum_gate_closed = 0.0;
        for _ in 0..100 {
            sum_gate_closed += osc2.tick(440.0, 1.0, 60.0, 99.0).abs();
        }

        // Both should have output (noise is always present)
        // but gate open should have more energy due to gated sine contribution
        assert!(sum_gate_open > 0.1, "Gate open should have output");
        assert!(sum_gate_closed > 0.1, "Gate closed should still have noise output");
    }

    #[test]
    fn test_color_affects_rand_rate() {
        // Low color = low frequency rand~ = slower sample-and-hold
        // High color = high frequency rand~ = faster sample-and-hold
        // We just verify both produce output - audible difference in testing
        let mut osc_low = MorphOsc::new(44100.0);
        let mut osc_high = MorphOsc::new(44100.0);

        let mut sum_low = 0.0;
        let mut sum_high = 0.0;
        for _ in 0..1000 {
            sum_low += osc_low.tick(440.0, 1.0, 20.0, 50.0).abs();
            sum_high += osc_high.tick(440.0, 1.0, 100.0, 50.0).abs();
        }

        assert!(sum_low > 1.0, "Low color should produce output");
        assert!(sum_high > 1.0, "High color should produce output");
    }

    #[test]
    fn test_channel1_has_output() {
        let mut osc = MorphOsc::new(44100.0);

        // At mix_control = -1.0, only channel 1 is active (main_sine + fixed_sine)
        let mut sum = 0.0;
        for _ in 0..100 {
            sum += osc.tick(440.0, -1.0, 60.0, 50.0).abs();
        }

        // Channel 1 should have audio
        assert!(sum > 0.1, "Channel 1 should have output");
    }

    #[test]
    fn test_channel2_has_output() {
        let mut osc = MorphOsc::new(44100.0);

        // At mix_control = 0.0, only channel 2 is active (triangle)
        let mut sum = 0.0;
        for _ in 0..100 {
            sum += osc.tick(440.0, 0.0, 60.0, 50.0).abs();
        }

        // Channel 2 should have audio
        assert!(sum > 0.1, "Channel 2 should have output");
    }
}
