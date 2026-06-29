use crate::effects::Effect;
use crate::frame::StereoFrame;
use crate::utils::oversampler::{Oversampler, OversamplingMode};
use crate::utils::smoother::SmoothedParam;
use std::cell::UnsafeCell;
use std::f32::consts::FRAC_2_PI;
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};

const DENORMAL_THRESHOLD: f32 = 1e-15;
const DC_BLOCKER_COEFF: f32 = 0.995;
const KNEE_WIDTH_DB: f32 = 6.0;
const HALF_KNEE_DB: f32 = KNEE_WIDTH_DB * 0.5;

struct CompressorState {
    threshold_smoothed: SmoothedParam,
    ratio_smoothed: SmoothedParam,
    attack_smoothed: SmoothedParam,
    release_smoothed: SmoothedParam,
    mix_smoothed: SmoothedParam,

    // Peak envelope follower level (linear)
    envelope: f32,

    // Smoothed gain reduction (linear)
    gain_smoothed: f32,

    // DC blocker state
    dc_x1: f32,
    dc_y1: f32,

    sample_rate: f32,

    // Selectable oversampler to reduce aliasing from the nonlinear tube-coloring path
    oversampler: Oversampler,
}

pub struct TubeCompressor {
    // Per-channel state (index 0 = mono/left, index 1 = right). The mono
    // `process`/`process_with_sidechain` paths use only index 0, so their
    // behavior is unchanged.
    state: UnsafeCell<[CompressorState; 2]>,

    threshold_target: AtomicU32,
    ratio_target: AtomicU32,
    attack_target: AtomicU32,
    release_target: AtomicU32,
    mix_target: AtomicU32,
    oversampling_mode_target: AtomicU8,
}

// SAFETY: UnsafeCell only accessed from single audio thread
unsafe impl Send for TubeCompressor {}
unsafe impl Sync for TubeCompressor {}

impl TubeCompressor {
    pub fn new(
        sample_rate: f32,
        threshold_db: f32,
        ratio: f32,
        attack_ms: f32,
        release_ms: f32,
        mix: f32,
    ) -> Self {
        let threshold_db = threshold_db.clamp(-60.0, 0.0);
        let ratio = ratio.clamp(1.0, 20.0);
        let attack_ms = attack_ms.clamp(0.1, 100.0);
        let release_ms = release_ms.clamp(5.0, 1000.0);
        let mix = mix.clamp(0.0, 1.0);

        let make_state = || CompressorState {
            threshold_smoothed: SmoothedParam::new(threshold_db, -60.0, 0.0, sample_rate, 30.0),
            ratio_smoothed: SmoothedParam::new(ratio, 1.0, 20.0, sample_rate, 30.0),
            attack_smoothed: SmoothedParam::new(attack_ms, 0.1, 100.0, sample_rate, 30.0),
            release_smoothed: SmoothedParam::new(release_ms, 5.0, 1000.0, sample_rate, 30.0),
            mix_smoothed: SmoothedParam::new(mix, 0.0, 1.0, sample_rate, 30.0),
            envelope: 0.0,
            gain_smoothed: 1.0,
            dc_x1: 0.0,
            dc_y1: 0.0,
            sample_rate,
            oversampler: Oversampler::default(),
        };

        Self {
            state: UnsafeCell::new([make_state(), make_state()]),
            threshold_target: AtomicU32::new(threshold_db.to_bits()),
            ratio_target: AtomicU32::new(ratio.to_bits()),
            attack_target: AtomicU32::new(attack_ms.to_bits()),
            release_target: AtomicU32::new(release_ms.to_bits()),
            mix_target: AtomicU32::new(mix.to_bits()),
            oversampling_mode_target: AtomicU8::new(OversamplingMode::X4 as u8),
        }
    }

    /// Compute attack/release envelope coefficient from time in ms
    #[inline]
    fn time_to_coeff(time_ms: f32, sample_rate: f32) -> f32 {
        (-1.0 / (time_ms * 0.001 * sample_rate)).exp()
    }

