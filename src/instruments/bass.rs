use crate::effects::waveshaper::Waveshaper;
use crate::envelope::{ADSRConfig, Envelope, EnvelopeCurve};
use crate::filters::StateVariableFilterTpt;
use crate::gen::polyblep::{polyblep_saw, polyblep_square};
use crate::utils::{Blendable, SmoothedParam, DEFAULT_SMOOTH_TIME_MS};
use std::f64::consts::TAU;

/// Normalization ranges for bass synth parameters.
/// All external-facing parameters use 0.0-1.0 normalized values.
pub(crate) mod ranges {
    /// Frequency: 0-1 maps to 30-200 Hz
    pub const FREQ_MIN: f32 = 30.0;
    pub const FREQ_MAX: f32 = 200.0;

    /// Detune amount: 0-1 maps to 0-30 cents
    pub const DETUNE_MIN: f32 = 0.0;
    pub const DETUNE_MAX: f32 = 30.0;

    /// Filter cutoff: 0-1 maps exponentially to 20-18000 Hz
    pub const FILTER_CUTOFF_MIN: f32 = 20.0;
    pub const FILTER_CUTOFF_MAX: f32 = 18000.0;

    /// Filter resonance: 0-1 maps to 0.5-15.0 Q
    pub const FILTER_RES_MIN: f32 = 0.5;
    pub const FILTER_RES_MAX: f32 = 15.0;

    /// Filter envelope decay: 0-1 maps to 0.01-2.0 seconds
    pub const FILTER_ENV_DECAY_MIN: f32 = 0.01;
    pub const FILTER_ENV_DECAY_MAX: f32 = 2.0;

    /// Filter envelope curve: 0-1 maps to 0.1-8.0
    pub const FILTER_ENV_CURVE_MIN: f32 = 0.1;
    pub const FILTER_ENV_CURVE_MAX: f32 = 8.0;

    /// Amp decay: 0-1 maps to 0.05-4.0 seconds
    pub const AMP_DECAY_MIN: f32 = 0.05;
    pub const AMP_DECAY_MAX: f32 = 4.0;

    /// Amp decay curve: 0-1 maps to 0.1-10.0
    pub const AMP_DECAY_CURVE_MIN: f32 = 0.1;
    pub const AMP_DECAY_CURVE_MAX: f32 = 10.0;

    /// Linear denormalization: 0-1 to [min, max]
    #[inline]
    pub fn denormalize(normalized: f32, min: f32, max: f32) -> f32 {
        min + normalized.clamp(0.0, 1.0) * (max - min)
    }

    /// Exponential denormalization for frequency-domain params (cutoff).
    /// Maps 0-1 to [min, max] with exponential curve.
    #[inline]
    pub fn exp_denormalize(normalized: f32, min: f32, max: f32) -> f32 {
        min * (max / min).powf(normalized.clamp(0.0, 1.0))
    }

    #[inline]
    #[allow(dead_code)]
    pub fn normalize(value: f32, min: f32, max: f32) -> f32 {
        ((value - min) / (max - min)).clamp(0.0, 1.0)
    }
}

/// Static configuration for bass synth presets.
/// All parameters use normalized 0.0-1.0 values.
#[derive(Clone, Copy, Debug)]
pub struct BassConfig {
    pub frequency: f32,          // Base frequency (0-1 -> 30-200 Hz)
    pub sub_level: f32,          // Sub sine level (0-1)
    pub osc_level: f32,          // Main saw/square level (0-1)
    pub detune_level: f32,       // Detuned layer level (0-1)
    pub detune_amount: f32,      // Detune spread (0-1 -> 0-30 cents)
    pub osc_shape: f32,          // Saw(0) to Square(1) morph (0-1)
    pub filter_cutoff: f32,      // SVF lowpass cutoff (0-1 -> 20-18000 Hz exp)
    pub filter_resonance: f32,   // SVF Q (0-1 -> 0.5-15.0)
    pub filter_env_amount: f32,  // Filter envelope depth (0-1)
    pub filter_env_decay: f32,   // Filter envelope decay (0-1 -> 0.01-2.0s)
    pub filter_env_curve: f32,   // Filter envelope curve (0-1 -> 0.1-8.0)
    pub amp_decay: f32,          // Amplitude decay (0-1 -> 0.05-4.0s)
    pub amp_decay_curve: f32,    // Amp curve shape (0-1 -> 0.1-10.0)
    pub overdrive: f32,          // Pre-filter saturation (0-1)
    pub volume: f32,             // Master volume (0-1)
}

