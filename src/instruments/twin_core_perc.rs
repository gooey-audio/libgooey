//! A twin-core percussion voice inspired by the SSF Entity Ultra-Perc signal flow.
//!
//! This is a digital interpretation, not a circuit model or product emulation.
//! Its identity is the fixed signal order: delayed body excitation, two serial
//! resonant cores, selectable spectral emphasis, wavefolding, a body VCA, and
//! an independently enveloped noise path that can instead excite the body.

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
pub enum TwinCorePercBodyMode {
    Low,
    Mid,
    High,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TwinCorePercNoiseMode {
    Lowpass,
    Highpass,
    /// Send low-passed noise into the resonant body instead of the direct mix.
    Body,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TwinCorePercConfig {
    pub master_tune_hz: f32,
    /// Downward offset of core two from core one, matching the panel behavior.
    pub detune_octaves: f32,
    pub length_seconds: f32,
    pub body_bias: f32,
    pub fm_decay_seconds: f32,
    pub fm_depth_octaves: f32,
    pub trigger_delay_seconds: f32,
    pub body_mode: TwinCorePercBodyMode,
    pub harmonics: f32,
    pub noise_mode: TwinCorePercNoiseMode,
    pub noise_filter_hz: f32,
    pub noise_decay_seconds: f32,
    pub noise_bias: f32,
    pub volume: f32,
    pub seed: u32,
}

impl Default for TwinCorePercConfig {
    fn default() -> Self {
        Self::kick()
    }
}

impl TwinCorePercConfig {
    pub fn kick() -> Self {
        Self {
            master_tune_hz: 52.0,
            detune_octaves: 0.04,
            length_seconds: 0.9,
            body_bias: 0.15,
            fm_decay_seconds: 0.055,
            fm_depth_octaves: 3.2,
            trigger_delay_seconds: 0.0,
            body_mode: TwinCorePercBodyMode::Low,
            harmonics: 0.14,
            noise_mode: TwinCorePercNoiseMode::Lowpass,
            noise_filter_hz: 1_800.0,
            noise_decay_seconds: 0.018,
            noise_bias: -0.75,
            volume: 0.72,
            seed: 0xe17_7001,
        }
    }

    pub fn tom() -> Self {
        Self {
            master_tune_hz: 112.0,
            detune_octaves: 0.58,
            length_seconds: 0.72,
            body_bias: 0.05,
            fm_decay_seconds: 0.11,
            fm_depth_octaves: 0.8,
            body_mode: TwinCorePercBodyMode::Mid,
            harmonics: 0.08,
            noise_bias: -1.0,
            volume: 0.7,
            ..Self::kick()
        }
    }

    pub fn snare() -> Self {
        Self {
            master_tune_hz: 185.0,
            detune_octaves: 0.7,
            length_seconds: 0.24,
            body_bias: -0.05,
            fm_decay_seconds: 0.025,
            fm_depth_octaves: 0.35,
            body_mode: TwinCorePercBodyMode::Mid,
            harmonics: 0.46,
            noise_mode: TwinCorePercNoiseMode::Highpass,
            noise_filter_hz: 3_800.0,
            noise_decay_seconds: 0.34,
            noise_bias: 0.2,
            volume: 0.56,
            ..Self::kick()
        }
    }

    pub fn clap() -> Self {
        Self {
            master_tune_hz: 240.0,
            detune_octaves: 1.15,
            length_seconds: 0.045,
            body_bias: 0.7,
            fm_decay_seconds: 0.012,
            fm_depth_octaves: -0.4,
            trigger_delay_seconds: 0.018,
            body_mode: TwinCorePercBodyMode::High,
            harmonics: 0.58,
            noise_mode: TwinCorePercNoiseMode::Body,
            noise_filter_hz: 2_600.0,
            noise_decay_seconds: 0.28,
            noise_bias: 0.35,
            volume: 0.5,
            ..Self::kick()
        }
    }

    pub fn metallic() -> Self {
        Self {
            master_tune_hz: 310.0,
            detune_octaves: 1.83,
            length_seconds: 2.8,
            body_bias: 0.35,
            fm_decay_seconds: 0.7,
            fm_depth_octaves: -1.4,
            body_mode: TwinCorePercBodyMode::High,
            harmonics: 0.78,
            noise_mode: TwinCorePercNoiseMode::Body,
            noise_filter_hz: 6_500.0,
            noise_decay_seconds: 0.8,
            noise_bias: -0.2,
            volume: 0.42,
            ..Self::kick()
        }
    }
}

pub struct TwinCorePercVoice {
    sample_rate: f32,
    cores: [Resonator; 2],
    exciter: Exciter,
    noise_filter: StateVariableFilterTpt,
    noise_rng: XorShift32,
    folder: Oversampler,
    params: [SmoothedParam; 12],
    pending_body_mode: TwinCorePercBodyMode,
    active_body_mode: TwinCorePercBodyMode,
    pending_noise_mode: TwinCorePercNoiseMode,
    active_noise_mode: TwinCorePercNoiseMode,
    seed: u32,
    velocity: f32,
    noise_velocity: f32,
    body_trigger_time: f64,
    noise_trigger_time: f64,
    body_armed: bool,
    body_started: bool,
    active: bool,
    midi_note: Option<u8>,
    ring_limit_secs: Option<f32>,
}

impl TwinCorePercVoice {
    pub fn new(sample_rate: f32) -> Self {
        Self::with_config(sample_rate, TwinCorePercConfig::default())
    }

    pub fn with_config(sample_rate: f32, config: TwinCorePercConfig) -> Self {
        let sr = sample_rate.max(1.0);
        let mut exciter = Exciter::new(sr);
        exciter.set_kind(ExciterKind::Pulse);
        exciter.set_width_ms(0.75);
        Self {
            sample_rate: sr,
            cores: [Resonator::new(sr), Resonator::new(sr)],
            exciter,
            noise_filter: StateVariableFilterTpt::new(sr, config.noise_filter_hz, 0.707),
            noise_rng: XorShift32::new(config.seed),
            folder: Oversampler::new(OversamplingMode::X2),
            params: [
                SmoothedParam::new(config.master_tune_hz, 20.0, 2_000.0, sr, SMOOTH_MS),
                SmoothedParam::new(config.detune_octaves, 0.0, 2.5, sr, SMOOTH_MS),
                SmoothedParam::new(config.length_seconds, 0.002, 20.0, sr, SMOOTH_MS),
                SmoothedParam::new(config.body_bias, -1.0, 1.0, sr, SMOOTH_MS),
                SmoothedParam::new(config.fm_decay_seconds, 0.002, 8.0, sr, SMOOTH_MS),
                SmoothedParam::new(config.fm_depth_octaves, -5.0, 5.0, sr, SMOOTH_MS),
                SmoothedParam::new(config.trigger_delay_seconds, 0.0, 0.075, sr, SMOOTH_MS),
                SmoothedParam::new_normalized(config.harmonics, sr),
                SmoothedParam::new(config.noise_filter_hz, 20.0, sr * 0.45, sr, SMOOTH_MS),
                SmoothedParam::new(config.noise_decay_seconds, 0.002, 20.0, sr, SMOOTH_MS),
                SmoothedParam::new(config.noise_bias, -1.0, 1.0, sr, SMOOTH_MS),
                SmoothedParam::new(config.volume, 0.0, 2.0, sr, SMOOTH_MS),
            ],
            pending_body_mode: config.body_mode,
            active_body_mode: config.body_mode,
            pending_noise_mode: config.noise_mode,
            active_noise_mode: config.noise_mode,
            seed: config.seed,
            velocity: 1.0,
            noise_velocity: 1.0,
            body_trigger_time: 0.0,
            noise_trigger_time: 0.0,
            body_armed: false,
            body_started: false,
            active: false,
            midi_note: None,
            ring_limit_secs: None,
        }
    }

    pub fn config_targets(&self) -> TwinCorePercConfig {
        TwinCorePercConfig {
            master_tune_hz: self.params[0].target(),
            detune_octaves: self.params[1].target(),
            length_seconds: self.params[2].target(),
            body_bias: self.params[3].target(),
            fm_decay_seconds: self.params[4].target(),
            fm_depth_octaves: self.params[5].target(),
            trigger_delay_seconds: self.params[6].target(),
            body_mode: self.pending_body_mode,
            harmonics: self.params[7].target(),
            noise_mode: self.pending_noise_mode,
            noise_filter_hz: self.params[8].target(),
            noise_decay_seconds: self.params[9].target(),
            noise_bias: self.params[10].target(),
            volume: self.params[11].target(),
            seed: self.seed,
        }
    }

    pub fn set_config(&mut self, config: TwinCorePercConfig) {
        let values = [
            config.master_tune_hz,
            config.detune_octaves,
            config.length_seconds,
            config.body_bias,
            config.fm_decay_seconds,
            config.fm_depth_octaves,
            config.trigger_delay_seconds,
            config.harmonics,
            config.noise_filter_hz,
            config.noise_decay_seconds,
            config.noise_bias,
            config.volume,
        ];
        for (param, value) in self.params.iter_mut().zip(values) {
            param.set_target(finite(value, param.target()));
        }
        self.pending_body_mode = config.body_mode;
        self.pending_noise_mode = config.noise_mode;
        self.seed = config.seed;
    }

    pub fn parameter_normalized(&self, index: usize) -> Option<f32> {
        let p = self.params.get(index)?;
        Some((p.target() - p.min) / (p.max - p.min))
    }

    pub fn set_parameter_normalized(&mut self, index: usize, value: f32) {
        if let Some(param) = self.params.get_mut(index) {
            param.set_normalized(finite(value, 0.0));
        }
    }

    pub fn set_body_mode(&mut self, mode: TwinCorePercBodyMode) {
        self.pending_body_mode = mode;
    }

    pub fn set_noise_mode(&mut self, mode: TwinCorePercNoiseMode) {
        self.pending_noise_mode = mode;
    }

    pub fn set_ring_limit_secs(&mut self, limit: Option<f32>) {
        self.ring_limit_secs = limit.map(|v| finite(v, 0.001).max(0.001));
    }

    pub fn set_seed(&mut self, seed: u32) {
        self.seed = seed;
        self.noise_rng = XorShift32::new(seed);
    }

    /// Trigger only the independent noise envelope, analogous to N-TRIG.
    pub fn trigger_noise_with_velocity(&mut self, time: f64, velocity: f32) {
        let velocity = finite(velocity, 0.0).clamp(0.0, 1.0);
        if velocity > 0.0 {
            self.noise_velocity = velocity;
            self.noise_trigger_time = time;
            self.active_noise_mode = self.pending_noise_mode;
            self.noise_filter.reset();
            self.active = true;
        }
    }

    pub fn reset(&mut self) {
        for core in &mut self.cores {
            core.reset();
        }
        self.exciter.reset();
        self.noise_filter.reset();
        self.folder.reset();
        self.body_armed = false;
        self.body_started = false;
        self.active = false;
    }

    fn start_body(&mut self) {
        self.exciter.set_kind(ExciterKind::Pulse);
        self.exciter.set_width_ms(0.75);
        self.exciter
            .trigger(0.9 * self.velocity, self.noise_rng.next_u32() ^ self.seed);
        self.body_started = true;
    }

    #[inline]
    fn wavefold(sample: f32, amount: f32) -> f32 {
        let driven = sample * (1.0 + amount * 7.0);
        let folded = ((driven + 1.0).rem_euclid(4.0) - 2.0).abs() - 1.0;
        sample * (1.0 - amount) + folded * amount
    }
}

impl Instrument for TwinCorePercVoice {
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        let velocity = finite(velocity, 0.0).clamp(0.0, 1.0);
        if velocity == 0.0 {
            return;
        }
        self.velocity = velocity;
        self.noise_velocity = velocity;
        self.body_trigger_time = time;
        self.noise_trigger_time = time;
        self.body_armed = true;
        self.body_started = false;
        self.active = true;
        self.active_body_mode = self.pending_body_mode;
        self.active_noise_mode = self.pending_noise_mode;
        self.noise_filter.reset();
        self.folder.reset();
        if self.params[6].target() <= 0.0 {
            self.start_body();
        }
    }

    fn tick(&mut self, time: f64) -> f32 {
        if !self.active {
            return 0.0;
        }
        let body_total = (time - self.body_trigger_time).max(0.0) as f32;
        let noise_elapsed = (time - self.noise_trigger_time).max(0.0) as f32;
        if self
            .ring_limit_secs
            .is_some_and(|limit| body_total >= limit && noise_elapsed >= limit)
        {
            self.reset();
            return 0.0;
        }
        let values: [f32; 12] = std::array::from_fn(|i| self.params[i].tick());
        let delay = values[6];
        if self.body_armed && !self.body_started && body_total >= delay {
            self.start_body();
        }
        let body_elapsed = (body_total - delay).max(0.0);
        let fm_env = (-body_elapsed * 6.91 / values[4]).exp();
        let master = self
            .midi_note
            .map(|note| 440.0 * 2.0_f32.powf((note as f32 - 69.0) / 12.0))
            .unwrap_or(values[0]);
        let pitch = (master * 2.0_f32.powf(values[5] * fm_env)).clamp(20.0, self.sample_rate * 0.4);
        let second_pitch = (pitch * 2.0_f32.powf(-values[1])).max(20.0);
        for (core, hz) in self.cores.iter_mut().zip([pitch, second_pitch]) {
            core.set_frequency(hz);
            core.set_decay_time(values[2] * (0.8 + values[3].max(0.0) * 2.2));
        }

        let noise_env = (-noise_elapsed * 6.91 / values[9]).exp();
        let noise_gain = (values[10] + 1.0) * 0.5;
        self.noise_filter.set_params(values[8], 0.707);
        let (noise_low, _, noise_high) =
            self.noise_filter.process_all(self.noise_rng.next_bipolar());
        let noise = match self.active_noise_mode {
            TwinCorePercNoiseMode::Lowpass | TwinCorePercNoiseMode::Body => noise_low,
            TwinCorePercNoiseMode::Highpass => noise_high,
        } * noise_env
            * noise_gain
            * self.noise_velocity.sqrt();

        let transient = if self.body_started {
            self.exciter.tick()
        } else {
            0.0
        };
        let body_noise = if self.active_noise_mode == TwinCorePercNoiseMode::Body {
            noise * 0.7
        } else {
            0.0
        };
        let core1 = self.cores[0].process(transient + body_noise);
        let core2 = self.cores[1].process(core1);
        let spectral = match self.active_body_mode {
            TwinCorePercBodyMode::Low => core2,
            TwinCorePercBodyMode::Mid => {
                0.55 * self.cores[0].bandpass() + 0.75 * self.cores[1].bandpass()
            }
            TwinCorePercBodyMode::High => self.cores[1].bandpass() - 0.45 * core2,
        };
        let harmonics = values[7];
        let folded = self
            .folder
            .process(spectral, |sample| Self::wavefold(sample, harmonics));
        let body_env = (-body_elapsed * 6.91 / values[2]).exp();
        let bias = values[3];
        let body_vca = if self.body_started {
            (body_env * (1.0 + bias.max(0.0) * 1.5) + bias.max(0.0) * 0.08) * (1.0 + bias.min(0.0))
        } else {
            0.0
        };
        let direct_noise = if self.active_noise_mode == TwinCorePercNoiseMode::Body {
            0.0
        } else {
            noise
        };
        let output = (folded * body_vca + direct_noise) * values[11] * self.velocity.sqrt();
        let body_finished = !self.body_armed || body_total > delay + values[2] * 2.0;
        if body_finished
            && noise_elapsed > values[9] * 2.0
            && self.cores.iter().all(Resonator::is_quiet)
        {
            self.active = false;
        }
        if output.is_finite() {
            output.clamp(-4.0, 4.0)
        } else {
            self.reset();
            0.0
        }
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn set_midi_note(&mut self, note: u8) {
        self.midi_note = Some(note);
    }

    fn as_modulatable(&mut self) -> Option<&mut dyn Modulatable> {
        Some(self)
    }
}

impl Modulatable for TwinCorePercVoice {
    fn modulatable_parameters(&self) -> Vec<&'static str> {
        vec![
            "tune",
            "detune",
            "length",
            "body_bias",
            "fm_decay",
            "fm_depth",
            "trigger_delay",
            "harmonics",
            "noise_filter",
            "noise_decay",
            "noise_bias",
            "volume",
        ]
    }

    fn apply_modulation(&mut self, parameter: &str, value: f32) -> Result<(), String> {
        if let Some(index) = self
            .modulatable_parameters()
            .iter()
            .position(|candidate| *candidate == parameter)
        {
            self.set_parameter_normalized(index, value);
            Ok(())
        } else {
            Err(format!("Unknown TwinCorePercVoice parameter: {parameter}"))
        }
    }

    fn parameter_range(&self, parameter: &str) -> Option<(f32, f32)> {
        self.modulatable_parameters()
            .iter()
            .position(|candidate| *candidate == parameter)
            .map(|_| (0.0, 1.0))
    }
}