    /// Compute gain reduction in dB for a given level above threshold (soft knee)
    #[inline]
    fn compute_gain_reduction_db(over_db: f32, ratio: f32) -> f32 {
        let slope = 1.0 - 1.0 / ratio;
        if over_db <= -HALF_KNEE_DB {
            // Below knee — no compression
            0.0
        } else if over_db >= HALF_KNEE_DB {
            // Above knee — full compression
            over_db * slope
        } else {
            // In the knee — quadratic interpolation
            let x = over_db + HALF_KNEE_DB;
            x * x / (2.0 * KNEE_WIDTH_DB) * slope
        }
    }

    /// DC blocking high-pass filter
    #[inline]
    fn dc_block(input: f32, x1: &mut f32, y1: &mut f32) -> f32 {
        let output = input - *x1 + DC_BLOCKER_COEFF * *y1;
        *x1 = input;
        *y1 = if output.abs() < DENORMAL_THRESHOLD {
            0.0
        } else {
            output
        };
        output
    }

    /// Core processing: input is the audio to compress, sidechain drives the
    /// detector. Operates on the supplied per-channel state.
    fn process_inner(&self, state: &mut CompressorState, input: f32, sidechain: f32) -> f32 {
        if !input.is_finite() || !sidechain.is_finite() {
            return 0.0;
        }

        // Read targets and update smoothers
        let threshold_target = f32::from_bits(self.threshold_target.load(Ordering::Relaxed));
        let ratio_target = f32::from_bits(self.ratio_target.load(Ordering::Relaxed));
        let attack_target = f32::from_bits(self.attack_target.load(Ordering::Relaxed));
        let release_target = f32::from_bits(self.release_target.load(Ordering::Relaxed));
        let mix_target = f32::from_bits(self.mix_target.load(Ordering::Relaxed));

        state.threshold_smoothed.set_target(threshold_target);
        state.ratio_smoothed.set_target(ratio_target);
        state.attack_smoothed.set_target(attack_target);
        state.release_smoothed.set_target(release_target);
        state.mix_smoothed.set_target(mix_target);

        let threshold_db = state.threshold_smoothed.tick();
        let ratio = state.ratio_smoothed.tick();
        let attack_ms = state.attack_smoothed.tick();
        let release_ms = state.release_smoothed.tick();
        let mix = state.mix_smoothed.tick();

        // Early exit if bypassed
        if mix < 0.0001 {
            return input;
        }

        // Peak envelope follower with attack/release ballistics
        let sidechain_abs = sidechain.abs();
        let coeff = if sidechain_abs > state.envelope {
            Self::time_to_coeff(attack_ms, state.sample_rate)
        } else {
            Self::time_to_coeff(release_ms, state.sample_rate)
        };
        state.envelope = coeff * state.envelope + (1.0 - coeff) * sidechain_abs;

        // Flush denormals
        if state.envelope < DENORMAL_THRESHOLD {
            state.envelope = 0.0;
        }

        // Log-domain gain computation
        // Convert envelope to dB (with floor to avoid log(0))
        let env_db = 20.0 * (state.envelope + 1e-20).log10();
        let over_db = env_db - threshold_db;
        let gain_reduction_db = Self::compute_gain_reduction_db(over_db, ratio);

        // Convert gain reduction to linear multiplier
        let gain_linear = 10.0_f32.powf(-gain_reduction_db * 0.05);

        // Smooth the gain to avoid zipper noise (one-pole, ~1ms)
        state.gain_smoothed += 0.05 * (gain_linear - state.gain_smoothed);

        // Apply gain reduction
        let compressed = input * state.gain_smoothed;

        // Subtle tube coloring when compressing (atan soft-clip), oversampled
        // to suppress aliasing from the nonlinear mapping.
        let oversampling_mode =
            OversamplingMode::from_u8(self.oversampling_mode_target.load(Ordering::Relaxed));
        state.oversampler.set_mode(oversampling_mode);

        let colored = if state.gain_smoothed < 0.99 {
            state.oversampler.process(compressed, |x| x.atan() * FRAC_2_PI * 1.1)
        } else {
            compressed
        };

        // DC blocker
        let dc_blocked = Self::dc_block(colored, &mut state.dc_x1, &mut state.dc_y1);

        // Dry/wet mix
        let output = input * (1.0 - mix) + dc_blocked * mix;

        // NaN protection at output
        if !output.is_finite() {
            state.dc_x1 = 0.0;
            state.dc_y1 = 0.0;
            state.envelope = 0.0;
            state.gain_smoothed = 1.0;
            return 0.0;
        }

        output
    }