impl BassConfig {
    pub fn new(
        frequency: f32,
        sub_level: f32,
        osc_level: f32,
        detune_level: f32,
        detune_amount: f32,
        osc_shape: f32,
        filter_cutoff: f32,
        filter_resonance: f32,
        filter_env_amount: f32,
        filter_env_decay: f32,
        filter_env_curve: f32,
        amp_decay: f32,
        amp_decay_curve: f32,
        overdrive: f32,
        volume: f32,
    ) -> Self {
        Self {
            frequency: frequency.clamp(0.0, 1.0),
            sub_level: sub_level.clamp(0.0, 1.0),
            osc_level: osc_level.clamp(0.0, 1.0),
            detune_level: detune_level.clamp(0.0, 1.0),
            detune_amount: detune_amount.clamp(0.0, 1.0),
            osc_shape: osc_shape.clamp(0.0, 1.0),
            filter_cutoff: filter_cutoff.clamp(0.0, 1.0),
            filter_resonance: filter_resonance.clamp(0.0, 1.0),
            filter_env_amount: filter_env_amount.clamp(0.0, 1.0),
            filter_env_decay: filter_env_decay.clamp(0.0, 1.0),
            filter_env_curve: filter_env_curve.clamp(0.0, 1.0),
            amp_decay: amp_decay.clamp(0.0, 1.0),
            amp_decay_curve: amp_decay_curve.clamp(0.0, 1.0),
            overdrive: overdrive.clamp(0.0, 1.0),
            volume: volume.clamp(0.0, 1.0),
        }
    }

    // Denormalization helpers

    #[inline]
    pub fn frequency_hz(&self) -> f32 {
        ranges::denormalize(self.frequency, ranges::FREQ_MIN, ranges::FREQ_MAX)
    }

    #[inline]
    pub fn detune_cents(&self) -> f32 {
        ranges::denormalize(self.detune_amount, ranges::DETUNE_MIN, ranges::DETUNE_MAX)
    }

    #[inline]
    pub fn filter_cutoff_hz(&self) -> f32 {
        ranges::exp_denormalize(
            self.filter_cutoff,
            ranges::FILTER_CUTOFF_MIN,
            ranges::FILTER_CUTOFF_MAX,
        )
    }

    #[inline]
    pub fn filter_resonance_q(&self) -> f32 {
        ranges::denormalize(
            self.filter_resonance,
            ranges::FILTER_RES_MIN,
            ranges::FILTER_RES_MAX,
        )
    }

    #[inline]
    pub fn filter_env_decay_secs(&self) -> f32 {
        ranges::denormalize(
            self.filter_env_decay,
            ranges::FILTER_ENV_DECAY_MIN,
            ranges::FILTER_ENV_DECAY_MAX,
        )
    }

    #[inline]
    pub fn filter_env_curve_value(&self) -> f32 {
        ranges::denormalize(
            self.filter_env_curve,
            ranges::FILTER_ENV_CURVE_MIN,
            ranges::FILTER_ENV_CURVE_MAX,
        )
    }

    #[inline]
    pub fn amp_decay_secs(&self) -> f32 {
        ranges::denormalize(self.amp_decay, ranges::AMP_DECAY_MIN, ranges::AMP_DECAY_MAX)
    }

    #[inline]
    pub fn amp_decay_curve_value(&self) -> f32 {
        ranges::denormalize(
            self.amp_decay_curve,
            ranges::AMP_DECAY_CURVE_MIN,
            ranges::AMP_DECAY_CURVE_MAX,
        )
    }

    pub fn default() -> Self {
        Self::acid()
    }

    /// Acid -- TB-303-style: high resonance, short filter sweep, saw-heavy
    pub fn acid() -> Self {
        Self::new(
            0.24, // frequency (~71 Hz, ~C#2)
            0.40, // sub_level
            0.80, // osc_level
            0.00, // detune_level
            0.00, // detune_amount
            0.10, // osc_shape (mostly saw)
            0.15, // filter_cutoff (low base)
            0.70, // filter_resonance (screamy)
            0.85, // filter_env_amount (big sweep)
            0.15, // filter_env_decay (short)
            0.08, // filter_env_curve (punchy)
            0.35, // amp_decay
            0.10, // amp_decay_curve
            0.30, // overdrive
            0.80, // volume
        )
    }

