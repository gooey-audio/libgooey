use crate::effects::waveshaper::Waveshaper;
use crate::envelope::{ADSRConfig, Envelope, EnvelopeCurve};
use crate::filters::{ResonantHighpassFilter, ResonantLowpassFilter};
use crate::gen::oscillator::Oscillator;
use crate::gen::pink_noise::PinkNoise;
use crate::gen::waveform::Waveform;
use crate::instruments::fm_snap::PhaseModulator;
use crate::utils::{Blendable, SmoothedParam, DEFAULT_SMOOTH_TIME_MS};

/// Normalization ranges for kick drum parameters
/// All external-facing parameters use 0.0-1.0 normalized values
pub(crate) mod ranges {
    /// Frequency: 0-1 maps to 30-120 Hz
    pub const FREQ_MIN: f32 = 30.0;
    pub const FREQ_MAX: f32 = 120.0;

    /// Oscillator decay: 0-1 maps to 0.01-4.0 seconds
    pub const OSC_DECAY_MIN: f32 = 0.01;
    pub const OSC_DECAY_MAX: f32 = 4.0;

    /// Pitch envelope curve: 0-1 maps to 0.1-4.0
    pub const PITCH_CURVE_MIN: f32 = 0.1;
    pub const PITCH_CURVE_MAX: f32 = 4.0;

    /// Pitch start ratio: 0-1 maps to 1.0-10.0x frequency multiplier
    pub const PITCH_RATIO_MIN: f32 = 1.0;
    pub const PITCH_RATIO_MAX: f32 = 10.0;

    /// Noise cutoff: 0-1 maps to 20-10000 Hz
    pub const NOISE_CUTOFF_MIN: f32 = 20.0;
    pub const NOISE_CUTOFF_MAX: f32 = 10000.0;

    /// Noise resonance: 0-1 maps to 0.0-5.0
    pub const NOISE_RES_MIN: f32 = 0.0;
    pub const NOISE_RES_MAX: f32 = 5.0;

    /// Amp decay: 0-1 maps to 0.0-4.0 seconds
    pub const AMP_DECAY_MIN: f32 = 0.0;
    pub const AMP_DECAY_MAX: f32 = 4.0;

    /// Amp decay curve: 0-1 maps to 0.1-10.0
    pub const AMP_DECAY_CURVE_MIN: f32 = 0.1;
    pub const AMP_DECAY_CURVE_MAX: f32 = 10.0;

    /// Map normalized 0-1 value to actual range
    #[inline]
    pub fn denormalize(normalized: f32, min: f32, max: f32) -> f32 {
        min + normalized.clamp(0.0, 1.0) * (max - min)
    }

    /// Map actual value to normalized 0-1 range
    #[inline]
    #[allow(dead_code)]
    pub fn normalize(value: f32, min: f32, max: f32) -> f32 {
        ((value - min) / (max - min)).clamp(0.0, 1.0)
    }
}

/// Static configuration for kick drum presets
/// All parameters use normalized 0.0-1.0 values for easy integration with external systems.
/// Use the `ranges` module to convert to/from actual values.
#[derive(Clone, Copy, Debug)]
pub struct KickConfig {
    pub frequency: f32,              // Base frequency (0-1 → 30-120Hz)
    pub punch_amount: f32,           // Mid-frequency presence (0.0-1.0)
    pub sub_amount: f32,             // Sub-bass presence (0.0-1.0)
    pub click_amount: f32,           // High-frequency click (0.0-1.0)
    pub oscillator_decay: f32,       // Oscillator decay time (0-1 → 0.01-4.0s)
    pub pitch_envelope_amount: f32,  // Frequency sweep amount (0.0-1.0)
    pub pitch_envelope_curve: f32,   // Pitch envelope decay curve (0-1 → 0.1-4.0)
    pub volume: f32,                 // Overall volume (0.0-1.0)
    pub pitch_start_ratio: f32,      // Starting pitch multiplier (0-1 → 1.0-10.0x)
    pub phase_mod_amount: f32,       // Phase modulation depth (0.0-1.0, 0 = disabled)
    pub noise_amount: f32,           // Pink noise layer amount (0.0-1.0)
    pub noise_cutoff: f32,           // Noise lowpass filter cutoff (0-1 → 20-10000Hz)
    pub noise_resonance: f32,        // Noise lowpass filter resonance (0-1 → 0.0-5.0)
    pub overdrive_amount: f32,       // Overdrive/saturation amount (0.0-1.0, 0.0 = bypass)
    // Master amplitude envelope parameters
    // Note: amp_attack is hardcoded to instant (0.001s) for kick transients
    pub amp_decay: f32,              // Amplitude decay time (0-1 → 0.0-4.0s)
    pub amp_decay_curve: f32,        // Decay curve (0-1 → 0.1-10.0, <0.5 = natural decay)
}

impl KickConfig {
    /// Create a new KickConfig with normalized 0-1 parameters.
    /// All parameters are clamped to 0.0-1.0 range.
    pub fn new(
        frequency: f32,            // 0-1 → 30-120 Hz
        punch_amount: f32,         // 0-1
        sub_amount: f32,           // 0-1
        click_amount: f32,         // 0-1
        oscillator_decay: f32,     // 0-1 → 0.01-4.0s
        pitch_envelope_amount: f32, // 0-1
        pitch_envelope_curve: f32, // 0-1 → 0.1-4.0
        volume: f32,               // 0-1
    ) -> Self {
        Self {
            frequency: frequency.clamp(0.0, 1.0),
            punch_amount: punch_amount.clamp(0.0, 1.0),
            sub_amount: sub_amount.clamp(0.0, 1.0),
            click_amount: click_amount.clamp(0.0, 1.0),
            oscillator_decay: oscillator_decay.clamp(0.0, 1.0),
            pitch_envelope_amount: pitch_envelope_amount.clamp(0.0, 1.0),
            pitch_envelope_curve: pitch_envelope_curve.clamp(0.0, 1.0),
            volume: volume.clamp(0.0, 1.0),
            // Defaults for additional parameters (normalized)
            pitch_start_ratio: 0.222, // ~3.0x (default pitch ratio)
            phase_mod_amount: 0.0,    // Disabled by default
            noise_amount: 0.0,
            noise_cutoff: 0.198,      // ~2000 Hz
            noise_resonance: 0.2,     // ~2.0
            overdrive_amount: 0.0,
            amp_decay: 0.125,         // ~0.5s
            amp_decay_curve: 0.091,   // ~1.0 (linear)
        }
    }