    /// Process with an external sidechain signal (mono path).
    /// The envelope follower tracks `sidechain` while gain reduction is applied to `input`.
    pub fn process_with_sidechain(&self, input: f32, sidechain: f32) -> f32 {
        let states = unsafe { &mut *self.state.get() };
        self.process_inner(&mut states[0], input, sidechain)
    }

    /// Process a stereo frame with an external stereo sidechain signal.
    /// Each channel's detector tracks its own sidechain value. When the engine's
    /// sidechain source is mono, the same value is supplied on both channels.
    pub fn process_stereo_with_sidechain(
        &self,
        input: StereoFrame,
        sidechain: StereoFrame,
    ) -> StereoFrame {
        let states = unsafe { &mut *self.state.get() };
        StereoFrame {
            l: self.process_inner(&mut states[0], input.l, sidechain.l),
            r: self.process_inner(&mut states[1], input.r, sidechain.r),
        }
    }

    // Parameter setters (thread-safe, called from control thread)

    pub fn set_threshold(&self, db: f32) {
        let clamped = db.clamp(-60.0, 0.0);
        self.threshold_target
            .store(clamped.to_bits(), Ordering::Relaxed);
    }

    pub fn set_ratio(&self, ratio: f32) {
        let clamped = ratio.clamp(1.0, 20.0);
        self.ratio_target
            .store(clamped.to_bits(), Ordering::Relaxed);
    }

    pub fn set_attack(&self, ms: f32) {
        let clamped = ms.clamp(0.1, 100.0);
        self.attack_target
            .store(clamped.to_bits(), Ordering::Relaxed);
    }

    pub fn set_release(&self, ms: f32) {
        let clamped = ms.clamp(5.0, 1000.0);
        self.release_target
            .store(clamped.to_bits(), Ordering::Relaxed);
    }

    pub fn set_mix(&self, mix: f32) {
        let clamped = mix.clamp(0.0, 1.0);
        self.mix_target.store(clamped.to_bits(), Ordering::Relaxed);
    }

    // Parameter getters

    pub fn get_threshold(&self) -> f32 {
        f32::from_bits(self.threshold_target.load(Ordering::Relaxed))
    }

    pub fn get_ratio(&self) -> f32 {
        f32::from_bits(self.ratio_target.load(Ordering::Relaxed))
    }

    pub fn get_attack(&self) -> f32 {
        f32::from_bits(self.attack_target.load(Ordering::Relaxed))
    }

    pub fn get_release(&self) -> f32 {
        f32::from_bits(self.release_target.load(Ordering::Relaxed))
    }

    pub fn get_mix(&self) -> f32 {
        f32::from_bits(self.mix_target.load(Ordering::Relaxed))
    }

    /// Set the oversampling rate for the tube-coloring nonlinear path.
    pub fn set_oversampling_mode(&self, mode: OversamplingMode) {
        self.oversampling_mode_target
            .store(mode as u8, Ordering::Relaxed);
    }

    /// Get the current oversampling mode.
    pub fn oversampling_mode(&self) -> OversamplingMode {
        OversamplingMode::from_u8(self.oversampling_mode_target.load(Ordering::Relaxed))
    }

    pub fn reset(&self) {
        let states = unsafe { &mut *self.state.get() };
        for state in states.iter_mut() {
            state.envelope = 0.0;
            state.gain_smoothed = 1.0;
            state.dc_x1 = 0.0;
            state.dc_y1 = 0.0;
            state.oversampler.reset();
        }
    }
}

impl Effect for TubeCompressor {
    fn process(&self, input: f32) -> f32 {
        let states = unsafe { &mut *self.state.get() };
        self.process_inner(&mut states[0], input, input)
    }

