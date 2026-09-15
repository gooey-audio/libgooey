//! Monophonic two-oscillator FM percussion synthesizer.
//!
//! Host-facing parameters are normalized to `0.0..=1.0`. Signed controls use
//! `0.5` as their neutral value. The voice is rendered at 2x the host sample
//! rate and half-band downsampled for lower aliasing from FM, ring modulation,
//! and the dirty output stages.

use std::f32::consts::TAU;

use halfband::iir::Downsampler8;

use crate::engine::{Instrument, Modulatable};
use crate::filters::StateVariableFilterTpt;
use crate::gen::polyblep::polyblep_square;
#[cfg(test)]
use crate::music::note::midi_to_freq;
use crate::utils::{Blendable, SmoothedParam, DEFAULT_SMOOTH_TIME_MS};

pub const FM_PARAM_BASE_PITCH: u32 = 0;
pub const FM_PARAM_OSC1_WAVEFORM: u32 = 1;
pub const FM_PARAM_OSC1_FREQUENCY: u32 = 2;
pub const FM_PARAM_OSC1_TRACKING: u32 = 3;
pub const FM_PARAM_OSC1_LEVEL: u32 = 4;
pub const FM_PARAM_OSC1_DROP: u32 = 5;
pub const FM_PARAM_OSC1_SLOPE: u32 = 6;
pub const FM_PARAM_OSC2_WAVEFORM: u32 = 7;
pub const FM_PARAM_OSC2_FREQUENCY: u32 = 8;
pub const FM_PARAM_OSC2_TRACKING: u32 = 9;
pub const FM_PARAM_OSC2_LEVEL: u32 = 10;
pub const FM_PARAM_OSC2_DROP: u32 = 11;
pub const FM_PARAM_OSC2_SLOPE: u32 = 12;
pub const FM_PARAM_NOISE_LEVEL: u32 = 13;
pub const FM_PARAM_NOISE_DECAY: u32 = 14;
pub const FM_PARAM_INDEX: u32 = 15;
pub const FM_PARAM_RING_MODE: u32 = 16;
pub const FM_PARAM_FILTER_MODE: u32 = 17;
pub const FM_PARAM_FILTER_CUTOFF: u32 = 18;
pub const FM_PARAM_FILTER_DECAY: u32 = 19;
pub const FM_PARAM_FILTER_ENV_AMOUNT: u32 = 20;
pub const FM_PARAM_GRIT: u32 = 21;
pub const FM_PARAM_AMP_ATTACK: u32 = 22;
pub const FM_PARAM_AMP_DECAY: u32 = 23;
pub const FM_PARAM_FREQUENCY_BOOST: u32 = 24;
pub const FM_PARAM_BIT_DRIVE: u32 = 25;
pub const FM_PARAM_VOLUME: u32 = 26;
pub const FM_PARAM_TUNING: u32 = 27;
pub const FM_PARAM_PITCH_TO_DROP: u32 = 28;
pub const FM_PARAM_PITCH_TO_FM: u32 = 29;
pub const FM_PARAM_PITCH_TO_NOISE: u32 = 30;
pub const FM_PARAM_PITCH_TO_BALANCE: u32 = 31;
pub const FM_PARAM_PITCH_TO_CUTOFF: u32 = 32;
pub const FM_PARAM_PITCH_TO_LEVEL: u32 = 33;
pub const FM_PARAM_VELOCITY_TO_DROP: u32 = 34;
pub const FM_PARAM_VELOCITY_TO_FM: u32 = 35;
pub const FM_PARAM_VELOCITY_TO_NOISE: u32 = 36;
pub const FM_PARAM_VELOCITY_TO_BALANCE: u32 = 37;
pub const FM_PARAM_VELOCITY_TO_CUTOFF: u32 = 38;
pub const FM_PARAM_VELOCITY_TO_LEVEL: u32 = 39;
pub const FM_PARAM_COUNT: u32 = 40;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum FmWaveform {
    #[default]
    Sine = 0,
    Triangle = 1,
    Square = 2,
    Metal = 3,
}

impl FmWaveform {
    pub fn from_normalized(value: f32) -> Self {
        match (value.clamp(0.0, 1.0) * 3.0).round() as u32 {
            0 => Self::Sine,
            1 => Self::Triangle,
            2 => Self::Square,
            _ => Self::Metal,
        }
    }