    /// Create a KickConfig with all parameters (all normalized 0-1)
    pub fn new_full(
        frequency: f32,
        punch_amount: f32,
        sub_amount: f32,
        click_amount: f32,
        oscillator_decay: f32,
        pitch_envelope_amount: f32,
        pitch_envelope_curve: f32,
        volume: f32,
        pitch_start_ratio: f32,
        phase_mod_amount: f32,
        noise_amount: f32,
        noise_cutoff: f32,
        noise_resonance: f32,
        overdrive_amount: f32,
        amp_decay: f32,
        amp_decay_curve: f32,
    ) -> Self {
        Self {
            frequency: frequency.clamp(0.0, 1.0),
            punch_amount: punch_amount.clamp(0.0, 1.0),
            sub_amount: sub_amount.clamp(0.0, 1.0),
            click_amount: click_amount.clamp(0.0, 1.0),
            oscillator_decay: oscillator_decay.clamp(0.0, 1.0),
            pitch_envelope_amount: pitch_envelope_amount.clamp(0.0, 1.0),
            pitch_envelope_curve: pitch_envelope_curve.clamp(0.0, 1.0),
            volume: volume.clamp(0.0, 1.0),
            pitch_start_ratio: pitch_start_ratio.clamp(0.0, 1.0),
            phase_mod_amount: phase_mod_amount.clamp(0.0, 1.0),
            noise_amount: noise_amount.clamp(0.0, 1.0),
            noise_cutoff: noise_cutoff.clamp(0.0, 1.0),
            noise_resonance: noise_resonance.clamp(0.0, 1.0),
            overdrive_amount: overdrive_amount.clamp(0.0, 1.0),
            amp_decay: amp_decay.clamp(0.0, 1.0),
            amp_decay_curve: amp_decay_curve.clamp(0.0, 1.0),
        }
    }

    // Helper methods to get actual (denormalized) values for audio processing

    /// Get actual frequency in Hz (30-120)
    #[inline]
    pub fn frequency_hz(&self) -> f32 {
        ranges::denormalize(self.frequency, ranges::FREQ_MIN, ranges::FREQ_MAX)
    }

    /// Get actual oscillator decay in seconds (0.01-4.0)
    #[inline]
    pub fn oscillator_decay_secs(&self) -> f32 {
        ranges::denormalize(self.oscillator_decay, ranges::OSC_DECAY_MIN, ranges::OSC_DECAY_MAX)
    }

    /// Get actual pitch envelope curve (0.1-4.0)
    #[inline]
    pub fn pitch_envelope_curve_value(&self) -> f32 {
        ranges::denormalize(self.pitch_envelope_curve, ranges::PITCH_CURVE_MIN, ranges::PITCH_CURVE_MAX)
    }

    /// Get actual pitch start ratio (1.0-10.0)
    #[inline]
    pub fn pitch_start_ratio_value(&self) -> f32 {
        ranges::denormalize(self.pitch_start_ratio, ranges::PITCH_RATIO_MIN, ranges::PITCH_RATIO_MAX)
    }

    /// Get actual noise cutoff in Hz (20-10000)
    #[inline]
    pub fn noise_cutoff_hz(&self) -> f32 {
        ranges::denormalize(self.noise_cutoff, ranges::NOISE_CUTOFF_MIN, ranges::NOISE_CUTOFF_MAX)
    }

    /// Get actual noise resonance (0.0-5.0)
    #[inline]
    pub fn noise_resonance_value(&self) -> f32 {
        ranges::denormalize(self.noise_resonance, ranges::NOISE_RES_MIN, ranges::NOISE_RES_MAX)
    }

    /// Get actual amp decay in seconds (0.0-4.0)
    #[inline]
    pub fn amp_decay_secs(&self) -> f32 {
        ranges::denormalize(self.amp_decay, ranges::AMP_DECAY_MIN, ranges::AMP_DECAY_MAX)
    }

    /// Get actual amp decay curve (0.1-10.0)
    #[inline]
    pub fn amp_decay_curve_value(&self) -> f32 {
        ranges::denormalize(self.amp_decay_curve, ranges::AMP_DECAY_CURVE_MIN, ranges::AMP_DECAY_CURVE_MAX)
    }

    pub fn default() -> Self {
        Self::tight()
    }

    /// Tight - Short, punchy kick with strong pitch envelope
    pub fn tight() -> Self {
        Self::new_full(
            0.22,  // frequency
            0.00,  // punch
            1.00,  // sub
            0.00,  // click
            0.12,  // osc_decay
            0.70,  // pitch_env_amt
            0.01,  // pitch_env_crv
            0.85,  // volume
            0.64,  // pitch_ratio
            1.00,  // phase_mod_amt
            0.07,  // noise_amount
            0.01,  // noise_cutoff
            0.02,  // noise_res
            0.20,  // overdrive
            0.12,  // amp_decay
            0.02,  // amp_dcy_crv
        )
    }

    /// Punch - Mid-focused with click and resonant noise
    pub fn punch() -> Self {
        Self::new_full(
            0.50,  // frequency
            0.20,  // punch
            1.00,  // sub
            0.20,  // click
            0.12,  // osc_decay
            0.60,  // pitch_env_amt
            0.10,  // pitch_env_crv
            0.85,  // volume
            0.24,  // pitch_ratio
            1.00,  // phase_mod_amt
            0.07,  // noise_amount
            0.11,  // noise_cutoff
            0.42,  // noise_res
            0.20,  // overdrive
            0.12,  // amp_decay
            0.02,  // amp_dcy_crv
        )
    }

    /// Loose - Longer decay, more punch, subtle pitch envelope
    pub fn loose() -> Self {
        Self::new_full(
            0.32,  // frequency
            0.40,  // punch
            1.00,  // sub
            0.00,  // click
            0.62,  // osc_decay
            0.20,  // pitch_env_amt
            0.12,  // pitch_env_crv
            0.85,  // volume
            0.84,  // pitch_ratio
            1.00,  // phase_mod_amt
            0.07,  // noise_amount
            0.01,  // noise_cutoff
            0.02,  // noise_res
            0.30,  // overdrive
            0.12,  // amp_decay
            0.12,  // amp_dcy_crv
        )
    }

