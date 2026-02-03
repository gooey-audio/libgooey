//! Morph oscillator based on Max/MSP morphosc subpatch
//!
//! Combines multiple oscillators with a 3-channel crossfade mixer:
//! - Channel 1: Ring modulation of main sine and fixed 190Hz sine
//! - Channel 2: Triangle wave
//! - Channel 3: Noise (currently disabled for A/B testing)
//!
//! Note: Noise components (noise~, rand~) are currently disabled for A/B testing.
//! Channel 3 outputs silence, so tone=100% (full channel 3) produces no sound.

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

/// Morph oscillator based on Max/MSP morphosc subpatch
///
/// Channel 1: Ring modulation of main sine (at input freq) and fixed 190Hz sine.
/// Ring mod creates sum/difference sidebands for a darker, complex timbre.
///
/// Channel 2: Triangle oscillator (tri~) at input frequency.
///
/// Channel 3 (noise) is currently disabled - outputs silence.
///
/// All mixed through a 3-channel crossfade controlled by mix_control.
pub struct MorphOsc {
    sample_rate: f32,

    // Phase accumulators for each oscillator (0.0 to 1.0)
    main_sine_phase: f32,
    tri_phase: f32,
    fixed_sine_phase: f32,
}

impl MorphOsc {
    /// Create a new morph oscillator
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            main_sine_phase: 0.0,
            tri_phase: 0.0,
            fixed_sine_phase: 0.0,
        }
    }

    /// Reset all phase accumulators (call on trigger)
    pub fn reset(&mut self) {
        self.main_sine_phase = 0.0;
        self.tri_phase = 0.0;
        self.fixed_sine_phase = 0.0;
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
    /// * `_color_midi` - Unused (reserved for noise frequency modulation)
    /// * `_tone` - Unused (reserved for gated sine control)
    pub fn tick(
        &mut self,
        frequency: f32,
        mix_control: f32,
        _color_midi: f32,
        _tone: f32,
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

        // === Mix into 3 channels ===

        // Channel 1: RING MODULATION of main_sine and fixed_sine
        // Max patch: (cycle~(freq) * 0.5) * (cycle~ 190 * 0.5)
        // Output range: ±0.25
        let ch1 = main_sine * fixed_sine;

        // Channel 2: triangle
        let ch2 = tri;

        // Channel 3: empty (noise disabled for A/B testing)
        // At tone=100% (mix_control=1), output will be silent
        let ch3 = 0.0;

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
    fn test_channel3_silent() {
        let mut osc = MorphOsc::new(44100.0);

        // At mix_control = 1.0, only channel 3 is active, which is empty
        let mut sum = 0.0;
        for _ in 0..100 {
            sum += osc.tick(440.0, 1.0, 60.0, 50.0).abs();
        }

        // Channel 3 is empty, so output should be zero
        assert!(sum < 0.001, "Channel 3 should be silent (noise disabled)");
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
