//! Dual-resonator kick voice inspired by struck analog filter cores.
//!
//! The first resonator creates the body and owns the decay. Its output passes
//! through an unnormalised soft clipper before driving a shorter, more heavily
//! damped character resonator. There is deliberately no amplitude envelope:
//! the stored energy in the resonators determines how long the sound lasts.

use crate::engine::{Instrument, Modulatable};
use crate::envelope::{ADSRConfig, Envelope, EnvelopeCurve};
use crate::filters::Resonator;
use crate::gen::{Exciter, ExciterKind};
use crate::utils::{
    tuning_to_multiplier, MacroCurve, MacroScale, MacroTarget, Oversampler, OversamplingMode,
    SmoothedParam, XorShift32,
};

const LINEAR_CURVE: MacroCurve = MacroCurve::new(&[(0.0, 0.0), (1.0, 1.0)]);
const RESONATE_CURVE: MacroCurve =
    MacroCurve::new(&[(0.0, 0.0), (0.6, 0.55), (0.85, 0.95), (1.0, 1.0)]);
const PUNCH_CURVE: MacroCurve = MacroCurve::new(&[(0.0, 0.0), (0.3, 0.15), (1.0, 1.0)]);

const FREQUENCY: MacroTarget = MacroTarget::new(LINEAR_CURVE, 8.0, 120.0, MacroScale::Log);
const PITCH_START_MULTIPLIER: MacroTarget =
    MacroTarget::new(LINEAR_CURVE, 1.0, 8.0, MacroScale::Linear);
const PITCH_DECAY_SECONDS: MacroTarget =
    MacroTarget::new(LINEAR_CURVE, 0.005, 0.4, MacroScale::Log);
const RESONATE_T60_SECONDS: MacroTarget =
    MacroTarget::new(LINEAR_CURVE, 0.05, 6.0, MacroScale::Log);
const CHARACTER_FREQUENCY: MacroTarget =
    MacroTarget::new(LINEAR_CURVE, 8.0, 4_000.0, MacroScale::Log);

/// Normalized controls for a dual-resonator kick preset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResoKickConfig {
    pub frequency: f32,
    pub depth: f32,
    pub pitch_decay: f32,
    pub resonate: f32,
    pub punch: f32,
    pub character: f32,
    pub ripple: f32,
    pub exciter_noise: f32,
    pub volume: f32,
}

impl ResoKickConfig {
    pub fn classic808() -> Self {
        Self {
            frequency: 0.67,
            depth: 0.55,
            pitch_decay: 0.35,
            resonate: 0.68,
            punch: 0.30,
            character: 0.30,
            ripple: 0.05,
            exciter_noise: 0.04,
            volume: 0.80,
        }
    }

    pub fn punch909() -> Self {
        Self {
            frequency: 0.72,
            depth: 0.72,
            pitch_decay: 0.24,
            resonate: 0.50,
            punch: 0.72,
            character: 0.55,
            ripple: 0.12,
            exciter_noise: 0.30,
            volume: 0.78,
        }
    }

    pub fn soft_bounce() -> Self {
        Self {
            frequency: 0.62,
            depth: 0.38,
            pitch_decay: 0.48,
            resonate: 0.60,
            punch: 0.12,
            character: 0.22,
            ripple: 0.0,
            exciter_noise: 0.01,
            volume: 0.88,
        }
    }

    pub fn tom() -> Self {
        Self {
            frequency: 0.82,
            depth: 0.50,
            pitch_decay: 0.70,
            resonate: 0.74,
            punch: 0.35,
            character: 0.55,
            ripple: 0.38,
            exciter_noise: 0.03,
            volume: 0.76,
        }
    }

    pub fn laser() -> Self {
        Self {
            frequency: 0.75,
            depth: 1.0,
            pitch_decay: 1.0,
            resonate: 0.72,
            punch: 0.25,
            character: 0.62,
            ripple: 0.48,
            exciter_noise: 0.02,
            volume: 0.72,
        }
    }

    pub fn sub_drone() -> Self {
        Self {
            frequency: 0.52,
            depth: 0.12,
            pitch_decay: 0.60,
            resonate: 1.0,
            punch: 0.18,
            character: 0.24,
            ripple: 0.22,
            exciter_noise: 0.0,
            volume: 0.58,
        }
    }
}

