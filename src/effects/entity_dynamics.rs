//! Entity-style dynamics: filtered-sidechain compression, gain drive, and a
//! deliberately reactive feedback limiter behind one dry/wet control.

use crate::effects::{Effect, TubeCompressor};
use crate::frame::StereoFrame;
use crate::utils::{Oversampler, OversamplingMode, SmoothedParam};
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU32, Ordering};

// At velocity 0.75 the raw voice peaks around -10 dBFS on Classic 808 and -8
// dBFS with Punch at midpoint. A -12 dBFS threshold accounts for the 2 ms
// detector attack and makes that midpoint the onset of obvious compression.
const COMPRESSOR_THRESHOLD_DB: f32 = -12.0;
const COMPRESSOR_RATIO: f32 = 4.0;
const COMPRESSOR_ATTACK_MS: f32 = 2.0;
const COMPRESSOR_RELEASE_MS: f32 = 120.0;

const BASS_DRIVE_MIN_HZ: f32 = 20.0;
const BASS_DRIVE_MAX_HZ: f32 = 530.0;
const GAIN_DIST_MIN_DB: f32 = -12.0;
const GAIN_DIST_MAX_DB: f32 = 24.0;
const LIMITER_CEILING: f32 = 0.95;
const LIMITER_ATTACK_MS: f32 = 0.2;
const LIMITER_RELEASE_MS: f32 = 80.0;
const DENORMAL_THRESHOLD: f32 = 1.0e-15;

struct EntityDynamicsState {
    bass_drive: SmoothedParam,
    gain_dist_db: SmoothedParam,
    dynamics: SmoothedParam,
    sidechain_x1: f32,
    sidechain_y1: f32,
    limiter_gain: f32,
    limiter_previous_output: f32,
    limiter_oversampler: Oversampler,
}

impl EntityDynamicsState {
    fn new(sample_rate: f32) -> Self {
        Self {
            bass_drive: SmoothedParam::new_normalized(0.0, sample_rate),
            gain_dist_db: SmoothedParam::new(
                0.0,
                GAIN_DIST_MIN_DB,
                GAIN_DIST_MAX_DB,
                sample_rate,
                15.0,
            ),
            dynamics: SmoothedParam::new_normalized(0.0, sample_rate),
            sidechain_x1: 0.0,
            sidechain_y1: 0.0,
            limiter_gain: 1.0,
            limiter_previous_output: 0.0,
            limiter_oversampler: Oversampler::new(OversamplingMode::X2),
        }
    }

    fn reset(&mut self) {
        self.sidechain_x1 = 0.0;
        self.sidechain_y1 = 0.0;
        self.limiter_gain = 1.0;
        self.limiter_previous_output = 0.0;
        self.limiter_oversampler.reset();
    }
}

/// Composite dynamics processor matching the Entity-style panel semantics.
///
/// `Dynamics = 0` is an exact bypass of the whole section. Bass Drive filters
/// only the compressor detector, Gain-Dist is clean gain after compression,
/// and the following feedback limiter supplies the audible saturation.
pub struct EntityDynamics {
    sample_rate: f32,
    compressor: TubeCompressor,
    state: UnsafeCell<[EntityDynamicsState; 2]>,
    bass_drive_target: AtomicU32,
    gain_dist_db_target: AtomicU32,
    dynamics_target: AtomicU32,
}

// SAFETY: mutable DSP state is touched only by the single audio thread. The
// control thread communicates through atomics, matching the other effects in
// this module.
unsafe impl Send for EntityDynamics {}
unsafe impl Sync for EntityDynamics {}

