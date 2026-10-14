//! A twin-core percussion voice inspired by the SSF Entity Ultra-Perc signal flow.
//!
//! This is a digital interpretation, not a circuit model or product emulation.
//! Its identity is the fixed signal order: delayed body excitation, two serial
//! resonant cores, selectable spectral emphasis, wavefolding, a body VCA, and
//! an independently enveloped noise path that can instead excite the body.
//!
//! Four extensions widen it beyond the hardware: fold symmetry for even
//! harmonics, audio-rate cross modulation from core one into core two, a
//! vactrol low-pass gate after the body VCA, and a chaos control that
//! stiffens the cores (Duffing-style) and then closes a saturating feedback
//! loop between them.

use crate::engine::{Instrument, Modulatable};
use crate::filters::{Resonator, StateVariableFilterTpt, VactrolLpg};
use crate::gen::{Exciter, ExciterKind};
use crate::utils::{Oversampler, OversamplingMode, SmoothedParam, XorShift32};

const SMOOTH_MS: f32 = 15.0;
const PARAM_COUNT: usize = 16;
/// Fold offset at full symmetry, after the harmonics drive gain. One unit is a
/// quarter of the fold period, the most asymmetric point.
const SYMMETRY_OFFSET: f32 = 1.0;
/// Corner of the DC trap that follows an asymmetric fold.
const SYMMETRY_DC_HZ: f32 = 8.0;
/// Core two deviation, in octaves, at full cross modulation.
const CROSS_MOD_OCTAVES: f32 = 2.0;
/// Core level at a 200 Hz reference reaching about three quarters of the
/// cross-mod deviation.
const XMOD_DRIVE: f32 = 6.0;
/// A struck core's level rises with its pitch; dividing by pitch over this
/// reference keeps chaos and cross mod consistent from kicks to bells.
const LEVEL_REFERENCE_HZ: f32 = 200.0;
/// Cubic stiffness reached at chaos 0.5 and held above it.
const DUFFING_STIFFNESS: f32 = 12.0;
/// Core two band-pass to core one feedback gain at full chaos.
const CHAOS_FEEDBACK: f32 = 1.5;
/// The vactrol release follows Length so the gate never outlasts the body.
const LPG_FALL_PER_LENGTH: f32 = 0.03;

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
    /// -1..=1 fold offset; non-zero values add even harmonics.
    pub symmetry: f32,
    /// 0..=1 audio-rate frequency modulation of core two by core one.
    pub cross_mod: f32,
    /// 0..=1 blend of a vactrol low-pass gate after the body VCA.
    pub lpg: f32,
    /// 0..=0.5 stiffens the cores; 0.5..=1 adds core-to-core feedback.
    pub chaos: f32,
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
            symmetry: 0.0,
            cross_mod: 0.0,
            lpg: 0.0,
            chaos: 0.0,
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
    lpg: VactrolLpg,
    params: [SmoothedParam; PARAM_COUNT],
    /// Channel pitch offset; independent of the indexed tune parameter.
    channel_tuning: SmoothedParam,
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
    /// Previous low-pass output of each core, for chaos and cross modulation.
    core_prev: [f32; 2],
    /// Previous core-two band-pass output; DC-free, so the chaos loop cannot latch.
    core2_band_prev: f32,
    dc_coeff: f32,
    dc_x1: f32,
    dc_y1: f32,
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
            lpg: VactrolLpg::new(sr),
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
                SmoothedParam::new(config.symmetry, -1.0, 1.0, sr, SMOOTH_MS),
                SmoothedParam::new_normalized(config.cross_mod, sr),
                SmoothedParam::new_normalized(config.lpg, sr),
                SmoothedParam::new_normalized(config.chaos, sr),
            ],
            channel_tuning: SmoothedParam::new_normalized(0.5, sr),
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
            core_prev: [0.0; 2],
            core2_band_prev: 0.0,
            dc_coeff: 1.0 - std::f32::consts::TAU * SYMMETRY_DC_HZ / sr,
            dc_x1: 0.0,
            dc_y1: 0.0,
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
            symmetry: self.params[12].target(),
            cross_mod: self.params[13].target(),
            lpg: self.params[14].target(),
            chaos: self.params[15].target(),
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
            config.symmetry,
            config.cross_mod,
            config.lpg,
            config.chaos,
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
        Some(if Self::exponential_parameter(index) {
            (p.target() / p.min).ln() / (p.max / p.min).ln()
        } else {
            (p.target() - p.min) / (p.max - p.min)
        })
    }

    pub fn set_parameter_normalized(&mut self, index: usize, value: f32) {
        if let Some(param) = self.params.get_mut(index) {
            if !value.is_finite() {
                return;
            }
            let normalized = value.clamp(0.0, 1.0);
            if Self::exponential_parameter(index) {
                param.set_target(param.min * (param.max / param.min).powf(normalized));
            } else {
                param.set_normalized(normalized);
            }
        }
    }

    /// Tune, length, FM decay, noise filter, and noise decay use logarithmic
    /// slider spacing while retaining the physical bounds in `params`.
    fn exponential_parameter(index: usize) -> bool {
        matches!(index, 0 | 2 | 4 | 8 | 9)
    }

    /// 0.5 is neutral; endpoints shift the base frequency by one octave.
    pub fn set_channel_tuning(&mut self, value: f32) {
        if value.is_finite() {
            self.channel_tuning.set_target(value);
        }
    }

    pub fn channel_tuning(&self) -> f32 {
        self.channel_tuning.target()
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
        self.lpg.reset();
        self.core_prev = [0.0; 2];
        self.core2_band_prev = 0.0;
        self.dc_x1 = 0.0;
        self.dc_y1 = 0.0;
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
    fn fold(driven: f32) -> f32 {
        ((driven + 1.0).rem_euclid(4.0) - 2.0).abs() - 1.0
    }

    /// Triangle fold blended with the dry signal. `offset` shifts the fold
    /// point for even harmonics; the static part of that shift is subtracted.
    #[inline]
    fn wavefold(sample: f32, amount: f32, offset: f32) -> f32 {
        let folded = Self::fold(sample * (1.0 + amount * 7.0) + offset) - Self::fold(offset);
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
        let values: [f32; PARAM_COUNT] = std::array::from_fn(|i| self.params[i].tick());
        let [symmetry, cross_mod, lpg_amount, chaos] =
            [values[12], values[13], values[14], values[15]];
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
        let master = master * 2.0_f32.powf((self.channel_tuning.tick() - 0.5) * 2.0);
        let pitch = (master * 2.0_f32.powf(values[5] * fm_env)).clamp(20.0, self.sample_rate * 0.4);
        let second_pitch = (pitch * 2.0_f32.powf(-values[1])).max(20.0);
        let core_t60 = values[2] * (0.8 + values[3].max(0.0) * 2.2);
        let stiffness = (chaos * 2.0).min(1.0) * DUFFING_STIFFNESS;
        let mut core_hz = [pitch, second_pitch];
        let level = [
            self.core_prev[0] * LEVEL_REFERENCE_HZ / pitch,
            self.core_prev[1] * LEVEL_REFERENCE_HZ / second_pitch,
        ];
        if cross_mod > 0.0 {
            core_hz[1] *=
                2.0_f32.powf(CROSS_MOD_OCTAVES * cross_mod * (XMOD_DRIVE * level[0]).tanh());
        }
        for ((core, hz), base_hz) in self
            .cores
            .iter_mut()
            .zip(core_hz)
            .zip([pitch, second_pitch])
        {
            if cross_mod > 0.0 {
                // Keep damping on the unmodulated pitch so Length stays put.
                core.set_frequency(hz.clamp(20.0, self.sample_rate * 0.4));
                core.set_decay_time_for(core_t60, base_hz);
            } else {
                core.set_frequency(hz);
                core.set_decay_time(core_t60);
            }
            // A hardening spring: loud motion rings higher, then glides down.
            core.set_stiffness(stiffness, LEVEL_REFERENCE_HZ / base_hz);
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
        let body_env = (-body_elapsed * 6.91 / values[2]).exp();
        let feedback = ((chaos - 0.5) * 2.0).max(0.0);
        let chaos_input = if feedback > 0.0 && self.body_started {
            // Gate the loop with the body envelope so every hit still ends.
            let band = self.core2_band_prev * LEVEL_REFERENCE_HZ / second_pitch;
            feedback * body_env * CHAOS_FEEDBACK * (3.0 * band).tanh()
        } else {
            0.0
        };
        let core1 = self.cores[0].process(transient + body_noise + chaos_input);
        let core2 = self.cores[1].process(core1);
        self.core_prev = [core1, core2];
        self.core2_band_prev = self.cores[1].bandpass();
        let spectral = match self.active_body_mode {
            TwinCorePercBodyMode::Low => core2,
            TwinCorePercBodyMode::Mid => {
                0.55 * self.cores[0].bandpass() + 0.75 * self.cores[1].bandpass()
            }
            TwinCorePercBodyMode::High => self.cores[1].bandpass() - 0.45 * core2,
        };
        let harmonics = values[7];
        let offset = symmetry * SYMMETRY_OFFSET;
        let mut folded = self
            .folder
            .process(spectral, |sample| Self::wavefold(sample, harmonics, offset));
        if offset != 0.0 && harmonics > 0.0 {
            // An asymmetric fold of a decaying tone leaves a moving DC offset.
            let blocked = folded - self.dc_x1 + self.dc_coeff * self.dc_y1;
            self.dc_x1 = folded;
            self.dc_y1 = blocked;
            folded = blocked;
        }
        let bias = values[3];
        let body_vca = if self.body_started {
            (body_env * (1.0 + bias.max(0.0) * 1.5) + bias.max(0.0) * 0.08) * (1.0 + bias.min(0.0))
        } else {
            0.0
        };
        let mut body = folded * body_vca;
        if lpg_amount > 0.0 {
            let control = if self.body_started { body_env } else { 0.0 };
            self.lpg
                .set_fall_seconds((values[2] * LPG_FALL_PER_LENGTH).min(0.25));
            // After the VCA, so Length keeps its meaning. The cubic drive puts
            // the gate's sweep in the audible part of the envelope.
            let gated = self.lpg.process(body, control * control * control);
            body += (gated - body) * lpg_amount;
        }
        let direct_noise = if self.active_noise_mode == TwinCorePercNoiseMode::Body {
            0.0
        } else {
            noise
        };
        let output = (body + direct_noise) * values[11] * self.velocity.sqrt();
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
            "symmetry",
            "cross_mod",
            "lpg",
            "chaos",
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

#[cfg(test)]
mod channel_mode_tests {
    use super::*;

    #[test]
    fn selected_modes_latch_on_the_next_trigger() {
        let mut voice = TwinCorePercVoice::new(44_100.0);
        assert_eq!(voice.active_body_mode, TwinCorePercBodyMode::Low);
        assert_eq!(voice.active_noise_mode, TwinCorePercNoiseMode::Lowpass);
        voice.set_body_mode(TwinCorePercBodyMode::High);
        voice.set_noise_mode(TwinCorePercNoiseMode::Body);
        assert_eq!(voice.active_body_mode, TwinCorePercBodyMode::Low);
        assert_eq!(voice.active_noise_mode, TwinCorePercNoiseMode::Lowpass);
        voice.trigger_with_velocity(0.0, 1.0);
        assert_eq!(voice.active_body_mode, TwinCorePercBodyMode::High);
        assert_eq!(voice.active_noise_mode, TwinCorePercNoiseMode::Body);
    }
}