impl Default for ResoKickConfig {
    fn default() -> Self {
        Self::classic808()
    }
}

crate::impl_blendable!(ResoKickConfig {
    frequency,
    depth,
    pitch_decay,
    resonate,
    punch,
    character,
    ripple,
    exciter_noise,
    volume,
});

/// Smoothed, normalized controls for real-time editing.
pub struct ResoKickParams {
    pub frequency: SmoothedParam,
    pub depth: SmoothedParam,
    pub pitch_decay: SmoothedParam,
    pub resonate: SmoothedParam,
    pub punch: SmoothedParam,
    pub character: SmoothedParam,
    pub ripple: SmoothedParam,
    pub exciter_noise: SmoothedParam,
    pub volume: SmoothedParam,
    pub tuning: SmoothedParam,
}

impl ResoKickParams {
    fn from_config(config: &ResoKickConfig, sample_rate: f32) -> Self {
        Self {
            frequency: SmoothedParam::new_normalized(config.frequency, sample_rate),
            depth: SmoothedParam::new_normalized(config.depth, sample_rate),
            pitch_decay: SmoothedParam::new_normalized(config.pitch_decay, sample_rate),
            resonate: SmoothedParam::new_normalized(config.resonate, sample_rate),
            punch: SmoothedParam::new_normalized(config.punch, sample_rate),
            character: SmoothedParam::new_normalized(config.character, sample_rate),
            ripple: SmoothedParam::new_normalized(config.ripple, sample_rate),
            exciter_noise: SmoothedParam::new_normalized(config.exciter_noise, sample_rate),
            volume: SmoothedParam::new_normalized(config.volume, sample_rate),
            tuning: SmoothedParam::new_normalized(0.5, sample_rate),
        }
    }
}

/// A monophonic kick whose body and character both come from resonators.
pub struct ResoKick {
    sample_rate: f32,
    pub params: ResoKickParams,
    core1: Resonator,
    core2: Resonator,
    exciter: Exciter,
    pitch_env: Envelope,
    punch_os: Oversampler,
    seed_rng: XorShift32,
    velocity: f32,
    trigger_time: f64,
    midi_note: Option<u8>,
    ring_limit_secs: Option<f32>,
    active: bool,
}

impl ResoKick {
    pub fn new(sample_rate: f32) -> Self {
        Self::with_config(sample_rate, ResoKickConfig::default())
    }

    pub fn with_config(sample_rate: f32, config: ResoKickConfig) -> Self {
        let sample_rate = sample_rate.max(1.0);
        let mut exciter = Exciter::new(sample_rate);
        exciter.set_kind(ExciterKind::NoiseBurst);
        exciter.set_width_ms(1.0);

        Self {
            sample_rate,
            params: ResoKickParams::from_config(&config, sample_rate),
            core1: Resonator::new(sample_rate),
            core2: Resonator::new(sample_rate),
            exciter,
            pitch_env: Self::make_pitch_envelope(PITCH_DECAY_SECONDS.value(config.pitch_decay)),
            punch_os: Oversampler::new(OversamplingMode::X2),
            seed_rng: XorShift32::new(0x5eed_8080),
            velocity: 1.0,
            trigger_time: 0.0,
            midi_note: None,
            ring_limit_secs: None,
            active: false,
        }
    }

    fn make_pitch_envelope(decay_seconds: f32) -> Envelope {
        Envelope::with_config(
            ADSRConfig::new(0.001, decay_seconds, 0.0, 0.001)
                .with_decay_curve(EnvelopeCurve::Exponential(0.35)),
        )
    }

    pub fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        self.velocity = if velocity.is_finite() {
            velocity.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if self.velocity == 0.0 {
            return;
        }
        self.trigger_time = time;
        self.active = true;

        if self.core1.is_quiet() && self.core2.is_quiet() {
            self.core1.reset();
            self.core2.reset();
            self.punch_os.reset();
        }

        let strike = 0.9 * (0.4 + 0.6 * self.velocity);
        self.core1.excite(strike);
        self.exciter
            .trigger(self.velocity, self.seed_rng.next_u32());
        self.pitch_env.trigger(time);
    }