impl EntityDynamics {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);
        let compressor = TubeCompressor::new(
            sample_rate,
            COMPRESSOR_THRESHOLD_DB,
            COMPRESSOR_RATIO,
            COMPRESSOR_ATTACK_MS,
            COMPRESSOR_RELEASE_MS,
            1.0,
        );
        compressor.set_oversampling_mode(OversamplingMode::Off);

        Self {
            sample_rate,
            compressor,
            state: UnsafeCell::new([
                EntityDynamicsState::new(sample_rate),
                EntityDynamicsState::new(sample_rate),
            ]),
            bass_drive_target: AtomicU32::new(0.0_f32.to_bits()),
            gain_dist_db_target: AtomicU32::new(0.0_f32.to_bits()),
            dynamics_target: AtomicU32::new(0.0_f32.to_bits()),
        }
    }

    /// Set the sidechain high-pass position from 0 to 1 (20 to 530 Hz, log).
    pub fn set_bass_drive(&self, value: f32) {
        if value.is_finite() {
            self.bass_drive_target
                .store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        }
    }

    pub fn bass_drive(&self) -> f32 {
        f32::from_bits(self.bass_drive_target.load(Ordering::Relaxed))
    }

    pub fn bass_drive_hz(&self) -> f32 {
        Self::bass_drive_to_hz(self.bass_drive())
    }

    /// Set clean post-compressor gain from -12 to +24 dB.
    pub fn set_gain_dist_db(&self, value: f32) {
        if value.is_finite() {
            self.gain_dist_db_target.store(
                value.clamp(GAIN_DIST_MIN_DB, GAIN_DIST_MAX_DB).to_bits(),
                Ordering::Relaxed,
            );
        }
    }

    pub fn gain_dist_db(&self) -> f32 {
        f32::from_bits(self.gain_dist_db_target.load(Ordering::Relaxed))
    }

    /// Set the crossfade from raw input (0) to the whole wet section (1).
    pub fn set_dynamics(&self, value: f32) {
        if value.is_finite() {
            self.dynamics_target
                .store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        }
    }

    pub fn dynamics(&self) -> f32 {
        f32::from_bits(self.dynamics_target.load(Ordering::Relaxed))
    }

    pub fn reset(&self) {
        self.compressor.reset();
        let states = unsafe { &mut *self.state.get() };
        for state in states {
            state.reset();
        }
    }

    fn bass_drive_to_hz(normalized: f32) -> f32 {
        BASS_DRIVE_MIN_HZ * (BASS_DRIVE_MAX_HZ / BASS_DRIVE_MIN_HZ).powf(normalized.clamp(0.0, 1.0))
    }

    fn smoothing_step(time_ms: f32, sample_rate: f32) -> f32 {
        1.0 - (-1.0 / (time_ms * 0.001 * sample_rate)).exp()
    }

    fn prepare_sidechain_and_controls(
        &self,
        state: &mut EntityDynamicsState,
        input: f32,
    ) -> (f32, f32, f32) {
        let bass_target = f32::from_bits(self.bass_drive_target.load(Ordering::Relaxed));
        let gain_target = f32::from_bits(self.gain_dist_db_target.load(Ordering::Relaxed));
        let dynamics_target = f32::from_bits(self.dynamics_target.load(Ordering::Relaxed));
        state.bass_drive.set_target(bass_target);
        state.gain_dist_db.set_target(gain_target);
        state.dynamics.set_target(dynamics_target);

        let cutoff_hz = Self::bass_drive_to_hz(state.bass_drive.tick());
        let highpass_coeff = (-std::f32::consts::TAU * cutoff_hz / self.sample_rate).exp();
        let sidechain = highpass_coeff * (state.sidechain_y1 + input - state.sidechain_x1);
        state.sidechain_x1 = input;
        state.sidechain_y1 = if sidechain.abs() < DENORMAL_THRESHOLD {
            0.0
        } else {
            sidechain
        };

        let gain_linear = 10.0_f32.powf(state.gain_dist_db.tick() * 0.05);
        let dynamics = state.dynamics.tick();
        (state.sidechain_y1, gain_linear, dynamics)
    }

    fn limit(&self, state: &mut EntityDynamicsState, input: f32, gain_linear: f32) -> f32 {
        let detector = state.limiter_previous_output.abs();
        let target = LIMITER_CEILING / detector.max(LIMITER_CEILING);
        let coefficient = if target < state.limiter_gain {
            Self::smoothing_step(LIMITER_ATTACK_MS, self.sample_rate)
        } else {
            Self::smoothing_step(LIMITER_RELEASE_MS, self.sample_rate)
        };
        state.limiter_gain += (target - state.limiter_gain) * coefficient;

        let driven = input * gain_linear * state.limiter_gain;
        let output = state.limiter_oversampler.process(driven, |sample| {
            LIMITER_CEILING * (sample / LIMITER_CEILING).tanh()
        });
        if output.is_finite() {
            state.limiter_previous_output = output;
            output
        } else {
            state.reset();
            0.0
        }
    }

    fn finish_channel(
        &self,
        state: &mut EntityDynamicsState,
        dry: f32,
        compressed: f32,
        gain_linear: f32,
        dynamics: f32,
    ) -> f32 {
        let wet = self.limit(state, compressed, gain_linear);
        let output = dry * (1.0 - dynamics) + wet * dynamics;
        if output.is_finite() {
            output
        } else {
            state.reset();
            0.0
        }
    }
}

impl Effect for EntityDynamics {
    fn process(&self, input: f32) -> f32 {
        if !input.is_finite() {
            self.reset();
            return 0.0;
        }
        let states = unsafe { &mut *self.state.get() };
        let state = &mut states[0];
        let (sidechain, gain_linear, dynamics) = self.prepare_sidechain_and_controls(state, input);
        let compressed = self.compressor.process_with_sidechain(input, sidechain);
        self.finish_channel(state, input, compressed, gain_linear, dynamics)
    }