    fn process_stereo(&self, input: StereoFrame) -> StereoFrame {
        // No external sidechain: each channel is its own detector.
        let states = unsafe { &mut *self.state.get() };
        StereoFrame {
            l: self.process_inner(&mut states[0], input.l, input.l),
            r: self.process_inner(&mut states[1], input.r, input.r),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bypass_when_mix_zero() {
        let comp = TubeCompressor::new(44100.0, -12.0, 4.0, 5.0, 100.0, 0.0);
        assert_eq!(comp.process(0.5), 0.5);
        assert_eq!(comp.process(-0.3), -0.3);
    }

    #[test]
    fn test_gain_reduction_above_threshold() {
        // Low threshold, high ratio, full wet — should compress loud signals
        let comp = TubeCompressor::new(44100.0, -20.0, 10.0, 0.1, 50.0, 1.0);
        // Feed alternating signal to build envelope (avoids DC blocker attenuation)
        for _ in 0..4000 {
            comp.process(0.8);
            comp.process(-0.8);
        }
        let output = comp.process(0.8);
        assert!(
            output.abs() < 0.8,
            "Expected compressed output < 0.8, got {}",
            output
        );
        assert!(
            output.abs() > 0.01,
            "Expected output > 0.01, got {}",
            output
        );
    }

    #[test]
    fn test_no_reduction_below_threshold() {
        // High threshold — quiet signal should pass through mostly unchanged
        let comp = TubeCompressor::new(44100.0, -1.0, 4.0, 5.0, 100.0, 1.0);
        // Let smoothers settle
        for _ in 0..2000 {
            comp.process(0.0);
        }
        // Very quiet signal, well below -1 dB threshold
        let input = 0.01;
        let output = comp.process(input);
        // Should be very close to input (within DC blocker settling)
        assert!(
            (output - input).abs() < 0.05,
            "Expected ~{}, got {}",
            input,
            output
        );
    }

    #[test]
    fn test_sidechain_triggers_compression() {
        let comp = TubeCompressor::new(44100.0, -20.0, 10.0, 0.1, 50.0, 1.0);
        // Let smoothers settle
        for _ in 0..2000 {
            comp.process(0.0);
        }
        // Loud sidechain should cause gain reduction on a quiet input
        for _ in 0..2000 {
            comp.process_with_sidechain(0.3, 0.9);
        }
        let output = comp.process_with_sidechain(0.3, 0.9);
        assert!(
            output.abs() < 0.3,
            "Expected sidechain-compressed output < 0.3, got {}",
            output
        );
    }

    #[test]
    fn test_parameter_clamping() {
        let comp = TubeCompressor::new(44100.0, 10.0, 0.5, 200.0, 2000.0, 5.0);
        assert_eq!(comp.get_threshold(), 0.0);
        assert_eq!(comp.get_ratio(), 1.0);
        assert_eq!(comp.get_attack(), 100.0);
        assert_eq!(comp.get_release(), 1000.0);
        assert_eq!(comp.get_mix(), 1.0);
    }

    #[test]
    fn test_nan_protection() {
        let comp = TubeCompressor::new(44100.0, -12.0, 4.0, 5.0, 100.0, 0.5);
        let output = comp.process(f32::NAN);
        assert!(output.is_finite());
        assert_eq!(output, 0.0);
    }

    #[test]
    fn test_dc_stability() {
        let comp = TubeCompressor::new(44100.0, -12.0, 4.0, 5.0, 100.0, 1.0);
        for _ in 0..44100 {
            let output = comp.process(0.5);
            assert!(output.is_finite());
        }
    }

    #[test]
    fn test_parameter_setters() {
        let comp = TubeCompressor::new(44100.0, -12.0, 4.0, 5.0, 100.0, 0.5);

        comp.set_threshold(-30.0);
        comp.set_ratio(8.0);
        comp.set_attack(10.0);
        comp.set_release(200.0);
        comp.set_mix(0.7);

        assert!((comp.get_threshold() - -30.0).abs() < 0.001);
        assert!((comp.get_ratio() - 8.0).abs() < 0.001);
        assert!((comp.get_attack() - 10.0).abs() < 0.001);
        assert!((comp.get_release() - 200.0).abs() < 0.001);
        assert!((comp.get_mix() - 0.7).abs() < 0.001);
    }

    #[test]
    fn test_compressor_oversampling_defaults_to_4x() {
        let comp = TubeCompressor::new(44100.0, -12.0, 4.0, 5.0, 100.0, 1.0);
        assert_eq!(comp.oversampling_mode(), OversamplingMode::X4);
    }

    #[test]
    fn test_compressor_oversampling_setter() {
        let comp = TubeCompressor::new(44100.0, -12.0, 4.0, 5.0, 100.0, 1.0);
        comp.set_oversampling_mode(OversamplingMode::Off);
        assert_eq!(comp.oversampling_mode(), OversamplingMode::Off);
        comp.set_oversampling_mode(OversamplingMode::X2);
        assert_eq!(comp.oversampling_mode(), OversamplingMode::X2);
        comp.set_oversampling_mode(OversamplingMode::X4);
        assert_eq!(comp.oversampling_mode(), OversamplingMode::X4);
    }

    #[test]
    fn test_compressor_oversampling_reset_matches_fresh() {
        let reset = TubeCompressor::new(44100.0, -20.0, 10.0, 0.1, 50.0, 1.0);
        // Build envelope and oversampler state with a strong signal
        for i in 0..2000 {
            let s = (i as f32 * 0.3).sin() * 0.9;
            reset.process(s);
        }
        reset.reset();

        let fresh = TubeCompressor::new(44100.0, -20.0, 10.0, 0.1, 50.0, 1.0);
        for i in 0..100 {
            let input = (i as f32 * 0.27).sin() * 0.9;
            assert_eq!(
                reset.process(input),
                fresh.process(input),
                "mismatch at sample {i}"
            );
        }
    }

    #[test]
    fn test_compressor_oversampling_reduces_aliasing() {
        // Compare alias power with oversampling off vs on.
        // Use aggressive settings to engage the tube-coloring atan path.
        let sr = 44100.0;
        let test_freq = 8_000.0;
        let test_samples = 4800;
        let warmup = 1024;

        fn render_compressor(
            comp: &TubeCompressor,
            test_freq: f32,
            sr: f32,
            warmup: usize,
            samples: usize,
        ) -> Vec<f32> {
            (0..warmup + samples)
                .filter_map(|i| {
                    let input = (std::f32::consts::TAU * test_freq * i as f32 / sr).sin() * 0.9;
                    let output = comp.process(input);
                    (i >= warmup).then_some(output)
                })
                .collect()
        }

        fn alias_power(samples: &[f32], sr: f32, _freq: f32, alias_freqs: &[f32]) -> f64 {
            alias_freqs
                .iter()
                .map(|&af| {
                    let phase_step = std::f64::consts::TAU * af as f64 / sr as f64;
                    let (real, imag) = samples.iter().enumerate().fold(
                        (0.0_f64, 0.0_f64),
                        |(real, imag), (i, &x)| {
                            let phase = phase_step * i as f64;
                            (real + x as f64 * phase.cos(), imag - x as f64 * phase.sin())
                        },
                    );
                    real * real + imag * imag
                })
                .sum()
        }

        // Compressor with oversampling off
        let comp_off = TubeCompressor::new(sr, -30.0, 20.0, 1.0, 200.0, 1.0);
        comp_off.set_oversampling_mode(OversamplingMode::Off);
        let off_samples = render_compressor(&comp_off, test_freq, sr, warmup, test_samples);

        // Compressor with oversampling on (4x)
        let comp_on = TubeCompressor::new(sr, -30.0, 20.0, 1.0, 200.0, 1.0);
        let on_samples = render_compressor(&comp_on, test_freq, sr, warmup, test_samples);

        // Aliases of an 8 kHz tone appear at harmonics folded back below Nyquist
        let alias_frequencies: [f32; 4] = [2_000.0, 14_000.0, 18_000.0, 22_000.0];
        let off_power = alias_power(&off_samples, sr, test_freq, &alias_frequencies);
        let on_power = alias_power(&on_samples, sr, test_freq, &alias_frequencies);

        let reduction_db = 10.0 * (off_power / on_power.max(1e-20)).log10();
        // The compressor's tube-coloring is a subtle nonlinearity, so the alias
        // reduction is modest compared to heavy waveshaping. The key check is that
        // oversampling reduces alias power at all (positive dB).
        assert!(
            reduction_db > 0.0,
            "expected positive alias reduction with 4x oversampling, measured {reduction_db:.2} dB"
        );
    }
}
