//! Six-voice expressive stereo synthesizer.
//!
//! Every host-facing parameter is normalized to `0.0..=1.0`. The two
//! documented exceptions are modulation-route `depth` and `key_scale`, which
//! are bipolar `-1.0..=1.0` values.

use std::f64::consts::TAU;

use crate::effects::waveshaper::Waveshaper;
use crate::engine::Instrument;
use crate::envelope::{ADSRConfig, Envelope, EnvelopeCurve};
use crate::filters::StateVariableFilterTpt;
use crate::frame::StereoFrame;
use crate::gen::polyblep::{polyblep_saw, polyblep_square};
use crate::music::note::midi_to_freq;
use crate::utils::SmoothedParam;

pub const POLY_PARAM_OSC_A_WAVEFORM: u32 = 0;
pub const POLY_PARAM_OSC_A_LEVEL: u32 = 1;
pub const POLY_PARAM_OSC_B_WAVEFORM: u32 = 2;
pub const POLY_PARAM_OSC_B_LEVEL: u32 = 3;
pub const POLY_PARAM_DETUNE: u32 = 4;
pub const POLY_PARAM_STEREO_WIDTH: u32 = 5;
pub const POLY_PARAM_AMP_ATTACK: u32 = 6;
pub const POLY_PARAM_AMP_DECAY: u32 = 7;
pub const POLY_PARAM_AMP_SUSTAIN: u32 = 8;
pub const POLY_PARAM_AMP_RELEASE: u32 = 9;
pub const POLY_PARAM_AMP_ATTACK_CURVE: u32 = 10;
pub const POLY_PARAM_AMP_FALL_CURVE: u32 = 11;
pub const POLY_PARAM_PITCH_ENV_AMOUNT: u32 = 12;
pub const POLY_PARAM_PITCH_ATTACK: u32 = 13;
pub const POLY_PARAM_PITCH_DECAY: u32 = 14;
pub const POLY_PARAM_PITCH_SUSTAIN: u32 = 15;
pub const POLY_PARAM_PITCH_RELEASE: u32 = 16;
pub const POLY_PARAM_PITCH_ATTACK_CURVE: u32 = 17;
pub const POLY_PARAM_PITCH_FALL_CURVE: u32 = 18;
pub const POLY_PARAM_FILTER_CUTOFF: u32 = 19;
pub const POLY_PARAM_FILTER_RESONANCE: u32 = 20;
pub const POLY_PARAM_FILTER_ENV_AMOUNT: u32 = 21;
pub const POLY_PARAM_FILTER_ATTACK: u32 = 22;
pub const POLY_PARAM_FILTER_DECAY: u32 = 23;
pub const POLY_PARAM_FILTER_SUSTAIN: u32 = 24;
pub const POLY_PARAM_FILTER_RELEASE: u32 = 25;
pub const POLY_PARAM_FILTER_ATTACK_CURVE: u32 = 26;
pub const POLY_PARAM_FILTER_FALL_CURVE: u32 = 27;
pub const POLY_PARAM_SATURATION: u32 = 28;
pub const POLY_PARAM_VOLUME: u32 = 29;
pub const POLY_PARAM_COUNT: u32 = 30;

pub const POLY_MOD_ROUTE_COUNT: usize = 8;
const NUM_VOICES: usize = 6;

mod ranges {
    pub fn filter_cutoff_hz(normalized: f32, sample_rate: f32) -> f32 {
        let hz = 20.0 * (18_000.0_f32 / 20.0).powf(normalized.clamp(0.0, 1.0));
        hz.min(sample_rate * 0.45)
    }

    pub fn filter_resonance_q(normalized: f32) -> f32 {
        0.5 + normalized.clamp(0.0, 1.0) * 14.5
    }

    pub fn env_time(normalized: f32) -> f32 {
        0.001 * 5000.0_f32.powf(normalized.clamp(0.0, 1.0))
    }

    pub fn curve_exponent(normalized: f32) -> f32 {
        // 0 -> 0.25, 0.5 -> 1, 1 -> 4.
        4.0_f32.powf(2.0 * normalized.clamp(0.0, 1.0) - 1.0)
    }

    pub fn pitch_env_semitones(normalized: f32) -> f32 {
        (normalized.clamp(0.0, 1.0) - 0.5) * 48.0
    }

    pub fn detune_half_cents(normalized: f32) -> f64 {
        // The public amount is total A-to-B separation: 0..30 cents.
        normalized.clamp(0.0, 1.0) as f64 * 15.0
    }

    pub fn saturation_drive(normalized: f32) -> f32 {
        1.0 + normalized.clamp(0.0, 1.0) * 1.5
    }