    fn process_stereo(&self, input: StereoFrame) -> StereoFrame {
        if !input.l.is_finite() || !input.r.is_finite() {
            self.reset();
            return StereoFrame::default();
        }
        let states = unsafe { &mut *self.state.get() };
        let (left, right) = states.split_at_mut(1);
        let left = &mut left[0];
        let right = &mut right[0];
        let (sidechain_l, gain_l, dynamics_l) = self.prepare_sidechain_and_controls(left, input.l);
        let (sidechain_r, gain_r, dynamics_r) = self.prepare_sidechain_and_controls(right, input.r);
        let compressed = self.compressor.process_stereo_with_sidechain(
            input,
            StereoFrame {
                l: sidechain_l,
                r: sidechain_r,
            },
        );
        StereoFrame {
            l: self.finish_channel(left, input.l, compressed.l, gain_l, dynamics_l),
            r: self.finish_channel(right, input.r, compressed.r, gain_r, dynamics_r),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f32 = 48_000.0;

    fn settle(effect: &EntityDynamics) {
        for _ in 0..4_800 {
            let _ = effect.process(0.0);
        }
    }

    fn sine_rms(bass_drive: f32, frequency: f32) -> f64 {
        let effect = EntityDynamics::new(SAMPLE_RATE);
        effect.set_bass_drive(bass_drive);
        effect.set_dynamics(1.0);
        settle(&effect);
        let mut energy = 0.0;
        for sample in 0..48_000 {
            let input =
                (std::f32::consts::TAU * frequency * sample as f32 / SAMPLE_RATE).sin() * 0.5;
            let output = effect.process(input);
            if sample >= 24_000 {
                energy += (output as f64).powi(2);
            }
        }
        (energy / 24_000.0).sqrt()
    }

    #[test]
    fn dynamics_zero_is_identity() {
        let effect = EntityDynamics::new(SAMPLE_RATE);
        effect.set_bass_drive(1.0);
        effect.set_gain_dist_db(24.0);
        effect.set_dynamics(0.0);
        for sample in [-1.2, -0.4, 0.0, 0.25, 1.4] {
            assert_eq!(effect.process(sample), sample);
        }
    }

    #[test]
    fn bass_drive_raises_low_band_relative_level() {
        let ratio_low = sine_rms(0.0, 60.0) / sine_rms(0.0, 1_000.0);
        let ratio_high = sine_rms(1.0, 60.0) / sine_rms(1.0, 1_000.0);
        let change_db = 20.0 * (ratio_high / ratio_low).log10();
        assert!(
            change_db >= 3.0,
            "relative low-band change={change_db:.2} dB"
        );
    }

    #[test]
    fn feedback_limiter_stays_bounded_when_driven() {
        let effect = EntityDynamics::new(SAMPLE_RATE);
        effect.set_gain_dist_db(24.0);
        effect.set_dynamics(1.0);
        settle(&effect);
        for sample in 0..48_000 {
            let input = if sample % 2 == 0 { 4.0 } else { -4.0 };
            let output = effect.process(input);
            assert!(output.is_finite());
            // The X2 half-band reconstruction can ring slightly above the
            // nonlinear function's 0.95 ceiling on a Nyquist-rate square.
            assert!(output.abs() < 1.25, "output={output}");
        }
    }

    #[test]
    fn gain_dist_increases_nonlinear_difference() {
        fn residual(gain_db: f32) -> f64 {
            let effect = EntityDynamics::new(SAMPLE_RATE);
            effect.set_gain_dist_db(gain_db);
            effect.set_dynamics(1.0);
            settle(&effect);
            let mut input_energy = 0.0;
            let mut error_energy = 0.0;
            for sample in 0..24_000 {
                let input =
                    (std::f32::consts::TAU * 100.0 * sample as f32 / SAMPLE_RATE).sin() * 0.25;
                let output = effect.process(input);
                if sample >= 12_000 {
                    input_energy += (input as f64).powi(2);
                    error_energy += (output as f64 - input as f64).powi(2);
                }
            }
            (error_energy / input_energy).sqrt()
        }

        assert!(residual(24.0) > residual(0.0) * 2.0);
    }

    #[test]
    fn parameters_clamp_and_ignore_non_finite_values() {
        let effect = EntityDynamics::new(SAMPLE_RATE);
        effect.set_bass_drive(2.0);
        effect.set_gain_dist_db(40.0);
        effect.set_dynamics(-1.0);
        assert_eq!(effect.bass_drive(), 1.0);
        assert_eq!(effect.gain_dist_db(), 24.0);
        assert_eq!(effect.dynamics(), 0.0);
        effect.set_gain_dist_db(f32::NAN);
        assert_eq!(effect.gain_dist_db(), 24.0);
    }

    #[test]
    fn stereo_channels_keep_independent_state() {
        let effect = EntityDynamics::new(SAMPLE_RATE);
        effect.set_dynamics(1.0);
        settle(&effect);
        for _ in 0..1_000 {
            let output = effect.process_stereo(StereoFrame { l: 0.8, r: 0.0 });
            assert_eq!(output.r, 0.0);
        }
    }
}
