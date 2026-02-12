use std::f32::consts::PI;

use crate::filters::{BiquadHighpass, StateVariableFilterTpt};
use crate::gen::pink_noise::PinkNoise;
use crate::max_curve::MaxCurveEnvelope;
use crate::utils::Blendable;
use crate::utils::{SmoothedParam, DEFAULT_SMOOTH_TIME_MS};

/// Normalization ranges for HiHat2 parameters
/// All external-facing parameters use 0.0-1.0 normalized values
pub(crate) mod ranges {
    /// Pitch: 0-1 maps to 3500-10000 Hz (after pow2 curve)
    pub const PITCH_MIN: f32 = 3500.0;
    pub const PITCH_MAX: f32 = 10000.0;

    /// Attack: 0-1 maps to 0.5-200 ms
    pub const ATTACK_MIN_MS: f32 = 0.5;
    pub const ATTACK_MAX_MS: f32 = 200.0;

    /// Decay: 0-1 maps to 0.5-4000 ms
    pub const DECAY_MIN_MS: f32 = 0.5;
    pub const DECAY_MAX_MS: f32 = 4000.0;

    /// Tone: 0-1 maps to 500-10000 Hz
    pub const TONE_MIN: f32 = 500.0;
    pub const TONE_MAX: f32 = 10000.0;