    /// Sub -- clean sub-bass: sine dominant, open filter, long decay
    pub fn sub() -> Self {
        Self::new(
            0.18, // frequency (~60 Hz)
            1.00, // sub_level (dominant)
            0.15, // osc_level (subtle harmonics)
            0.00, // detune_level
            0.00, // detune_amount
            0.00, // osc_shape (saw for mild harmonics)
            0.70, // filter_cutoff (open)
            0.05, // filter_resonance (flat)
            0.10, // filter_env_amount (minimal)
            0.30, // filter_env_decay
            0.20, // filter_env_curve
            0.60, // amp_decay (long)
            0.15, // amp_decay_curve
            0.00, // overdrive (clean)
            0.85, // volume
        )
    }

    /// Reese -- two detuned saws, moderate filter, heavy overdrive for growl
    pub fn reese() -> Self {
        Self::new(
            0.18, // frequency (~60 Hz)
            0.30, // sub_level
            0.80, // osc_level
            0.80, // detune_level (strong second layer)
            0.50, // detune_amount (~15 cents)
            0.05, // osc_shape (saw)
            0.35, // filter_cutoff
            0.30, // filter_resonance
            0.50, // filter_env_amount
            0.40, // filter_env_decay
            0.15, // filter_env_curve
            0.55, // amp_decay
            0.12, // amp_decay_curve
            0.60, // overdrive (heavy)
            0.80, // volume
        )
    }

    /// Stab -- square wave, sharp filter env, short decay
    pub fn stab() -> Self {
        Self::new(
            0.30, // frequency (~81 Hz)
            0.20, // sub_level
            0.90, // osc_level
            0.00, // detune_level
            0.00, // detune_amount
            0.90, // osc_shape (mostly square)
            0.20, // filter_cutoff
            0.40, // filter_resonance
            0.90, // filter_env_amount (big sweep)
            0.08, // filter_env_decay (very short)
            0.05, // filter_env_curve (snappy)
            0.20, // amp_decay (short, staccato)
            0.08, // amp_decay_curve
            0.20, // overdrive
            0.80, // volume
        )
    }
}

impl Blendable for BassConfig {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let inv_t = 1.0 - t;
        Self {
            frequency: self.frequency * inv_t + other.frequency * t,
            sub_level: self.sub_level * inv_t + other.sub_level * t,
            osc_level: self.osc_level * inv_t + other.osc_level * t,
            detune_level: self.detune_level * inv_t + other.detune_level * t,
            detune_amount: self.detune_amount * inv_t + other.detune_amount * t,
            osc_shape: self.osc_shape * inv_t + other.osc_shape * t,
            filter_cutoff: self.filter_cutoff * inv_t + other.filter_cutoff * t,
            filter_resonance: self.filter_resonance * inv_t + other.filter_resonance * t,
            filter_env_amount: self.filter_env_amount * inv_t + other.filter_env_amount * t,
            filter_env_decay: self.filter_env_decay * inv_t + other.filter_env_decay * t,
            filter_env_curve: self.filter_env_curve * inv_t + other.filter_env_curve * t,
            amp_decay: self.amp_decay * inv_t + other.amp_decay * t,
            amp_decay_curve: self.amp_decay_curve * inv_t + other.amp_decay_curve * t,
            overdrive: self.overdrive * inv_t + other.overdrive * t,
            volume: self.volume * inv_t + other.volume * t,
        }
    }
}

/// Smoothed parameters for real-time control of the bass synth.
/// All parameters use normalized 0-1 ranges for external interface.
pub struct BassParams {
    pub frequency: SmoothedParam,
    pub sub_level: SmoothedParam,
    pub osc_level: SmoothedParam,
    pub detune_level: SmoothedParam,
    pub detune_amount: SmoothedParam,
    pub osc_shape: SmoothedParam,
    pub filter_cutoff: SmoothedParam,
    pub filter_resonance: SmoothedParam,
    pub filter_env_amount: SmoothedParam,
    pub filter_env_decay: SmoothedParam,
    pub filter_env_curve: SmoothedParam,
    pub amp_decay: SmoothedParam,
    pub amp_decay_curve: SmoothedParam,
    pub overdrive: SmoothedParam,
    pub volume: SmoothedParam,
}