    /// Dirt - Higher frequency, more noise with high resonance
    pub fn dirt() -> Self {
        Self::new_full(
            0.62,  // frequency
            0.10,  // punch
            1.00,  // sub
            0.10,  // click
            0.10,  // osc_decay
            0.60,  // pitch_env_amt
            0.10,  // pitch_env_crv
            0.85,  // volume
            0.44,  // pitch_ratio
            1.00,  // phase_mod_amt
            0.20,  // noise_amount
            0.10,  // noise_cutoff
            0.82,  // noise_res
            0.20,  // overdrive
            0.10,  // amp_decay
            0.10,  // amp_dcy_crv
        )
    }
}

impl Blendable for KickConfig {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let inv_t = 1.0 - t;
        Self {
            frequency: self.frequency * inv_t + other.frequency * t,
            punch_amount: self.punch_amount * inv_t + other.punch_amount * t,
            sub_amount: self.sub_amount * inv_t + other.sub_amount * t,
            click_amount: self.click_amount * inv_t + other.click_amount * t,
            oscillator_decay: self.oscillator_decay * inv_t + other.oscillator_decay * t,
            pitch_envelope_amount: self.pitch_envelope_amount * inv_t + other.pitch_envelope_amount * t,
            pitch_envelope_curve: self.pitch_envelope_curve * inv_t + other.pitch_envelope_curve * t,
            volume: self.volume * inv_t + other.volume * t,
            pitch_start_ratio: self.pitch_start_ratio * inv_t + other.pitch_start_ratio * t,
            phase_mod_amount: self.phase_mod_amount * inv_t + other.phase_mod_amount * t,
            noise_amount: self.noise_amount * inv_t + other.noise_amount * t,
            noise_cutoff: self.noise_cutoff * inv_t + other.noise_cutoff * t,
            noise_resonance: self.noise_resonance * inv_t + other.noise_resonance * t,
            overdrive_amount: self.overdrive_amount * inv_t + other.overdrive_amount * t,
            amp_decay: self.amp_decay * inv_t + other.amp_decay * t,
            amp_decay_curve: self.amp_decay_curve * inv_t + other.amp_decay_curve * t,
        }
    }
}

/// Smoothed parameters for real-time control of the kick drum
/// These use one-pole smoothing to prevent clicks/pops during parameter changes
/// All parameters use normalized 0-1 ranges for external interface
pub struct KickParams {
    pub frequency: SmoothedParam,            // Base frequency (0-1 → 30-120 Hz)
    pub punch: SmoothedParam,                // Mid-frequency presence (0-1)
    pub sub: SmoothedParam,                  // Sub-bass presence (0-1)
    pub click: SmoothedParam,                // High-frequency click (0-1)
    pub oscillator_decay: SmoothedParam,     // Decay time (0-1 → 0.01-4.0s)
    pub pitch_envelope_amount: SmoothedParam, // Pitch envelope amount (0-1)
    pub pitch_envelope_curve: SmoothedParam, // Pitch envelope curve (0-1 → 0.1-4.0)
    pub volume: SmoothedParam,               // Overall volume (0-1)
    pub pitch_start_ratio: SmoothedParam,    // Starting pitch multiplier (0-1 → 1.0-10.0)
    pub phase_mod_amount: SmoothedParam,     // Phase modulation depth (0-1, 0 = disabled)
    pub noise_amount: SmoothedParam,         // Pink noise layer amount (0-1)
    pub noise_cutoff: SmoothedParam,         // Noise filter cutoff (0-1 → 20-10000 Hz)
    pub noise_resonance: SmoothedParam,      // Noise filter resonance (0-1 → 0.0-5.0)
    pub overdrive: SmoothedParam,            // Overdrive/saturation amount (0-1, 0 = bypass)
    // Master amplitude envelope parameters (amp_attack hardcoded to instant)
    pub amp_decay: SmoothedParam,            // Amplitude decay time (0-1 → 0.0-4.0s)
    pub amp_decay_curve: SmoothedParam,      // Decay curve (0-1 → 0.1-10.0)
}

