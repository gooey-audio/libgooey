use crate::effects::Effect;
use crate::utils::smoother::SmoothedParam;
use std::cell::UnsafeCell;
use std::f32::consts::FRAC_2_PI;
use std::sync::atomic::{AtomicU32, Ordering};

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
}

pub struct TubeCompressor {
    state: UnsafeCell<CompressorState>,

    threshold_target: AtomicU32,
    ratio_target: AtomicU32,
    attack_target: AtomicU32,
    release_target: AtomicU32,
    mix_target: AtomicU32,
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

        Self {
            state: UnsafeCell::new(CompressorState {
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
            }),
            threshold_target: AtomicU32::new(threshold_db.to_bits()),
            ratio_target: AtomicU32::new(ratio.to_bits()),
            attack_target: AtomicU32::new(attack_ms.to_bits()),
            release_target: AtomicU32::new(release_ms.to_bits()),
            mix_target: AtomicU32::new(mix.to_bits()),
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

    /// Core processing: input is the audio to compress, sidechain drives the detector
    fn process_inner(&self, input: f32, sidechain: f32) -> f32 {
        if !input.is_finite() || !sidechain.is_finite() {
            return 0.0;
        }

        let state = unsafe { &mut *self.state.get() };

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

        // Subtle tube coloring when compressing (atan soft-clip)
        let colored = if state.gain_smoothed < 0.99 {
            compressed.atan() * FRAC_2_PI * 1.1
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

    /// Process with an external sidechain signal.
    /// The envelope follower tracks `sidechain` while gain reduction is applied to `input`.
    pub fn process_with_sidechain(&self, input: f32, sidechain: f32) -> f32 {
        self.process_inner(input, sidechain)
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

    pub fn reset(&self) {
        let state = unsafe { &mut *self.state.get() };
        state.envelope = 0.0;
        state.gain_smoothed = 1.0;
        state.dc_x1 = 0.0;
        state.dc_y1 = 0.0;
    }
}

impl Effect for TubeCompressor {
    fn process(&self, input: f32) -> f32 {
        self.process_inner(input, input)
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
}