impl BassParams {
    pub fn from_config(config: &BassConfig, sample_rate: f32) -> Self {
        Self {
            frequency: SmoothedParam::new(
                config.frequency,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            sub_level: SmoothedParam::new(
                config.sub_level,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            osc_level: SmoothedParam::new(
                config.osc_level,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            detune_level: SmoothedParam::new(
                config.detune_level,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            detune_amount: SmoothedParam::new(
                config.detune_amount,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            osc_shape: SmoothedParam::new(
                config.osc_shape,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            filter_cutoff: SmoothedParam::new(
                config.filter_cutoff,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            filter_resonance: SmoothedParam::new(
                config.filter_resonance,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            filter_env_amount: SmoothedParam::new(
                config.filter_env_amount,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            filter_env_decay: SmoothedParam::new(
                config.filter_env_decay,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            filter_env_curve: SmoothedParam::new(
                config.filter_env_curve,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            amp_decay: SmoothedParam::new(
                config.amp_decay,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            amp_decay_curve: SmoothedParam::new(
                config.amp_decay_curve,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            overdrive: SmoothedParam::new(
                config.overdrive,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            volume: SmoothedParam::new(
                config.volume,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
        }
    }

    #[inline]
    pub fn tick(&mut self) {
        self.frequency.tick();
        self.sub_level.tick();
        self.osc_level.tick();
        self.detune_level.tick();
        self.detune_amount.tick();
        self.osc_shape.tick();
        self.filter_cutoff.tick();
        self.filter_resonance.tick();
        self.filter_env_amount.tick();
        self.filter_env_decay.tick();
        self.filter_env_curve.tick();
        self.amp_decay.tick();
        self.amp_decay_curve.tick();
        self.overdrive.tick();
        self.volume.tick();
    }

    pub fn snap_all(&mut self) {
        self.frequency.snap();
        self.sub_level.snap();
        self.osc_level.snap();
        self.detune_level.snap();
        self.detune_amount.snap();
        self.osc_shape.snap();
        self.filter_cutoff.snap();
        self.filter_resonance.snap();
        self.filter_env_amount.snap();
        self.filter_env_decay.snap();
        self.filter_env_curve.snap();
        self.amp_decay.snap();
        self.amp_decay_curve.snap();
        self.overdrive.snap();
        self.volume.snap();
    }

    pub fn to_config(&self) -> BassConfig {
        BassConfig {
            frequency: self.frequency.get(),
            sub_level: self.sub_level.get(),
            osc_level: self.osc_level.get(),
            detune_level: self.detune_level.get(),
            detune_amount: self.detune_amount.get(),
            osc_shape: self.osc_shape.get(),
            filter_cutoff: self.filter_cutoff.get(),
            filter_resonance: self.filter_resonance.get(),
            filter_env_amount: self.filter_env_amount.get(),
            filter_env_decay: self.filter_env_decay.get(),
            filter_env_curve: self.filter_env_curve.get(),
            amp_decay: self.amp_decay.get(),
            amp_decay_curve: self.amp_decay_curve.get(),
            overdrive: self.overdrive.get(),
            volume: self.volume.get(),
        }
    }

    // Denormalization helpers for audio processing

    #[inline]
    pub fn frequency_hz(&self) -> f32 {
        ranges::denormalize(self.frequency.get(), ranges::FREQ_MIN, ranges::FREQ_MAX)
    }

    #[inline]
    pub fn detune_cents(&self) -> f32 {
        ranges::denormalize(
            self.detune_amount.get(),
            ranges::DETUNE_MIN,
            ranges::DETUNE_MAX,
        )
    }

    #[inline]
    pub fn filter_cutoff_hz(&self) -> f32 {
        ranges::exp_denormalize(
            self.filter_cutoff.get(),
            ranges::FILTER_CUTOFF_MIN,
            ranges::FILTER_CUTOFF_MAX,
        )
    }

    #[inline]
    pub fn filter_resonance_q(&self) -> f32 {
        ranges::denormalize(
            self.filter_resonance.get(),
            ranges::FILTER_RES_MIN,
            ranges::FILTER_RES_MAX,
        )
    }

    #[inline]
    pub fn filter_env_decay_secs(&self) -> f32 {
        ranges::denormalize(
            self.filter_env_decay.get(),
            ranges::FILTER_ENV_DECAY_MIN,
            ranges::FILTER_ENV_DECAY_MAX,
        )
    }

    #[inline]
    pub fn filter_env_curve_value(&self) -> f32 {
        ranges::denormalize(
            self.filter_env_curve.get(),
            ranges::FILTER_ENV_CURVE_MIN,
            ranges::FILTER_ENV_CURVE_MAX,
        )
    }

    #[inline]
    pub fn amp_decay_secs(&self) -> f32 {
        ranges::denormalize(
            self.amp_decay.get(),
            ranges::AMP_DECAY_MIN,
            ranges::AMP_DECAY_MAX,
        )
    }

    #[inline]
    pub fn amp_decay_curve_value(&self) -> f32 {
        ranges::denormalize(
            self.amp_decay_curve.get(),
            ranges::AMP_DECAY_CURVE_MIN,
            ranges::AMP_DECAY_CURVE_MAX,
        )
    }
}

pub struct BassSynth {
    pub sample_rate: f32,
    pub params: BassParams,

    // Phase accumulators (f64 to avoid pitch drift)
    sub_phase: f64,
    osc_phase: f64,
    detune_phase: f64,

    // Filter (TPT SVF for stability at high resonance)
    filter: StateVariableFilterTpt,

    // Envelopes
    amp_envelope: Envelope,
    filter_envelope: Envelope,

    // Saturation
    waveshaper: Waveshaper,

    // State
    is_active: bool,
    current_velocity: f32,

    // Frequency snapshot frozen at trigger time
    triggered_frequency: f32,
}

impl BassSynth {
    pub fn new(sample_rate: f32) -> Self {
        let config = BassConfig::default();
        Self::with_config(sample_rate, config)
    }

    pub fn with_config(sample_rate: f32, config: BassConfig) -> Self {
        let params = BassParams::from_config(&config, sample_rate);

        Self {
            sample_rate,
            params,
            sub_phase: 0.0,
            osc_phase: 0.0,
            detune_phase: 0.0,
            filter: StateVariableFilterTpt::new(sample_rate, config.filter_cutoff_hz(), config.filter_resonance_q()),
            amp_envelope: Envelope::new(),
            filter_envelope: Envelope::new(),
            waveshaper: Waveshaper::new(config.overdrive, 1.0),
            is_active: false,
            current_velocity: 1.0,
            triggered_frequency: config.frequency_hz(),
        }
    }

    pub fn set_config(&mut self, config: BassConfig) {
        self.params.frequency.set_target(config.frequency);
        self.params.sub_level.set_target(config.sub_level);
        self.params.osc_level.set_target(config.osc_level);
        self.params.detune_level.set_target(config.detune_level);
        self.params.detune_amount.set_target(config.detune_amount);
        self.params.osc_shape.set_target(config.osc_shape);
        self.params.filter_cutoff.set_target(config.filter_cutoff);
        self.params.filter_resonance.set_target(config.filter_resonance);
        self.params.filter_env_amount.set_target(config.filter_env_amount);
        self.params.filter_env_decay.set_target(config.filter_env_decay);
        self.params.filter_env_curve.set_target(config.filter_env_curve);
        self.params.amp_decay.set_target(config.amp_decay);
        self.params.amp_decay_curve.set_target(config.amp_decay_curve);
        self.params.overdrive.set_target(config.overdrive);
        self.params.volume.set_target(config.volume);
    }

    pub fn snap_params(&mut self) {
        self.params.snap_all();
    }

    // Individual parameter setters (normalized 0-1)

    pub fn set_frequency(&mut self, value: f32) {
        self.params.frequency.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_sub_level(&mut self, value: f32) {
        self.params.sub_level.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_osc_level(&mut self, value: f32) {
        self.params.osc_level.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_detune_level(&mut self, value: f32) {
        self.params.detune_level.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_detune_amount(&mut self, value: f32) {
        self.params.detune_amount.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_osc_shape(&mut self, value: f32) {
        self.params.osc_shape.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_filter_cutoff(&mut self, value: f32) {
        self.params.filter_cutoff.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_filter_resonance(&mut self, value: f32) {
        self.params
            .filter_resonance
            .set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_filter_env_amount(&mut self, value: f32) {
        self.params
            .filter_env_amount
            .set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_filter_env_decay(&mut self, value: f32) {
        self.params
            .filter_env_decay
            .set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_filter_env_curve(&mut self, value: f32) {
        self.params
            .filter_env_curve
            .set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_amp_decay(&mut self, value: f32) {
        self.params.amp_decay.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_amp_decay_curve(&mut self, value: f32) {
        self.params
            .amp_decay_curve
            .set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_overdrive(&mut self, value: f32) {
        self.params.overdrive.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_volume(&mut self, value: f32) {
        self.params.volume.set_target(value.clamp(0.0, 1.0));
    }
}

impl crate::engine::Instrument for BassSynth {
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        self.current_velocity = velocity.clamp(0.0, 1.0);
        self.is_active = true;

        // Reset phase accumulators
        self.sub_phase = 0.0;
        self.osc_phase = 0.0;
        self.detune_phase = 0.0;

        // Snapshot frequency at trigger time
        self.triggered_frequency = self.params.frequency_hz();

        // Configure amplitude envelope
        let amp_decay = self.params.amp_decay_secs();
        let amp_curve = self.params.amp_decay_curve_value();
        self.amp_envelope.set_config(ADSRConfig {
            attack_time: 0.002, // 2ms click-free attack
            decay_time: amp_decay,
            sustain_level: 0.0,
            release_time: amp_decay * 0.1,
            attack_curve: EnvelopeCurve::Linear,
            decay_curve: EnvelopeCurve::Exponential(amp_curve),
        });
        self.amp_envelope.trigger(time);

        // Configure filter envelope
        let filter_decay = self.params.filter_env_decay_secs();
        let filter_curve = self.params.filter_env_curve_value();
        self.filter_envelope.set_config(ADSRConfig {
            attack_time: 0.001, // Nearly instant attack for snappy filter
            decay_time: filter_decay,
            sustain_level: 0.0,
            release_time: filter_decay * 0.1,
            attack_curve: EnvelopeCurve::Linear,
            decay_curve: EnvelopeCurve::Exponential(filter_curve),
        });
        self.filter_envelope.trigger(time);

        // Reset filter state to avoid artifacts from previous note
        self.filter.reset();

        // Update waveshaper drive
        let overdrive = self.params.overdrive.get();
        self.waveshaper.set_drive(1.0 + overdrive * 9.0);
    }

    fn tick(&mut self, current_time: f64) -> f32 {
        // Always tick smoothed params
        self.params.tick();

        if !self.is_active {
            return 0.0;
        }

        // Read params
        let freq = self.triggered_frequency;
        let sub_level = self.params.sub_level.get();
        let osc_level = self.params.osc_level.get();
        let detune_level = self.params.detune_level.get();
        let detune_cents = self.params.detune_cents();
        let osc_shape = self.params.osc_shape.get();

        // Calculate detuned frequency
        let detune_ratio = 2.0_f32.powf(detune_cents / 1200.0);
        let detune_freq = freq * detune_ratio;

        // Phase increments
        let dt = 1.0 / self.sample_rate as f64;
        let sub_inc = freq as f64 * dt;
        let osc_inc = freq as f64 * dt;
        let detune_inc = detune_freq as f64 * dt;

        // Advance phases
        self.sub_phase += sub_inc;
        self.sub_phase -= self.sub_phase.floor();
        self.osc_phase += osc_inc;
        self.osc_phase -= self.osc_phase.floor();
        self.detune_phase += detune_inc;
        self.detune_phase -= self.detune_phase.floor();

        // Generate oscillators
        let sub_out = (self.sub_phase * TAU).sin() as f32;

        // Main osc: crossfade saw/square
        let saw_main = polyblep_saw(self.osc_phase, osc_inc);
        let square_main = polyblep_square(self.osc_phase, osc_inc);
        let osc_out = saw_main * (1.0 - osc_shape) + square_main * osc_shape;

        // Detuned osc: same shape
        let saw_det = polyblep_saw(self.detune_phase, detune_inc);
        let square_det = polyblep_square(self.detune_phase, detune_inc);
        let det_out = saw_det * (1.0 - osc_shape) + square_det * osc_shape;

        // Mix oscillator layers
        let mix = sub_out * sub_level + osc_out * osc_level + det_out * detune_level;

        // Pre-filter saturation
        let overdrive_amt = self.params.overdrive.get();
        self.waveshaper.set_drive(1.0 + overdrive_amt * 9.0);
        let saturated = if overdrive_amt > 0.001 {
            self.waveshaper.process(mix)
        } else {
            mix
        };

        // Filter with envelope modulation
        let filter_env = self.filter_envelope.get_amplitude(current_time);
        let base_cutoff = self.params.filter_cutoff_hz();
        let env_amount = self.params.filter_env_amount.get();
        // Envelope sweeps from (base + offset) down to base
        let env_offset = (ranges::FILTER_CUTOFF_MAX - base_cutoff) * env_amount * filter_env;
        let cutoff = (base_cutoff + env_offset).clamp(
            ranges::FILTER_CUTOFF_MIN,
            ranges::FILTER_CUTOFF_MAX,
        );
        let resonance = self.params.filter_resonance_q();
        self.filter.set_params(cutoff, resonance);
        let (filtered, _, _) = self.filter.process_all(saturated);

        // Amplitude envelope
        let amp_env = self.amp_envelope.get_amplitude(current_time);
        let velocity_amp = self.current_velocity.sqrt();
        let volume = self.params.volume.get();

        let output = filtered * amp_env * velocity_amp * volume;

        // Deactivate when amplitude envelope finishes
        if !self.amp_envelope.is_active {
            self.is_active = false;
        }

        output
    }

    fn is_active(&self) -> bool {
        self.is_active
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Instrument;

    #[test]
    fn test_bass_synth_produces_audio() {
        let mut bass = BassSynth::new(44100.0);
        bass.trigger_with_velocity(0.0, 1.0);

        let mut energy = 0.0_f64;
        let sample_rate = 44100.0;
        for i in 0..44100 {
            let time = i as f64 / sample_rate as f64;
            let sample = bass.tick(time) as f64;
            energy += sample * sample;
        }

        assert!(energy > 0.1, "bass synth should produce audible output");
    }

    #[test]
    fn test_bass_synth_deactivates() {
        let mut config = BassConfig::default();
        config.amp_decay = 0.01; // Very short decay
        let mut bass = BassSynth::with_config(44100.0, config);
        bass.trigger_with_velocity(0.0, 1.0);

        // Run for 2 seconds -- should deactivate well before then
        let sample_rate = 44100.0;
        for i in 0..(sample_rate as usize * 2) {
            let time = i as f64 / sample_rate as f64;
            bass.tick(time);
        }

        assert!(!bass.is_active(), "bass should deactivate after envelope finishes");
    }

    #[test]
    fn test_bass_presets_produce_different_sounds() {
        let sample_rate = 44100.0;
        let presets = [
            BassConfig::acid(),
            BassConfig::sub(),
            BassConfig::reese(),
            BassConfig::stab(),
        ];

        let mut energies = Vec::new();
        for config in &presets {
            let mut bass = BassSynth::with_config(sample_rate, *config);
            bass.trigger_with_velocity(0.0, 1.0);

            let mut energy = 0.0_f64;
            for i in 0..22050 {
                let time = i as f64 / sample_rate as f64;
                let s = bass.tick(time) as f64;
                energy += s * s;
            }
            energies.push(energy);
        }

        // All presets should produce sound
        for (i, e) in energies.iter().enumerate() {
            assert!(*e > 0.01, "preset {} should produce sound, got energy {}", i, e);
        }
    }

    #[test]
    fn test_bass_blend() {
        let acid = BassConfig::acid();
        let sub = BassConfig::sub();
        let blended = acid.lerp(&sub, 0.5);

        // Blended values should be between the two presets
        assert!(blended.frequency > acid.frequency.min(sub.frequency));
        assert!(blended.frequency < acid.frequency.max(sub.frequency));
    }
}