impl KickParams {
    /// Create new smoothed parameters from a config
    /// All parameters use normalized 0-1 range for external interface
    pub fn from_config(config: &KickConfig, sample_rate: f32) -> Self {
        Self {
            frequency: SmoothedParam::new(
                config.frequency,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            punch: SmoothedParam::new(
                config.punch_amount,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            sub: SmoothedParam::new(
                config.sub_amount,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            click: SmoothedParam::new(
                config.click_amount,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            oscillator_decay: SmoothedParam::new(
                config.oscillator_decay,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            pitch_envelope_amount: SmoothedParam::new(
                config.pitch_envelope_amount,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            pitch_envelope_curve: SmoothedParam::new(
                config.pitch_envelope_curve,
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
            pitch_start_ratio: SmoothedParam::new(
                config.pitch_start_ratio,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            phase_mod_amount: SmoothedParam::new(
                config.phase_mod_amount,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            noise_amount: SmoothedParam::new(
                config.noise_amount,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            noise_cutoff: SmoothedParam::new(
                config.noise_cutoff,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            noise_resonance: SmoothedParam::new(
                config.noise_resonance,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            overdrive: SmoothedParam::new(
                config.overdrive_amount,
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
        }
    }

    /// Tick all smoothers and return whether any are still smoothing
    #[inline]
    pub fn tick(&mut self) -> bool {
        self.frequency.tick();
        self.punch.tick();
        self.sub.tick();
        self.click.tick();
        self.oscillator_decay.tick();
        self.pitch_envelope_amount.tick();
        self.pitch_envelope_curve.tick();
        self.volume.tick();
        self.pitch_start_ratio.tick();
        self.phase_mod_amount.tick();
        self.noise_amount.tick();
        self.noise_cutoff.tick();
        self.noise_resonance.tick();
        self.overdrive.tick();
        self.amp_decay.tick();
        self.amp_decay_curve.tick();

        // Return true if any smoother is still active
        !self.is_settled()
    }

    /// Check if all parameters have settled
    pub fn is_settled(&self) -> bool {
        self.frequency.is_settled()
            && self.punch.is_settled()
            && self.sub.is_settled()
            && self.click.is_settled()
            && self.oscillator_decay.is_settled()
            && self.pitch_envelope_amount.is_settled()
            && self.pitch_envelope_curve.is_settled()
            && self.volume.is_settled()
            && self.pitch_start_ratio.is_settled()
            && self.phase_mod_amount.is_settled()
            && self.noise_amount.is_settled()
            && self.noise_cutoff.is_settled()
            && self.noise_resonance.is_settled()
            && self.overdrive.is_settled()
            && self.amp_decay.is_settled()
            && self.amp_decay_curve.is_settled()
    }

    /// Get a snapshot of current normalized values as a KickConfig
    pub fn to_config(&self) -> KickConfig {
        KickConfig {
            frequency: self.frequency.get(),
            punch_amount: self.punch.get(),
            sub_amount: self.sub.get(),
            click_amount: self.click.get(),
            oscillator_decay: self.oscillator_decay.get(),
            pitch_envelope_amount: self.pitch_envelope_amount.get(),
            pitch_envelope_curve: self.pitch_envelope_curve.get(),
            volume: self.volume.get(),
            pitch_start_ratio: self.pitch_start_ratio.get(),
            phase_mod_amount: self.phase_mod_amount.get(),
            noise_amount: self.noise_amount.get(),
            noise_cutoff: self.noise_cutoff.get(),
            noise_resonance: self.noise_resonance.get(),
            overdrive_amount: self.overdrive.get(),
            amp_decay: self.amp_decay.get(),
            amp_decay_curve: self.amp_decay_curve.get(),
        }
    }

    // Helper methods to get actual (denormalized) values for audio processing

    /// Get actual frequency in Hz (30-120)
    #[inline]
    pub fn frequency_hz(&self) -> f32 {
        ranges::denormalize(self.frequency.get(), ranges::FREQ_MIN, ranges::FREQ_MAX)
    }

    /// Get actual oscillator decay in seconds (0.01-4.0)
    #[inline]
    pub fn oscillator_decay_secs(&self) -> f32 {
        ranges::denormalize(self.oscillator_decay.get(), ranges::OSC_DECAY_MIN, ranges::OSC_DECAY_MAX)
    }

    /// Get actual pitch envelope curve (0.1-4.0)
    #[inline]
    pub fn pitch_envelope_curve_value(&self) -> f32 {
        ranges::denormalize(self.pitch_envelope_curve.get(), ranges::PITCH_CURVE_MIN, ranges::PITCH_CURVE_MAX)
    }

    /// Get actual pitch start ratio (1.0-10.0)
    #[inline]
    pub fn pitch_start_ratio_value(&self) -> f32 {
        ranges::denormalize(self.pitch_start_ratio.get(), ranges::PITCH_RATIO_MIN, ranges::PITCH_RATIO_MAX)
    }

    /// Get actual noise cutoff in Hz (20-10000)
    #[inline]
    pub fn noise_cutoff_hz(&self) -> f32 {
        ranges::denormalize(self.noise_cutoff.get(), ranges::NOISE_CUTOFF_MIN, ranges::NOISE_CUTOFF_MAX)
    }

    /// Get actual noise resonance (0.0-5.0)
    #[inline]
    pub fn noise_resonance_value(&self) -> f32 {
        ranges::denormalize(self.noise_resonance.get(), ranges::NOISE_RES_MIN, ranges::NOISE_RES_MAX)
    }

    /// Get actual amp decay in seconds (0.0-4.0)
    #[inline]
    pub fn amp_decay_secs(&self) -> f32 {
        ranges::denormalize(self.amp_decay.get(), ranges::AMP_DECAY_MIN, ranges::AMP_DECAY_MAX)
    }

    /// Get actual amp decay curve (0.1-10.0)
    #[inline]
    pub fn amp_decay_curve_value(&self) -> f32 {
        ranges::denormalize(self.amp_decay_curve.get(), ranges::AMP_DECAY_CURVE_MIN, ranges::AMP_DECAY_CURVE_MAX)
    }
}

pub struct KickDrum {
    pub sample_rate: f32,

    /// Smoothed parameters for click-free real-time control
    pub params: KickParams,

    // Three oscillators for different frequency ranges
    pub sub_oscillator: Oscillator,   // Sub-bass (fundamental)
    pub punch_oscillator: Oscillator, // Mid-range punch
    pub click_oscillator: Oscillator, // High-frequency click

    // Pitch envelope for frequency sweeping
    pub pitch_envelope: Envelope,
    /// Pitch start multiplier snapshot (frozen at trigger time)
    triggered_pitch_multiplier: f32,
    /// Base frequency snapshot in Hz (frozen at trigger time)
    triggered_frequency: f32,

    // High-pass filter for click oscillator
    pub click_filter: ResonantHighpassFilter,

    // DS Kick-style phase modulator for transient snap
    pub phase_modulator: PhaseModulator,

    // Pink noise layer with resonant lowpass filter (DS Kick-style)
    pub pink_noise: PinkNoise,
    pub noise_filter: ResonantLowpassFilter,
    pub noise_envelope: Envelope,

    // Overdrive/saturation effect (Max MSP overdrive~ style)
    pub waveshaper: Waveshaper,

    // Master amplitude envelope (DS Kick "p curvey" style)
    // Applied multiplicatively on top of oscillator envelopes
    pub amplitude_envelope: Envelope,

    pub is_active: bool,

    // Velocity-responsive state
    /// Current trigger velocity (0.0-1.0), set on trigger
    current_velocity: f32,

    // Velocity scaling configuration
    /// How much velocity affects decay time (0.0-1.0)
    /// Higher values = more velocity sensitivity (shorter decay at high velocity)
    velocity_to_decay: f32,

    /// How much velocity affects pitch envelope decay (0.0-1.0)
    /// Higher velocity = faster pitch decay (sharper, more aggressive pitch drop)
    /// Lower velocity = slower pitch decay (gentler, more subtle pitch sweep)
    velocity_to_pitch: f32,
}

impl KickDrum {
    pub fn new(sample_rate: f32) -> Self {
        let config = KickConfig::default();
        Self::with_config(sample_rate, config)
    }

    pub fn with_config(sample_rate: f32, config: KickConfig) -> Self {
        let params = KickParams::from_config(&config, sample_rate);

        // Get actual Hz values for oscillators
        let freq_hz = config.frequency_hz();
        let noise_cutoff_hz = config.noise_cutoff_hz();
        let noise_res = config.noise_resonance_value();

        // Calculate initial pitch start multiplier from pitch_start_ratio and pitch_envelope
        let pitch_start_ratio_actual = config.pitch_start_ratio_value();
        let triggered_pitch_multiplier =
            1.0 + (pitch_start_ratio_actual - 1.0) * config.pitch_envelope_amount;

        let mut kick = Self {
            sample_rate,
            params,
            sub_oscillator: Oscillator::new(sample_rate, freq_hz),
            punch_oscillator: Oscillator::new(sample_rate, freq_hz * 2.5),
            click_oscillator: Oscillator::new(sample_rate, freq_hz * 40.0),
            pitch_envelope: Envelope::new(),
            triggered_pitch_multiplier,
            triggered_frequency: freq_hz,
            click_filter: ResonantHighpassFilter::new(sample_rate, 8000.0, 4.0),
            phase_modulator: PhaseModulator::new(sample_rate),
            pink_noise: PinkNoise::new(),
            noise_filter: ResonantLowpassFilter::new(sample_rate, noise_cutoff_hz, noise_res),
            noise_envelope: Envelope::new(),
            waveshaper: Waveshaper::new(config.overdrive_amount, 1.0), // Full wet mix
            amplitude_envelope: Envelope::new(),
            is_active: false,

            // Initialize velocity state
            current_velocity: 1.0,

            // Velocity scaling: 0.5 means velocity can reduce decay by up to 50%
            // (higher velocity = shorter, tighter decay)
            velocity_to_decay: 0.5,

            // Pitch velocity scaling: 0.7 gives strong pitch response to velocity
            // Higher velocity = sharper/faster pitch drop
            // Lower velocity = gentler/slower pitch sweep
            velocity_to_pitch: 0.7,
        };

        kick.configure_oscillators();
        kick
    }

    /// Configure oscillators from current smoothed parameter values
    /// Called once at initialization and when decay changes significantly
    fn configure_oscillators(&mut self) {
        // Get actual decay time in seconds from normalized value
        let decay = self.params.oscillator_decay_secs();

        // Sub oscillator: Deep sine wave
        self.sub_oscillator.waveform = Waveform::Sine;
        self.sub_oscillator.set_adsr(ADSRConfig::new(
            0.001,       // Very fast attack
            decay,       // Synchronized decay time
            0.0,         // No sustain
            decay * 0.2, // Synchronized release
        ));

        // Punch oscillator: Triangle for mid-range impact
        self.punch_oscillator.waveform = Waveform::Triangle;
        self.punch_oscillator.set_adsr(ADSRConfig::new(
            0.001,       // Very fast attack
            decay,       // Synchronized decay time
            0.0,         // No sustain
            decay * 0.2, // Synchronized release
        ));

        // Click oscillator: High-frequency filtered noise transient
        self.click_oscillator.waveform = Waveform::Noise;
        self.click_oscillator.set_adsr(ADSRConfig::new(
            0.001,        // Very fast attack
            decay * 0.2,  // Much shorter decay for click
            0.0,          // No sustain
            decay * 0.02, // Extremely short release
        ));

        // Pitch envelope: Fast attack, shorter decay to settle before amplitude
        // Pitch envelope uses 60% of amplitude decay to prevent "phantom pitch" artifacts
        let pitch_decay = decay * 0.6;
        self.pitch_envelope.set_config(ADSRConfig::new(
            0.001,        // Instant attack
            pitch_decay,  // Shorter than amplitude decay
            0.0,          // Drop to base frequency
            pitch_decay * 0.1, // Very short release
        ));

        // Noise envelope: Synchronized with amplitude envelope for consistent body
        self.noise_envelope.set_config(ADSRConfig::new(
            0.001,       // Very fast attack
            decay,       // Synchronized decay time
            0.0,         // No sustain
            decay * 0.2, // Synchronized release
        ));
    }

    /// Apply current smoothed parameters to oscillators (called per-sample)
    ///
    /// NOTE: Only mix/volume parameters are applied here. Pitch-related parameters
    /// (frequency, pitch_start_multiplier) are frozen at trigger time to prevent
    /// discontinuities when parameters change during decay.
    #[inline]
    fn apply_params(&mut self) {
        let punch = self.params.punch.get();
        let sub = self.params.sub.get();
        let click = self.params.click.get();
        let volume = self.params.volume.get();

        // Light velocity scaling for click: range [0.6, 1.0]
        // Higher velocity = more click, lower velocity = less click
        let click_vel_scale = 0.6 + 0.4 * self.current_velocity;

        // Update oscillator volumes (these can change smoothly without pops)
        self.sub_oscillator.set_volume(sub * volume);
        self.punch_oscillator.set_volume(punch * volume * 0.7);
        // Click reduced from 0.3 to 0.15, with velocity scaling
        self.click_oscillator
            .set_volume(click * volume * 0.15 * click_vel_scale);
    }

    pub fn set_config(&mut self, config: KickConfig) {
        // Set all parameter targets (normalized 0-1 values)
        // These will smoothly transition via SmoothedParam
        self.params.frequency.set_target(config.frequency);
        self.params.punch.set_target(config.punch_amount);
        self.params.sub.set_target(config.sub_amount);
        self.params.click.set_target(config.click_amount);
        self.params.oscillator_decay.set_target(config.oscillator_decay);
        self.params.pitch_envelope_amount.set_target(config.pitch_envelope_amount);
        self.params.pitch_envelope_curve.set_target(config.pitch_envelope_curve);
        self.params.volume.set_target(config.volume);
        self.params.pitch_start_ratio.set_target(config.pitch_start_ratio);
        self.params.phase_mod_amount.set_target(config.phase_mod_amount);
        self.params.noise_amount.set_target(config.noise_amount);
        self.params.noise_cutoff.set_target(config.noise_cutoff);
        self.params.noise_resonance.set_target(config.noise_resonance);
        self.params.overdrive.set_target(config.overdrive_amount);
        self.params.amp_decay.set_target(config.amp_decay);
        self.params.amp_decay_curve.set_target(config.amp_decay_curve);

        // NOTE: We intentionally do NOT call configure_oscillators() here.
        // Envelope configurations (decay times, curves) are applied on trigger,
        // not during parameter changes. This prevents pops/discontinuities when
        // parameters change while a sound is still decaying.
        // Smoothable params (volume, filter, mix) update in real-time via apply_params().
    }

    /// Get current config snapshot (reads current smoothed values)
    pub fn config(&self) -> KickConfig {
        self.params.to_config()
    }

    /// Trigger at full velocity (convenience method)
    pub fn trigger(&mut self, time: f32) {
        self.trigger_with_velocity(time, 0.5);
    }

    /// Trigger with velocity (0.0-1.0)
    ///
    /// Velocity affects the amplitude envelope decay time:
    /// - Higher velocity = shorter decay (tighter, punchier sound)
    /// - Lower velocity = longer decay (deeper, more sustained sound)
    pub fn trigger_with_velocity(&mut self, time: f32, velocity: f32) {
        self.current_velocity = velocity.clamp(0.0, 1.0);
        self.is_active = true;

        let vel = self.current_velocity;

        // Quadratic curve for natural acoustic-like response
        let vel_squared = vel * vel;

        // --- Decay time scaling ---
        // Higher velocity = shorter decay (tighter, punchier sound)
        // Scale factor: 1.0 at vel=0, down to 0.5 at vel=1 (50% reduction)
        let decay_scale = 1.0 - (self.velocity_to_decay * vel_squared);

        // --- Pitch envelope scaling ---
        // Higher velocity = faster/sharper pitch decay (more aggressive pitch drop)
        // Lower velocity = slower pitch decay (gentler, more subtle sweep)
        // Use a more aggressive scaling for pitch to make high velocity hits snappy
        // NOTE: Currently unused - pitch envelope duration matches amplitude to prevent pops
        let _pitch_decay_scale = 1.0 - (self.velocity_to_pitch * vel_squared);

        // Get base parameters (denormalized to actual values)
        let base_decay = self.params.oscillator_decay_secs() * decay_scale;
        let base_freq = self.params.frequency_hz();

        // Snapshot pitch parameters at trigger time
        // These values are frozen for the entire decay to prevent discontinuities
        self.triggered_frequency = base_freq;
        let pitch_envelope_amount = self.params.pitch_envelope_amount.get();
        let pitch_start_ratio = self.params.pitch_start_ratio_value();
        self.triggered_pitch_multiplier = 1.0 + (pitch_start_ratio - 1.0) * pitch_envelope_amount;

        // Configure pitch envelope with same duration as amplitude envelope
        // The exponential curve will make the pitch sweep complete early,
        // but the envelope stays active (at sustain=0) to prevent artifacts
        // High velocity = short pitch decay (sharp, punchy attack)
        // Low velocity = long pitch decay (smooth, subtle pitch sweep)
        let pitch_curve_value = self.params.pitch_envelope_curve_value();
        let decay_curve = if (pitch_curve_value - 1.0).abs() < 0.01 {
            // Close enough to 1.0 = use linear for efficiency
            EnvelopeCurve::Linear
        } else {
            EnvelopeCurve::Exponential(pitch_curve_value)
        };

        // CRITICAL: Pitch envelope must have same total duration as amplitude envelope
        // to prevent phase discontinuities and pops at the end
        // The exponential curve ensures pitch sweep completes early (within ~60% of decay)
        // while the envelope stays active to keep frequency stable
        self.pitch_envelope.set_config(
            ADSRConfig::new(0.001, base_decay, 0.0, base_decay * 0.2)
                .with_decay_curve(decay_curve),
        );

        // Configure amplitude envelopes with velocity-scaled decay
        self.sub_oscillator
            .set_adsr(ADSRConfig::new(0.001, base_decay, 0.0, base_decay * 0.2));
        self.punch_oscillator
            .set_adsr(ADSRConfig::new(0.001, base_decay, 0.0, base_decay * 0.2));
        self.click_oscillator.set_adsr(ADSRConfig::new(
            0.001,
            base_decay * 0.2, // Click always shorter
            0.0,
            base_decay * 0.02,
        ));

        // Update base frequencies
        self.sub_oscillator.frequency_hz = base_freq;
        self.punch_oscillator.frequency_hz = base_freq * 2.5;
        self.click_oscillator.frequency_hz = base_freq * 40.0;

        // Trigger all oscillators
        self.sub_oscillator.trigger(time);
        self.punch_oscillator.trigger(time);
        self.click_oscillator.trigger(time);

        // Trigger pitch envelope
        self.pitch_envelope.trigger(time);

        // Trigger phase modulator if enabled (amount > 0, DS Kick-style transient)
        if self.params.phase_mod_amount.get() > 0.001 {
            self.phase_modulator.trigger(time);
        }

        // Configure and trigger noise envelope with velocity-scaled decay
        self.noise_envelope.set_config(ADSRConfig::new(
            0.001,
            base_decay,
            0.0,
            base_decay * 0.2,
        ));
        self.noise_envelope.trigger(time);

        // Configure and trigger master amplitude envelope (DS Kick "p curvey" style)
        // This is applied multiplicatively on top of oscillator envelopes
        // amp_attack is hardcoded to instant (0.001s) for kick drum transients
        const AMP_ATTACK: f32 = 0.001;
        const AMP_ATTACK_CURVE: f32 = 0.5; // Fast rise
        let amp_decay = self.params.amp_decay_secs() * decay_scale; // Velocity scales decay
        let amp_decay_curve_val = self.params.amp_decay_curve_value();

        let amp_attack_curve = EnvelopeCurve::Exponential(AMP_ATTACK_CURVE);
        let amp_decay_curve = if (amp_decay_curve_val - 1.0).abs() < 0.01 {
            EnvelopeCurve::Linear
        } else {
            EnvelopeCurve::Exponential(amp_decay_curve_val)
        };

        self.amplitude_envelope.set_config(
            ADSRConfig::new(AMP_ATTACK, amp_decay, 0.0, amp_decay * 0.2)
                .with_attack_curve(amp_attack_curve)
                .with_decay_curve(amp_decay_curve),
        );
        self.amplitude_envelope.trigger(time);

        // Reset filter states for clean transients
        self.click_filter.reset();
        self.noise_filter.reset();
        self.pink_noise.reset();
    }

    pub fn release(&mut self, time: f32) {
        if self.is_active {
            self.sub_oscillator.release(time);
            self.punch_oscillator.release(time);
            self.click_oscillator.release(time);
            self.pitch_envelope.release(time);
        }
    }

    pub fn tick(&mut self, current_time: f32) -> f32 {
        // Always tick smoothers (even when not active, to settle values)
        self.params.tick();

        if !self.is_active {
            return 0.0;
        }

        // Apply smoothed parameters to oscillators (mix/volume only)
        self.apply_params();

        // Use triggered (snapshot) frequency - frozen at trigger time to prevent pitch snaps
        let base_frequency = self.triggered_frequency;

        // Calculate pitch modulation from envelope using triggered pitch multiplier
        let pitch_envelope_value = self.pitch_envelope.get_amplitude(current_time);
        let mut frequency_multiplier =
            1.0 + (self.triggered_pitch_multiplier - 1.0) * pitch_envelope_value;

        // Apply phase modulation if enabled (amount > 0, DS Kick-style transient snap)
        // This adds a brief frequency boost at the attack for extra punch
        let phase_mod_amount = self.params.phase_mod_amount.get();
        if phase_mod_amount > 0.001 {
            let phase_mod = self.phase_modulator.tick(current_time);
            // Phase mod adds brief frequency boost (multiplier of up to 3x at full amount)
            frequency_multiplier *= 1.0 + (phase_mod * phase_mod_amount * 2.0);
        }

        // Apply pitch envelope to oscillators
        self.sub_oscillator.frequency_hz = base_frequency * frequency_multiplier;
        self.punch_oscillator.frequency_hz = base_frequency * 2.5 * frequency_multiplier;

        // Click oscillator gets less pitch modulation to maintain transient character
        let click_pitch_mod = 1.0 + (frequency_multiplier - 1.0) * 0.3;
        self.click_oscillator.frequency_hz = base_frequency * 40.0 * click_pitch_mod;

        // Sum all oscillator outputs
        let sub_output = self.sub_oscillator.tick(current_time);
        let punch_output = self.punch_oscillator.tick(current_time);
        let raw_click_output = self.click_oscillator.tick(current_time);

        // Apply resonant high-pass filtering to click for more realistic sound
        let filtered_click_output = self.click_filter.process(raw_click_output);

        // Generate and process pink noise layer (DS Kick-style)
        let noise_amount = self.params.noise_amount.get();
        let noise_output = if noise_amount > 0.001 {
            // Generate pink noise sample
            let pink_noise_sample = self.pink_noise.tick();

            // Update filter parameters from smoothed params (denormalized to actual Hz/resonance)
            let noise_cutoff = self.params.noise_cutoff_hz();
            let noise_resonance = self.params.noise_resonance_value();
            self.noise_filter.set_cutoff_freq(noise_cutoff);
            self.noise_filter.set_resonance(noise_resonance);

            // Apply resonant lowpass filter
            let filtered_noise = self.noise_filter.process(pink_noise_sample);

            // Apply noise envelope
            // Scale noise_amount by 0.5 to reduce maximum volume
            let noise_env = self.noise_envelope.get_amplitude(current_time);
            filtered_noise * noise_env * noise_amount * 0.5 * self.params.volume.get()
        } else {
            0.0
        };

        let total_output = sub_output
            + punch_output
            + filtered_click_output
            + noise_output;

        // Map overdrive amount (0.0-1.0) to drive (1.0-10.0)
        // 0.0 = bypass (drive 1.0), 1.0 = maximum saturation (drive 10.0)
        let overdrive_amount = self.params.overdrive.get();
        let drive = 1.0 + (overdrive_amount * 9.0);
        self.waveshaper.set_drive(drive);

        // Apply overdrive/saturation effect (Max MSP overdrive~ style)
        let overdriven_output = self.waveshaper.process(total_output);

        // Apply master amplitude envelope (DS Kick "p curvey" style)
        // Multiplicative with existing oscillator envelopes
        let amp_env = self.amplitude_envelope.get_amplitude(current_time);

        // Apply velocity amplitude scaling (sqrt for perceptually linear loudness)
        let velocity_amplitude = self.current_velocity.sqrt();
        let final_output = overdriven_output * amp_env * velocity_amplitude;

        // Check if kick is still active
        // Master amplitude envelope controls overall activity
        if !self.amplitude_envelope.is_active {
            self.is_active = false;
        }

        final_output
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    /// Set volume (smoothed, 0-1)
    pub fn set_volume(&mut self, volume: f32) {
        self.params.volume.set_target(volume.clamp(0.0, 1.0));
    }

    /// Set base frequency (smoothed, normalized 0-1 → 30-120 Hz)
    pub fn set_frequency(&mut self, frequency: f32) {
        self.params.frequency.set_target(frequency.clamp(0.0, 1.0));
    }

    /// Set oscillator decay time (smoothed, normalized 0-1 → 0.01-4.0s)
    pub fn set_oscillator_decay(&mut self, decay: f32) {
        self.params.oscillator_decay.set_target(decay.clamp(0.0, 1.0));
    }

    /// Set punch amount (smoothed, 0-1)
    pub fn set_punch(&mut self, punch_amount: f32) {
        self.params.punch.set_target(punch_amount.clamp(0.0, 1.0));
    }

    /// Set sub amount (smoothed, 0-1)
    pub fn set_sub(&mut self, sub_amount: f32) {
        self.params.sub.set_target(sub_amount.clamp(0.0, 1.0));
    }

    /// Set click amount (smoothed, 0-1)
    pub fn set_click(&mut self, click_amount: f32) {
        self.params.click.set_target(click_amount.clamp(0.0, 1.0));
    }

    /// Set pitch envelope amount (smoothed, 0-1)
    pub fn set_pitch_envelope_amount(&mut self, amount: f32) {
        self.params.pitch_envelope_amount.set_target(amount.clamp(0.0, 1.0));
    }

    /// Set pitch envelope curve (smoothed, normalized 0-1 → 0.1-4.0)
    /// 0.0 = fast initial pitch drop (punchy 808-style)
    /// 0.5 = linear pitch sweep
    /// 1.0 = slow initial pitch drop (softer)
    pub fn set_pitch_envelope_curve(&mut self, curve: f32) {
        self.params.pitch_envelope_curve.set_target(curve.clamp(0.0, 1.0));
    }

    /// Set pitch start ratio (smoothed, normalized 0-1 → 1.0-10.0x)
    /// Controls how much higher the initial pitch is relative to the base frequency
    pub fn set_pitch_start_ratio(&mut self, ratio: f32) {
        self.params.pitch_start_ratio.set_target(ratio.clamp(0.0, 1.0));
    }

    /// Set phase modulation amount (smoothed, 0-1, 0 = disabled)
    /// DS Kick-style phase modulation adds a brief frequency burst at note onset for transient snap
    pub fn set_phase_mod_amount(&mut self, amount: f32) {
        self.params.phase_mod_amount.set_target(amount.clamp(0.0, 1.0));
    }

    /// Set noise layer amount (smoothed, 0-1)
    pub fn set_noise_amount(&mut self, amount: f32) {
        self.params.noise_amount.set_target(amount.clamp(0.0, 1.0));
    }

    /// Set noise filter cutoff (smoothed, normalized 0-1 → 20-10000 Hz)
    pub fn set_noise_cutoff(&mut self, cutoff: f32) {
        self.params.noise_cutoff.set_target(cutoff.clamp(0.0, 1.0));
    }

    /// Set noise filter resonance (smoothed, normalized 0-1 → 0.0-5.0)
    pub fn set_noise_resonance(&mut self, resonance: f32) {
        self.params.noise_resonance.set_target(resonance.clamp(0.0, 1.0));
    }

    /// Set overdrive amount (smoothed, 0-1)
    pub fn set_overdrive(&mut self, amount: f32) {
        self.params.overdrive.set_target(amount.clamp(0.0, 1.0));
    }

    /// Set amplitude envelope decay time (smoothed, normalized 0-1 → 0.0-4.0s)
    pub fn set_amp_decay(&mut self, decay: f32) {
        self.params.amp_decay.set_target(decay.clamp(0.0, 1.0));
    }

    /// Set amplitude envelope decay curve (smoothed, normalized 0-1 → 0.1-10.0)
    /// 0.0 = fast initial decay (natural acoustic decay)
    /// 0.5 = linear decay
    /// 1.0 = slow initial decay
    pub fn set_amp_decay_curve(&mut self, curve: f32) {
        self.params.amp_decay_curve.set_target(curve.clamp(0.0, 1.0));
    }
}

impl crate::engine::Instrument for KickDrum {
    fn trigger_with_velocity(&mut self, time: f32, velocity: f32) {
        KickDrum::trigger_with_velocity(self, time, velocity);
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

// Implement modulation support for KickDrum
// All parameters use normalized 0-1 ranges
impl crate::engine::Modulatable for KickDrum {
    fn modulatable_parameters(&self) -> Vec<&'static str> {
        vec![
            "frequency",
            "punch",
            "sub",
            "click",
            "oscillator_decay",
            "pitch_envelope_amount",
            "pitch_envelope_curve",
            "volume",
            "pitch_start_ratio",
            "phase_mod_amount",
            "noise_amount",
            "noise_cutoff",
            "noise_resonance",
            "overdrive",
            "amp_decay",
            "amp_decay_curve",
        ]
    }

    fn apply_modulation(&mut self, parameter: &str, value: f32) -> Result<(), String> {
        // value is -1.0 to 1.0 (bipolar), set_bipolar maps this to the param range
        match parameter {
            "frequency" => {
                self.params.frequency.set_bipolar(value);
                Ok(())
            }
            "punch" => {
                self.params.punch.set_bipolar(value);
                Ok(())
            }
            "sub" => {
                self.params.sub.set_bipolar(value);
                Ok(())
            }
            "click" => {
                self.params.click.set_bipolar(value);
                Ok(())
            }
            "oscillator_decay" => {
                self.params.oscillator_decay.set_bipolar(value);
                Ok(())
            }
            "pitch_envelope_amount" => {
                self.params.pitch_envelope_amount.set_bipolar(value);
                Ok(())
            }
            "pitch_envelope_curve" => {
                self.params.pitch_envelope_curve.set_bipolar(value);
                Ok(())
            }
            "volume" => {
                self.params.volume.set_bipolar(value);
                Ok(())
            }
            "pitch_start_ratio" => {
                self.params.pitch_start_ratio.set_bipolar(value);
                Ok(())
            }
            "phase_mod_amount" => {
                self.params.phase_mod_amount.set_bipolar(value);
                Ok(())
            }
            "noise_amount" => {
                self.params.noise_amount.set_bipolar(value);
                Ok(())
            }
            "noise_cutoff" => {
                self.params.noise_cutoff.set_bipolar(value);
                Ok(())
            }
            "noise_resonance" => {
                self.params.noise_resonance.set_bipolar(value);
                Ok(())
            }
            "overdrive" => {
                self.params.overdrive.set_bipolar(value);
                Ok(())
            }
            "amp_decay" => {
                self.params.amp_decay.set_bipolar(value);
                Ok(())
            }
            "amp_decay_curve" => {
                self.params.amp_decay_curve.set_bipolar(value);
                Ok(())
            }
            _ => Err(format!("Unknown parameter: {}", parameter)),
        }
    }

    fn parameter_range(&self, parameter: &str) -> Option<(f32, f32)> {
        // All parameters now use normalized 0-1 range
        match parameter {
            "frequency" => Some(self.params.frequency.range()),
            "punch" => Some(self.params.punch.range()),
            "sub" => Some(self.params.sub.range()),
            "click" => Some(self.params.click.range()),
            "oscillator_decay" => Some(self.params.oscillator_decay.range()),
            "pitch_envelope_amount" => Some(self.params.pitch_envelope_amount.range()),
            "pitch_envelope_curve" => Some(self.params.pitch_envelope_curve.range()),
            "volume" => Some(self.params.volume.range()),
            "pitch_start_ratio" => Some(self.params.pitch_start_ratio.range()),
            "phase_mod_amount" => Some(self.params.phase_mod_amount.range()),
            "noise_amount" => Some(self.params.noise_amount.range()),
            "noise_cutoff" => Some(self.params.noise_cutoff.range()),
            "noise_resonance" => Some(self.params.noise_resonance.range()),
            "overdrive" => Some(self.params.overdrive.range()),
            "amp_decay" => Some(self.params.amp_decay.range()),
            "amp_decay_curve" => Some(self.params.amp_decay_curve.range()),
            _ => None,
        }
    }
}