    pub fn tick(&mut self, current_time: f64) -> f32 {
        if !self.active {
            return 0.0;
        }
        if self
            .ring_limit_secs
            .is_some_and(|limit| current_time - self.trigger_time >= limit as f64)
        {
            self.reset();
            return 0.0;
        }

        let frequency = self.params.frequency.tick();
        let depth = self.params.depth.tick();
        let pitch_decay = self.params.pitch_decay.tick();
        let resonate = self.params.resonate.tick();
        let punch = self.params.punch.tick();
        let character = self.params.character.tick();
        let ripple = self.params.ripple.tick();
        let exciter_noise = self.params.exciter_noise.tick();
        let volume = self.params.volume.tick();
        let tuning = self.params.tuning.tick();

        let base_hz = self
            .midi_note
            .map(|note| 440.0 * 2.0_f32.powf((note as f32 - 69.0) / 12.0))
            .unwrap_or_else(|| FREQUENCY.value(frequency))
            .clamp(8.0, 120.0)
            * tuning_to_multiplier(tuning);

        let pitch_decay_seconds = PITCH_DECAY_SECONDS.value(pitch_decay);
        self.pitch_env.set_decay_time(pitch_decay_seconds);
        let env = self.pitch_env.get_amplitude(current_time);
        let start_multiplier = PITCH_START_MULTIPLIER.value(depth);
        let pitch_multiplier = 1.0 + (start_multiplier - 1.0) * env;

        let core1_frequency = (base_hz * pitch_multiplier).clamp(4.0, self.sample_rate * 0.45);
        let mut t60 = RESONATE_T60_SECONDS.value(resonate);
        if self.velocity > 0.75 && current_time - self.trigger_time < 0.015 {
            t60 *= 0.5;
        }
        self.core1.set_frequency(core1_frequency);
        self.core1.set_decay_time(t60);
        self.core1
            .set_feedback(RESONATE_CURVE.eval(resonate) * 1.05);

        let noise = self.exciter.tick() * exciter_noise * 0.6;
        let core1 = self.core1.process(noise);

        let punch_gain = 0.7 + PUNCH_CURVE.eval(punch) * 7.3;
        let punched = self
            .punch_os
            .process(core1, |sample| (sample * punch_gain).tanh());

        let character_hz = CHARACTER_FREQUENCY.value(character);
        let ripple_octaves = ripple * 2.0;
        let character_frequency =
            (character_hz * pitch_multiplier * 2.0_f32.powf(ripple_octaves * core1))
                .clamp(4.0, self.sample_rate * 0.45);
        let character_damping = 0.35 + (0.08 - 0.35) * (env * depth).clamp(0.0, 1.0);
        self.core2.set_frequency(character_frequency);
        self.core2.set_damping(character_damping);
        self.core2.set_feedback(0.0);
        let output = self.core2.process(punched);

        if self.core1.is_quiet() && self.core2.is_quiet() && !self.exciter.is_active() {
            self.active = false;
        }

        output * self.velocity.sqrt() * volume * 1.5
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn reset(&mut self) {
        self.core1.reset();
        self.core2.reset();
        self.exciter.reset();
        self.punch_os.reset();
        self.active = false;
    }

    pub fn set_ring_limit_secs(&mut self, limit: Option<f32>) {
        self.ring_limit_secs = limit.map(|seconds| seconds.max(0.001));
    }

    pub fn set_config(&mut self, config: ResoKickConfig) {
        self.midi_note = None;
        self.params.frequency.set_target(config.frequency);
        self.params.depth.set_target(config.depth);
        self.params.pitch_decay.set_target(config.pitch_decay);
        self.params.resonate.set_target(config.resonate);
        self.params.punch.set_target(config.punch);
        self.params.character.set_target(config.character);
        self.params.ripple.set_target(config.ripple);
        self.params.exciter_noise.set_target(config.exciter_noise);
        self.params.volume.set_target(config.volume);
    }

    pub fn config(&self) -> ResoKickConfig {
        ResoKickConfig {
            frequency: self.params.frequency.target(),
            depth: self.params.depth.target(),
            pitch_decay: self.params.pitch_decay.target(),
            resonate: self.params.resonate.target(),
            punch: self.params.punch.target(),
            character: self.params.character.target(),
            ripple: self.params.ripple.target(),
            exciter_noise: self.params.exciter_noise.target(),
            volume: self.params.volume.target(),
        }
    }

    pub fn set_frequency(&mut self, value: f32) {
        self.midi_note = None;
        self.params.frequency.set_target(value);
    }

    pub fn set_depth(&mut self, value: f32) {
        self.params.depth.set_target(value);
    }

    pub fn set_pitch_decay(&mut self, value: f32) {
        self.params.pitch_decay.set_target(value);
    }

    pub fn set_resonate(&mut self, value: f32) {
        self.params.resonate.set_target(value);
    }

    pub fn set_punch(&mut self, value: f32) {
        self.params.punch.set_target(value);
    }

    pub fn set_character(&mut self, value: f32) {
        self.params.character.set_target(value);
    }

    pub fn set_ripple(&mut self, value: f32) {
        self.params.ripple.set_target(value);
    }

    pub fn set_exciter_noise(&mut self, value: f32) {
        self.params.exciter_noise.set_target(value);
    }

    pub fn set_volume(&mut self, value: f32) {
        self.params.volume.set_target(value);
    }

    pub fn set_tuning(&mut self, value: f32) {
        self.params.tuning.set_target(value);
    }

    pub fn tuning(&self) -> f32 {
        self.params.tuning.target()
    }

    pub fn frequency_hz(&self) -> f32 {
        FREQUENCY.value(self.params.frequency.target())
            * tuning_to_multiplier(self.params.tuning.target())
    }

    pub fn pitch_decay_ms(&self) -> f32 {
        PITCH_DECAY_SECONDS.value(self.params.pitch_decay.target()) * 1_000.0
    }

    pub fn resonate_t60_seconds(&self) -> f32 {
        RESONATE_T60_SECONDS.value(self.params.resonate.target())
    }

    pub fn punch_gain(&self) -> f32 {
        0.7 + PUNCH_CURVE.eval(self.params.punch.target()) * 7.3
    }

    pub fn character_hz(&self) -> f32 {
        CHARACTER_FREQUENCY.value(self.params.character.target())
    }
}

impl Instrument for ResoKick {
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        ResoKick::trigger_with_velocity(self, time, velocity);
    }

