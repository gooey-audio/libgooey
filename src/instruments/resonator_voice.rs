//! Flexible, allocation-free percussion voice built from two resonant modes.
//!
//! This is deliberately a synthesis primitive rather than a model of one drum.
//! A short transient and a separately enveloped, filtered noise source can excite
//! either resonator. Mode one can also feed mode two, producing parallel, serial,
//! and hybrid structures without a cyclic inter-mode feedback path.

use crate::engine::{Instrument, Modulatable};
use crate::filters::{Resonator, StateVariableFilterTpt};
use crate::gen::{Exciter, ExciterKind};
use crate::utils::{Oversampler, OversamplingMode, SmoothedParam, XorShift32};

const SMOOTH_MS: f32 = 15.0;

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResonatorExciterShape {
    Pulse,
    Click,
    Noise,
}

impl ResonatorExciterShape {
    fn kind(self) -> ExciterKind {
        match self {
            Self::Pulse => ExciterKind::Pulse,
            Self::Click => ExciterKind::ClickTable,
            Self::Noise => ExciterKind::NoiseBurst,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResonatorOutputTap {
    Lowpass,
    Bandpass,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResonatorModeConfig {
    pub frequency_ratio: f32,
    pub tuning_semitones: f32,
    pub pitch_sweep_octaves: f32,
    pub decay_seconds: f32,
    pub feedback: f32,
    /// A positive value selects damping directly; zero derives damping from T60.
    pub damping_override: f32,
    pub drive: f32,
    pub tap: ResonatorOutputTap,
    pub level: f32,
}

impl Default for ResonatorModeConfig {
    fn default() -> Self {
        Self {
            frequency_ratio: 1.0,
            tuning_semitones: 0.0,
            pitch_sweep_octaves: 0.0,
            decay_seconds: 0.7,
            feedback: 0.0,
            damping_override: 0.0,
            drive: 1.0,
            tap: ResonatorOutputTap::Lowpass,
            level: 0.7,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResonatorExciterConfig {
    pub shape: ResonatorExciterShape,
    pub width_ms: f32,
    pub level: f32,
    pub seed: u32,
}

impl Default for ResonatorExciterConfig {
    fn default() -> Self {
        Self {
            shape: ResonatorExciterShape::Pulse,
            width_ms: 1.0,
            level: 0.8,
            seed: 0x51a7_0001,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResonatorNoiseConfig {
    pub attack_seconds: f32,
    pub decay_seconds: f32,
    pub filter_hz: f32,
    pub filter_q: f32,
    pub level: f32,
    /// Zero makes noise level independent of velocity; one scales it fully.
    pub velocity_response: f32,
}

impl Default for ResonatorNoiseConfig {
    fn default() -> Self {
        Self {
            attack_seconds: 0.001,
            decay_seconds: 0.12,
            filter_hz: 4_000.0,
            filter_q: 0.8,
            level: 0.0,
            velocity_response: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResonatorRoutingConfig {
    pub transient_to_mode1: f32,
    pub transient_to_mode2: f32,
    pub noise_to_mode1: f32,
    pub noise_to_mode2: f32,
    pub mode1_to_mode2: f32,
    pub transient_to_output: f32,
    pub noise_to_output: f32,
    pub mode1_to_output: f32,
    pub mode2_to_output: f32,
}

impl Default for ResonatorRoutingConfig {
    fn default() -> Self {
        Self {
            transient_to_mode1: 1.0,
            transient_to_mode2: 0.0,
            noise_to_mode1: 0.0,
            noise_to_mode2: 0.0,
            mode1_to_mode2: 0.0,
            transient_to_output: 0.0,
            noise_to_output: 0.0,
            mode1_to_output: 1.0,
            mode2_to_output: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResonatorMacroConfig {
    pub pitch: f32,
    pub pitch_sweep: f32,
    pub sweep_time: f32,
    pub decay: f32,
    pub body_character: f32,
    pub noise: f32,
    pub coupling: f32,
    pub drive: f32,
    pub volume: f32,
}

impl Default for ResonatorMacroConfig {
    fn default() -> Self {
        Self {
            pitch: 0.35,
            pitch_sweep: 0.0,
            sweep_time: 0.3,
            decay: 0.45,
            body_character: 0.35,
            noise: 0.0,
            coupling: 0.0,
            drive: 0.15,
            volume: 0.75,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResonatorVoiceConfig {
    pub base_frequency_hz: f32,
    pub sweep_seconds: f32,
    pub mode1: ResonatorModeConfig,
    pub mode2: ResonatorModeConfig,
    pub exciter: ResonatorExciterConfig,
    pub noise: ResonatorNoiseConfig,
    pub routing: ResonatorRoutingConfig,
    pub macros: ResonatorMacroConfig,
    pub volume: f32,
}

impl Default for ResonatorVoiceConfig {
    fn default() -> Self {
        Self::kick()
    }
}

impl ResonatorVoiceConfig {
    pub fn kick() -> Self {
        Self {
            base_frequency_hz: 52.0,
            sweep_seconds: 0.055,
            mode1: ResonatorModeConfig {
                pitch_sweep_octaves: 3.0,
                decay_seconds: 1.1,
                feedback: 0.35,
                drive: 2.5,
                level: 1.0,
                ..Default::default()
            },
            mode2: ResonatorModeConfig {
                frequency_ratio: 2.7,
                pitch_sweep_octaves: 1.0,
                decay_seconds: 0.12,
                tap: ResonatorOutputTap::Bandpass,
                level: 0.2,
                ..Default::default()
            },
            exciter: ResonatorExciterConfig {
                shape: ResonatorExciterShape::Noise,
                ..Default::default()
            },
            noise: ResonatorNoiseConfig {
                level: 0.08,
                decay_seconds: 0.018,
                ..Default::default()
            },
            routing: ResonatorRoutingConfig {
                transient_to_mode1: 0.8,
                transient_to_mode2: 0.3,
                noise_to_mode1: 0.4,
                noise_to_mode2: 0.7,
                mode1_to_mode2: 0.2,
                transient_to_output: 0.15,
                ..Default::default()
            },
            macros: ResonatorMacroConfig {
                pitch_sweep: 0.75,
                decay: 0.55,
                drive: 0.45,
                ..Default::default()
            },
            volume: 0.7,
        }
    }

    pub fn tom() -> Self {
        Self {
            base_frequency_hz: 105.0,
            sweep_seconds: 0.12,
            mode1: ResonatorModeConfig {
                pitch_sweep_octaves: 0.7,
                decay_seconds: 0.8,
                feedback: 0.15,
                level: 0.9,
                ..Default::default()
            },
            mode2: ResonatorModeConfig {
                frequency_ratio: 1.53,
                decay_seconds: 0.42,
                tap: ResonatorOutputTap::Bandpass,
                level: 0.32,
                ..Default::default()
            },
            routing: ResonatorRoutingConfig {
                transient_to_mode1: 1.0,
                transient_to_mode2: 0.5,
                mode1_to_mode2: 0.2,
                ..Default::default()
            },
            macros: ResonatorMacroConfig {
                pitch: 0.52,
                pitch_sweep: 0.25,
                decay: 0.5,
                body_character: 0.3,
                ..Default::default()
            },
            exciter: Default::default(),
            noise: Default::default(),
            volume: 0.72,
        }
    }

    pub fn snare() -> Self {
        Self {
            base_frequency_hz: 175.0,
            sweep_seconds: 0.035,
            mode1: ResonatorModeConfig {
                decay_seconds: 0.24,
                drive: 1.4,
                level: 0.45,
                ..Default::default()
            },
            mode2: ResonatorModeConfig {
                frequency_ratio: 1.62,
                decay_seconds: 0.18,
                tap: ResonatorOutputTap::Bandpass,
                level: 0.32,
                ..Default::default()
            },
            exciter: ResonatorExciterConfig {
                shape: ResonatorExciterShape::Noise,
                width_ms: 2.0,
                ..Default::default()
            },
            noise: ResonatorNoiseConfig {
                attack_seconds: 0.001,
                decay_seconds: 0.34,
                filter_hz: 5_500.0,
                filter_q: 0.7,
                level: 0.9,
                velocity_response: 0.7,
            },
            routing: ResonatorRoutingConfig {
                transient_to_mode1: 0.7,
                transient_to_mode2: 0.7,
                noise_to_mode1: 0.25,
                noise_to_mode2: 0.35,
                noise_to_output: 0.75,
                mode1_to_output: 0.5,
                mode2_to_output: 0.5,
                ..Default::default()
            },
            macros: ResonatorMacroConfig {
                pitch: 0.63,
                decay: 0.38,
                body_character: 0.6,
                noise: 0.85,
                drive: 0.3,
                ..Default::default()
            },
            volume: 0.55,
        }
    }

    pub fn hybrid() -> Self {
        let mut c = Self::snare();
        c.base_frequency_hz = 72.0;
        c.mode1.pitch_sweep_octaves = 1.7;
        c.mode1.decay_seconds = 1.4;
        c.mode2.frequency_ratio = 3.73;
        c.routing.mode1_to_mode2 = 0.65;
        c.noise.decay_seconds = 0.7;
        c.macros.coupling = 0.65;
        c
    }

    pub fn metallic_drone() -> Self {
        Self {
            base_frequency_hz: 44.0,
            sweep_seconds: 1.4,
            mode1: ResonatorModeConfig {
                decay_seconds: 8.0,
                feedback: 0.72,
                drive: 1.8,
                level: 0.55,
                ..Default::default()
            },
            mode2: ResonatorModeConfig {
                frequency_ratio: 6.81,
                tuning_semitones: 0.3,
                decay_seconds: 5.0,
                feedback: 0.6,
                tap: ResonatorOutputTap::Bandpass,
                level: 0.5,
                ..Default::default()
            },
            exciter: ResonatorExciterConfig {
                shape: ResonatorExciterShape::Noise,
                width_ms: 8.0,
                ..Default::default()
            },
            noise: ResonatorNoiseConfig {
                decay_seconds: 1.5,
                filter_hz: 7_000.0,
                level: 0.18,
                ..Default::default()
            },
            routing: ResonatorRoutingConfig {
                transient_to_mode1: 0.8,
                transient_to_mode2: 0.8,
                noise_to_mode1: 0.4,
                noise_to_mode2: 0.8,
                mode1_to_mode2: 0.7,
                mode1_to_output: 0.7,
                mode2_to_output: 0.7,
                ..Default::default()
            },
            macros: ResonatorMacroConfig {
                pitch: 0.3,
                decay: 0.95,
                body_character: 0.65,
                noise: 0.2,
                coupling: 0.7,
                drive: 0.35,
                volume: 0.55,
                ..Default::default()
            },
            volume: 0.5,
        }
    }
}

struct ModeParams {
    ratio: SmoothedParam,
    tuning: SmoothedParam,
    sweep: SmoothedParam,
    decay: SmoothedParam,
    feedback: SmoothedParam,
    damping: SmoothedParam,
    drive: SmoothedParam,
    level: SmoothedParam,
}

impl ModeParams {
    fn new(c: ResonatorModeConfig, sr: f32) -> Self {
        Self {
            ratio: SmoothedParam::new(finite(c.frequency_ratio, 1.0), 0.0625, 32.0, sr, SMOOTH_MS),
            tuning: SmoothedParam::new(finite(c.tuning_semitones, 0.0), -48.0, 48.0, sr, SMOOTH_MS),
            sweep: SmoothedParam::new(finite(c.pitch_sweep_octaves, 0.0), -8.0, 8.0, sr, SMOOTH_MS),
            decay: SmoothedParam::new(finite(c.decay_seconds, 0.7), 0.005, 20.0, sr, SMOOTH_MS),
            feedback: SmoothedParam::new(finite(c.feedback, 0.0), 0.0, 0.8, sr, SMOOTH_MS),
            damping: SmoothedParam::new(finite(c.damping_override, 0.0), 0.0, 1.0, sr, SMOOTH_MS),
            drive: SmoothedParam::new(finite(c.drive, 1.0), 0.1, 24.0, sr, SMOOTH_MS),
            level: SmoothedParam::new(finite(c.level, 0.7), 0.0, 2.0, sr, SMOOTH_MS),
        }
    }
}

struct RoutingParams {
    values: [SmoothedParam; 9],
}

impl RoutingParams {
    fn new(c: ResonatorRoutingConfig, sr: f32) -> Self {
        let v = [
            c.transient_to_mode1,
            c.transient_to_mode2,
            c.noise_to_mode1,
            c.noise_to_mode2,
            c.mode1_to_mode2,
            c.transient_to_output,
            c.noise_to_output,
            c.mode1_to_output,
            c.mode2_to_output,
        ];
        Self {
            values: std::array::from_fn(|i| {
                SmoothedParam::new(finite(v[i], 0.0), 0.0, 1.0, sr, SMOOTH_MS)
            }),
        }
    }
    fn tick(&mut self) -> [f32; 9] {
        std::array::from_fn(|i| self.values[i].tick())
    }
}

pub struct ResonatorVoiceParams {
    pub pitch: SmoothedParam,
    pub pitch_sweep: SmoothedParam,
    pub sweep_time: SmoothedParam,
    pub decay: SmoothedParam,
    pub body_character: SmoothedParam,
    pub noise: SmoothedParam,
    pub coupling: SmoothedParam,
    pub drive: SmoothedParam,
    pub volume: SmoothedParam,
}

impl ResonatorVoiceParams {
    fn new(c: ResonatorMacroConfig, sr: f32) -> Self {
        Self {
            pitch: SmoothedParam::new_normalized(c.pitch, sr),
            pitch_sweep: SmoothedParam::new_normalized(c.pitch_sweep, sr),
            sweep_time: SmoothedParam::new_normalized(c.sweep_time, sr),
            decay: SmoothedParam::new_normalized(c.decay, sr),
            body_character: SmoothedParam::new_normalized(c.body_character, sr),
            noise: SmoothedParam::new_normalized(c.noise, sr),
            coupling: SmoothedParam::new_normalized(c.coupling, sr),
            drive: SmoothedParam::new_normalized(c.drive, sr),
            volume: SmoothedParam::new_normalized(c.volume, sr),
        }
    }
}

pub struct ResonatorVoice {
    sample_rate: f32,
    pub params: ResonatorVoiceParams,
    mode_params: [ModeParams; 2],
    routing: RoutingParams,
    modes: [Resonator; 2],
    oversamplers: [Oversampler; 2],
    exciter: Exciter,
    noise_filter: StateVariableFilterTpt,
    noise_rng: XorShift32,
    exciter_config: ResonatorExciterConfig,
    exciter_width: SmoothedParam,
    exciter_level: SmoothedParam,
    noise_params: [SmoothedParam; 6],
    pending_taps: [ResonatorOutputTap; 2],
    active_taps: [ResonatorOutputTap; 2],
    base_frequency: SmoothedParam,
    sweep_seconds: SmoothedParam,
    output_volume: SmoothedParam,
    velocity: f32,
    trigger_time: f64,
    midi_note: Option<u8>,
    ring_limit_secs: Option<f32>,
    active: bool,
}

impl ResonatorVoice {
    pub fn new(sample_rate: f32) -> Self {
        Self::with_config(sample_rate, ResonatorVoiceConfig::default())
    }
    pub fn with_config(sample_rate: f32, config: ResonatorVoiceConfig) -> Self {
        let sr = sample_rate.max(1.0);
        let mut exciter = Exciter::new(sr);
        exciter.set_kind(config.exciter.shape.kind());
        exciter.set_width_ms(config.exciter.width_ms);
        Self {
            sample_rate: sr,
            params: ResonatorVoiceParams::new(config.macros, sr),
            mode_params: [
                ModeParams::new(config.mode1, sr),
                ModeParams::new(config.mode2, sr),
            ],
            routing: RoutingParams::new(config.routing, sr),
            modes: [Resonator::new(sr), Resonator::new(sr)],
            oversamplers: [
                Oversampler::new(OversamplingMode::X2),
                Oversampler::new(OversamplingMode::X2),
            ],
            exciter,
            noise_filter: StateVariableFilterTpt::new(
                sr,
                config.noise.filter_hz,
                config.noise.filter_q,
            ),
            noise_rng: XorShift32::new(config.exciter.seed),
            exciter_config: config.exciter,
            exciter_width: SmoothedParam::new(config.exciter.width_ms, 0.25, 8.0, sr, SMOOTH_MS),
            exciter_level: SmoothedParam::new(config.exciter.level, 0.0, 2.0, sr, SMOOTH_MS),
            noise_params: [
                SmoothedParam::new(config.noise.attack_seconds, 0.0001, 2.0, sr, SMOOTH_MS),
                SmoothedParam::new(config.noise.decay_seconds, 0.001, 20.0, sr, SMOOTH_MS),
                SmoothedParam::new(config.noise.filter_hz, 20.0, sr * 0.45, sr, SMOOTH_MS),
                SmoothedParam::new(config.noise.filter_q, 0.5, 20.0, sr, SMOOTH_MS),
                SmoothedParam::new(config.noise.level, 0.0, 2.0, sr, SMOOTH_MS),
                SmoothedParam::new_normalized(config.noise.velocity_response, sr),
            ],
            pending_taps: [config.mode1.tap, config.mode2.tap],
            active_taps: [config.mode1.tap, config.mode2.tap],
            base_frequency: SmoothedParam::new(
                config.base_frequency_hz,
                2.0,
                sr * 0.45,
                sr,
                SMOOTH_MS,
            ),
            sweep_seconds: SmoothedParam::new(config.sweep_seconds, 0.002, 4.0, sr, SMOOTH_MS),
            output_volume: SmoothedParam::new(config.volume, 0.0, 2.0, sr, SMOOTH_MS),
            velocity: 1.0,
            trigger_time: 0.0,
            midi_note: None,
            ring_limit_secs: None,
            active: false,
        }
    }

    pub fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        let velocity = finite(velocity, 0.0).clamp(0.0, 1.0);
        if velocity == 0.0 {
            return;
        }
        self.velocity = velocity;
        self.trigger_time = time;
        self.active = true;
        self.active_taps = self.pending_taps;
        self.exciter.set_kind(self.exciter_config.shape.kind());
        self.exciter.set_width_ms(self.exciter_width.target());
        self.exciter.trigger(
            velocity * self.exciter_level.target(),
            self.noise_rng.next_u32() ^ self.exciter_config.seed,
        );
        self.noise_filter.reset();
        for os in &mut self.oversamplers {
            os.reset();
        }
    }

    pub fn tick(&mut self, time: f64) -> f32 {
        if !self.active {
            return 0.0;
        }
        let elapsed = (time - self.trigger_time).max(0.0) as f32;
        if self.ring_limit_secs.is_some_and(|limit| elapsed >= limit) {
            self.reset();
            return 0.0;
        }
        let pitch = self.params.pitch.tick();
        let sweep_macro = self.params.pitch_sweep.tick();
        let sweep_time_macro = self.params.sweep_time.tick();
        let decay_macro = self.params.decay.tick();
        let balance = self.params.body_character.tick();
        let noise_macro = self.params.noise.tick();
        let coupling = self.params.coupling.tick();
        let drive_macro = self.params.drive.tick();
        let macro_volume = self.params.volume.tick();
        let base = self
            .midi_note
            .map(|n| 440.0 * 2.0_f32.powf((n as f32 - 69.0) / 12.0))
            .unwrap_or_else(|| 20.0 * 20.0_f32.powf(pitch));
        let base = if self.midi_note.is_some() {
            base
        } else {
            0.5 * base + 0.5 * self.base_frequency.tick()
        };
        let sweep_seconds = (0.5 * self.sweep_seconds.tick()
            + 0.5 * (0.002 * 1000.0_f32.powf(sweep_time_macro)))
        .max(0.002);
        let sweep_env = (-elapsed * 6.91 / sweep_seconds).exp();
        let transient = self.exciter.tick();
        let attack = self.noise_params[0].tick();
        let noise_decay = self.noise_params[1].tick();
        let noise_hz = self.noise_params[2].tick();
        let noise_q = self.noise_params[3].tick();
        let noise_level = self.noise_params[4].tick();
        let noise_velocity = self.noise_params[5].tick();
        let noise_env = if elapsed < attack {
            elapsed / attack
        } else {
            (-(elapsed - attack) * 6.91 / noise_decay).exp()
        };
        self.noise_filter.set_params(noise_hz, noise_q);
        let velocity_scale = 1.0 - noise_velocity * (1.0 - self.velocity);
        let noise = self
            .noise_filter
            .process_mode(self.noise_rng.next_bipolar(), 1)
            * noise_env
            * noise_level
            * noise_macro
            * velocity_scale;
        let route = self.routing.tick();
        let mut taps = [0.0; 2];
        let mut raw = [0.0; 2];
        for i in 0..2 {
            let p = &mut self.mode_params[i];
            let ratio = p.ratio.tick() * 2.0_f32.powf(p.tuning.tick() / 12.0);
            let sweep = p.sweep.tick() * (0.25 + 1.5 * sweep_macro);
            let hz = (base * ratio * 2.0_f32.powf(sweep * sweep_env))
                .clamp(2.0, self.sample_rate * 0.45);
            self.modes[i].set_frequency(hz);
            self.modes[i].set_feedback(p.feedback.tick());
            let damping = p.damping.tick();
            if damping > 0.0001 {
                self.modes[i].set_damping(damping);
            } else {
                self.modes[i].set_decay_time(p.decay.tick() * (0.15 + 1.7 * decay_macro));
            }
            let input = if i == 0 {
                transient * route[0] + noise * route[2]
            } else {
                transient * route[1] + noise * route[3] + raw[0] * route[4] * coupling
            };
            raw[i] = self.modes[i].process(input);
            let source = match self.active_taps[i] {
                ResonatorOutputTap::Lowpass => raw[i],
                ResonatorOutputTap::Bandpass => self.modes[i].bandpass(),
            };
            let drive = p.drive.tick() * (0.5 + 3.0 * drive_macro);
            taps[i] = self.oversamplers[i].process(source, |s| (s * drive).tanh()) * p.level.tick();
        }
        let body = taps[0] * (1.0 - 0.65 * balance);
        let character = taps[1] * (0.35 + 0.65 * balance);
        let out = transient * route[5] + noise * route[6] + body * route[7] + character * route[8];
        let noise_finished = elapsed > attack + noise_decay * 1.7;
        if !self.exciter.is_active()
            && noise_finished
            && noise_env < 1.0e-5
            && self.modes.iter().all(Resonator::is_quiet)
        {
            self.active = false;
        }
        out * self.velocity.sqrt() * self.output_volume.tick() * macro_volume
    }

    pub fn is_active(&self) -> bool {
        self.active
    }
    pub fn reset(&mut self) {
        for m in &mut self.modes {
            m.reset();
        }
        for o in &mut self.oversamplers {
            o.reset();
        }
        self.exciter.reset();
        self.noise_filter.reset();
        self.active = false;
    }
    pub fn set_ring_limit_secs(&mut self, limit: Option<f32>) {
        self.ring_limit_secs = limit.map(|v| finite(v, 0.001).max(0.001));
    }
    pub fn set_exciter_shape(&mut self, shape: ResonatorExciterShape) {
        self.exciter_config.shape = shape;
    }
    pub fn set_output_tap(&mut self, mode: usize, tap: ResonatorOutputTap) {
        if mode < 2 {
            self.pending_taps[mode] = tap;
        }
    }
    pub fn active_output_tap(&self, mode: usize) -> Option<ResonatorOutputTap> {
        self.active_taps.get(mode).copied()
    }
    pub fn set_mode_frequency_ratio(&mut self, mode: usize, value: f32) {
        if let Some(p) = self.mode_params.get_mut(mode) {
            p.ratio.set_target(finite(value, 1.0));
        }
    }
    pub fn set_base_frequency_hz(&mut self, value: f32) {
        self.base_frequency.set_target(finite(value, 60.0));
        self.midi_note = None;
    }
    pub fn set_sweep_seconds(&mut self, value: f32) {
        self.sweep_seconds.set_target(finite(value, 0.1));
    }
    pub fn set_exciter_width_ms(&mut self, value: f32) {
        self.exciter_width.set_target(finite(value, 1.0));
    }
    pub fn set_exciter_level(&mut self, value: f32) {
        self.exciter_level.set_target(finite(value, 0.8));
    }
    pub fn set_noise_config(&mut self, config: ResonatorNoiseConfig) {
        let values = [
            config.attack_seconds,
            config.decay_seconds,
            config.filter_hz,
            config.filter_q,
            config.level,
            config.velocity_response,
        ];
        for (param, value) in self.noise_params.iter_mut().zip(values) {
            param.set_target(finite(value, param.target()));
        }
    }
    pub fn set_mode_tuning_semitones(&mut self, mode: usize, value: f32) {
        if let Some(p) = self.mode_params.get_mut(mode) {
            p.tuning.set_target(finite(value, 0.0));
        }
    }
    pub fn set_mode_pitch_sweep_octaves(&mut self, mode: usize, value: f32) {
        if let Some(p) = self.mode_params.get_mut(mode) {
            p.sweep.set_target(finite(value, 0.0));
        }
    }
    pub fn set_mode_damping_override(&mut self, mode: usize, value: f32) {
        if let Some(p) = self.mode_params.get_mut(mode) {
            p.damping.set_target(finite(value, 0.0));
        }
    }
    pub fn set_mode_level(&mut self, mode: usize, value: f32) {
        if let Some(p) = self.mode_params.get_mut(mode) {
            p.level.set_target(finite(value, 0.7));
        }
    }
    pub fn set_output_volume(&mut self, value: f32) {
        self.output_volume.set_target(finite(value, 1.0));
    }
    pub fn set_mode_decay_seconds(&mut self, mode: usize, value: f32) {
        if let Some(p) = self.mode_params.get_mut(mode) {
            p.decay.set_target(finite(value, 0.7));
        }
    }
    pub fn set_mode_feedback(&mut self, mode: usize, value: f32) {
        if let Some(p) = self.mode_params.get_mut(mode) {
            p.feedback.set_target(finite(value, 0.0));
        }
    }
    pub fn set_mode_drive(&mut self, mode: usize, value: f32) {
        if let Some(p) = self.mode_params.get_mut(mode) {
            p.drive.set_target(finite(value, 1.0));
        }
    }
    pub fn set_routing(&mut self, config: ResonatorRoutingConfig) {
        let v = [
            config.transient_to_mode1,
            config.transient_to_mode2,
            config.noise_to_mode1,
            config.noise_to_mode2,
            config.mode1_to_mode2,
            config.transient_to_output,
            config.noise_to_output,
            config.mode1_to_output,
            config.mode2_to_output,
        ];
        for (p, v) in self.routing.values.iter_mut().zip(v) {
            p.set_target(finite(v, 0.0));
        }
    }
    pub fn set_macro(&mut self, name: &str, value: f32) -> Result<(), String> {
        let p = match name {
            "pitch" => &mut self.params.pitch,
            "pitch_sweep" => &mut self.params.pitch_sweep,
            "sweep_time" => &mut self.params.sweep_time,
            "decay" => &mut self.params.decay,
            "body_character" => &mut self.params.body_character,
            "noise" => &mut self.params.noise,
            "coupling" => &mut self.params.coupling,
            "drive" => &mut self.params.drive,
            "volume" => &mut self.params.volume,
            _ => return Err(format!("Unknown ResonatorVoice parameter: {name}")),
        };
        p.set_target(finite(value, 0.0));
        Ok(())
    }
}

impl Instrument for ResonatorVoice {
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        ResonatorVoice::trigger_with_velocity(self, time, velocity);
    }
    fn tick(&mut self, time: f64) -> f32 {
        ResonatorVoice::tick(self, time)
    }
    fn is_active(&self) -> bool {
        ResonatorVoice::is_active(self)
    }
    fn set_midi_note(&mut self, note: u8) {
        self.midi_note = Some(note);
    }
    fn set_frequency_normalized(&mut self, value: f32) {
        self.params.pitch.set_target(value);
        self.midi_note = None;
    }
    fn get_frequency(&self) -> Option<f32> {
        Some(self.params.pitch.target())
    }
    fn as_modulatable(&mut self) -> Option<&mut dyn Modulatable> {
        Some(self)
    }
}

impl Modulatable for ResonatorVoice {
    fn modulatable_parameters(&self) -> Vec<&'static str> {
        vec![
            "pitch",
            "pitch_sweep",
            "sweep_time",
            "decay",
            "body_character",
            "noise",
            "coupling",
            "drive",
            "volume",
        ]
    }
    fn apply_modulation(&mut self, parameter: &str, value: f32) -> Result<(), String> {
        let p = match parameter {
            "pitch" => &mut self.params.pitch,
            "pitch_sweep" => &mut self.params.pitch_sweep,
            "sweep_time" => &mut self.params.sweep_time,
            "decay" => &mut self.params.decay,
            "body_character" => &mut self.params.body_character,
            "noise" => &mut self.params.noise,
            "coupling" => &mut self.params.coupling,
            "drive" => &mut self.params.drive,
            "volume" => &mut self.params.volume,
            _ => return Err(format!("Unknown ResonatorVoice parameter: {parameter}")),
        };
        p.set_bipolar(value);
        Ok(())
    }
    fn parameter_range(&self, parameter: &str) -> Option<(f32, f32)> {
        self.modulatable_parameters()
            .contains(&parameter)
            .then_some((0.0, 1.0))
    }
}