    /// Map normalized 0-1 value to actual range
    #[inline]
    pub fn denormalize(normalized: f32, min: f32, max: f32) -> f32 {
        min + normalized.clamp(0.0, 1.0) * (max - min)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseColor {
    White,
    Pink,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterSlope {
    Db12,
    Db24,
}

#[derive(Clone, Copy, Debug)]
pub struct HiHat2Config {
    pub pitch: f32,   // 0-1 normalized (pow2 curve -> 3500-10000 Hz)
    pub decay: f32,   // 0-1 normalized (0.5-4000 ms)
    pub attack: f32,  // 0-1 normalized (0.5-200 ms)
    pub noise_color: NoiseColor,
    pub filter_slope: FilterSlope,
    pub tone: f32,    // 0-1 normalized (500-10000 Hz)
    pub volume: f32,  // 0-1 overall volume
}

impl HiHat2Config {
    pub fn new(
        pitch: f32,
        decay: f32,
        attack: f32,
        noise_color: NoiseColor,
        filter_slope: FilterSlope,
        tone: f32,
    ) -> Self {
        Self {
            pitch: pitch.clamp(0.0, 1.0),
            decay: decay.clamp(0.0, 1.0),
            attack: attack.clamp(0.0, 1.0),
            noise_color,
            filter_slope,
            tone: tone.clamp(0.0, 1.0),
            volume: 1.0,
        }
    }

    /// Short preset
    pub fn short() -> Self {
        Self::new(0.76, 0.05, 0.00, NoiseColor::White, FilterSlope::Db24, 1.00)
    }

    /// Loose preset
    pub fn loose() -> Self {
        Self::new(0.76, 0.30, 0.00, NoiseColor::White, FilterSlope::Db24, 1.00)
    }

    /// Dark preset
    pub fn dark() -> Self {
        Self::new(0.41, 0.05, 0.00, NoiseColor::White, FilterSlope::Db24, 0.15)
    }

    /// Soft preset
    pub fn soft() -> Self {
        Self::new(0.41, 0.05, 0.15, NoiseColor::White, FilterSlope::Db24, 0.60)
    }

    #[inline]
    pub fn pitch_hz(&self) -> f32 {
        let curved = self.pitch * self.pitch;
        ranges::denormalize(curved, ranges::PITCH_MIN, ranges::PITCH_MAX)
    }

    #[inline]
    pub fn attack_ms(&self) -> f32 {
        ranges::denormalize(self.attack, ranges::ATTACK_MIN_MS, ranges::ATTACK_MAX_MS)
    }

    #[inline]
    pub fn decay_ms(&self) -> f32 {
        ranges::denormalize(self.decay, ranges::DECAY_MIN_MS, ranges::DECAY_MAX_MS)
    }

    #[inline]
    pub fn tone_hz(&self) -> f32 {
        ranges::denormalize(self.tone, ranges::TONE_MIN, ranges::TONE_MAX)
    }
}

impl Default for HiHat2Config {
    fn default() -> Self {
        Self::short()
    }
}

impl Blendable for HiHat2Config {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let inv_t = 1.0 - t;

        Self {
            pitch: self.pitch * inv_t + other.pitch * t,
            decay: self.decay * inv_t + other.decay * t,
            attack: self.attack * inv_t + other.attack * t,
            noise_color: if t < 0.5 {
                self.noise_color
            } else {
                other.noise_color
            },
            filter_slope: if t < 0.5 {
                self.filter_slope
            } else {
                other.filter_slope
            },
            tone: self.tone * inv_t + other.tone * t,
            volume: self.volume * inv_t + other.volume * t,
        }
    }
}

/// Smoothed parameters for real-time control
pub struct HiHat2Params {
    pub pitch: SmoothedParam,
    pub decay: SmoothedParam,
    pub attack: SmoothedParam,
    pub tone: SmoothedParam,
    pub volume: SmoothedParam,
}

impl HiHat2Params {
    pub fn from_config(config: &HiHat2Config, sample_rate: f32) -> Self {
        Self {
            pitch: SmoothedParam::new(config.pitch, 0.0, 1.0, sample_rate, DEFAULT_SMOOTH_TIME_MS),
            decay: SmoothedParam::new(config.decay, 0.0, 1.0, sample_rate, DEFAULT_SMOOTH_TIME_MS),
            attack: SmoothedParam::new(
                config.attack,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            tone: SmoothedParam::new(config.tone, 0.0, 1.0, sample_rate, DEFAULT_SMOOTH_TIME_MS),
            volume: SmoothedParam::new(config.volume, 0.0, 1.0, sample_rate, DEFAULT_SMOOTH_TIME_MS),
        }
    }

    #[inline]
    pub fn tick(&mut self) -> bool {
        self.pitch.tick();
        self.decay.tick();
        self.attack.tick();
        self.tone.tick();
        self.volume.tick();

        !self.is_settled()
    }

    pub fn is_settled(&self) -> bool {
        self.pitch.is_settled()
            && self.decay.is_settled()
            && self.attack.is_settled()
            && self.tone.is_settled()
            && self.volume.is_settled()
    }

    #[inline]
    pub fn pitch_hz(&self) -> f32 {
        let curved = self.pitch.get() * self.pitch.get();
        ranges::denormalize(curved, ranges::PITCH_MIN, ranges::PITCH_MAX)
    }

    #[inline]
    pub fn attack_ms(&self) -> f32 {
        ranges::denormalize(
            self.attack.get(),
            ranges::ATTACK_MIN_MS,
            ranges::ATTACK_MAX_MS,
        )
    }

    #[inline]
    pub fn decay_ms(&self) -> f32 {
        ranges::denormalize(self.decay.get(), ranges::DECAY_MIN_MS, ranges::DECAY_MAX_MS)
    }

    #[inline]
    pub fn tone_hz(&self) -> f32 {
        ranges::denormalize(self.tone.get(), ranges::TONE_MIN, ranges::TONE_MAX)
    }

    pub fn to_config(&self, noise_color: NoiseColor, filter_slope: FilterSlope) -> HiHat2Config {
        HiHat2Config {
            pitch: self.pitch.get(),
            decay: self.decay.get(),
            attack: self.attack.get(),
            noise_color,
            filter_slope,
            tone: self.tone.get(),
            volume: self.volume.get(),
        }
    }
}

struct PhaseModOsc {
    sample_rate: f32,
    frequency_hz: f32,
    phase_cycle: f32,
}

impl PhaseModOsc {
    fn new(sample_rate: f32, frequency_hz: f32) -> Self {
        Self {
            sample_rate,
            frequency_hz,
            phase_cycle: 0.0,
        }
    }

    fn set_frequency(&mut self, frequency_hz: f32) {
        self.frequency_hz = frequency_hz.max(0.0);
    }

    fn reset_phase(&mut self) {
        self.phase_cycle = 0.0;
    }

    fn tick(&mut self, phase_mod: f32) -> f32 {
        let phase_inc = self.frequency_hz / self.sample_rate;
        self.phase_cycle = (self.phase_cycle + phase_inc) % 1.0;

        let mut phase = self.phase_cycle + phase_mod;
        phase -= phase.floor();

        (2.0 * PI * phase).sin()
    }
}

/// Asymmetric one-pole smoothing (instant up, smoothed down)
struct AsymmetricSmoother {
    current: f32,
    down_coeff: f32,
}

impl AsymmetricSmoother {
    fn new(down_samples: f32) -> Self {
        let down_coeff = if down_samples <= 0.0 {
            1.0
        } else {
            1.0 - (-1.0 / down_samples).exp()
        };
        Self {
            current: 0.0,
            down_coeff,
        }
    }

    fn reset(&mut self, value: f32) {
        self.current = value;
    }

    fn process(&mut self, target: f32) -> f32 {
        if target >= self.current {
            self.current = target;
        } else {
            self.current += self.down_coeff * (target - self.current);
        }
        self.current
    }

    fn current(&self) -> f32 {
        self.current
    }
}

pub struct HiHat2 {
    pub sample_rate: f32,
    pub params: HiHat2Params,
    pub noise_color: NoiseColor,
    pub filter_slope: FilterSlope,

    mod_osc: PhaseModOsc,
    main_osc: PhaseModOsc,

    envelope: MaxCurveEnvelope,
    envelope_smoother: AsymmetricSmoother,

    hpf_stage_1: BiquadHighpass,
    hpf_stage_2: BiquadHighpass,
    svf: StateVariableFilterTpt,

    white_noise_state: u64,
    pink_noise: PinkNoise,

    is_active: bool,
    current_velocity: f32,
}

impl HiHat2 {
    pub fn new(sample_rate: f32) -> Self {
        Self::with_config(sample_rate, HiHat2Config::default())
    }

    pub fn with_config(sample_rate: f32, config: HiHat2Config) -> Self {
        let params = HiHat2Params::from_config(&config, sample_rate);
        let pitch_hz = config.pitch_hz();
        let tone_hz = config.tone_hz();

        Self {
            sample_rate,
            params,
            noise_color: config.noise_color,
            filter_slope: config.filter_slope,
            mod_osc: PhaseModOsc::new(sample_rate, pitch_hz * 0.1),
            main_osc: PhaseModOsc::new(sample_rate, pitch_hz),
            envelope: MaxCurveEnvelope::new(Vec::new()),
            envelope_smoother: AsymmetricSmoother::new(100.0),
            hpf_stage_1: BiquadHighpass::new(sample_rate),
            hpf_stage_2: BiquadHighpass::new(sample_rate),
            svf: StateVariableFilterTpt::new(sample_rate, tone_hz, 0.5),
            white_noise_state: 0x1234_5678_9abc_def0,
            pink_noise: PinkNoise::new(),
            is_active: false,
            current_velocity: 1.0,
        }
    }

    pub fn config(&self) -> HiHat2Config {
        self.params.to_config(self.noise_color, self.filter_slope)
    }

    pub fn set_config(&mut self, config: HiHat2Config) {
        self.params.pitch.set_target(config.pitch);
        self.params.decay.set_target(config.decay);
        self.params.attack.set_target(config.attack);
        self.params.tone.set_target(config.tone);
        self.params.volume.set_target(config.volume);
        self.noise_color = config.noise_color;
        self.filter_slope = config.filter_slope;
    }

    pub fn set_pitch(&mut self, pitch: f32) {
        self.params.pitch.set_target(pitch);
    }

    pub fn set_decay(&mut self, decay: f32) {
        self.params.decay.set_target(decay);
    }

    pub fn set_attack(&mut self, attack: f32) {
        self.params.attack.set_target(attack);
    }

    pub fn set_tone(&mut self, tone: f32) {
        self.params.tone.set_target(tone);
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.params.volume.set_target(volume.clamp(0.0, 1.0));
    }

    pub fn set_noise_color(&mut self, noise_color: NoiseColor) {
        self.noise_color = noise_color;
    }

    pub fn set_filter_slope(&mut self, filter_slope: FilterSlope) {
        self.filter_slope = filter_slope;
    }

    pub fn trigger(&mut self, time: f32) {
        self.trigger_with_velocity(time, 1.0);
    }

    pub fn trigger_with_velocity(&mut self, time: f32, velocity: f32) {
        self.is_active = true;
        self.current_velocity = velocity.clamp(0.0, 1.0);

        let attack_ms = self.params.attack_ms();
        let decay_ms = self.params.decay_ms();

        self.envelope = MaxCurveEnvelope::new(vec![(1.0, attack_ms, -0.3), (0.0, decay_ms, -0.8)]);
        self.envelope.set_initial_value(0.0);
        self.envelope.trigger(time);
        self.envelope_smoother.reset(0.0);

        self.mod_osc.reset_phase();
        self.main_osc.reset_phase();
        self.hpf_stage_1.reset();
        self.hpf_stage_2.reset();
        self.svf.reset();
    }

    pub fn tick(&mut self, current_time: f32) -> f32 {
        self.params.tick();

        if !self.is_active {
            return 0.0;
        }

        let pitch_hz = self.params.pitch_hz();
        let mod_freq = pitch_hz * 0.1;
        self.mod_osc.set_frequency(mod_freq);
        self.main_osc.set_frequency(pitch_hz);

        let noise = match self.noise_color {
            NoiseColor::White => self.white_noise_tick(),
            NoiseColor::Pink => self.pink_noise.tick(),
        };

        let mod_signal = noise * 0.25;
        let mod_output = self.mod_osc.tick(mod_signal);
        let main_output = self.main_osc.tick(mod_output * 0.75);
        //
        let mut filtered = {
            self.hpf_stage_1.set_params(pitch_hz, 1.0);
            self.hpf_stage_1.process(main_output)
        };

        if self.filter_slope == FilterSlope::Db24 {
            self.hpf_stage_2.set_params(pitch_hz, 1.0);
            filtered = self.hpf_stage_2.process(filtered) * 0.8;
        }

        let env = self.envelope.get_value(current_time);
        let env = self.envelope_smoother.process(env);

        let volume = self.params.volume.get();
        let output = filtered * env * self.current_velocity * volume * 0.35;

        let tone_hz = self.params.tone_hz();
        self.svf.set_params(tone_hz, 0.5);
        let (_, _, high) = self.svf.process_all(output);
        let output = high;

        if self.envelope.is_complete() && self.envelope_smoother.current() < 1e-4 {
            self.is_active = false;
        }

        output
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    fn white_noise_tick(&mut self) -> f32 {
        // xorshift64*
        let mut x = self.white_noise_state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.white_noise_state = x;
        let hashed = x.wrapping_mul(0x2545F4914F6CDD1D);

        let normalized = (hashed as f32) / (u64::MAX as f32);
        (normalized * 2.0) - 1.0
    }
}

impl crate::engine::Instrument for HiHat2 {
    fn trigger_with_velocity(&mut self, time: f32, velocity: f32) {
        HiHat2::trigger_with_velocity(self, time, velocity);
    }

    fn tick(&mut self, current_time: f32) -> f32 {
        self.tick(current_time)
    }

    fn is_active(&self) -> bool {
        self.is_active()
    }

    fn as_modulatable(&mut self) -> Option<&mut dyn crate::engine::Modulatable> {
        Some(self)
    }
}

impl crate::engine::Modulatable for HiHat2 {
    fn modulatable_parameters(&self) -> Vec<&'static str> {
        vec!["attack", "decay", "pitch", "tone", "volume"]
    }

    fn apply_modulation(&mut self, parameter: &str, value: f32) -> Result<(), String> {
        match parameter {
            "attack" => {
                self.params.attack.set_bipolar(value);
                Ok(())
            }
            "decay" => {
                self.params.decay.set_bipolar(value);
                Ok(())
            }
            "pitch" => {
                self.params.pitch.set_bipolar(value);
                Ok(())
            }
            "tone" => {
                self.params.tone.set_bipolar(value);
                Ok(())
            }
            "volume" => {
                self.params.volume.set_bipolar(value);
                Ok(())
            }
            _ => Err(format!("Unknown parameter: {}", parameter)),
        }
    }

    fn parameter_range(&self, parameter: &str) -> Option<(f32, f32)> {
        match parameter {
            "attack" => Some(self.params.attack.range()),
            "decay" => Some(self.params.decay.range()),
            "pitch" => Some(self.params.pitch.range()),
            "tone" => Some(self.params.tone.range()),
            "volume" => Some(self.params.volume.range()),
            _ => None,
        }
    }
}