    pub fn saturation_mix(normalized: f32) -> f32 {
        normalized.clamp(0.0, 1.0) * 0.2
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum PolyModSource {
    Velocity = 0,
    KeyPosition = 1,
}

impl PolyModSource {
    pub fn from_id(id: u32) -> Option<Self> {
        match id {
            0 => Some(Self::Velocity),
            1 => Some(Self::KeyPosition),
            _ => None,
        }
    }

    pub const fn id(self) -> u32 {
        self as u32
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PolyModRoute {
    pub enabled: bool,
    pub source: PolyModSource,
    pub destination: u32,
    pub depth: f32,
    pub curve: f32,
    pub key_scale: f32,
}

impl PolyModRoute {
    pub const fn disabled() -> Self {
        Self {
            enabled: false,
            source: PolyModSource::Velocity,
            destination: POLY_PARAM_FILTER_CUTOFF,
            depth: 0.0,
            curve: 0.5,
            key_scale: 0.0,
        }
    }

    pub fn validated(mut self) -> Option<Self> {
        if self.destination >= POLY_PARAM_COUNT
            || !self.depth.is_finite()
            || !self.curve.is_finite()
            || !self.key_scale.is_finite()
        {
            return None;
        }
        self.depth = self.depth.clamp(-1.0, 1.0);
        self.curve = self.curve.clamp(0.0, 1.0);
        self.key_scale = self.key_scale.clamp(-1.0, 1.0);
        Some(self)
    }
}

impl Default for PolyModRoute {
    fn default() -> Self {
        Self::disabled()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PolyOscillatorConfig {
    pub waveform: f32,
    pub level: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct PolyEnvelopeConfig {
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    pub attack_curve: f32,
    pub fall_curve: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct PolyFilterConfig {
    pub cutoff: f32,
    pub resonance: f32,
    pub env_amount: f32,
    pub envelope: PolyEnvelopeConfig,
}

#[derive(Clone, Copy, Debug)]
pub struct PolySynthConfig {
    pub oscillator_a: PolyOscillatorConfig,
    pub oscillator_b: PolyOscillatorConfig,
    pub detune: f32,
    pub stereo_width: f32,
    pub amp_envelope: PolyEnvelopeConfig,
    pub pitch_env_amount: f32,
    pub pitch_envelope: PolyEnvelopeConfig,
    pub filter: PolyFilterConfig,
    pub saturation: f32,
    pub volume: f32,
    pub mod_routes: [PolyModRoute; POLY_MOD_ROUTE_COUNT],
}

impl PolySynthConfig {
    fn expressive_routes() -> [PolyModRoute; POLY_MOD_ROUTE_COUNT] {
        let mut routes = [PolyModRoute::disabled(); POLY_MOD_ROUTE_COUNT];
        routes[0] = PolyModRoute {
            enabled: true,
            source: PolyModSource::Velocity,
            destination: POLY_PARAM_FILTER_CUTOFF,
            depth: 0.18,
            curve: 0.65,
            key_scale: -0.06,
        };
        routes[1] = PolyModRoute {
            enabled: true,
            source: PolyModSource::KeyPosition,
            destination: POLY_PARAM_FILTER_CUTOFF,
            depth: 0.10,
            curve: 0.50,
            key_scale: 0.0,
        };
        routes[2] = PolyModRoute {
            enabled: true,
            source: PolyModSource::Velocity,
            destination: POLY_PARAM_SATURATION,
            depth: 0.06,
            curve: 0.65,
            key_scale: -0.02,
        };
        routes
    }

    #[allow(clippy::too_many_arguments)]
    fn migrated(
        osc_shape: f32,
        detune: f32,
        width: f32,
        filter_cutoff: f32,
        filter_resonance: f32,
        filter_env_amount: f32,
        amp_attack: f32,
        amp_decay: f32,
        amp_sustain: f32,
        amp_release: f32,
        filter_attack: f32,
        filter_decay: f32,
        filter_sustain: f32,
        filter_release: f32,
        volume: f32,
    ) -> Self {
        // Preserve the old saw-to-square timbre within the final third of the
        // new sine->triangle->saw->square morph.
        let waveform = 2.0 / 3.0 + osc_shape.clamp(0.0, 1.0) / 3.0;
        let amp_envelope = PolyEnvelopeConfig {
            attack: amp_attack,
            decay: amp_decay,
            sustain: amp_sustain,
            release: amp_release,
            attack_curve: 0.5,
            fall_curve: 0.25,
        };
        let pitch_envelope = PolyEnvelopeConfig {
            attack: 0.0,
            decay: 0.4,
            sustain: 0.0,
            release: 0.3,
            attack_curve: 0.5,
            fall_curve: 0.25,
        };
        let filter_envelope = PolyEnvelopeConfig {
            attack: filter_attack,
            decay: filter_decay,
            sustain: filter_sustain,
            release: filter_release,
            attack_curve: 0.5,
            fall_curve: 0.25,
        };
        Self {
            oscillator_a: PolyOscillatorConfig {
                waveform,
                level: 1.0,
            },
            oscillator_b: PolyOscillatorConfig {
                waveform,
                level: 1.0,
            },
            detune,
            stereo_width: width,
            amp_envelope,
            pitch_env_amount: 0.5,
            pitch_envelope,
            filter: PolyFilterConfig {
                cutoff: filter_cutoff,
                resonance: filter_resonance,
                env_amount: 0.5 + filter_env_amount.clamp(0.0, 1.0) * 0.5,
                envelope: filter_envelope,
            },
            saturation: 0.08,
            volume,
            mod_routes: Self::expressive_routes(),
        }
    }

    pub fn default() -> Self {
        Self::migrated(
            0.0, 0.2, 0.45, 0.6, 0.15, 0.3, 0.55, 0.7, 0.7, 0.8, 0.5, 0.65, 0.4, 0.75, 0.7,
        )
    }

    pub fn pad() -> Self {
        Self::migrated(
            0.0, 0.4, 0.80, 0.45, 0.2, 0.2, 0.8, 0.75, 0.8, 0.85, 0.75, 0.7, 0.5, 0.8, 0.6,
        )
    }

    pub fn pluck() -> Self {
        Self::migrated(
            0.3, 0.1, 0.25, 0.7, 0.25, 0.6, 0.0, 0.75, 0.0, 0.65, 0.0, 0.7, 0.1, 0.65, 0.7,
        )
    }

    pub fn keys() -> Self {
        Self::migrated(
            0.5, 0.15, 0.35, 0.55, 0.1, 0.4, 0.35, 0.7, 0.5, 0.75, 0.3, 0.65, 0.3, 0.7, 0.7,
        )
    }

    pub fn strings() -> Self {
        Self::migrated(
            0.0, 0.5, 0.75, 0.5, 0.1, 0.15, 0.85, 0.7, 0.9, 0.85, 0.8, 0.7, 0.6, 0.8, 0.5,
        )
    }

    pub fn set_param(&mut self, param: u32, value: f32) -> bool {
        if !value.is_finite() {
            return false;
        }
        let value = value.clamp(0.0, 1.0);
        let destination = match param {
            POLY_PARAM_OSC_A_WAVEFORM => &mut self.oscillator_a.waveform,
            POLY_PARAM_OSC_A_LEVEL => &mut self.oscillator_a.level,
            POLY_PARAM_OSC_B_WAVEFORM => &mut self.oscillator_b.waveform,
            POLY_PARAM_OSC_B_LEVEL => &mut self.oscillator_b.level,
            POLY_PARAM_DETUNE => &mut self.detune,
            POLY_PARAM_STEREO_WIDTH => &mut self.stereo_width,
            POLY_PARAM_AMP_ATTACK => &mut self.amp_envelope.attack,
            POLY_PARAM_AMP_DECAY => &mut self.amp_envelope.decay,
            POLY_PARAM_AMP_SUSTAIN => &mut self.amp_envelope.sustain,
            POLY_PARAM_AMP_RELEASE => &mut self.amp_envelope.release,
            POLY_PARAM_AMP_ATTACK_CURVE => &mut self.amp_envelope.attack_curve,
            POLY_PARAM_AMP_FALL_CURVE => &mut self.amp_envelope.fall_curve,
            POLY_PARAM_PITCH_ENV_AMOUNT => &mut self.pitch_env_amount,
            POLY_PARAM_PITCH_ATTACK => &mut self.pitch_envelope.attack,
            POLY_PARAM_PITCH_DECAY => &mut self.pitch_envelope.decay,
            POLY_PARAM_PITCH_SUSTAIN => &mut self.pitch_envelope.sustain,
            POLY_PARAM_PITCH_RELEASE => &mut self.pitch_envelope.release,
            POLY_PARAM_PITCH_ATTACK_CURVE => &mut self.pitch_envelope.attack_curve,
            POLY_PARAM_PITCH_FALL_CURVE => &mut self.pitch_envelope.fall_curve,
            POLY_PARAM_FILTER_CUTOFF => &mut self.filter.cutoff,
            POLY_PARAM_FILTER_RESONANCE => &mut self.filter.resonance,
            POLY_PARAM_FILTER_ENV_AMOUNT => &mut self.filter.env_amount,
            POLY_PARAM_FILTER_ATTACK => &mut self.filter.envelope.attack,
            POLY_PARAM_FILTER_DECAY => &mut self.filter.envelope.decay,
            POLY_PARAM_FILTER_SUSTAIN => &mut self.filter.envelope.sustain,
            POLY_PARAM_FILTER_RELEASE => &mut self.filter.envelope.release,
            POLY_PARAM_FILTER_ATTACK_CURVE => &mut self.filter.envelope.attack_curve,
            POLY_PARAM_FILTER_FALL_CURVE => &mut self.filter.envelope.fall_curve,
            POLY_PARAM_SATURATION => &mut self.saturation,
            POLY_PARAM_VOLUME => &mut self.volume,
            _ => return false,
        };
        *destination = value;
        true
    }

    pub fn param(&self, param: u32) -> Option<f32> {
        Some(match param {
            POLY_PARAM_OSC_A_WAVEFORM => self.oscillator_a.waveform,
            POLY_PARAM_OSC_A_LEVEL => self.oscillator_a.level,
            POLY_PARAM_OSC_B_WAVEFORM => self.oscillator_b.waveform,
            POLY_PARAM_OSC_B_LEVEL => self.oscillator_b.level,
            POLY_PARAM_DETUNE => self.detune,
            POLY_PARAM_STEREO_WIDTH => self.stereo_width,
            POLY_PARAM_AMP_ATTACK => self.amp_envelope.attack,
            POLY_PARAM_AMP_DECAY => self.amp_envelope.decay,
            POLY_PARAM_AMP_SUSTAIN => self.amp_envelope.sustain,
            POLY_PARAM_AMP_RELEASE => self.amp_envelope.release,
            POLY_PARAM_AMP_ATTACK_CURVE => self.amp_envelope.attack_curve,
            POLY_PARAM_AMP_FALL_CURVE => self.amp_envelope.fall_curve,
            POLY_PARAM_PITCH_ENV_AMOUNT => self.pitch_env_amount,
            POLY_PARAM_PITCH_ATTACK => self.pitch_envelope.attack,
            POLY_PARAM_PITCH_DECAY => self.pitch_envelope.decay,
            POLY_PARAM_PITCH_SUSTAIN => self.pitch_envelope.sustain,
            POLY_PARAM_PITCH_RELEASE => self.pitch_envelope.release,
            POLY_PARAM_PITCH_ATTACK_CURVE => self.pitch_envelope.attack_curve,
            POLY_PARAM_PITCH_FALL_CURVE => self.pitch_envelope.fall_curve,
            POLY_PARAM_FILTER_CUTOFF => self.filter.cutoff,
            POLY_PARAM_FILTER_RESONANCE => self.filter.resonance,
            POLY_PARAM_FILTER_ENV_AMOUNT => self.filter.env_amount,
            POLY_PARAM_FILTER_ATTACK => self.filter.envelope.attack,
            POLY_PARAM_FILTER_DECAY => self.filter.envelope.decay,
            POLY_PARAM_FILTER_SUSTAIN => self.filter.envelope.sustain,
            POLY_PARAM_FILTER_RELEASE => self.filter.envelope.release,
            POLY_PARAM_FILTER_ATTACK_CURVE => self.filter.envelope.attack_curve,
            POLY_PARAM_FILTER_FALL_CURVE => self.filter.envelope.fall_curve,
            POLY_PARAM_SATURATION => self.saturation,
            POLY_PARAM_VOLUME => self.volume,
            _ => return None,
        })
    }

    pub fn set_mod_route(&mut self, slot: usize, route: PolyModRoute) -> bool {
        let Some(destination) = self.mod_routes.get_mut(slot) else {
            return false;
        };
        let Some(route) = route.validated() else {
            return false;
        };
        *destination = route;
        true
    }

    pub fn clear_mod_route(&mut self, slot: usize) -> bool {
        let Some(route) = self.mod_routes.get_mut(slot) else {
            return false;
        };
        *route = PolyModRoute::disabled();
        true
    }
}

impl Default for PolySynthConfig {
    fn default() -> Self {
        Self::default()
    }
}

pub struct PolySynthParams {
    values: [SmoothedParam; POLY_PARAM_COUNT as usize],
}

impl PolySynthParams {
    pub fn from_config(config: &PolySynthConfig, sample_rate: f32) -> Self {
        Self {
            values: std::array::from_fn(|index| {
                SmoothedParam::new_normalized(config.param(index as u32).unwrap(), sample_rate)
            }),
        }
    }

    pub fn set(&mut self, param: u32, value: f32) -> bool {
        if !value.is_finite() {
            return false;
        }
        let Some(destination) = self.values.get_mut(param as usize) else {
            return false;
        };
        destination.set_target(value);
        true
    }

    pub fn current(&self, param: u32) -> Option<f32> {
        self.values.get(param as usize).map(SmoothedParam::get)
    }

    pub fn target(&self, param: u32) -> Option<f32> {
        self.values.get(param as usize).map(SmoothedParam::target)
    }

    pub fn tick(&mut self) {
        for value in &mut self.values {
            value.tick();
        }
    }

    pub fn snap_all(&mut self) {
        for value in &mut self.values {
            value.snap();
        }
    }
}

struct Voice {
    midi_note: u8,
    frequency: f64,
    phase_a: f64,
    phase_b: f64,
    amp_envelope: Envelope,
    pitch_envelope: Envelope,
    filter_envelope: Envelope,
    filter_l: StateVariableFilterTpt,
    filter_r: StateVariableFilterTpt,
    shaper_a: Waveshaper,
    shaper_b: Waveshaper,
    velocity: f32,
    modulation: [f32; POLY_PARAM_COUNT as usize],
    active: bool,
    trigger_order: u64,
}

impl Voice {
    fn new(sample_rate: f32) -> Self {
        Self {
            midi_note: 0,
            frequency: 440.0,
            phase_a: 0.0,
            phase_b: 0.0,
            amp_envelope: Envelope::new(),
            pitch_envelope: Envelope::new(),
            filter_envelope: Envelope::new(),
            filter_l: StateVariableFilterTpt::new(sample_rate, 1000.0, 1.0),
            filter_r: StateVariableFilterTpt::new(sample_rate, 1000.0, 1.0),
            shaper_a: Waveshaper::default(),
            shaper_b: Waveshaper::default(),
            velocity: 1.0,
            modulation: [0.0; POLY_PARAM_COUNT as usize],
            active: false,
            trigger_order: 0,
        }
    }
}

pub struct PolySynth {
    sample_rate: f32,
    pub params: PolySynthParams,
    mod_routes: [PolyModRoute; POLY_MOD_ROUTE_COUNT],
    voices: Vec<Voice>,
    trigger_counter: u64,
    pending_note: Option<u8>,
    current_time: f64,
}

impl PolySynth {
    pub fn new(sample_rate: f32) -> Self {
        Self::with_config(sample_rate, PolySynthConfig::default())
    }

    pub fn with_config(sample_rate: f32, config: PolySynthConfig) -> Self {
        let sample_rate = if sample_rate.is_finite() && sample_rate > 0.0 {
            sample_rate
        } else {
            44_100.0
        };
        Self {
            sample_rate,
            params: PolySynthParams::from_config(&config, sample_rate),
            mod_routes: config.mod_routes,
            voices: (0..NUM_VOICES).map(|_| Voice::new(sample_rate)).collect(),
            trigger_counter: 0,
            pending_note: None,
            current_time: 0.0,
        }
    }

    pub fn set_config(&mut self, config: PolySynthConfig) {
        for param in 0..POLY_PARAM_COUNT {
            self.params.set(param, config.param(param).unwrap());
        }
        self.mod_routes = config.mod_routes;
    }

    pub fn snap_params(&mut self) {
        self.params.snap_all();
    }

    pub fn set_param(&mut self, param: u32, value: f32) -> bool {
        self.params.set(param, value)
    }

    pub fn param(&self, param: u32) -> Option<f32> {
        self.params.target(param)
    }

    pub fn set_mod_route(&mut self, slot: usize, route: PolyModRoute) -> bool {
        let Some(destination) = self.mod_routes.get_mut(slot) else {
            return false;
        };
        let Some(route) = route.validated() else {
            return false;
        };
        *destination = route;
        true
    }

    pub fn mod_route(&self, slot: usize) -> Option<PolyModRoute> {
        self.mod_routes.get(slot).copied()
    }

    pub fn clear_mod_route(&mut self, slot: usize) -> bool {
        let Some(route) = self.mod_routes.get_mut(slot) else {
            return false;
        };
        *route = PolyModRoute::disabled();
        true
    }

    fn shaped_source(value: f32, curve: f32) -> f32 {
        value.signum() * value.abs().powf(ranges::curve_exponent(curve))
    }

    fn key_position(note: u8) -> f32 {
        ((note as f32 - 60.0) / 60.0).clamp(-1.0, 1.0)
    }

    fn resolve_modulation(&self, note: u8, velocity: f32) -> [f32; POLY_PARAM_COUNT as usize] {
        let mut result = [0.0; POLY_PARAM_COUNT as usize];
        let velocity = velocity.clamp(0.0, 1.0) * 2.0 - 1.0;
        let key_position = Self::key_position(note);
        for route in self.mod_routes.iter().filter(|route| route.enabled) {
            if route.destination >= POLY_PARAM_COUNT {
                continue;
            }
            let source = match route.source {
                PolyModSource::Velocity => velocity,
                PolyModSource::KeyPosition => key_position,
            };
            let shaped = Self::shaped_source(source, route.curve);
            let depth = (route.depth + route.key_scale * key_position).clamp(-1.0, 1.0);
            result[route.destination as usize] += shaped * depth;
        }
        result
    }

    fn modulated_target(&self, param: u32, modulation: &[f32; POLY_PARAM_COUNT as usize]) -> f32 {
        (self.params.target(param).unwrap() + modulation[param as usize]).clamp(0.0, 1.0)
    }

    #[allow(clippy::too_many_arguments)]
    fn envelope_config(
        &self,
        modulation: &[f32; POLY_PARAM_COUNT as usize],
        attack: u32,
        decay: u32,
        sustain: u32,
        release: u32,
        attack_curve: u32,
        fall_curve: u32,
    ) -> (ADSRConfig, EnvelopeCurve) {
        let attack_curve = EnvelopeCurve::Exponential(ranges::curve_exponent(
            self.modulated_target(attack_curve, modulation),
        ));
        let fall_curve = EnvelopeCurve::Exponential(ranges::curve_exponent(
            self.modulated_target(fall_curve, modulation),
        ));
        let config = ADSRConfig::new(
            ranges::env_time(self.modulated_target(attack, modulation)),
            ranges::env_time(self.modulated_target(decay, modulation)),
            self.modulated_target(sustain, modulation),
            ranges::env_time(self.modulated_target(release, modulation)),
        )
        .with_attack_curve(attack_curve)
        .with_decay_curve(fall_curve);
        (config, fall_curve)
    }

    pub fn trigger_note(&mut self, note: u8, velocity: f32) {
        self.trigger_note_at(note, velocity, self.current_time);
    }

    fn trigger_note_at(&mut self, note: u8, velocity: f32, time: f64) {
        let velocity = if velocity.is_finite() {
            velocity.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let modulation = self.resolve_modulation(note, velocity);
        let (amp_config, amp_fall) = self.envelope_config(
            &modulation,
            POLY_PARAM_AMP_ATTACK,
            POLY_PARAM_AMP_DECAY,
            POLY_PARAM_AMP_SUSTAIN,
            POLY_PARAM_AMP_RELEASE,
            POLY_PARAM_AMP_ATTACK_CURVE,
            POLY_PARAM_AMP_FALL_CURVE,
        );
        let (pitch_config, pitch_fall) = self.envelope_config(
            &modulation,
            POLY_PARAM_PITCH_ATTACK,
            POLY_PARAM_PITCH_DECAY,
            POLY_PARAM_PITCH_SUSTAIN,
            POLY_PARAM_PITCH_RELEASE,
            POLY_PARAM_PITCH_ATTACK_CURVE,
            POLY_PARAM_PITCH_FALL_CURVE,
        );
        let (filter_config, filter_fall) = self.envelope_config(
            &modulation,
            POLY_PARAM_FILTER_ATTACK,
            POLY_PARAM_FILTER_DECAY,
            POLY_PARAM_FILTER_SUSTAIN,
            POLY_PARAM_FILTER_RELEASE,
            POLY_PARAM_FILTER_ATTACK_CURVE,
            POLY_PARAM_FILTER_FALL_CURVE,
        );
        let voice_idx = self.allocate_voice();
        let voice = &mut self.voices[voice_idx];
        voice.midi_note = note;
        voice.frequency = midi_to_freq(note);
        voice.phase_a = 0.0;
        voice.phase_b = 0.0;
        voice.velocity = velocity;
        voice.modulation = modulation;
        voice.active = true;
        voice.trigger_order = self.trigger_counter;
        self.trigger_counter = self.trigger_counter.wrapping_add(1);

        voice.amp_envelope.set_config(amp_config);
        voice.amp_envelope.set_release_curve(amp_fall);
        voice.amp_envelope.trigger(time);
        voice.pitch_envelope.set_config(pitch_config);
        voice.pitch_envelope.set_release_curve(pitch_fall);
        voice.pitch_envelope.trigger(time);
        voice.filter_envelope.set_config(filter_config);
        voice.filter_envelope.set_release_curve(filter_fall);
        voice.filter_envelope.trigger(time);
        voice.filter_l.reset();
        voice.filter_r.reset();
        voice.shaper_a.reset();
        voice.shaper_b.reset();
    }

    pub fn release_note(&mut self, note: u8) {
        let time = self.current_time;
        for voice in &mut self.voices {
            if voice.active
                && voice.midi_note == note
                && voice.amp_envelope.release_time_start.is_none()
            {
                voice.amp_envelope.release(time);
                voice.pitch_envelope.release(time);
                voice.filter_envelope.release(time);
            }
        }
    }

    pub fn release_all(&mut self) {
        let time = self.current_time;
        for voice in &mut self.voices {
            if voice.active {
                voice.amp_envelope.release(time);
                voice.pitch_envelope.release(time);
                voice.filter_envelope.release(time);
            }
        }
    }

    fn allocate_voice(&self) -> usize {
        if let Some(index) = self.voices.iter().position(|voice| !voice.active) {
            return index;
        }
        self.voices
            .iter()
            .enumerate()
            .min_by_key(|(_, voice)| voice.trigger_order)
            .map(|(index, _)| index)
            .unwrap_or(0)
    }

    fn waveform(phase: f64, phase_inc: f64, position: f32) -> f32 {
        let sine = (phase * TAU).sin() as f32;
        let triangle = if phase < 0.5 {
            (4.0 * phase - 1.0) as f32
        } else {
            (3.0 - 4.0 * phase) as f32
        };
        let saw = polyblep_saw(phase, phase_inc);
        let square = polyblep_square(phase, phase_inc);
        let position = position.clamp(0.0, 1.0);
        if position <= 1.0 / 3.0 {
            let t = position * 3.0;
            sine + (triangle - sine) * t
        } else if position <= 2.0 / 3.0 {
            let t = (position - 1.0 / 3.0) * 3.0;
            triangle + (saw - triangle) * t
        } else {
            let t = (position - 2.0 / 3.0) * 3.0;
            saw + (square - saw) * t
        }
    }

    fn modulated_current(params: &PolySynthParams, voice: &Voice, param: u32) -> f32 {
        (params.current(param).unwrap() + voice.modulation[param as usize]).clamp(0.0, 1.0)
    }

    fn generate_voice(&mut self, voice_idx: usize, current_time: f64, stereo: bool) -> StereoFrame {
        let params = &self.params;
        let voice = &mut self.voices[voice_idx];
        if !voice.active {
            return StereoFrame::default();
        }

        let amp_env = voice.amp_envelope.get_amplitude(current_time);
        if !voice.amp_envelope.is_active {
            voice.active = false;
            return StereoFrame::default();
        }
        let pitch_env = voice.pitch_envelope.get_amplitude(current_time);
        let filter_env = voice.filter_envelope.get_amplitude(current_time);

        let pitch_amount = Self::modulated_current(params, voice, POLY_PARAM_PITCH_ENV_AMOUNT);
        let pitch_semitones = ranges::pitch_env_semitones(pitch_amount) * pitch_env;
        let base_frequency = voice.frequency * 2.0_f64.powf(pitch_semitones as f64 / 12.0);
        let detune = Self::modulated_current(params, voice, POLY_PARAM_DETUNE);
        let detune_half = ranges::detune_half_cents(detune);
        let frequency_a = base_frequency * 2.0_f64.powf(-detune_half / 1200.0);
        let frequency_b = base_frequency * 2.0_f64.powf(detune_half / 1200.0);
        let sample_period = 1.0 / self.sample_rate as f64;
        let phase_inc_a = frequency_a * sample_period;
        let phase_inc_b = frequency_b * sample_period;

        let waveform_a = Self::modulated_current(params, voice, POLY_PARAM_OSC_A_WAVEFORM);
        let waveform_b = Self::modulated_current(params, voice, POLY_PARAM_OSC_B_WAVEFORM);
        let level_a = Self::modulated_current(params, voice, POLY_PARAM_OSC_A_LEVEL);
        let level_b = Self::modulated_current(params, voice, POLY_PARAM_OSC_B_LEVEL);
        let raw_a = Self::waveform(voice.phase_a, phase_inc_a, waveform_a) * level_a;
        let raw_b = Self::waveform(voice.phase_b, phase_inc_b, waveform_b) * level_b;
        voice.phase_a = (voice.phase_a + phase_inc_a).fract();
        voice.phase_b = (voice.phase_b + phase_inc_b).fract();

        let saturation = Self::modulated_current(params, voice, POLY_PARAM_SATURATION);
        let drive = ranges::saturation_drive(saturation);
        let mix = ranges::saturation_mix(saturation);
        voice.shaper_a.set_drive(drive);
        voice.shaper_a.set_mix(mix);
        voice.shaper_b.set_drive(drive);
        voice.shaper_b.set_mix(mix);
        let oscillator_a = voice.shaper_a.process(raw_a);
        let oscillator_b = voice.shaper_b.process(raw_b);

        let input = if stereo {
            let width = Self::modulated_current(params, voice, POLY_PARAM_STEREO_WIDTH);
            StereoFrame::panned(oscillator_a * 0.5, 0.5 - width * 0.5)
                + StereoFrame::panned(oscillator_b * 0.5, 0.5 + width * 0.5)
        } else {
            StereoFrame::mono((oscillator_a + oscillator_b) * 0.5)
        };

        let base_cutoff = Self::modulated_current(params, voice, POLY_PARAM_FILTER_CUTOFF);
        let filter_amount =
            Self::modulated_current(params, voice, POLY_PARAM_FILTER_ENV_AMOUNT) * 2.0 - 1.0;
        let cutoff_normalized = (base_cutoff + filter_amount * filter_env).clamp(0.0, 1.0);
        let cutoff = ranges::filter_cutoff_hz(cutoff_normalized, self.sample_rate);
        let resonance = ranges::filter_resonance_q(Self::modulated_current(
            params,
            voice,
            POLY_PARAM_FILTER_RESONANCE,
        ));
        voice.filter_l.set_params(cutoff, resonance);
        voice.filter_r.set_params(cutoff, resonance);
        let (left, _, _) = voice.filter_l.process_all(input.l);
        let (right, _, _) = voice.filter_r.process_all(input.r);

        let volume = Self::modulated_current(params, voice, POLY_PARAM_VOLUME);
        StereoFrame { l: left, r: right }.scaled(amp_env * voice.velocity.sqrt() * volume)
    }

    fn render(&mut self, current_time: f64, stereo: bool) -> StereoFrame {
        self.current_time = current_time;
        self.params.tick();
        let mut output = StereoFrame::default();
        for index in 0..NUM_VOICES {
            output += self.generate_voice(index, current_time, stereo);
        }
        // Fixed headroom avoids gain jumps as voices enter and leave.
        output.scaled(0.25)
    }

    /// Render the synth's native stereo image exactly once for this sample.
    pub fn tick_frame(&mut self, current_time: f64) -> StereoFrame {
        self.render(current_time, true)
    }
}

impl Instrument for PolySynth {
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        let note = self.pending_note.unwrap_or(60);
        self.trigger_note_at(note, velocity, time);
        self.pending_note = None;
    }

    fn tick(&mut self, current_time: f64) -> f32 {
        self.render(current_time, false).l
    }

    fn tick_stereo(&mut self, current_time: f64) -> Option<StereoFrame> {
        Some(self.tick_frame(current_time))
    }

    fn is_active(&self) -> bool {
        self.voices.iter().any(|voice| voice.active)
    }

    fn set_midi_note(&mut self, note: u8) {
        self.pending_note = Some(note);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_config() -> PolySynthConfig {
        let mut config = PolySynthConfig::default();
        config.filter.cutoff = 1.0;
        config.filter.env_amount = 0.5;
        config.filter.resonance = 0.0;
        config.amp_envelope.attack = 0.0;
        config.amp_envelope.decay = 0.0;
        config.amp_envelope.sustain = 1.0;
        config.pitch_env_amount = 0.5;
        config.saturation = 0.0;
        config.mod_routes = [PolyModRoute::disabled(); POLY_MOD_ROUTE_COUNT];
        config
    }

    #[test]
    fn waveform_anchors_are_exact() {
        let phase = 0.2;
        let increment = 440.0 / 44_100.0;
        let sine = (phase * TAU).sin() as f32;
        let triangle = (4.0 * phase - 1.0) as f32;
        assert!((PolySynth::waveform(phase, increment, 0.0) - sine).abs() < 1e-6);
        assert!((PolySynth::waveform(phase, increment, 1.0 / 3.0) - triangle).abs() < 1e-6);
        assert!(
            (PolySynth::waveform(phase, increment, 2.0 / 3.0) - polyblep_saw(phase, increment))
                .abs()
                < 1e-6
        );
        assert!(
            (PolySynth::waveform(phase, increment, 1.0) - polyblep_square(phase, increment)).abs()
                < 1e-6
        );

        let epsilon = 1e-6;
        for anchor in [1.0 / 3.0, 2.0 / 3.0] {
            let left = PolySynth::waveform(phase, increment, anchor - epsilon);
            let right = PolySynth::waveform(phase, increment, anchor + epsilon);
            assert!((left - right).abs() < 1e-4, "{anchor}: {left} != {right}");
        }
    }

    #[test]
    fn normalized_ranges_pin_neutral_and_extremes() {
        assert_eq!(ranges::curve_exponent(0.0), 0.25);
        assert_eq!(ranges::curve_exponent(0.5), 1.0);
        assert_eq!(ranges::curve_exponent(1.0), 4.0);
        assert_eq!(ranges::pitch_env_semitones(0.0), -24.0);
        assert_eq!(ranges::pitch_env_semitones(0.5), 0.0);
        assert_eq!(ranges::pitch_env_semitones(1.0), 24.0);
        assert_eq!(ranges::detune_half_cents(1.0), 15.0);

        let down = 2.0_f64.powf(ranges::pitch_env_semitones(0.0) as f64 / 12.0);
        let up = 2.0_f64.powf(ranges::pitch_env_semitones(1.0) as f64 / 12.0);
        assert!((down - 0.25).abs() < 1e-12);
        assert!((up - 4.0).abs() < 1e-12);

        let detune_half = ranges::detune_half_cents(1.0);
        let ratio_a = 2.0_f64.powf(-detune_half / 1200.0);
        let ratio_b = 2.0_f64.powf(detune_half / 1200.0);
        assert!((ratio_a * ratio_b - 1.0).abs() < 1e-12);
        assert!((ratio_b / ratio_a - 2.0_f64.powf(30.0 / 1200.0)).abs() < 1e-12);
    }

    #[test]
    fn modulation_curve_and_key_scaled_depth_are_deterministic() {
        let mut config = open_config();
        config.mod_routes[0] = PolyModRoute {
            enabled: true,
            source: PolyModSource::Velocity,
            destination: POLY_PARAM_FILTER_CUTOFF,
            depth: 0.2,
            curve: 0.5,
            key_scale: -0.1,
        };
        let synth = PolySynth::with_config(44_100.0, config);
        let low = synth.resolve_modulation(0, 1.0)[POLY_PARAM_FILTER_CUTOFF as usize];
        let center = synth.resolve_modulation(60, 1.0)[POLY_PARAM_FILTER_CUTOFF as usize];
        let high = synth.resolve_modulation(120, 1.0)[POLY_PARAM_FILTER_CUTOFF as usize];
        assert!((low - 0.3).abs() < 1e-6);
        assert!((center - 0.2).abs() < 1e-6);
        assert!((high - 0.1).abs() < 1e-6);
    }

    #[test]
    fn factory_expression_is_nonlinear_velocity_and_register_aware() {
        let synth = PolySynth::new(44_100.0);
        let soft_low = synth.resolve_modulation(36, 0.25);
        let soft_high = synth.resolve_modulation(84, 0.25);
        let hard_low = synth.resolve_modulation(36, 1.0);
        let hard_high = synth.resolve_modulation(84, 1.0);

        let cutoff = POLY_PARAM_FILTER_CUTOFF as usize;
        let saturation = POLY_PARAM_SATURATION as usize;
        assert!(hard_low[cutoff] > soft_low[cutoff]);
        assert!(hard_high[cutoff] > soft_high[cutoff]);
        assert!(hard_low[saturation] > soft_low[saturation]);
        // Negative key scaling makes velocity depth stronger below C4 than
        // above it, independent of the separate key-tracking contribution.
        let low_velocity_swing = hard_low[saturation] - soft_low[saturation];
        let high_velocity_swing = hard_high[saturation] - soft_high[saturation];
        assert!(low_velocity_swing > high_velocity_swing);

        // Curve .65 has an exponent greater than one, so a halfway bipolar
        // source is deliberately smaller than its linear value.
        assert!(PolySynth::shaped_source(0.5, 0.65) < 0.5);
    }

    #[test]
    fn routes_sum_before_the_destination_is_clamped_once() {
        let mut config = open_config();
        config.filter.cutoff = 0.6;
        for slot in 0..2 {
            config.mod_routes[slot] = PolyModRoute {
                enabled: true,
                source: PolyModSource::Velocity,
                destination: POLY_PARAM_FILTER_CUTOFF,
                depth: 0.4,
                curve: 0.5,
                key_scale: 0.0,
            };
        }
        let synth = PolySynth::with_config(44_100.0, config);
        let modulation = synth.resolve_modulation(60, 1.0);
        assert!((modulation[POLY_PARAM_FILTER_CUTOFF as usize] - 0.8).abs() < 1e-6);
        assert_eq!(
            synth.modulated_target(POLY_PARAM_FILTER_CUTOFF, &modulation),
            1.0
        );
    }

    #[test]
    fn config_and_runtime_parameters_round_trip_and_reject_non_finite_values() {
        let mut config = PolySynthConfig::default();
        for param in 0..POLY_PARAM_COUNT {
            let value = param as f32 / (POLY_PARAM_COUNT - 1) as f32;
            assert!(config.set_param(param, value));
            assert!((config.param(param).unwrap() - value).abs() < 1e-6);
        }
        assert!(!config.set_param(POLY_PARAM_COUNT, 0.5));
        assert!(!config.set_param(POLY_PARAM_VOLUME, f32::NAN));

        let mut synth = PolySynth::new(44_100.0);
        assert!(synth.set_param(POLY_PARAM_VOLUME, 0.25));
        assert_eq!(synth.param(POLY_PARAM_VOLUME), Some(0.25));
        assert!(!synth.set_param(POLY_PARAM_VOLUME, f32::INFINITY));
        assert_eq!(synth.param(POLY_PARAM_VOLUME), Some(0.25));
    }

    #[test]
    fn width_zero_is_centered_and_full_width_has_side_energy() {
        let mut centered_config = open_config();
        centered_config.detune = 0.7;
        centered_config.stereo_width = 0.0;
        let mut centered = PolySynth::with_config(44_100.0, centered_config);
        centered.trigger_note(60, 1.0);

        let mut wide_config = centered_config;
        wide_config.stereo_width = 1.0;
        let mut wide = PolySynth::with_config(44_100.0, wide_config);
        wide.trigger_note(60, 1.0);

        let mut centered_difference = 0.0;
        let mut wide_difference = 0.0;
        for sample in 0..4096 {
            let time = sample as f64 / 44_100.0;
            let center_frame = centered.tick_frame(time);
            let wide_frame = wide.tick_frame(time);
            centered_difference += (center_frame.l - center_frame.r).abs();
            wide_difference += (wide_frame.l - wide_frame.r).abs();
        }
        assert!(centered_difference < 1e-4, "{centered_difference}");
        assert!(wide_difference > 0.1, "{wide_difference}");
    }

    #[test]
    fn restrained_saturation_changes_the_waveform_without_runaway_level() {
        let mut dry_config = open_config();
        dry_config.oscillator_b.level = 0.0;
        dry_config.oscillator_a.waveform = 0.0;
        dry_config.saturation = 0.0;
        let mut dry = PolySynth::with_config(44_100.0, dry_config);
        dry.trigger_note(60, 1.0);

        let mut wet_config = dry_config;
        wet_config.saturation = 1.0;
        let mut wet = PolySynth::with_config(44_100.0, wet_config);
        wet.trigger_note(60, 1.0);

        let mut difference = 0.0;
        let mut wet_peak = 0.0_f32;
        for sample in 0..4096 {
            let time = sample as f64 / 44_100.0;
            let dry_sample = dry.tick(time);
            let wet_sample = wet.tick(time);
            difference += (dry_sample - wet_sample).abs();
            wet_peak = wet_peak.max(wet_sample.abs());
        }
        assert!(difference > 0.01, "{difference}");
        assert!(wet_peak < 0.5, "{wet_peak}");
    }

    #[test]
    fn rendered_velocity_brightens_the_filter_and_adds_saturation_harmonics() {
        fn render_mono(mut synth: PolySynth, velocity: f32) -> Vec<f32> {
            synth.trigger_note(60, velocity);
            (0..4096)
                .map(|sample| synth.tick(sample as f64 / 44_100.0))
                .collect()
        }

        let mut filter_config = open_config();
        filter_config.oscillator_a.waveform = 2.0 / 3.0;
        filter_config.oscillator_b.level = 0.0;
        filter_config.filter.cutoff = 0.35;
        filter_config.mod_routes[0] = PolyModRoute {
            enabled: true,
            source: PolyModSource::Velocity,
            destination: POLY_PARAM_FILTER_CUTOFF,
            depth: 0.3,
            curve: 0.65,
            key_scale: 0.0,
        };
        let soft = render_mono(PolySynth::with_config(44_100.0, filter_config), 0.25);
        let hard = render_mono(PolySynth::with_config(44_100.0, filter_config), 1.0);
        let brightness = |samples: &[f32]| {
            let signal: f32 = samples.iter().skip(128).map(|sample| sample.abs()).sum();
            let changes: f32 = samples
                .iter()
                .skip(128)
                .zip(samples.iter().skip(129))
                .map(|(left, right)| (right - left).abs())
                .sum();
            changes / signal.max(1e-9)
        };
        assert!(brightness(&hard) > brightness(&soft));

        let mut saturation_config = open_config();
        saturation_config.oscillator_a.waveform = 0.0;
        saturation_config.oscillator_b.level = 0.0;
        saturation_config.saturation = 0.0;
        let dry_config = saturation_config;
        saturation_config.mod_routes[0] = PolyModRoute {
            enabled: true,
            source: PolyModSource::Velocity,
            destination: POLY_PARAM_SATURATION,
            depth: 0.4,
            curve: 0.65,
            key_scale: 0.0,
        };
        let soft_dry = render_mono(PolySynth::with_config(44_100.0, dry_config), 0.25);
        let soft_expressive =
            render_mono(PolySynth::with_config(44_100.0, saturation_config), 0.25);
        let hard_dry = render_mono(PolySynth::with_config(44_100.0, dry_config), 1.0);
        let hard_expressive = render_mono(PolySynth::with_config(44_100.0, saturation_config), 1.0);
        let residual = |left: &[f32], right: &[f32]| {
            left.iter()
                .zip(right)
                .map(|(left, right)| (left - right).abs())
                .sum::<f32>()
        };
        let soft_change = residual(&soft_dry, &soft_expressive);
        let hard_change = residual(&hard_dry, &hard_expressive);
        assert!(soft_change < 1e-6, "{soft_change}");
        assert!(hard_change > 0.01, "{hard_change}");
    }

    #[test]
    fn poly_synth_produces_finite_mono_audio() {
        let mut synth = PolySynth::new(44_100.0);
        synth.trigger_note(60, f32::NAN);
        for sample in 0..1024 {
            assert!(synth.tick(sample as f64 / 44_100.0).is_finite());
        }
        synth.trigger_note(60, 1.0);
        let energy: f64 = (0..4410)
            .map(|sample| synth.tick(sample as f64 / 44_100.0) as f64)
            .map(|value| value * value)
            .sum();
        assert!(energy > 0.001, "{energy}");
    }

    #[test]
    fn six_voices_and_oldest_voice_stealing_are_preserved() {
        let mut synth = PolySynth::new(44_100.0);
        for note in 60..66 {
            synth.trigger_note(note, 1.0);
        }
        assert_eq!(synth.voices.iter().filter(|voice| voice.active).count(), 6);
        synth.trigger_note(66, 1.0);
        assert_eq!(synth.voices.iter().filter(|voice| voice.active).count(), 6);
        assert!(synth.voices.iter().any(|voice| voice.midi_note == 66));
        assert!(!synth.voices.iter().any(|voice| voice.midi_note == 60));
    }

    #[test]
    fn presets_construct_with_valid_parameters_and_routes() {
        for (config, expected_width) in [
            (PolySynthConfig::default(), 0.45),
            (PolySynthConfig::pad(), 0.80),
            (PolySynthConfig::pluck(), 0.25),
            (PolySynthConfig::keys(), 0.35),
            (PolySynthConfig::strings(), 0.75),
        ] {
            for param in 0..POLY_PARAM_COUNT {
                assert!((0.0..=1.0).contains(&config.param(param).unwrap()));
            }
            assert!((config.stereo_width - expected_width).abs() < 1e-6);
            assert!((config.saturation - 0.08).abs() < 1e-6);
            assert_eq!(
                config
                    .mod_routes
                    .iter()
                    .filter(|route| route.enabled)
                    .count(),
                3
            );
            assert_eq!(config.mod_routes[0].source, PolyModSource::Velocity);
            assert_eq!(config.mod_routes[0].destination, POLY_PARAM_FILTER_CUTOFF);
            assert_eq!(config.mod_routes[1].source, PolyModSource::KeyPosition);
            assert_eq!(config.mod_routes[1].destination, POLY_PARAM_FILTER_CUTOFF);
            assert_eq!(config.mod_routes[2].source, PolyModSource::Velocity);
            assert_eq!(config.mod_routes[2].destination, POLY_PARAM_SATURATION);
        }
    }
}