    fn tick(&mut self, current_time: f64) -> f32 {
        ResoKick::tick(self, current_time)
    }

    fn is_active(&self) -> bool {
        ResoKick::is_active(self)
    }

    fn set_midi_note(&mut self, note: u8) {
        self.midi_note = Some(note);
    }

    fn set_frequency_normalized(&mut self, value: f32) {
        self.set_frequency(value);
    }

    fn get_frequency(&self) -> Option<f32> {
        Some(self.params.frequency.target())
    }

    fn as_modulatable(&mut self) -> Option<&mut dyn Modulatable> {
        Some(self)
    }
}

impl Modulatable for ResoKick {
    fn modulatable_parameters(&self) -> Vec<&'static str> {
        vec![
            "frequency",
            "depth",
            "pitch_decay",
            "resonate",
            "punch",
            "character",
            "ripple",
            "exciter_noise",
            "volume",
            "tuning",
        ]
    }

    fn apply_modulation(&mut self, parameter: &str, value: f32) -> Result<(), String> {
        let target = match parameter {
            "frequency" => &mut self.params.frequency,
            "depth" => &mut self.params.depth,
            "pitch_decay" => &mut self.params.pitch_decay,
            "resonate" => &mut self.params.resonate,
            "punch" => &mut self.params.punch,
            "character" => &mut self.params.character,
            "ripple" => &mut self.params.ripple,
            "exciter_noise" => &mut self.params.exciter_noise,
            "volume" => &mut self.params.volume,
            "tuning" => &mut self.params.tuning,
            _ => return Err(format!("Unknown ResoKick parameter: {parameter}")),
        };
        target.set_bipolar(value);
        Ok(())
    }

    fn parameter_range(&self, parameter: &str) -> Option<(f32, f32)> {
        self.modulatable_parameters()
            .contains(&parameter)
            .then_some((0.0, 1.0))
    }
}