    pub const fn normalized(self) -> f32 {
        self as u32 as f32 / 3.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum FmRingMode {
    #[default]
    Off = 0,
    Ring = 1,
    CrossRing = 2,
}

impl FmRingMode {
    pub fn from_normalized(value: f32) -> Self {
        match (value.clamp(0.0, 1.0) * 2.0).round() as u32 {
            0 => Self::Off,
            1 => Self::Ring,
            _ => Self::CrossRing,
        }
    }

    pub const fn normalized(self) -> f32 {
        self as u32 as f32 / 2.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum FmFilterMode {
    #[default]
    Lowpass = 0,
    Highpass = 1,
}

impl FmFilterMode {
    pub fn from_normalized(value: f32) -> Self {
        if value >= 0.5 {
            Self::Highpass
        } else {
            Self::Lowpass
        }
    }

    pub const fn normalized(self) -> f32 {
        self as u32 as f32
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FmMacroDepths {
    pub drop: f32,
    pub fm: f32,
    pub noise: f32,
    pub balance: f32,
    pub cutoff: f32,
    pub level: f32,
}

impl FmMacroDepths {
    pub const fn neutral() -> Self {
        Self {
            drop: 0.5,
            fm: 0.5,
            noise: 0.5,
            balance: 0.5,
            cutoff: 0.5,
            level: 0.5,
        }
    }

    fn clamped(mut self) -> Self {
        self.drop = normalized_or(self.drop, 0.5);
        self.fm = normalized_or(self.fm, 0.5);
        self.noise = normalized_or(self.noise, 0.5);
        self.balance = normalized_or(self.balance, 0.5);
        self.cutoff = normalized_or(self.cutoff, 0.5);
        self.level = normalized_or(self.level, 0.5);
        self
    }
}

impl Default for FmMacroDepths {
    fn default() -> Self {
        Self::neutral()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FmPercussionConfig {
    pub base_pitch: f32,
    pub osc1_waveform: FmWaveform,
    pub osc1_frequency: f32,
    pub osc1_tracking: bool,
    pub osc1_level: f32,
    pub osc1_drop: f32,
    pub osc1_slope: f32,
    pub osc2_waveform: FmWaveform,
    pub osc2_frequency: f32,
    pub osc2_tracking: bool,
    pub osc2_level: f32,
    pub osc2_drop: f32,
    pub osc2_slope: f32,
    pub noise_level: f32,
    pub noise_decay: f32,
    pub fm_index: f32,
    pub ring_mode: FmRingMode,
    pub filter_mode: FmFilterMode,
    pub filter_cutoff: f32,
    pub filter_decay: f32,
    pub filter_env_amount: f32,
    pub grit: f32,
    pub amp_attack: f32,
    pub amp_decay: f32,
    pub frequency_boost: f32,
    pub bit_drive: f32,
    pub volume: f32,
    pub tuning: f32,
    pub pitch_mod: FmMacroDepths,
    pub velocity_mod: FmMacroDepths,
}

impl FmPercussionConfig {
    pub fn sub_kick() -> Self {
        Self {
            base_pitch: 0.17,
            osc1_waveform: FmWaveform::Sine,
            osc1_frequency: 0.48,
            osc1_tracking: true,
            osc1_level: 0.28,
            osc1_drop: 0.67,
            osc1_slope: 0.20,
            osc2_waveform: FmWaveform::Sine,
            osc2_frequency: 0.50,
            osc2_tracking: true,
            osc2_level: 0.88,
            osc2_drop: 0.78,
            osc2_slope: 0.23,
            noise_level: 0.025,
            noise_decay: 0.10,
            fm_index: 0.14,
            ring_mode: FmRingMode::Off,
            filter_mode: FmFilterMode::Lowpass,
            filter_cutoff: 0.72,
            filter_decay: 0.18,
            filter_env_amount: 0.35,
            grit: 0.08,
            amp_attack: 0.0,
            amp_decay: 0.42,
            frequency_boost: 0.12,
            bit_drive: 0.04,
            volume: 0.80,
            tuning: 0.5,
            pitch_mod: FmMacroDepths::neutral(),
            velocity_mod: FmMacroDepths {
                level: 1.0,
                fm: 0.62,
                ..FmMacroDepths::neutral()
            },
        }
    }

    pub fn metal_hat() -> Self {
        Self {
            base_pitch: 0.72,
            osc1_waveform: FmWaveform::Metal,
            osc1_frequency: 0.58,
            osc1_tracking: false,
            osc1_level: 0.42,
            osc1_drop: 0.50,
            osc1_slope: 0.05,
            osc2_waveform: FmWaveform::Square,
            osc2_frequency: 0.65,
            osc2_tracking: false,
            osc2_level: 0.58,
            osc2_drop: 0.53,
            osc2_slope: 0.06,
            noise_level: 0.34,
            noise_decay: 0.16,
            fm_index: 0.44,
            ring_mode: FmRingMode::CrossRing,
            filter_mode: FmFilterMode::Highpass,
            filter_cutoff: 0.68,
            filter_decay: 0.12,
            filter_env_amount: 0.20,
            grit: 0.24,
            amp_attack: 0.0,
            amp_decay: 0.15,
            frequency_boost: 0.68,
            bit_drive: 0.22,
            volume: 0.62,
            tuning: 0.5,
            pitch_mod: FmMacroDepths {
                cutoff: 0.68,
                ..FmMacroDepths::neutral()
            },
            velocity_mod: FmMacroDepths {
                level: 1.0,
                noise: 0.68,
                ..FmMacroDepths::neutral()
            },
        }
    }

    pub fn zap() -> Self {
        Self {
            base_pitch: 0.42,
            osc1_waveform: FmWaveform::Triangle,
            osc1_frequency: 0.36,
            osc1_tracking: true,
            osc1_level: 0.20,
            osc1_drop: 0.82,
            osc1_slope: 0.12,
            osc2_waveform: FmWaveform::Sine,
            osc2_frequency: 0.61,
            osc2_tracking: true,
            osc2_level: 0.82,
            osc2_drop: 0.90,
            osc2_slope: 0.18,
            noise_level: 0.04,
            noise_decay: 0.07,
            fm_index: 0.53,
            ring_mode: FmRingMode::Off,
            filter_mode: FmFilterMode::Lowpass,
            filter_cutoff: 0.83,
            filter_decay: 0.16,
            filter_env_amount: 0.50,
            grit: 0.12,
            amp_attack: 0.0,
            amp_decay: 0.25,
            frequency_boost: 0.48,
            bit_drive: 0.15,
            volume: 0.68,
            tuning: 0.5,
            pitch_mod: FmMacroDepths {
                fm: 0.70,
                balance: 0.66,
                ..FmMacroDepths::neutral()
            },
            velocity_mod: FmMacroDepths {
                level: 1.0,
                drop: 0.68,
                ..FmMacroDepths::neutral()
            },
        }
    }

    pub fn industrial() -> Self {
        Self {
            base_pitch: 0.30,
            osc1_waveform: FmWaveform::Square,
            osc1_frequency: 0.26,
            osc1_tracking: true,
            osc1_level: 0.62,
            osc1_drop: 0.61,
            osc1_slope: 0.30,
            osc2_waveform: FmWaveform::Metal,
            osc2_frequency: 0.72,
            osc2_tracking: false,
            osc2_level: 0.68,
            osc2_drop: 0.42,
            osc2_slope: 0.34,
            noise_level: 0.20,
            noise_decay: 0.35,
            fm_index: 0.68,
            ring_mode: FmRingMode::Ring,
            filter_mode: FmFilterMode::Highpass,
            filter_cutoff: 0.38,
            filter_decay: 0.42,
            filter_env_amount: 0.62,
            grit: 0.56,
            amp_attack: 0.02,
            amp_decay: 0.52,
            frequency_boost: 0.34,
            bit_drive: 0.58,
            volume: 0.58,
            tuning: 0.5,
            pitch_mod: FmMacroDepths {
                drop: 0.30,
                fm: 0.78,
                cutoff: 0.30,
                ..FmMacroDepths::neutral()
            },
            velocity_mod: FmMacroDepths {
                level: 1.0,
                noise: 0.76,
                balance: 0.28,
                ..FmMacroDepths::neutral()
            },
        }
    }

    fn clamped(mut self) -> Self {
        macro_rules! clamp_fields {
            ($($field:ident),+ $(,)?) => {
                $(self.$field = normalized_or(self.$field, 0.5);)+
            };
        }
        clamp_fields!(
            base_pitch,
            osc1_frequency,
            osc1_level,
            osc1_drop,
            osc1_slope,
            osc2_frequency,
            osc2_level,
            osc2_drop,
            osc2_slope,
            noise_level,
            noise_decay,
            fm_index,
            filter_cutoff,
            filter_decay,
            filter_env_amount,
            grit,
            amp_attack,
            amp_decay,
            frequency_boost,
            bit_drive,
            volume,
            tuning
        );
        self.pitch_mod = self.pitch_mod.clamped();
        self.velocity_mod = self.velocity_mod.clamped();
        self
    }
}

impl Default for FmPercussionConfig {
    fn default() -> Self {
        Self::sub_kick()
    }
}

impl Blendable for FmMacroDepths {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let a = 1.0 - t;
        Self {
            drop: self.drop * a + other.drop * t,
            fm: self.fm * a + other.fm * t,
            noise: self.noise * a + other.noise * t,
            balance: self.balance * a + other.balance * t,
            cutoff: self.cutoff * a + other.cutoff * t,
            level: self.level * a + other.level * t,
        }
    }
}

impl Blendable for FmPercussionConfig {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let a = 1.0 - t;
        macro_rules! blend {
            ($field:ident) => {
                self.$field * a + other.$field * t
            };
        }
        Self {
            base_pitch: blend!(base_pitch),
            osc1_waveform: if t < 0.5 {
                self.osc1_waveform
            } else {
                other.osc1_waveform
            },
            osc1_frequency: blend!(osc1_frequency),
            osc1_tracking: if t < 0.5 {
                self.osc1_tracking
            } else {
                other.osc1_tracking
            },
            osc1_level: blend!(osc1_level),
            osc1_drop: blend!(osc1_drop),
            osc1_slope: blend!(osc1_slope),
            osc2_waveform: if t < 0.5 {
                self.osc2_waveform
            } else {
                other.osc2_waveform
            },
            osc2_frequency: blend!(osc2_frequency),
            osc2_tracking: if t < 0.5 {
                self.osc2_tracking
            } else {
                other.osc2_tracking
            },
            osc2_level: blend!(osc2_level),
            osc2_drop: blend!(osc2_drop),
            osc2_slope: blend!(osc2_slope),
            noise_level: blend!(noise_level),
            noise_decay: blend!(noise_decay),
            fm_index: blend!(fm_index),
            ring_mode: if t < 0.5 {
                self.ring_mode
            } else {
                other.ring_mode
            },
            filter_mode: if t < 0.5 {
                self.filter_mode
            } else {
                other.filter_mode
            },
            filter_cutoff: blend!(filter_cutoff),
            filter_decay: blend!(filter_decay),
            filter_env_amount: blend!(filter_env_amount),
            grit: blend!(grit),
            amp_attack: blend!(amp_attack),
            amp_decay: blend!(amp_decay),
            frequency_boost: blend!(frequency_boost),
            bit_drive: blend!(bit_drive),
            volume: blend!(volume),
            tuning: blend!(tuning),
            pitch_mod: self.pitch_mod.lerp(&other.pitch_mod, t),
            velocity_mod: self.velocity_mod.lerp(&other.velocity_mod, t),
        }
    }
}

#[inline]
fn normalized_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        fallback
    }
}

mod ranges {
    #[inline]
    pub fn midi_from_base_pitch(value: f32) -> f32 {
        24.0 + value.clamp(0.0, 1.0) * 72.0
    }

    #[inline]
    pub fn tracked_offset_semitones(value: f32) -> f32 {
        (value.clamp(0.0, 1.0) - 0.5) * 48.0
    }

    #[inline]
    pub fn fixed_frequency(value: f32) -> f32 {
        20.0 * (12_000.0_f32 / 20.0).powf(value.clamp(0.0, 1.0))
    }

    #[inline]
    pub fn drop_semitones(value: f32) -> f32 {
        (value.clamp(0.0, 1.0) - 0.5) * 120.0
    }

    #[inline]
    pub fn pitch_slope_seconds(value: f32) -> f32 {
        0.001 * 1_500.0_f32.powf(value.clamp(0.0, 1.0))
    }

    #[inline]
    pub fn short_decay_seconds(value: f32) -> f32 {
        0.001 * 2_000.0_f32.powf(value.clamp(0.0, 1.0))
    }

    #[inline]
    pub fn amp_attack_seconds(value: f32) -> f32 {
        0.0001 * 1_000.0_f32.powf(value.clamp(0.0, 1.0))
    }

    #[inline]
    pub fn amp_decay_seconds(value: f32) -> f32 {
        0.005 * 800.0_f32.powf(value.clamp(0.0, 1.0))
    }

    #[inline]
    pub fn cutoff_hz(value: f32, sample_rate: f32) -> f32 {
        let max = (sample_rate * 0.45).max(20.0);
        20.0 * (max / 20.0).powf(value.clamp(0.0, 1.0))
    }

    #[inline]
    pub fn fm_index(value: f32) -> f32 {
        let normalized = value.clamp(0.0, 1.0);
        20.0 * (5.0 * normalized).exp_m1() / 5.0_f32.exp_m1()
    }

    #[inline]
    pub fn boost_hz(value: f32, sample_rate: f32) -> f32 {
        let max = 8_000.0_f32.min(sample_rate * 0.4);
        40.0 * (max / 40.0).powf(value.clamp(0.0, 1.0))
    }
}

#[derive(Clone, Copy)]
struct OneShotEnvelope {
    trigger_time: f64,
    attack_secs: f32,
    decay_secs: f32,
    attack_curve: f32,
    decay_curve: f32,
    active: bool,
}

impl OneShotEnvelope {
    fn new() -> Self {
        Self {
            trigger_time: 0.0,
            attack_secs: 0.0001,
            decay_secs: 0.1,
            attack_curve: 0.3,
            decay_curve: 0.3,
            active: false,
        }
    }

    fn trigger(&mut self, time: f64, attack_secs: f32, decay_secs: f32) {
        self.trigger_time = time;
        self.attack_secs = attack_secs.max(0.0);
        self.decay_secs = decay_secs.max(1e-6);
        self.active = true;
    }

    fn reset(&mut self) {
        self.active = false;
        self.trigger_time = 0.0;
    }

    #[inline]
    fn value(&mut self, time: f64) -> f32 {
        if !self.active {
            return 0.0;
        }
        let elapsed = (time - self.trigger_time).max(0.0) as f32;
        if self.attack_secs > 0.0 && elapsed < self.attack_secs {
            return (elapsed / self.attack_secs).powf(self.attack_curve);
        }
        let decay_elapsed = elapsed - self.attack_secs;
        if decay_elapsed >= self.decay_secs {
            self.active = false;
            return 0.0;
        }
        1.0 - (decay_elapsed / self.decay_secs).powf(self.decay_curve)
    }
}

const METAL_RATIOS: [f64; 6] = [1.0, 1.342, 1.487, 1.654, 1.877, 2.113];

struct PhaseOscillator {
    phase: f64,
    metal_phases: [f64; 6],
}

impl PhaseOscillator {
    fn new() -> Self {
        Self {
            phase: 0.0,
            metal_phases: [0.0; 6],
        }
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.metal_phases = [0.0; 6];
    }

    #[inline]
    fn basic_sample(waveform: FmWaveform, phase: f64, phase_inc: f64) -> f32 {
        let phase = phase - phase.floor();
        match waveform {
            FmWaveform::Sine => (phase * std::f64::consts::TAU).sin() as f32,
            FmWaveform::Triangle => {
                let p = phase as f32;
                if p < 0.5 {
                    4.0 * p - 1.0
                } else {
                    3.0 - 4.0 * p
                }
            }
            FmWaveform::Square | FmWaveform::Metal => polyblep_square(phase, phase_inc),
        }
    }

    #[inline]
    fn sample(
        &self,
        waveform: FmWaveform,
        phase_offset_cycles: f64,
        frequency: f32,
        sample_rate: f32,
    ) -> f32 {
        let phase_inc = (frequency as f64 / sample_rate as f64).clamp(0.0, 0.45);
        if waveform != FmWaveform::Metal {
            return Self::basic_sample(waveform, self.phase + phase_offset_cycles, phase_inc);
        }
        let mut sum = 0.0;
        for (phase, ratio) in self.metal_phases.iter().zip(METAL_RATIOS) {
            let inc = (phase_inc * ratio).min(0.45);
            sum += polyblep_square(*phase + phase_offset_cycles, inc);
        }
        sum / METAL_RATIOS.len() as f32
    }

    #[inline]
    fn advance(&mut self, frequency: f32, sample_rate: f32) {
        let increment = (frequency as f64 / sample_rate as f64).clamp(0.0, 0.45);
        self.phase = (self.phase + increment).fract();
        for (phase, ratio) in self.metal_phases.iter_mut().zip(METAL_RATIOS) {
            *phase = (*phase + (increment * ratio).min(0.45)).fract();
        }
    }
}

const CONTINUOUS_PARAMS: [u32; 34] = [
    FM_PARAM_BASE_PITCH,
    FM_PARAM_OSC1_FREQUENCY,
    FM_PARAM_OSC1_LEVEL,
    FM_PARAM_OSC1_DROP,
    FM_PARAM_OSC1_SLOPE,
    FM_PARAM_OSC2_FREQUENCY,
    FM_PARAM_OSC2_LEVEL,
    FM_PARAM_OSC2_DROP,
    FM_PARAM_OSC2_SLOPE,
    FM_PARAM_NOISE_LEVEL,
    FM_PARAM_NOISE_DECAY,
    FM_PARAM_INDEX,
    FM_PARAM_FILTER_CUTOFF,
    FM_PARAM_FILTER_DECAY,
    FM_PARAM_FILTER_ENV_AMOUNT,
    FM_PARAM_GRIT,
    FM_PARAM_AMP_ATTACK,
    FM_PARAM_AMP_DECAY,
    FM_PARAM_FREQUENCY_BOOST,
    FM_PARAM_BIT_DRIVE,
    FM_PARAM_VOLUME,
    FM_PARAM_TUNING,
    FM_PARAM_PITCH_TO_DROP,
    FM_PARAM_PITCH_TO_FM,
    FM_PARAM_PITCH_TO_NOISE,
    FM_PARAM_PITCH_TO_BALANCE,
    FM_PARAM_PITCH_TO_CUTOFF,
    FM_PARAM_PITCH_TO_LEVEL,
    FM_PARAM_VELOCITY_TO_DROP,
    FM_PARAM_VELOCITY_TO_FM,
    FM_PARAM_VELOCITY_TO_NOISE,
    FM_PARAM_VELOCITY_TO_BALANCE,
    FM_PARAM_VELOCITY_TO_CUTOFF,
    FM_PARAM_VELOCITY_TO_LEVEL,
];

const PARAMETER_NAMES: [&str; FM_PARAM_COUNT as usize] = [
    "base_pitch",
    "osc1_waveform",
    "osc1_frequency",
    "osc1_tracking",
    "osc1_level",
    "osc1_drop",
    "osc1_slope",
    "osc2_waveform",
    "osc2_frequency",
    "osc2_tracking",
    "osc2_level",
    "osc2_drop",
    "osc2_slope",
    "noise_level",
    "noise_decay",
    "fm_index",
    "ring_mode",
    "filter_mode",
    "filter_cutoff",
    "filter_decay",
    "filter_env_amount",
    "grit",
    "amp_attack",
    "amp_decay",
    "frequency_boost",
    "bit_drive",
    "volume",
    "tuning",
    "pitch_to_drop",
    "pitch_to_fm",
    "pitch_to_noise",
    "pitch_to_balance",
    "pitch_to_cutoff",
    "pitch_to_level",
    "velocity_to_drop",
    "velocity_to_fm",
    "velocity_to_noise",
    "velocity_to_balance",
    "velocity_to_cutoff",
    "velocity_to_level",
];

fn config_param(config: &FmPercussionConfig, param: u32) -> f32 {
    match param {
        FM_PARAM_BASE_PITCH => config.base_pitch,
        FM_PARAM_OSC1_WAVEFORM => config.osc1_waveform.normalized(),
        FM_PARAM_OSC1_FREQUENCY => config.osc1_frequency,
        FM_PARAM_OSC1_TRACKING => config.osc1_tracking as u8 as f32,
        FM_PARAM_OSC1_LEVEL => config.osc1_level,
        FM_PARAM_OSC1_DROP => config.osc1_drop,
        FM_PARAM_OSC1_SLOPE => config.osc1_slope,
        FM_PARAM_OSC2_WAVEFORM => config.osc2_waveform.normalized(),
        FM_PARAM_OSC2_FREQUENCY => config.osc2_frequency,
        FM_PARAM_OSC2_TRACKING => config.osc2_tracking as u8 as f32,
        FM_PARAM_OSC2_LEVEL => config.osc2_level,
        FM_PARAM_OSC2_DROP => config.osc2_drop,
        FM_PARAM_OSC2_SLOPE => config.osc2_slope,
        FM_PARAM_NOISE_LEVEL => config.noise_level,
        FM_PARAM_NOISE_DECAY => config.noise_decay,
        FM_PARAM_INDEX => config.fm_index,
        FM_PARAM_RING_MODE => config.ring_mode.normalized(),
        FM_PARAM_FILTER_MODE => config.filter_mode.normalized(),
        FM_PARAM_FILTER_CUTOFF => config.filter_cutoff,
        FM_PARAM_FILTER_DECAY => config.filter_decay,
        FM_PARAM_FILTER_ENV_AMOUNT => config.filter_env_amount,
        FM_PARAM_GRIT => config.grit,
        FM_PARAM_AMP_ATTACK => config.amp_attack,
        FM_PARAM_AMP_DECAY => config.amp_decay,
        FM_PARAM_FREQUENCY_BOOST => config.frequency_boost,
        FM_PARAM_BIT_DRIVE => config.bit_drive,
        FM_PARAM_VOLUME => config.volume,
        FM_PARAM_TUNING => config.tuning,
        FM_PARAM_PITCH_TO_DROP => config.pitch_mod.drop,
        FM_PARAM_PITCH_TO_FM => config.pitch_mod.fm,
        FM_PARAM_PITCH_TO_NOISE => config.pitch_mod.noise,
        FM_PARAM_PITCH_TO_BALANCE => config.pitch_mod.balance,
        FM_PARAM_PITCH_TO_CUTOFF => config.pitch_mod.cutoff,
        FM_PARAM_PITCH_TO_LEVEL => config.pitch_mod.level,
        FM_PARAM_VELOCITY_TO_DROP => config.velocity_mod.drop,
        FM_PARAM_VELOCITY_TO_FM => config.velocity_mod.fm,
        FM_PARAM_VELOCITY_TO_NOISE => config.velocity_mod.noise,
        FM_PARAM_VELOCITY_TO_BALANCE => config.velocity_mod.balance,
        FM_PARAM_VELOCITY_TO_CUTOFF => config.velocity_mod.cutoff,
        FM_PARAM_VELOCITY_TO_LEVEL => config.velocity_mod.level,
        _ => f32::NAN,
    }
}

/// Smoothed normalized values used by the real-time voice.
pub struct FmPercussionParams {
    values: [SmoothedParam; FM_PARAM_COUNT as usize],
}

impl FmPercussionParams {
    fn new(config: FmPercussionConfig, sample_rate: f32) -> Self {
        Self {
            values: std::array::from_fn(|index| {
                SmoothedParam::new(
                    config_param(&config, index as u32),
                    0.0,
                    1.0,
                    sample_rate,
                    DEFAULT_SMOOTH_TIME_MS,
                )
            }),
        }
    }

    fn tick(&mut self) -> [f32; FM_PARAM_COUNT as usize] {
        std::array::from_fn(|index| self.values[index].tick())
    }

    fn target(&self, param: u32) -> f32 {
        self.values[param as usize].target()
    }

    fn set_target(&mut self, param: u32, value: f32) {
        self.values[param as usize].set_target(value);
    }

    fn snap(&mut self) {
        for value in &mut self.values {
            value.snap();
        }
    }
}

/// A monophonic, oversampled FM percussion voice.
pub struct FmPercussion {
    sample_rate: f32,
    synth_sample_rate: f32,
    params: FmPercussionParams,
    osc1_waveform: FmWaveform,
    osc1_tracking: bool,
    osc2_waveform: FmWaveform,
    osc2_tracking: bool,
    ring_mode: FmRingMode,
    filter_mode: FmFilterMode,
    osc1: PhaseOscillator,
    osc2: PhaseOscillator,
    pitch1_envelope: OneShotEnvelope,
    pitch2_envelope: OneShotEnvelope,
    noise_envelope: OneShotEnvelope,
    filter_envelope: OneShotEnvelope,
    amplitude_envelope: OneShotEnvelope,
    filter: StateVariableFilterTpt,
    boost_filter: StateVariableFilterTpt,
    downsampler: Downsampler8,
    rng_state: u64,
    previous_osc1: f32,
    previous_osc2: f32,
    staged_note: Option<u8>,
    current_note: u8,
    current_velocity: f32,
    triggered_drop1: f32,
    triggered_drop2: f32,
    active: bool,
}

impl FmPercussion {
    pub fn new(sample_rate: f32) -> Self {
        Self::with_config(sample_rate, FmPercussionConfig::default())
    }

    pub fn with_config(sample_rate: f32, config: FmPercussionConfig) -> Self {
        let sample_rate = if sample_rate.is_finite() {
            sample_rate.clamp(8_000.0, 384_000.0)
        } else {
            48_000.0
        };
        let synth_sample_rate = sample_rate * 2.0;
        let config = config.clamped();
        let current_note = ranges::midi_from_base_pitch(config.base_pitch).round() as u8;
        Self {
            sample_rate,
            synth_sample_rate,
            params: FmPercussionParams::new(config, sample_rate),
            osc1_waveform: config.osc1_waveform,
            osc1_tracking: config.osc1_tracking,
            osc2_waveform: config.osc2_waveform,
            osc2_tracking: config.osc2_tracking,
            ring_mode: config.ring_mode,
            filter_mode: config.filter_mode,
            osc1: PhaseOscillator::new(),
            osc2: PhaseOscillator::new(),
            pitch1_envelope: OneShotEnvelope::new(),
            pitch2_envelope: OneShotEnvelope::new(),
            noise_envelope: OneShotEnvelope::new(),
            filter_envelope: OneShotEnvelope::new(),
            amplitude_envelope: OneShotEnvelope::new(),
            filter: StateVariableFilterTpt::new(synth_sample_rate, 8_000.0, 0.707),
            boost_filter: StateVariableFilterTpt::new(synth_sample_rate, 100.0, 1.0),
            downsampler: Downsampler8::default(),
            rng_state: 0x9e37_79b9_7f4a_7c15,
            previous_osc1: 0.0,
            previous_osc2: 0.0,
            staged_note: None,
            current_note,
            current_velocity: 1.0,
            triggered_drop1: 0.0,
            triggered_drop2: 0.0,
            active: false,
        }
    }

    pub fn set_config(&mut self, config: FmPercussionConfig) {
        let config = config.clamped();
        self.osc1_waveform = config.osc1_waveform;
        self.osc1_tracking = config.osc1_tracking;
        self.osc2_waveform = config.osc2_waveform;
        self.osc2_tracking = config.osc2_tracking;
        self.ring_mode = config.ring_mode;
        self.filter_mode = config.filter_mode;
        for param in CONTINUOUS_PARAMS {
            self.params.set_target(param, config_param(&config, param));
        }
    }

    pub fn config(&self) -> FmPercussionConfig {
        FmPercussionConfig {
            base_pitch: self.get_param(FM_PARAM_BASE_PITCH).unwrap(),
            osc1_waveform: self.osc1_waveform,
            osc1_frequency: self.get_param(FM_PARAM_OSC1_FREQUENCY).unwrap(),
            osc1_tracking: self.osc1_tracking,
            osc1_level: self.get_param(FM_PARAM_OSC1_LEVEL).unwrap(),
            osc1_drop: self.get_param(FM_PARAM_OSC1_DROP).unwrap(),
            osc1_slope: self.get_param(FM_PARAM_OSC1_SLOPE).unwrap(),
            osc2_waveform: self.osc2_waveform,
            osc2_frequency: self.get_param(FM_PARAM_OSC2_FREQUENCY).unwrap(),
            osc2_tracking: self.osc2_tracking,
            osc2_level: self.get_param(FM_PARAM_OSC2_LEVEL).unwrap(),
            osc2_drop: self.get_param(FM_PARAM_OSC2_DROP).unwrap(),
            osc2_slope: self.get_param(FM_PARAM_OSC2_SLOPE).unwrap(),
            noise_level: self.get_param(FM_PARAM_NOISE_LEVEL).unwrap(),
            noise_decay: self.get_param(FM_PARAM_NOISE_DECAY).unwrap(),
            fm_index: self.get_param(FM_PARAM_INDEX).unwrap(),
            ring_mode: self.ring_mode,
            filter_mode: self.filter_mode,
            filter_cutoff: self.get_param(FM_PARAM_FILTER_CUTOFF).unwrap(),
            filter_decay: self.get_param(FM_PARAM_FILTER_DECAY).unwrap(),
            filter_env_amount: self.get_param(FM_PARAM_FILTER_ENV_AMOUNT).unwrap(),
            grit: self.get_param(FM_PARAM_GRIT).unwrap(),
            amp_attack: self.get_param(FM_PARAM_AMP_ATTACK).unwrap(),
            amp_decay: self.get_param(FM_PARAM_AMP_DECAY).unwrap(),
            frequency_boost: self.get_param(FM_PARAM_FREQUENCY_BOOST).unwrap(),
            bit_drive: self.get_param(FM_PARAM_BIT_DRIVE).unwrap(),
            volume: self.get_param(FM_PARAM_VOLUME).unwrap(),
            tuning: self.get_param(FM_PARAM_TUNING).unwrap(),
            pitch_mod: FmMacroDepths {
                drop: self.get_param(FM_PARAM_PITCH_TO_DROP).unwrap(),
                fm: self.get_param(FM_PARAM_PITCH_TO_FM).unwrap(),
                noise: self.get_param(FM_PARAM_PITCH_TO_NOISE).unwrap(),
                balance: self.get_param(FM_PARAM_PITCH_TO_BALANCE).unwrap(),
                cutoff: self.get_param(FM_PARAM_PITCH_TO_CUTOFF).unwrap(),
                level: self.get_param(FM_PARAM_PITCH_TO_LEVEL).unwrap(),
            },
            velocity_mod: FmMacroDepths {
                drop: self.get_param(FM_PARAM_VELOCITY_TO_DROP).unwrap(),
                fm: self.get_param(FM_PARAM_VELOCITY_TO_FM).unwrap(),
                noise: self.get_param(FM_PARAM_VELOCITY_TO_NOISE).unwrap(),
                balance: self.get_param(FM_PARAM_VELOCITY_TO_BALANCE).unwrap(),
                cutoff: self.get_param(FM_PARAM_VELOCITY_TO_CUTOFF).unwrap(),
                level: self.get_param(FM_PARAM_VELOCITY_TO_LEVEL).unwrap(),
            },
        }
    }

    /// Set a normalized parameter. Returns false for invalid indices or values.
    pub fn set_param(&mut self, param: u32, value: f32) -> bool {
        if param >= FM_PARAM_COUNT || !value.is_finite() {
            return false;
        }
        let value = value.clamp(0.0, 1.0);
        match param {
            FM_PARAM_OSC1_WAVEFORM => self.osc1_waveform = FmWaveform::from_normalized(value),
            FM_PARAM_OSC1_TRACKING => self.osc1_tracking = value >= 0.5,
            FM_PARAM_OSC2_WAVEFORM => self.osc2_waveform = FmWaveform::from_normalized(value),
            FM_PARAM_OSC2_TRACKING => self.osc2_tracking = value >= 0.5,
            FM_PARAM_RING_MODE => self.ring_mode = FmRingMode::from_normalized(value),
            FM_PARAM_FILTER_MODE => self.filter_mode = FmFilterMode::from_normalized(value),
            _ => self.params.set_target(param, value),
        }
        true
    }

    /// Get the exact target value for a normalized parameter.
    pub fn get_param(&self, param: u32) -> Option<f32> {
        if param >= FM_PARAM_COUNT {
            return None;
        }
        Some(match param {
            FM_PARAM_OSC1_WAVEFORM => self.osc1_waveform.normalized(),
            FM_PARAM_OSC1_TRACKING => self.osc1_tracking as u8 as f32,
            FM_PARAM_OSC2_WAVEFORM => self.osc2_waveform.normalized(),
            FM_PARAM_OSC2_TRACKING => self.osc2_tracking as u8 as f32,
            FM_PARAM_RING_MODE => self.ring_mode.normalized(),
            FM_PARAM_FILTER_MODE => self.filter_mode.normalized(),
            _ => self.params.target(param),
        })
    }

    pub fn snap_params(&mut self) {
        self.params.snap();
    }

    /// Reset all voice state while preserving the current patch.
    pub fn reset(&mut self) {
        self.osc1.reset();
        self.osc2.reset();
        self.pitch1_envelope.reset();
        self.pitch2_envelope.reset();
        self.noise_envelope.reset();
        self.filter_envelope.reset();
        self.amplitude_envelope.reset();
        self.filter.reset();
        self.boost_filter.reset();
        self.downsampler.clear();
        self.rng_state = 0x9e37_79b9_7f4a_7c15;
        self.previous_osc1 = 0.0;
        self.previous_osc2 = 0.0;
        self.staged_note = None;
        self.active = false;
    }

    pub fn set_base_pitch(&mut self, value: f32) -> bool {
        self.set_param(FM_PARAM_BASE_PITCH, value)
    }

    pub fn set_osc1_waveform(&mut self, waveform: FmWaveform) {
        self.osc1_waveform = waveform;
    }

    pub fn set_osc1_frequency(&mut self, value: f32) -> bool {
        self.set_param(FM_PARAM_OSC1_FREQUENCY, value)
    }

    pub fn set_osc1_tracking(&mut self, tracking: bool) {
        self.osc1_tracking = tracking;
    }

    pub fn set_osc1_level(&mut self, value: f32) -> bool {
        self.set_param(FM_PARAM_OSC1_LEVEL, value)
    }

    pub fn set_osc1_drop(&mut self, value: f32) -> bool {
        self.set_param(FM_PARAM_OSC1_DROP, value)
    }

    pub fn set_osc1_slope(&mut self, value: f32) -> bool {
        self.set_param(FM_PARAM_OSC1_SLOPE, value)
    }

    pub fn set_osc2_waveform(&mut self, waveform: FmWaveform) {
        self.osc2_waveform = waveform;
    }

    pub fn set_osc2_frequency(&mut self, value: f32) -> bool {
        self.set_param(FM_PARAM_OSC2_FREQUENCY, value)
    }

    pub fn set_osc2_tracking(&mut self, tracking: bool) {
        self.osc2_tracking = tracking;
    }

    pub fn set_osc2_level(&mut self, value: f32) -> bool {
        self.set_param(FM_PARAM_OSC2_LEVEL, value)
    }

    pub fn set_osc2_drop(&mut self, value: f32) -> bool {
        self.set_param(FM_PARAM_OSC2_DROP, value)
    }

    pub fn set_osc2_slope(&mut self, value: f32) -> bool {
        self.set_param(FM_PARAM_OSC2_SLOPE, value)
    }

    pub fn set_noise_level(&mut self, value: f32) -> bool {
        self.set_param(FM_PARAM_NOISE_LEVEL, value)
    }

    pub fn set_noise_decay(&mut self, value: f32) -> bool {
        self.set_param(FM_PARAM_NOISE_DECAY, value)
    }

    pub fn set_fm_index(&mut self, value: f32) -> bool {
        self.set_param(FM_PARAM_INDEX, value)
    }

    pub fn set_ring_mode(&mut self, mode: FmRingMode) {
        self.ring_mode = mode;
    }

    pub fn set_filter_mode(&mut self, mode: FmFilterMode) {
        self.filter_mode = mode;
    }

    pub fn set_filter_cutoff(&mut self, value: f32) -> bool {
        self.set_param(FM_PARAM_FILTER_CUTOFF, value)
    }

    pub fn set_filter_decay(&mut self, value: f32) -> bool {
        self.set_param(FM_PARAM_FILTER_DECAY, value)
    }

    pub fn set_filter_env_amount(&mut self, value: f32) -> bool {
        self.set_param(FM_PARAM_FILTER_ENV_AMOUNT, value)
    }

    pub fn set_grit(&mut self, value: f32) -> bool {
        self.set_param(FM_PARAM_GRIT, value)
    }

    pub fn set_amp_attack(&mut self, value: f32) -> bool {
        self.set_param(FM_PARAM_AMP_ATTACK, value)
    }

    pub fn set_amp_decay(&mut self, value: f32) -> bool {
        self.set_param(FM_PARAM_AMP_DECAY, value)
    }

    pub fn set_frequency_boost(&mut self, value: f32) -> bool {
        self.set_param(FM_PARAM_FREQUENCY_BOOST, value)
    }

    pub fn set_bit_drive(&mut self, value: f32) -> bool {
        self.set_param(FM_PARAM_BIT_DRIVE, value)
    }

    pub fn set_volume(&mut self, value: f32) -> bool {
        self.set_param(FM_PARAM_VOLUME, value)
    }

    pub fn set_tuning(&mut self, value: f32) -> bool {
        self.set_param(FM_PARAM_TUNING, value)
    }

    pub fn set_pitch_macro_depths(&mut self, depths: FmMacroDepths) {
        let depths = depths.clamped();
        let _ = self.set_param(FM_PARAM_PITCH_TO_DROP, depths.drop);
        let _ = self.set_param(FM_PARAM_PITCH_TO_FM, depths.fm);
        let _ = self.set_param(FM_PARAM_PITCH_TO_NOISE, depths.noise);
        let _ = self.set_param(FM_PARAM_PITCH_TO_BALANCE, depths.balance);
        let _ = self.set_param(FM_PARAM_PITCH_TO_CUTOFF, depths.cutoff);
        let _ = self.set_param(FM_PARAM_PITCH_TO_LEVEL, depths.level);
    }

    pub fn set_velocity_macro_depths(&mut self, depths: FmMacroDepths) {
        let depths = depths.clamped();
        let _ = self.set_param(FM_PARAM_VELOCITY_TO_DROP, depths.drop);
        let _ = self.set_param(FM_PARAM_VELOCITY_TO_FM, depths.fm);
        let _ = self.set_param(FM_PARAM_VELOCITY_TO_NOISE, depths.noise);
        let _ = self.set_param(FM_PARAM_VELOCITY_TO_BALANCE, depths.balance);
        let _ = self.set_param(FM_PARAM_VELOCITY_TO_CUTOFF, depths.cutoff);
        let _ = self.set_param(FM_PARAM_VELOCITY_TO_LEVEL, depths.level);
    }

    pub fn tuning(&self) -> f32 {
        self.get_param(FM_PARAM_TUNING).unwrap()
    }

    /// Apply bipolar modulation by stable parameter index. Categorical controls
    /// are intentionally ignored and return `false`.
    pub fn apply_modulation_index(&mut self, param: u32, value: f32) -> bool {
        if param >= FM_PARAM_COUNT || !value.is_finite() || !CONTINUOUS_PARAMS.contains(&param) {
            return false;
        }
        self.params.values[param as usize].set_bipolar(value);
        true
    }

    pub fn load_preset(&mut self, preset: u32) -> bool {
        let config = match preset {
            0 => FmPercussionConfig::sub_kick(),
            1 => FmPercussionConfig::metal_hat(),
            2 => FmPercussionConfig::zap(),
            3 => FmPercussionConfig::industrial(),
            _ => return false,
        };
        self.set_config(config);
        true
    }

    #[inline]
    fn macro_value(
        base: f32,
        pitch_depth: f32,
        velocity_depth: f32,
        pitch: f32,
        velocity: f32,
    ) -> f32 {
        fn delta(base: f32, depth: f32, source: f32) -> f32 {
            let signed = (depth - 0.5) * 2.0;
            if signed.abs() < f32::EPSILON {
                return 0.0;
            }
            let destination = if signed > 0.0 { source } else { 1.0 - source };
            (destination - base) * signed.abs()
        }
        (base + delta(base, pitch_depth, pitch) + delta(base, velocity_depth, velocity))
            .clamp(0.0, 1.0)
    }

    #[inline]
    fn frequency(
        &self,
        tracking: bool,
        control: f32,
        pitch_envelope: f32,
        drop: f32,
        tuning: f32,
    ) -> f32 {
        let tune = (tuning - 0.5) * 24.0;
        let base = if tracking {
            let note = self.current_note as f32 + ranges::tracked_offset_semitones(control) + tune;
            440.0 * 2.0_f32.powf((note - 69.0) / 12.0)
        } else {
            ranges::fixed_frequency(control) * 2.0_f32.powf(tune / 12.0)
        };
        let swept = base * 2.0_f32.powf((drop * pitch_envelope) / 12.0);
        swept.clamp(1.0, self.synth_sample_rate * 0.45)
    }

    #[inline]
    fn white_noise(&mut self) -> f32 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng_state = x;
        ((x >> 40) as u32 as f32 / 8_388_607.5) - 1.0
    }

    #[inline]
    fn bit_drive(input: f32, amount: f32) -> f32 {
        if amount <= 0.0 {
            return input;
        }
        let amount = amount.clamp(0.0, 1.0);
        let drive = 1.0 + amount * amount * 20.0;
        let saturated = (input * drive).tanh() / drive.tanh();
        let bits = 16.0 - 13.0 * amount;
        let levels = 2.0_f32.powf(bits).max(8.0);
        let quantized = (saturated * levels).round() / levels;
        input + (quantized - input) * amount
    }

    fn trigger_internal(&mut self, time: f64, velocity: f32) {
        self.current_velocity = normalized_or(velocity, 1.0);
        self.current_note = self.staged_note.take().unwrap_or_else(|| {
            ranges::midi_from_base_pitch(self.params.target(FM_PARAM_BASE_PITCH)).round() as u8
        });
        let pitch_source = self.current_note as f32 / 127.0;
        let velocity_source = self.current_velocity;
        self.triggered_drop1 = ranges::drop_semitones(Self::macro_value(
            self.params.target(FM_PARAM_OSC1_DROP),
            self.params.target(FM_PARAM_PITCH_TO_DROP),
            self.params.target(FM_PARAM_VELOCITY_TO_DROP),
            pitch_source,
            velocity_source,
        ));
        self.triggered_drop2 = ranges::drop_semitones(Self::macro_value(
            self.params.target(FM_PARAM_OSC2_DROP),
            self.params.target(FM_PARAM_PITCH_TO_DROP),
            self.params.target(FM_PARAM_VELOCITY_TO_DROP),
            pitch_source,
            velocity_source,
        ));
        self.pitch1_envelope.trigger(
            time,
            0.0,
            ranges::pitch_slope_seconds(self.params.target(FM_PARAM_OSC1_SLOPE)),
        );
        self.pitch2_envelope.trigger(
            time,
            0.0,
            ranges::pitch_slope_seconds(self.params.target(FM_PARAM_OSC2_SLOPE)),
        );
        self.noise_envelope.trigger(
            time,
            0.0,
            ranges::short_decay_seconds(self.params.target(FM_PARAM_NOISE_DECAY)),
        );
        self.filter_envelope.trigger(
            time,
            0.0,
            ranges::short_decay_seconds(self.params.target(FM_PARAM_FILTER_DECAY)),
        );
        self.amplitude_envelope.trigger(
            time,
            ranges::amp_attack_seconds(self.params.target(FM_PARAM_AMP_ATTACK)),
            ranges::amp_decay_seconds(self.params.target(FM_PARAM_AMP_DECAY)),
        );
        self.osc1.reset();
        self.osc2.reset();
        self.filter.reset();
        self.boost_filter.reset();
        self.downsampler.clear();
        self.rng_state = 0x9e37_79b9_7f4a_7c15;
        self.previous_osc1 = 0.0;
        self.previous_osc2 = 0.0;
        self.active = true;
    }

    #[inline]
    fn render_subsample(&mut self, time: f64, p: &[f32; FM_PARAM_COUNT as usize]) -> f32 {
        let pitch_source = self.current_note as f32 / 127.0;
        let velocity = self.current_velocity;
        let fm_normalized = Self::macro_value(
            p[FM_PARAM_INDEX as usize],
            p[FM_PARAM_PITCH_TO_FM as usize],
            p[FM_PARAM_VELOCITY_TO_FM as usize],
            pitch_source,
            velocity,
        );
        let noise_level = Self::macro_value(
            p[FM_PARAM_NOISE_LEVEL as usize],
            p[FM_PARAM_PITCH_TO_NOISE as usize],
            p[FM_PARAM_VELOCITY_TO_NOISE as usize],
            pitch_source,
            velocity,
        );
        let balance = Self::macro_value(
            0.5,
            p[FM_PARAM_PITCH_TO_BALANCE as usize],
            p[FM_PARAM_VELOCITY_TO_BALANCE as usize],
            pitch_source,
            velocity,
        );
        let cutoff_normalized = Self::macro_value(
            p[FM_PARAM_FILTER_CUTOFF as usize],
            p[FM_PARAM_PITCH_TO_CUTOFF as usize],
            p[FM_PARAM_VELOCITY_TO_CUTOFF as usize],
            pitch_source,
            velocity,
        );

        let pitch1 = self.pitch1_envelope.value(time);
        let pitch2 = self.pitch2_envelope.value(time);
        let frequency1 = self.frequency(
            self.osc1_tracking,
            p[FM_PARAM_OSC1_FREQUENCY as usize],
            pitch1,
            self.triggered_drop1,
            p[FM_PARAM_TUNING as usize],
        );
        let frequency2 = self.frequency(
            self.osc2_tracking,
            p[FM_PARAM_OSC2_FREQUENCY as usize],
            pitch2,
            self.triggered_drop2,
            p[FM_PARAM_TUNING as usize],
        );

        let cross_amount = if self.ring_mode == FmRingMode::CrossRing {
            0.08
        } else {
            0.0
        };
        let osc1_phase_feedback = (self.previous_osc2 * cross_amount).tanh() as f64;
        let osc1 = self.osc1.sample(
            self.osc1_waveform,
            osc1_phase_feedback,
            frequency1,
            self.synth_sample_rate,
        );
        let fm_phase = ranges::fm_index(fm_normalized) * osc1 / TAU;
        let osc2_phase_feedback = (self.previous_osc1 * cross_amount).tanh();
        let osc2 = self.osc2.sample(
            self.osc2_waveform,
            (fm_phase + osc2_phase_feedback) as f64,
            frequency2,
            self.synth_sample_rate,
        );
        self.osc1.advance(frequency1, self.synth_sample_rate);
        self.osc2.advance(frequency2, self.synth_sample_rate);
        self.previous_osc1 = osc1;
        self.previous_osc2 = osc2;

        let carrier = match self.ring_mode {
            FmRingMode::Off => osc2,
            FmRingMode::Ring | FmRingMode::CrossRing => osc1 * osc2,
        };
        let balance_angle = balance * std::f32::consts::FRAC_PI_2;
        let osc1_gain = balance_angle.cos() * std::f32::consts::SQRT_2;
        let osc2_gain = balance_angle.sin() * std::f32::consts::SQRT_2;
        let noise = self.white_noise() * noise_level * self.noise_envelope.value(time);
        let mut signal = osc1 * p[FM_PARAM_OSC1_LEVEL as usize] * osc1_gain
            + carrier * p[FM_PARAM_OSC2_LEVEL as usize] * osc2_gain
            + noise;

        let grit = p[FM_PARAM_GRIT as usize];
        signal += self.white_noise() * grit * 0.08;
        let filter_env = self.filter_envelope.value(time);
        let cutoff = (cutoff_normalized
            + (1.0 - cutoff_normalized) * p[FM_PARAM_FILTER_ENV_AMOUNT as usize] * filter_env)
            .clamp(0.0, 1.0);
        self.filter.set_params(
            ranges::cutoff_hz(cutoff, self.sample_rate),
            0.707 + grit * grit * 8.0,
        );
        let (low, _, high) = self.filter.process_all(signal);
        signal = match self.filter_mode {
            FmFilterMode::Lowpass => low,
            FmFilterMode::Highpass => high,
        };
        signal *= self.amplitude_envelope.value(time);

        let boost = p[FM_PARAM_FREQUENCY_BOOST as usize];
        if boost > 0.0 {
            self.boost_filter.set_params(
                ranges::boost_hz(boost, self.synth_sample_rate),
                1.5 + boost * 5.0,
            );
            let (_, band, _) = self.boost_filter.process_all(signal);
            signal += band * boost * 1.5;
        }
        signal = Self::bit_drive(signal, p[FM_PARAM_BIT_DRIVE as usize]);
        if signal.is_finite() {
            signal.clamp(-4.0, 4.0)
        } else {
            self.filter.reset();
            self.boost_filter.reset();
            0.0
        }
    }
}

impl Instrument for FmPercussion {
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        self.trigger_internal(time, velocity);
    }

    fn tick(&mut self, current_time: f64) -> f32 {
        let values = self.params.tick();
        if !self.active {
            return 0.0;
        }
        let half_sample = 0.5 / self.sample_rate as f64;
        let first = self.render_subsample(current_time, &values);
        let second = self.render_subsample(current_time + half_sample, &values);
        let signal = self.downsampler.process(first, second);
        let pitch_source = self.current_note as f32 / 127.0;
        let output_level = Self::macro_value(
            1.0,
            values[FM_PARAM_PITCH_TO_LEVEL as usize],
            values[FM_PARAM_VELOCITY_TO_LEVEL as usize],
            pitch_source,
            self.current_velocity,
        );
        if !self.amplitude_envelope.active {
            self.active = false;
        }
        let output = signal * values[FM_PARAM_VOLUME as usize] * output_level;
        if output.is_finite() {
            output.clamp(-4.0, 4.0)
        } else {
            0.0
        }
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn set_midi_note(&mut self, note: u8) {
        self.staged_note = Some(note.min(127));
    }

    fn set_frequency_normalized(&mut self, value: f32) {
        let _ = self.set_base_pitch(value);
    }

    fn get_frequency(&self) -> Option<f32> {
        self.get_param(FM_PARAM_BASE_PITCH)
    }

    fn as_modulatable(&mut self) -> Option<&mut dyn Modulatable> {
        Some(self)
    }
}

impl Modulatable for FmPercussion {
    fn modulatable_parameters(&self) -> Vec<&'static str> {
        CONTINUOUS_PARAMS
            .iter()
            .map(|index| PARAMETER_NAMES[*index as usize])
            .collect()
    }

    fn apply_modulation(&mut self, parameter: &str, value: f32) -> Result<(), String> {
        let Some(index) = PARAMETER_NAMES.iter().position(|name| *name == parameter) else {
            return Err(format!("unknown FM percussion parameter: {parameter}"));
        };
        let index = index as u32;
        if !CONTINUOUS_PARAMS.contains(&index) {
            return Err(format!(
                "categorical parameter cannot be modulated: {parameter}"
            ));
        }
        if !value.is_finite() {
            return Err("modulation value must be finite".to_string());
        }
        self.apply_modulation_index(index, value);
        Ok(())
    }

    fn parameter_range(&self, parameter: &str) -> Option<(f32, f32)> {
        let index = PARAMETER_NAMES.iter().position(|name| *name == parameter)? as u32;
        CONTINUOUS_PARAMS.contains(&index).then_some((0.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bin_magnitude(signal: &[f32], frequency: f32, sample_rate: f32) -> f64 {
        let mut real = 0.0_f64;
        let mut imaginary = 0.0_f64;
        for (index, sample) in signal.iter().enumerate() {
            let phase =
                std::f64::consts::TAU * frequency as f64 * index as f64 / sample_rate as f64;
            real += *sample as f64 * phase.cos();
            imaginary -= *sample as f64 * phase.sin();
        }
        (real * real + imaginary * imaginary).sqrt() / signal.len() as f64
    }

    fn sine_fm(
        carrier: f32,
        modulator: f32,
        index: f32,
        sample_rate: f32,
        length: usize,
    ) -> Vec<f32> {
        (0..length)
            .map(|sample| {
                let time = sample as f32 / sample_rate;
                (TAU * carrier * time + index * (TAU * modulator * time).sin()).sin()
            })
            .collect()
    }

    fn render(voice: &mut FmPercussion, seconds: f64) -> Vec<f32> {
        voice.trigger_with_velocity(0.0, 1.0);
        let samples = (voice.sample_rate as f64 * seconds) as usize;
        (0..samples)
            .map(|sample| voice.tick(sample as f64 / voice.sample_rate as f64))
            .collect()
    }

    #[test]
    fn parameter_table_round_trips_and_rejects_non_finite_values() {
        let mut voice = FmPercussion::new(48_000.0);
        for param in 0..FM_PARAM_COUNT {
            assert!(voice.set_param(param, 0.234));
            let expected = match param {
                FM_PARAM_OSC1_WAVEFORM | FM_PARAM_OSC2_WAVEFORM => 1.0 / 3.0,
                FM_PARAM_OSC1_TRACKING | FM_PARAM_OSC2_TRACKING | FM_PARAM_FILTER_MODE => 0.0,
                FM_PARAM_RING_MODE => 0.0,
                _ => 0.234,
            };
            assert_eq!(voice.get_param(param), Some(expected), "parameter {param}");
        }
        assert!(!voice.set_param(FM_PARAM_COUNT, 0.5));
        assert!(!voice.set_param(0, f32::NAN));
        assert!(!voice.set_param(0, f32::INFINITY));
        assert_eq!(voice.get_param(FM_PARAM_COUNT), None);
    }

    #[test]
    fn presets_and_extremes_remain_finite_and_bounded() {
        for sample_rate in [44_100.0, 48_000.0, 96_000.0] {
            for preset in 0..4 {
                let mut voice = FmPercussion::new(sample_rate);
                assert!(voice.load_preset(preset));
                voice.snap_params();
                for sample in render(&mut voice, 0.25) {
                    assert!(sample.is_finite());
                    assert!(sample.abs() <= 4.0);
                }
            }
        }
    }

    #[test]
    fn retrigger_is_deterministic_and_envelope_finishes() {
        let mut voice = FmPercussion::with_config(48_000.0, FmPercussionConfig::industrial());
        voice.snap_params();
        let first = render(&mut voice, 0.05);
        for sample in 0..32 {
            let _ = voice.tick(sample as f64 / 48_000.0);
        }
        voice.reset();
        let second = render(&mut voice, 0.05);
        assert_eq!(first, second);
        for sample in 2_400..300_000 {
            let _ = voice.tick(sample as f64 / 48_000.0);
            if !voice.is_active() {
                return;
            }
        }
        panic!("voice envelope did not deactivate");
    }

    #[test]
    fn zero_volume_is_silent_and_zero_bit_drive_is_exact_bypass() {
        for input in [-2.0, -0.25, 0.0, 0.375, 2.0] {
            assert_eq!(FmPercussion::bit_drive(input, 0.0), input);
        }
        let mut config = FmPercussionConfig::industrial();
        config.volume = 0.0;
        let mut voice = FmPercussion::with_config(48_000.0, config);
        assert!(render(&mut voice, 0.1).iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn bipolar_drop_moves_in_both_directions_and_converges() {
        let mut voice = FmPercussion::new(48_000.0);
        voice.current_note = 60;
        let base = midi_to_freq(60) as f32;
        let high = voice.frequency(true, 0.5, 1.0, 60.0, 0.5);
        let low = voice.frequency(true, 0.5, 1.0, -60.0, 0.5);
        let converged = voice.frequency(true, 0.5, 0.0, 60.0, 0.5);
        assert!(high > base && low < base);
        assert!((converged - base).abs() < 0.01);
    }

    #[test]
    fn zero_index_is_the_carrier_and_nonzero_index_creates_sidebands() {
        let sample_rate = 48_000.0;
        let length = 48_000;
        let carrier = sine_fm(1_200.0, 300.0, 0.0, sample_rate, length);
        let unmodulated: Vec<f32> = (0..length)
            .map(|sample| (TAU * 1_200.0 * sample as f32 / sample_rate).sin())
            .collect();
        let max_difference = carrier
            .iter()
            .zip(&unmodulated)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(max_difference < 0.001, "zero-index error: {max_difference}");

        let modulated = sine_fm(1_200.0, 300.0, 2.0, sample_rate, length);
        let lower = bin_magnitude(&modulated, 900.0, sample_rate);
        let upper = bin_magnitude(&modulated, 1_500.0, sample_rate);
        let zero_lower = bin_magnitude(&carrier, 900.0, sample_rate);
        assert!(lower > 0.1 && upper > 0.1);
        assert!(lower > zero_lower * 1_000.0);
    }

    #[test]
    fn harmonic_and_inharmonic_ratios_have_distinct_spectra() {
        let sample_rate = 48_000.0;
        let harmonic = sine_fm(1_200.0, 300.0, 2.0, sample_rate, 48_000);
        let inharmonic = sine_fm(1_200.0, 173.0, 2.0, sample_rate, 48_000);
        let off_grid = 1_027.0;
        assert!(
            bin_magnitude(&inharmonic, off_grid, sample_rate)
                > bin_magnitude(&harmonic, off_grid, sample_rate) * 100.0
        );
    }

    #[test]
    fn slope_changes_pitch_convergence_time() {
        let mut fast = OneShotEnvelope::new();
        let mut slow = OneShotEnvelope::new();
        fast.trigger(0.0, 0.0, ranges::pitch_slope_seconds(0.25));
        slow.trigger(0.0, 0.0, ranges::pitch_slope_seconds(0.75));
        let time = 0.008;
        assert!(fast.value(time) < slow.value(time));
        assert_eq!(fast.value(2.0), 0.0);
        assert_eq!(slow.value(2.0), 0.0);
    }

    #[test]
    fn every_categorical_mode_at_extreme_controls_is_finite() {
        for sample_rate in [44_100.0, 48_000.0, 96_000.0] {
            for waveform in [
                FmWaveform::Sine,
                FmWaveform::Triangle,
                FmWaveform::Square,
                FmWaveform::Metal,
            ] {
                for ring in [FmRingMode::Off, FmRingMode::Ring, FmRingMode::CrossRing] {
                    for filter in [FmFilterMode::Lowpass, FmFilterMode::Highpass] {
                        let mut config = FmPercussionConfig::industrial();
                        config.osc1_waveform = waveform;
                        config.osc2_waveform = waveform;
                        config.ring_mode = ring;
                        config.filter_mode = filter;
                        config.fm_index = 1.0;
                        config.grit = 1.0;
                        config.bit_drive = 1.0;
                        config.filter_cutoff = if filter == FmFilterMode::Lowpass {
                            0.0
                        } else {
                            1.0
                        };
                        config.osc1_frequency = 1.0;
                        config.osc2_frequency = 1.0;
                        let mut voice = FmPercussion::with_config(sample_rate, config);
                        voice.snap_params();
                        for sample in render(&mut voice, 0.025) {
                            assert!(sample.is_finite());
                            assert!(sample.abs() <= 4.0);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn two_x_path_reduces_aggressive_high_band_alias_energy_by_six_db() {
        let sample_rate = 48_000.0_f32;
        let length = 4_096;
        let base = sine_fm(9_000.0, 7_300.0, 12.0, sample_rate, length);
        let oversampled = sine_fm(9_000.0, 7_300.0, 12.0, sample_rate * 2.0, length * 2);
        let mut downsampler = Downsampler8::default();
        let two_x: Vec<f32> = oversampled
            .chunks_exact(2)
            .map(|pair| downsampler.process(pair[0], pair[1]))
            .collect();

        let high_band_energy = |signal: &[f32]| {
            (1_850..2_035)
                .map(|bin| {
                    let frequency = bin as f32 * sample_rate / length as f32;
                    let magnitude = bin_magnitude(signal, frequency, sample_rate);
                    magnitude * magnitude
                })
                .sum::<f64>()
        };
        let base_alias = high_band_energy(&base);
        let two_x_alias = high_band_energy(&two_x);
        let reduction_db = 10.0 * (base_alias / two_x_alias.max(f64::MIN_POSITIVE)).log10();
        assert!(
            reduction_db >= 6.0,
            "2x high-band reduction was only {reduction_db:.2} dB"
        );
    }

    #[test]
    fn neutral_macros_preserve_base_and_combination_is_commutative() {
        assert_eq!(FmPercussion::macro_value(0.37, 0.5, 0.5, 0.2, 0.9), 0.37);
        let combined = FmPercussion::macro_value(0.37, 0.9, 0.1, 0.2, 0.9);
        let pitch_delta = FmPercussion::macro_value(0.37, 0.9, 0.5, 0.2, 0.9) - 0.37;
        let velocity_delta = FmPercussion::macro_value(0.37, 0.5, 0.1, 0.2, 0.9) - 0.37;
        assert!((combined - (0.37 + pitch_delta + velocity_delta).clamp(0.0, 1.0)).abs() < 1e-6);
    }
}
