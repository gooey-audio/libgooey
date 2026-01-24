use crate::effects::waveshaper::Waveshaper;
use crate::envelope::{ADSRConfig, Envelope, EnvelopeCurve};
use crate::filters::StateVariableFilter;
use crate::gen::oscillator::Oscillator;
use crate::gen::waveform::Waveform;
use crate::instruments::fm_snap::PhaseModulator;
use crate::utils::{Blendable, SmoothedParam, DEFAULT_SMOOTH_TIME_MS};

/// Normalization ranges for snare drum parameters
/// All external-facing parameters use 0.0-1.0 normalized values
pub(crate) mod ranges {
    /// Frequency: 0-1 maps to 100-600 Hz
    pub const FREQ_MIN: f32 = 100.0;
    pub const FREQ_MAX: f32 = 600.0;

    /// Decay: 0-1 maps to 0.05-3.5 seconds
    pub const DECAY_MIN: f32 = 0.05;
    pub const DECAY_MAX: f32 = 3.5;

    /// Tonal decay: 0-1 maps to 0.0-3.5 seconds
    pub const TONAL_DECAY_MIN: f32 = 0.0;
    pub const TONAL_DECAY_MAX: f32 = 3.5;

    /// Tonal decay curve: 0-1 maps to 0.1-10.0 (exponential)
    pub const TONAL_DECAY_CURVE_MIN: f32 = 0.1;
    pub const TONAL_DECAY_CURVE_MAX: f32 = 10.0;

    /// Noise decay: 0-1 maps to 0.0-3.5 seconds
    pub const NOISE_DECAY_MIN: f32 = 0.0;
    pub const NOISE_DECAY_MAX: f32 = 3.5;

    /// Noise tail decay: 0-1 maps to 0.0-3.5 seconds
    pub const NOISE_TAIL_DECAY_MIN: f32 = 0.0;
    pub const NOISE_TAIL_DECAY_MAX: f32 = 3.5;

    /// Filter cutoff: 0-1 maps to 100-10000 Hz (capped at 10k to avoid filter instability)
    pub const FILTER_CUTOFF_MIN: f32 = 100.0;
    pub const FILTER_CUTOFF_MAX: f32 = 10000.0;

    /// Filter resonance: 0-1 maps to 0.5-10.0
    pub const FILTER_RES_MIN: f32 = 0.5;
    pub const FILTER_RES_MAX: f32 = 10.0;

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

/// Static configuration for snare drum presets
/// All parameters use normalized 0.0-1.0 values for easy integration with external systems.
/// Use the `ranges` module to convert to/from actual values.
#[derive(Clone, Copy, Debug)]
pub struct SnareConfig {
    pub frequency: f32,       // Base frequency (0-1 → 100-600Hz)
    pub tonal_amount: f32,    // Tonal component presence (0.0-1.0)
    pub noise_amount: f32,    // Noise component presence (0.0-1.0)
    pub crack_amount: f32,    // High-frequency crack (0.0-1.0)
    pub decay: f32,           // Overall decay length (0-1 → 0.05-3.5s)
    pub pitch_drop: f32,      // Frequency sweep amount (0.0-1.0)
    pub volume: f32,          // Overall volume (0.0-1.0)

    // DS-style parameters (all normalized 0-1)
    pub tonal_decay: f32,       // Separate tonal decay (0-1 → 0-3.5s)
    pub tonal_decay_curve: f32, // Tonal decay curve shape (0-1 → 0.1-10.0)
    pub noise_decay: f32,       // Noise envelope decay (0-1 → 0-3.5s)
    pub noise_tail_decay: f32,  // Noise tail decay (0-1 → 0-3.5s)
    pub filter_cutoff: f32,     // SVF filter cutoff (0-1 → 100-10000 Hz)
    pub filter_resonance: f32,  // SVF filter resonance (0-1 → 0.5-10.0)
    pub filter_type: u8,        // 0=LP, 1=BP, 2=HP, 3=notch
    pub xfade: f32,             // Tonal/noise crossfade (0.0-1.0)
    pub phase_mod_amount: f32,  // Phase mod depth (0.0-1.0, 0 = disabled)

    // New parameters (matching kick design)
    pub overdrive_amount: f32,  // Overdrive/saturation (0.0-1.0, 0.0 = bypass)
    pub amp_decay: f32,         // Master amplitude decay (0-1 → 0-4.0s)
    pub amp_decay_curve: f32,   // Decay curve shape (0-1 → 0.1-10.0)
}

impl SnareConfig {
    /// Create a new SnareConfig with normalized 0-1 parameters.
    /// All parameters are clamped to 0.0-1.0 range.
    pub fn new(
        frequency: f32,
        tonal_amount: f32,
        noise_amount: f32,
        crack_amount: f32,
        decay: f32,
        pitch_drop: f32,
        volume: f32,
    ) -> Self {
        Self {
            frequency: frequency.clamp(0.0, 1.0),
            tonal_amount: tonal_amount.clamp(0.0, 1.0),
            noise_amount: noise_amount.clamp(0.0, 1.0),
            crack_amount: crack_amount.clamp(0.0, 1.0),
            decay: decay.clamp(0.0, 1.0),
            pitch_drop: pitch_drop.clamp(0.0, 1.0),
            volume: volume.clamp(0.0, 1.0),
            // DS parameters with defaults
            tonal_decay: decay * 0.8,
            tonal_decay_curve: 0.091,  // ~1.0 (linear)
            noise_decay: decay * 0.6,
            noise_tail_decay: decay,
            filter_cutoff: 0.495,    // ~5000 Hz
            filter_resonance: 0.053, // ~1.0
            filter_type: 1,          // Bandpass default
            xfade: 0.5,
            phase_mod_amount: 0.0,
            // New parameters
            overdrive_amount: 0.0,
            amp_decay: 0.125,        // ~0.5s
            amp_decay_curve: 0.091,  // ~1.0 (linear)
        }
    }

    /// Create a SnareConfig with all parameters (all normalized 0-1)
    #[allow(clippy::too_many_arguments)]
    pub fn new_full(
        frequency: f32,
        tonal_amount: f32,
        noise_amount: f32,
        crack_amount: f32,
        decay: f32,
        pitch_drop: f32,
        volume: f32,
        tonal_decay: f32,
        tonal_decay_curve: f32,
        noise_decay: f32,
        noise_tail_decay: f32,
        filter_cutoff: f32,
        filter_resonance: f32,
        filter_type: u8,
        xfade: f32,
        phase_mod_amount: f32,
        overdrive_amount: f32,
        amp_decay: f32,
        amp_decay_curve: f32,
    ) -> Self {
        Self {
            frequency: frequency.clamp(0.0, 1.0),
            tonal_amount: tonal_amount.clamp(0.0, 1.0),
            noise_amount: noise_amount.clamp(0.0, 1.0),
            crack_amount: crack_amount.clamp(0.0, 1.0),
            decay: decay.clamp(0.0, 1.0),
            pitch_drop: pitch_drop.clamp(0.0, 1.0),
            volume: volume.clamp(0.0, 1.0),
            tonal_decay: tonal_decay.clamp(0.0, 1.0),
            tonal_decay_curve: tonal_decay_curve.clamp(0.0, 1.0),
            noise_decay: noise_decay.clamp(0.0, 1.0),
            noise_tail_decay: noise_tail_decay.clamp(0.0, 1.0),
            filter_cutoff: filter_cutoff.clamp(0.0, 1.0),
            filter_resonance: filter_resonance.clamp(0.0, 1.0),
            filter_type: filter_type.min(3),
            xfade: xfade.clamp(0.0, 1.0),
            phase_mod_amount: phase_mod_amount.clamp(0.0, 1.0),
            overdrive_amount: overdrive_amount.clamp(0.0, 1.0),
            amp_decay: amp_decay.clamp(0.0, 1.0),
            amp_decay_curve: amp_decay_curve.clamp(0.0, 1.0),
        }
    }

    // Helper methods to get actual (denormalized) values for audio processing

    /// Get actual frequency in Hz (100-600)
    #[inline]
    pub fn frequency_hz(&self) -> f32 {
        ranges::denormalize(self.frequency, ranges::FREQ_MIN, ranges::FREQ_MAX)
    }

    /// Get actual decay in seconds (0.05-3.5)
    #[inline]
    pub fn decay_secs(&self) -> f32 {
        ranges::denormalize(self.decay, ranges::DECAY_MIN, ranges::DECAY_MAX)
    }

    /// Get actual tonal decay in seconds (0-3.5)
    #[inline]
    pub fn tonal_decay_secs(&self) -> f32 {
        ranges::denormalize(self.tonal_decay, ranges::TONAL_DECAY_MIN, ranges::TONAL_DECAY_MAX)
    }

    /// Get actual tonal decay curve (0.1-10.0)
    #[inline]
    pub fn tonal_decay_curve_value(&self) -> f32 {
        ranges::denormalize(self.tonal_decay_curve, ranges::TONAL_DECAY_CURVE_MIN, ranges::TONAL_DECAY_CURVE_MAX)
    }

    /// Get actual noise decay in seconds (0-3.5)
    #[inline]
    pub fn noise_decay_secs(&self) -> f32 {
        ranges::denormalize(self.noise_decay, ranges::NOISE_DECAY_MIN, ranges::NOISE_DECAY_MAX)
    }

    /// Get actual noise tail decay in seconds (0-3.5)
    #[inline]
    pub fn noise_tail_decay_secs(&self) -> f32 {
        ranges::denormalize(self.noise_tail_decay, ranges::NOISE_TAIL_DECAY_MIN, ranges::NOISE_TAIL_DECAY_MAX)
    }

    /// Get actual filter cutoff in Hz (100-10000)
    #[inline]
    pub fn filter_cutoff_hz(&self) -> f32 {
        ranges::denormalize(self.filter_cutoff, ranges::FILTER_CUTOFF_MIN, ranges::FILTER_CUTOFF_MAX)
    }

    /// Get actual filter resonance (0.5-10.0)
    #[inline]
    pub fn filter_resonance_value(&self) -> f32 {
        ranges::denormalize(self.filter_resonance, ranges::FILTER_RES_MIN, ranges::FILTER_RES_MAX)
    }

    /// Get actual amp decay in seconds (0-4.0)
    #[inline]
    pub fn amp_decay_secs(&self) -> f32 {
        ranges::denormalize(self.amp_decay, ranges::AMP_DECAY_MIN, ranges::AMP_DECAY_MAX)
    }

    /// Get actual amp decay curve (0.1-10.0)
    #[inline]
    pub fn amp_decay_curve_value(&self) -> f32 {
        ranges::denormalize(self.amp_decay_curve, ranges::AMP_DECAY_CURVE_MIN, ranges::AMP_DECAY_CURVE_MAX)
    }

    /// Tight snare - short, punchy
    pub fn tight() -> Self {
        // 200 Hz → (200-100)/(600-100) = 0.2
        // 0.15s decay → (0.15-0.05)/(3.5-0.05) ≈ 0.029
        Self::new(0.2, 0.4, 0.7, 0.5, 0.029, 0.3, 0.8)
    }

    /// Loose snare - longer decay, more body
    pub fn loose() -> Self {
        Self::new_full(
            0.16,  // frequency
            0.80,  // tonal_amount
            0.60,  // noise_amount
            0.30,  // crack_amount (brightness)
            0.79,  // decay
            0.10,  // pitch_drop
            0.90,  // volume
            0.33,  // tonal_decay
            0.20,  // tonal_decay_curve
            0.23,  // noise_decay
            0.34,  // noise_tail_decay
            0.55,  // filter_cutoff
            0.05,  // filter_resonance
            1,     // filter_type: BP
            0.50,  // xfade
            0.00,  // phase_mod_amount
            0.10,  // overdrive_amount
            0.12,  // amp_decay
            0.09,  // amp_decay_curve
        )
    }

    /// Hiss snare - noise-focused with phase modulation
    pub fn hiss() -> Self {
        Self::new_full(
            0.16,  // frequency
            0.00,  // tonal_amount (no tonal)
            0.60,  // noise_amount
            0.30,  // crack_amount (brightness)
            0.04,  // decay
            0.40,  // pitch_drop
            0.90,  // volume
            0.53,  // tonal_decay
            0.09,  // tonal_decay_curve
            0.38,  // noise_decay
            0.29,  // noise_tail_decay
            0.29,  // filter_cutoff
            0.45,  // filter_resonance
            1,     // filter_type: BP
            0.50,  // xfade
            1.00,  // phase_mod_amount
            0.20,  // overdrive_amount
            0.18,  // amp_decay
            0.09,  // amp_decay_curve
        )
    }

    /// Smack snare - Ableton Drum Synth style
    /// Features: phase modulation transient, SVF-filtered noise, tonal/noise crossfade
    pub fn smack() -> Self {
        Self::new_full(
            0.2,    // frequency: 200 Hz
            0.3,    // tonal_amount
            0.8,    // noise_amount
            0.0,    // crack_amount (using phase mod instead)
            0.029,  // decay: ~0.15s
            0.3,    // pitch_drop
            0.85,   // volume
            // DS parameters (normalized):
            0.014,  // tonal_decay: ~50ms
            0.091,  // tonal_decay_curve: ~1.0 (linear)
            0.034,  // noise_decay: ~120ms
            0.086,  // noise_tail_decay: ~300ms
            0.293,  // filter_cutoff: ~3000 Hz
            0.158,  // filter_resonance: ~2.0
            1,      // filter_type: Bandpass
            0.4,    // xfade: 40% tonal / 60% noise
            0.5,    // phase_mod_amount
            0.0,    // overdrive_amount
            0.125,  // amp_decay: ~0.5s
            0.091,  // amp_decay_curve: ~1.0
        )
    }
}

impl Blendable for SnareConfig {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let inv_t = 1.0 - t;
        Self {
            frequency: self.frequency * inv_t + other.frequency * t,
            tonal_amount: self.tonal_amount * inv_t + other.tonal_amount * t,
            noise_amount: self.noise_amount * inv_t + other.noise_amount * t,
            crack_amount: self.crack_amount * inv_t + other.crack_amount * t,
            decay: self.decay * inv_t + other.decay * t,
            pitch_drop: self.pitch_drop * inv_t + other.pitch_drop * t,
            volume: self.volume * inv_t + other.volume * t,
            tonal_decay: self.tonal_decay * inv_t + other.tonal_decay * t,
            tonal_decay_curve: self.tonal_decay_curve * inv_t + other.tonal_decay_curve * t,
            noise_decay: self.noise_decay * inv_t + other.noise_decay * t,
            noise_tail_decay: self.noise_tail_decay * inv_t + other.noise_tail_decay * t,
            filter_cutoff: self.filter_cutoff * inv_t + other.filter_cutoff * t,
            filter_resonance: self.filter_resonance * inv_t + other.filter_resonance * t,
            filter_type: if t < 0.5 { self.filter_type } else { other.filter_type },
            xfade: self.xfade * inv_t + other.xfade * t,
            phase_mod_amount: self.phase_mod_amount * inv_t + other.phase_mod_amount * t,
            overdrive_amount: self.overdrive_amount * inv_t + other.overdrive_amount * t,
            amp_decay: self.amp_decay * inv_t + other.amp_decay * t,
            amp_decay_curve: self.amp_decay_curve * inv_t + other.amp_decay_curve * t,
        }
    }
}

/// Smoothed parameters for real-time control of the snare drum
/// These use one-pole smoothing to prevent clicks/pops during parameter changes
/// All parameters use normalized 0-1 ranges for external interface
pub struct SnareParams {
    pub frequency: SmoothedParam,        // Base frequency (0-1 → 100-600 Hz)
    pub decay: SmoothedParam,            // Decay time (0-1 → 0.05-3.5s)
    pub brightness: SmoothedParam,       // Snap/crack tone amount (0-1)
    pub volume: SmoothedParam,           // Overall volume (0-1)
    pub tonal: SmoothedParam,            // Tonal component amount (0-1)
    pub noise: SmoothedParam,            // Noise component amount (0-1)
    pub pitch_drop: SmoothedParam,       // Pitch drop amount (0-1)

    // DS-style smoothed parameters (all normalized 0-1)
    pub tonal_decay: SmoothedParam,      // Tonal envelope decay (0-1 → 0-3.5s)
    pub tonal_decay_curve: SmoothedParam, // Tonal decay curve shape (0-1 → 0.1-10.0)
    pub noise_decay: SmoothedParam,      // Noise envelope decay (0-1 → 0-3.5s)
    pub noise_tail_decay: SmoothedParam, // Noise tail decay (0-1 → 0-3.5s)
    pub filter_cutoff: SmoothedParam,    // SVF filter cutoff (0-1 → 100-10000 Hz)
    pub filter_resonance: SmoothedParam, // SVF filter resonance (0-1 → 0.5-10.0)
    pub filter_type: u8,                 // 0=LP, 1=BP, 2=HP, 3=notch (not smoothed)
    pub xfade: SmoothedParam,            // Tonal/noise crossfade (0-1)
    pub phase_mod_amount: SmoothedParam, // Phase mod depth (0-1, 0 = disabled)

    // New parameters (matching kick design)
    pub overdrive: SmoothedParam,        // Overdrive/saturation (0-1, 0 = bypass)
    pub amp_decay: SmoothedParam,        // Master amplitude decay (0-1 → 0-4.0s)
    pub amp_decay_curve: SmoothedParam,  // Decay curve shape (0-1 → 0.1-10.0)
}

impl SnareParams {
    /// Create new smoothed parameters from a config
    /// All parameters use normalized 0-1 range for external interface
    pub fn from_config(config: &SnareConfig, sample_rate: f32) -> Self {
        Self {
            frequency: SmoothedParam::new(
                config.frequency,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            decay: SmoothedParam::new(
                config.decay,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            brightness: SmoothedParam::new(
                config.crack_amount,
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
            tonal: SmoothedParam::new(
                config.tonal_amount,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            noise: SmoothedParam::new(
                config.noise_amount,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            pitch_drop: SmoothedParam::new(
                config.pitch_drop,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            // DS-style parameters (all normalized 0-1)
            tonal_decay: SmoothedParam::new(
                config.tonal_decay,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            tonal_decay_curve: SmoothedParam::new(
                config.tonal_decay_curve,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            noise_decay: SmoothedParam::new(
                config.noise_decay,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            noise_tail_decay: SmoothedParam::new(
                config.noise_tail_decay,
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
            filter_type: config.filter_type,
            xfade: SmoothedParam::new(
                config.xfade,
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
            // New parameters
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

    // Helper methods to get denormalized values for audio processing

    /// Get actual frequency in Hz (100-600)
    #[inline]
    pub fn frequency_hz(&self) -> f32 {
        ranges::denormalize(self.frequency.get(), ranges::FREQ_MIN, ranges::FREQ_MAX)
    }

    /// Get actual decay in seconds (0.05-3.5)
    #[inline]
    pub fn decay_secs(&self) -> f32 {
        ranges::denormalize(self.decay.get(), ranges::DECAY_MIN, ranges::DECAY_MAX)
    }

    /// Get actual tonal decay in seconds (0-3.5)
    #[inline]
    pub fn tonal_decay_secs(&self) -> f32 {
        ranges::denormalize(self.tonal_decay.get(), ranges::TONAL_DECAY_MIN, ranges::TONAL_DECAY_MAX)
    }

    /// Get actual tonal decay curve (0.1-10.0)
    #[inline]
    pub fn tonal_decay_curve_value(&self) -> f32 {
        ranges::denormalize(self.tonal_decay_curve.get(), ranges::TONAL_DECAY_CURVE_MIN, ranges::TONAL_DECAY_CURVE_MAX)
    }

    /// Get actual noise decay in seconds (0-3.5)
    #[inline]
    pub fn noise_decay_secs(&self) -> f32 {
        ranges::denormalize(self.noise_decay.get(), ranges::NOISE_DECAY_MIN, ranges::NOISE_DECAY_MAX)
    }

    /// Get actual noise tail decay in seconds (0-3.5)
    #[inline]
    pub fn noise_tail_decay_secs(&self) -> f32 {
        ranges::denormalize(self.noise_tail_decay.get(), ranges::NOISE_TAIL_DECAY_MIN, ranges::NOISE_TAIL_DECAY_MAX)
    }

    /// Get actual filter cutoff in Hz (100-10000)
    #[inline]
    pub fn filter_cutoff_hz(&self) -> f32 {
        ranges::denormalize(self.filter_cutoff.get(), ranges::FILTER_CUTOFF_MIN, ranges::FILTER_CUTOFF_MAX)
    }

    /// Get actual filter resonance (0.5-10.0)
    #[inline]
    pub fn filter_resonance_value(&self) -> f32 {
        ranges::denormalize(self.filter_resonance.get(), ranges::FILTER_RES_MIN, ranges::FILTER_RES_MAX)
    }

    /// Get actual amp decay in seconds (0-4.0)
    #[inline]
    pub fn amp_decay_secs(&self) -> f32 {
        ranges::denormalize(self.amp_decay.get(), ranges::AMP_DECAY_MIN, ranges::AMP_DECAY_MAX)
    }

    /// Get actual amp decay curve (0.1-10.0)
    #[inline]
    pub fn amp_decay_curve_value(&self) -> f32 {
        ranges::denormalize(self.amp_decay_curve.get(), ranges::AMP_DECAY_CURVE_MIN, ranges::AMP_DECAY_CURVE_MAX)
    }

    /// Tick all smoothers and return whether any are still smoothing
    #[inline]
    pub fn tick(&mut self) -> bool {
        self.frequency.tick();
        self.decay.tick();
        self.brightness.tick();
        self.volume.tick();
        self.tonal.tick();
        self.noise.tick();
        self.pitch_drop.tick();
        self.tonal_decay.tick();
        self.tonal_decay_curve.tick();
        self.noise_decay.tick();
        self.noise_tail_decay.tick();
        self.filter_cutoff.tick();
        self.filter_resonance.tick();
        self.xfade.tick();
        self.phase_mod_amount.tick();
        self.overdrive.tick();
        self.amp_decay.tick();
        self.amp_decay_curve.tick();
        !self.is_settled()
    }

    /// Check if all parameters have settled
    pub fn is_settled(&self) -> bool {
        self.frequency.is_settled()
            && self.decay.is_settled()
            && self.brightness.is_settled()
            && self.volume.is_settled()
            && self.tonal.is_settled()
            && self.noise.is_settled()
            && self.pitch_drop.is_settled()
            && self.tonal_decay.is_settled()
            && self.tonal_decay_curve.is_settled()
            && self.noise_decay.is_settled()
            && self.noise_tail_decay.is_settled()
            && self.filter_cutoff.is_settled()
            && self.filter_resonance.is_settled()
            && self.xfade.is_settled()
            && self.phase_mod_amount.is_settled()
            && self.overdrive.is_settled()
            && self.amp_decay.is_settled()
            && self.amp_decay_curve.is_settled()
    }
}

pub struct SnareDrum {
    pub sample_rate: f32,
    pub config: SnareConfig,

    /// Smoothed parameters for click-free real-time control
    pub params: SnareParams,

    // Three oscillators for different components
    pub tonal_oscillator: Oscillator, // Tonal component (triangle/sine)
    pub noise_oscillator: Oscillator, // Main noise component
    pub crack_oscillator: Oscillator, // High-frequency crack

    // Pitch envelope for frequency sweeping
    pub pitch_envelope: Envelope,
    pub base_frequency: f32,
    pub pitch_start_multiplier: f32,

    pub is_active: bool,

    // Velocity-responsive state
    /// Current trigger velocity (0.0-1.0), set on trigger
    current_velocity: f32,

    /// How much velocity affects decay time (0.0-1.0)
    /// Higher values = more velocity sensitivity (shorter decay at high velocity)
    velocity_to_decay: f32,

    /// How much velocity affects pitch envelope decay (0.0-1.0)
    /// Higher velocity = faster pitch decay (sharper, more aggressive attack)
    velocity_to_pitch: f32,

    // DS Snare-style components
    /// State variable filter for noise shaping
    noise_filter: StateVariableFilter,

    /// Phase modulator for DS-style transient
    phase_modulator: PhaseModulator,

    /// Noise tail envelope (separate from main noise)
    noise_tail_envelope: Envelope,

    /// Tonal-specific envelope (DS-style separate decay)
    tonal_envelope: Envelope,

    /// Main noise envelope (DS-style)
    main_noise_envelope: Envelope,

    /// Waveshaper for overdrive/saturation
    waveshaper: Waveshaper,

    /// Master amplitude envelope
    amplitude_envelope: Envelope,
}

impl SnareDrum {
    pub fn new(sample_rate: f32) -> Self {
        let config = SnareConfig::tight();
        Self::with_config(sample_rate, config)
    }

    pub fn with_config(sample_rate: f32, config: SnareConfig) -> Self {
        let params = SnareParams::from_config(&config, sample_rate);
        let base_freq = config.frequency_hz();
        let mut snare = Self {
            sample_rate,
            config,
            params,
            tonal_oscillator: Oscillator::new(sample_rate, base_freq),
            noise_oscillator: Oscillator::new(sample_rate, base_freq * 8.0),
            crack_oscillator: Oscillator::new(sample_rate, base_freq * 25.0),
            pitch_envelope: Envelope::new(),
            base_frequency: base_freq,
            pitch_start_multiplier: 1.0 + config.pitch_drop * 1.5, // Start 1-2.5x higher
            is_active: false,

            // Initialize velocity state (matches default trigger velocity)
            current_velocity: 0.5,
            // Velocity scaling: 0.45 means velocity can reduce decay by up to 45%
            velocity_to_decay: 0.45,
            // Pitch velocity scaling: 0.5 for moderate pitch response
            velocity_to_pitch: 0.5,

            // DS Snare-style components
            noise_filter: StateVariableFilter::new(
                sample_rate,
                config.filter_cutoff_hz(),
                config.filter_resonance_value(),
            ),
            phase_modulator: PhaseModulator::new(sample_rate),
            noise_tail_envelope: Envelope::new(),
            tonal_envelope: Envelope::new(),
            main_noise_envelope: Envelope::new(),

            // New components
            waveshaper: Waveshaper::new(1.0, 1.0), // Will be configured based on overdrive_amount
            amplitude_envelope: Envelope::new(),
        };

        snare.setup_waveforms();
        snare
    }

    /// Set up oscillator waveforms (called once at construction)
    fn setup_waveforms(&mut self) {
        self.tonal_oscillator.waveform = Waveform::Triangle;
        self.noise_oscillator.waveform = Waveform::Noise;
        self.crack_oscillator.waveform = Waveform::Noise;
    }

    pub fn set_config(&mut self, config: SnareConfig) {
        self.config = config;
        self.base_frequency = config.frequency_hz();
        self.pitch_start_multiplier = 1.0 + config.pitch_drop * 1.5;
        // Update smoothed params to match new config (all normalized 0-1)
        self.params.frequency.set_target(config.frequency);
        self.params.decay.set_target(config.decay);
        self.params.brightness.set_target(config.crack_amount);
        self.params.volume.set_target(config.volume);
        self.params.tonal.set_target(config.tonal_amount);
        self.params.noise.set_target(config.noise_amount);
        self.params.pitch_drop.set_target(config.pitch_drop);
        // DS parameters (all normalized 0-1)
        self.params.tonal_decay.set_target(config.tonal_decay);
        self.params.tonal_decay_curve.set_target(config.tonal_decay_curve);
        self.params.noise_decay.set_target(config.noise_decay);
        self.params.noise_tail_decay.set_target(config.noise_tail_decay);
        self.params.filter_cutoff.set_target(config.filter_cutoff);
        self.params.filter_resonance.set_target(config.filter_resonance);
        self.params.filter_type = config.filter_type;
        self.params.xfade.set_target(config.xfade);
        self.params.phase_mod_amount.set_target(config.phase_mod_amount);
        // New parameters
        self.params.overdrive.set_target(config.overdrive_amount);
        self.params.amp_decay.set_target(config.amp_decay);
        self.params.amp_decay_curve.set_target(config.amp_decay_curve);
    }

    /// Trigger the snare drum at default velocity (0.5)
    pub fn trigger(&mut self, time: f32) {
        self.trigger_with_velocity(time, 0.5);
    }

    /// Trigger the snare drum with velocity sensitivity
    ///
    /// Velocity affects:
    /// - Decay time: Higher velocity = shorter decay (tighter, punchier)
    /// - Pitch envelope: Higher velocity = faster pitch decay (sharper attack)
    /// - Crack volume: Higher velocity = more crack (brighter, snappier)
    /// - Amplitude: Perceptually linear scaling via sqrt
    pub fn trigger_with_velocity(&mut self, time: f32, velocity: f32) {
        self.current_velocity = velocity.clamp(0.0, 1.0);
        self.is_active = true;

        let vel = self.current_velocity;

        // Quadratic curve for natural acoustic-like response
        let vel_squared = vel * vel;

        // --- Decay time scaling ---
        // Higher velocity = shorter decay (tighter, punchier sound)
        let decay_scale = 1.0 - (self.velocity_to_decay * vel_squared);

        // --- Pitch envelope scaling ---
        // Higher velocity = faster pitch decay (sharper attack)
        let pitch_decay_scale = 1.0 - (self.velocity_to_pitch * vel_squared);

        // Get current smoothed parameter values (use helper methods for denormalized values)
        let base_freq = self.params.frequency_hz();
        let base_decay = self.params.decay_secs();
        let volume = self.params.volume.get();
        let brightness = self.params.brightness.get();
        let tonal_amount = self.params.tonal.get();
        let noise_amount = self.params.noise.get();
        let pitch_drop = self.params.pitch_drop.get();

        // Get DS-style parameters (denormalized)
        let tonal_decay = self.params.tonal_decay_secs();
        let tonal_decay_curve = self.params.tonal_decay_curve_value();
        let noise_decay = self.params.noise_decay_secs();
        let noise_tail_decay = self.params.noise_tail_decay_secs();

        // Get amp envelope parameters (denormalized)
        let amp_decay = self.params.amp_decay_secs();
        let amp_decay_curve = self.params.amp_decay_curve_value();

        // Calculate velocity-scaled decay
        let scaled_decay = base_decay * decay_scale;

        // Update pitch start multiplier from smoothed value
        self.pitch_start_multiplier = 1.0 + pitch_drop * 1.5;

        // Configure pitch envelope with velocity-scaled decay
        // Base: 30% of amplitude decay, scaled by velocity
        // Clamped to max 25% to ensure pitch settles before tonal decays (avoids pitch artifacts)
        let pitch_decay_time = (scaled_decay * 0.3 * pitch_decay_scale).min(scaled_decay * 0.25);
        self.pitch_envelope.set_config(ADSRConfig::new(
            0.001,                   // Instant attack
            pitch_decay_time,        // Velocity-scaled pitch drop
            0.0,                     // Drop to base frequency
            pitch_decay_time * 0.1,  // Quick release
        ));

        // Configure tonal oscillator envelope to hold (sustain=1.0)
        // The dedicated tonal_envelope controls the actual decay shape
        self.tonal_oscillator.frequency_hz = base_freq;
        self.tonal_oscillator.set_volume(tonal_amount * volume);
        self.tonal_oscillator.set_adsr(ADSRConfig::new(
            0.001,                  // Very fast attack
            0.001,                  // Minimal decay (go straight to sustain)
            1.0,                    // Full sustain - tonal_envelope controls amplitude
            scaled_decay * 0.4,     // Medium release (when note releases)
        ));

        // Configure noise oscillator envelope to hold (sustain=1.0)
        // The dedicated noise envelopes control the actual decay shape
        self.noise_oscillator.frequency_hz = base_freq * 8.0;
        self.noise_oscillator.set_volume(noise_amount * volume * 0.8);
        self.noise_oscillator.set_adsr(ADSRConfig::new(
            0.001,                  // Very fast attack
            0.001,                  // Minimal decay (go straight to sustain)
            1.0,                    // Full sustain - noise envelopes control amplitude
            scaled_decay * 0.3,     // Quick release (when note releases)
        ));

        // Configure crack oscillator envelope with velocity-scaled decay
        // Crack gets velocity boost: range [0.7, 1.0] for more snap at high velocity
        let crack_vel_scale = 0.7 + 0.3 * vel;
        self.crack_oscillator.frequency_hz = base_freq * 25.0;
        self.crack_oscillator.set_volume(brightness * volume * 0.4 * crack_vel_scale);
        self.crack_oscillator.set_adsr(ADSRConfig::new(
            0.001,                  // Very fast attack
            scaled_decay * 0.2,     // Very short decay for crack (velocity-scaled)
            0.0,                    // No sustain
            scaled_decay * 0.1,     // Very short release
        ));

        // --- DS Snare-style envelopes ---

        // Tonal envelope (DS-style separate decay with exponential curve)
        let scaled_tonal_decay = tonal_decay * decay_scale;
        let mut tonal_config = ADSRConfig::new(
            0.001,                       // Fast attack
            scaled_tonal_decay,          // DS-style tonal decay
            0.0,                         // No sustain
            scaled_tonal_decay * 0.2,    // Short release
        );
        tonal_config.decay_curve = EnvelopeCurve::Exponential(tonal_decay_curve);
        self.tonal_envelope.set_config(tonal_config);

        // Main noise envelope (DS-style)
        let scaled_noise_decay = noise_decay * decay_scale;
        self.main_noise_envelope.set_config(ADSRConfig::new(
            0.001,                       // Fast attack
            scaled_noise_decay,          // DS-style noise decay
            0.0,                         // No sustain
            scaled_noise_decay * 0.2,    // Short release
        ));

        // Noise tail envelope (longer decay for snare ring)
        let scaled_tail_decay = noise_tail_decay * decay_scale;
        self.noise_tail_envelope.set_config(ADSRConfig::new(
            0.001,                       // Fast attack
            scaled_tail_decay,           // Longer tail decay
            0.0,                         // No sustain
            scaled_tail_decay * 0.3,     // Medium release
        ));

        // --- Master amplitude envelope ---
        // Scale amp decay by velocity (higher velocity = shorter decay)
        let scaled_amp_decay = amp_decay * decay_scale;
        let mut amp_config = ADSRConfig::new(
            0.001,                       // Instant attack (like kick)
            scaled_amp_decay,            // Velocity-scaled amplitude decay
            0.0,                         // No sustain
            scaled_amp_decay * 0.2,      // Short release
        );
        amp_config.decay_curve = EnvelopeCurve::Exponential(amp_decay_curve);
        self.amplitude_envelope.set_config(amp_config);

        // Trigger all oscillators
        self.tonal_oscillator.trigger(time);
        self.noise_oscillator.trigger(time);
        self.crack_oscillator.trigger(time);

        // Trigger pitch envelope
        self.pitch_envelope.trigger(time);

        // Trigger DS-style envelopes
        self.tonal_envelope.trigger(time);
        self.main_noise_envelope.trigger(time);
        self.noise_tail_envelope.trigger(time);

        // Trigger amplitude envelope
        self.amplitude_envelope.trigger(time);

        // Trigger phase modulator if amount > 0
        let phase_mod_amount = self.params.phase_mod_amount.get();
        if phase_mod_amount > 0.001 {
            self.phase_modulator.trigger(time);
        }

        // Reset filter state for clean transient
        self.noise_filter.reset();
    }

    pub fn release(&mut self, time: f32) {
        if self.is_active {
            self.tonal_oscillator.release(time);
            self.noise_oscillator.release(time);
            self.crack_oscillator.release(time);
            self.pitch_envelope.release(time);
            // DS-style envelopes
            self.tonal_envelope.release(time);
            self.main_noise_envelope.release(time);
            self.noise_tail_envelope.release(time);
            // Amplitude envelope
            self.amplitude_envelope.release(time);
        }
    }

    pub fn tick(&mut self, current_time: f32) -> f32 {
        // Always tick smoothers (even when not active, to settle values)
        // Returns true if any params are still changing
        let params_changing = self.params.tick();

        if !self.is_active {
            return 0.0;
        }

        // Only apply params when they're actively changing (optimization)
        if params_changing {
            self.apply_params();
        }

        // Use denormalized frequency for pitch calculations
        let base_frequency = self.params.frequency_hz();

        // Calculate pitch modulation from envelope
        let pitch_envelope_value = self.pitch_envelope.get_amplitude(current_time);
        let mut frequency_multiplier =
            1.0 + (self.pitch_start_multiplier - 1.0) * pitch_envelope_value;

        // Apply phase modulation if amount > 0 (DS-style transient snap)
        let phase_mod_amount = self.params.phase_mod_amount.get();
        if phase_mod_amount > 0.001 {
            let phase_mod = self.phase_modulator.tick(current_time);
            // Phase mod adds brief frequency boost (multiplier of up to 2x at full amount)
            frequency_multiplier *= 1.0 + (phase_mod * phase_mod_amount * 1.0);
        }

        // Apply pitch envelope to tonal oscillator only
        self.tonal_oscillator.frequency_hz = base_frequency * frequency_multiplier;

        // Update filter parameters (denormalized)
        let filter_cutoff = self.params.filter_cutoff_hz();
        let filter_resonance = self.params.filter_resonance_value();
        self.noise_filter.set_params(filter_cutoff, filter_resonance);

        // Get xfade parameter (0 = all tonal, 1 = all noise)
        let xfade = self.params.xfade.get();
        let tonal_mix = 1.0 - xfade;
        let noise_mix = xfade;

        // --- Generate tonal component ---
        let raw_tonal_output = self.tonal_oscillator.tick(current_time);
        // Apply DS-style tonal envelope
        let tonal_env = self.tonal_envelope.get_amplitude(current_time);
        let tonal_output = raw_tonal_output * tonal_env * tonal_mix;

        // --- Generate noise component ---
        let raw_noise_output = self.noise_oscillator.tick(current_time);

        // Apply SVF filter to noise based on filter_type
        let filter_type = self.params.filter_type;
        let filtered_noise = self.noise_filter.process_mode(raw_noise_output, filter_type);

        // Apply DS-style noise envelopes (main + tail)
        let noise_env = self.main_noise_envelope.get_amplitude(current_time);
        let tail_env = self.noise_tail_envelope.get_amplitude(current_time);
        // Combine envelopes: main for body, tail for ring
        let combined_noise_env = (noise_env * 0.7) + (tail_env * 0.3);
        let noise_output = filtered_noise * combined_noise_env * noise_mix;

        // --- Generate crack component (original behavior) ---
        let crack_output = self.crack_oscillator.tick(current_time);

        // Sum all components
        let total_output = tonal_output + noise_output + crack_output;

        // Apply waveshaper/overdrive BEFORE amplitude envelope (matches kick behavior)
        // This ensures overdrive-added harmonics are scaled down by the envelope
        let overdrive_amount = self.params.overdrive.get();
        let drive = 1.0 + (overdrive_amount * 9.0);
        self.waveshaper.set_drive(drive);
        // Waveshaper bypasses when drive <= 1.0, so always safe to call
        let overdriven_output = self.waveshaper.process(total_output);

        // Apply master amplitude envelope (after overdrive, like kick)
        let amp_env = self.amplitude_envelope.get_amplitude(current_time);

        // Apply velocity amplitude scaling (sqrt for perceptually linear loudness)
        let velocity_amplitude = self.current_velocity.sqrt();
        let final_output = overdriven_output * amp_env * velocity_amplitude;

        // Check if snare is still active
        let classic_active = self.tonal_oscillator.envelope.is_active
            || self.noise_oscillator.envelope.is_active
            || self.crack_oscillator.envelope.is_active;
        let ds_active = self.tonal_envelope.is_active
            || self.main_noise_envelope.is_active
            || self.noise_tail_envelope.is_active
            || self.amplitude_envelope.is_active
            || self.phase_modulator.is_active();

        if !classic_active && !ds_active {
            self.is_active = false;
        }

        final_output
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    /// Apply current smoothed parameters to oscillators (called per-sample)
    #[inline]
    fn apply_params(&mut self) {
        let volume = self.params.volume.get();
        let brightness = self.params.brightness.get();
        let tonal_amount = self.params.tonal.get();
        let noise_amount = self.params.noise.get();

        // Crack gets velocity-sensitive volume boost (more snap at high velocity)
        let crack_vel_scale = 0.7 + 0.3 * self.current_velocity;

        // Update oscillator volumes with smoothed values
        self.tonal_oscillator.set_volume(tonal_amount * volume);
        self.noise_oscillator.set_volume(noise_amount * volume * 0.8);
        self.crack_oscillator.set_volume(brightness * volume * 0.4 * crack_vel_scale);
    }

    /// Set volume (smoothed, 0-1)
    pub fn set_volume(&mut self, volume: f32) {
        self.params.volume.set_target(volume.clamp(0.0, 1.0));
    }

    /// Set base frequency (smoothed, normalized 0-1 → 100-600 Hz)
    pub fn set_frequency(&mut self, frequency: f32) {
        self.params.frequency.set_target(frequency.clamp(0.0, 1.0));
    }

    /// Set decay time (smoothed, normalized 0-1 → 0.05-3.5s)
    /// Envelope will be reconfigured on next trigger
    pub fn set_decay(&mut self, decay: f32) {
        self.params.decay.set_target(decay.clamp(0.0, 1.0));
    }

    /// Set brightness/snap amount (smoothed, 0-1)
    pub fn set_brightness(&mut self, brightness: f32) {
        self.params.brightness.set_target(brightness.clamp(0.0, 1.0));
    }

    /// Set tonal amount (smoothed, 0-1)
    pub fn set_tonal(&mut self, tonal_amount: f32) {
        self.params.tonal.set_target(tonal_amount.clamp(0.0, 1.0));
    }

    /// Set noise amount (smoothed, 0-1)
    pub fn set_noise(&mut self, noise_amount: f32) {
        self.params.noise.set_target(noise_amount.clamp(0.0, 1.0));
    }

    /// Set crack amount (alias for set_brightness, 0-1)
    pub fn set_crack(&mut self, crack_amount: f32) {
        self.set_brightness(crack_amount);
    }

    /// Set pitch drop amount (smoothed, 0-1)
    pub fn set_pitch_drop(&mut self, pitch_drop: f32) {
        self.params.pitch_drop.set_target(pitch_drop.clamp(0.0, 1.0));
    }

    // --- DS-style parameter setters (all normalized 0-1) ---

    /// Set tonal decay time (smoothed, normalized 0-1 → 0-3.5s)
    pub fn set_tonal_decay(&mut self, decay: f32) {
        self.params.tonal_decay.set_target(decay.clamp(0.0, 1.0));
    }

    /// Set tonal decay curve shape (smoothed, normalized 0-1 → 0.1-10.0)
    pub fn set_tonal_decay_curve(&mut self, curve: f32) {
        self.params.tonal_decay_curve.set_target(curve.clamp(0.0, 1.0));
    }

    /// Set noise decay time (smoothed, normalized 0-1 → 0-3.5s)
    pub fn set_noise_decay(&mut self, decay: f32) {
        self.params.noise_decay.set_target(decay.clamp(0.0, 1.0));
    }

    /// Set noise tail decay time (smoothed, normalized 0-1 → 0-3.5s)
    pub fn set_noise_tail_decay(&mut self, decay: f32) {
        self.params.noise_tail_decay.set_target(decay.clamp(0.0, 1.0));
    }

    /// Set filter cutoff frequency (smoothed, normalized 0-1 → 100-10000 Hz)
    pub fn set_filter_cutoff(&mut self, cutoff: f32) {
        self.params.filter_cutoff.set_target(cutoff.clamp(0.0, 1.0));
    }

    /// Set filter resonance (smoothed, normalized 0-1 → 0.5-10.0)
    pub fn set_filter_resonance(&mut self, resonance: f32) {
        self.params.filter_resonance.set_target(resonance.clamp(0.0, 1.0));
    }

    /// Set filter type (0=LP, 1=BP, 2=HP, 3=notch)
    pub fn set_filter_type(&mut self, filter_type: u8) {
        self.params.filter_type = filter_type.min(3);
    }

    /// Set tonal/noise crossfade (smoothed, 0-1)
    /// 0.0 = all tonal, 1.0 = all noise
    pub fn set_xfade(&mut self, xfade: f32) {
        self.params.xfade.set_target(xfade.clamp(0.0, 1.0));
    }

    /// Set phase modulation amount (smoothed, 0-1, 0 = disabled)
    pub fn set_phase_mod_amount(&mut self, amount: f32) {
        self.params.phase_mod_amount.set_target(amount.clamp(0.0, 1.0));
    }

    // --- New parameter setters ---

    /// Set overdrive/saturation amount (smoothed, 0-1, 0 = bypass)
    pub fn set_overdrive(&mut self, amount: f32) {
        self.params.overdrive.set_target(amount.clamp(0.0, 1.0));
    }

    /// Set master amplitude decay time (smoothed, normalized 0-1 → 0-4.0s)
    pub fn set_amp_decay(&mut self, decay: f32) {
        self.params.amp_decay.set_target(decay.clamp(0.0, 1.0));
    }

    /// Set amplitude decay curve shape (smoothed, normalized 0-1 → 0.1-10.0)
    pub fn set_amp_decay_curve(&mut self, curve: f32) {
        self.params.amp_decay_curve.set_target(curve.clamp(0.0, 1.0));
    }
}

impl crate::engine::Instrument for SnareDrum {
    fn trigger_with_velocity(&mut self, time: f32, velocity: f32) {
        SnareDrum::trigger_with_velocity(self, time, velocity);
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

// Implement modulation support for SnareDrum
impl crate::engine::Modulatable for SnareDrum {
    fn modulatable_parameters(&self) -> Vec<&'static str> {
        vec![
            "frequency",
            "decay",
            "brightness",
            "crack", // Alias for brightness (backward compatibility)
            "volume",
            "tonal",
            "noise",
            "pitch_drop",
            // DS-style parameters
            "tonal_decay",
            "tonal_decay_curve",
            "noise_decay",
            "noise_tail_decay",
            "filter_cutoff",
            "filter_resonance",
            "xfade",
            "phase_mod_amount",
            // New parameters
            "overdrive",
            "amp_decay",
            "amp_decay_curve",
        ]
    }

    fn apply_modulation(&mut self, parameter: &str, value: f32) -> Result<(), String> {
        match parameter {
            "frequency" => {
                self.params.frequency.set_bipolar(value);
                Ok(())
            }
            "decay" => {
                self.params.decay.set_bipolar(value);
                Ok(())
            }
            "brightness" | "crack" => {
                self.params.brightness.set_bipolar(value);
                Ok(())
            }
            "volume" => {
                self.params.volume.set_bipolar(value);
                Ok(())
            }
            "tonal" => {
                self.params.tonal.set_bipolar(value);
                Ok(())
            }
            "noise" => {
                self.params.noise.set_bipolar(value);
                Ok(())
            }
            "pitch_drop" => {
                self.params.pitch_drop.set_bipolar(value);
                Ok(())
            }
            // DS-style parameters
            "tonal_decay" => {
                self.params.tonal_decay.set_bipolar(value);
                Ok(())
            }
            "tonal_decay_curve" => {
                self.params.tonal_decay_curve.set_bipolar(value);
                Ok(())
            }
            "noise_decay" => {
                self.params.noise_decay.set_bipolar(value);
                Ok(())
            }
            "noise_tail_decay" => {
                self.params.noise_tail_decay.set_bipolar(value);
                Ok(())
            }
            "filter_cutoff" => {
                self.params.filter_cutoff.set_bipolar(value);
                Ok(())
            }
            "filter_resonance" => {
                self.params.filter_resonance.set_bipolar(value);
                Ok(())
            }
            "xfade" => {
                self.params.xfade.set_bipolar(value);
                Ok(())
            }
            "phase_mod_amount" => {
                self.params.phase_mod_amount.set_bipolar(value);
                Ok(())
            }
            // New parameters
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
        match parameter {
            "frequency" => Some(self.params.frequency.range()),
            "decay" => Some(self.params.decay.range()),
            "brightness" | "crack" => Some(self.params.brightness.range()),
            "volume" => Some(self.params.volume.range()),
            "tonal" => Some(self.params.tonal.range()),
            "noise" => Some(self.params.noise.range()),
            "pitch_drop" => Some(self.params.pitch_drop.range()),
            // DS-style parameters
            "tonal_decay" => Some(self.params.tonal_decay.range()),
            "tonal_decay_curve" => Some(self.params.tonal_decay_curve.range()),
            "noise_decay" => Some(self.params.noise_decay.range()),
            "noise_tail_decay" => Some(self.params.noise_tail_decay.range()),
            "filter_cutoff" => Some(self.params.filter_cutoff.range()),
            "filter_resonance" => Some(self.params.filter_resonance.range()),
            "xfade" => Some(self.params.xfade.range()),
            "phase_mod_amount" => Some(self.params.phase_mod_amount.range()),
            // New parameters
            "overdrive" => Some(self.params.overdrive.range()),
            "amp_decay" => Some(self.params.amp_decay.range()),
            "amp_decay_curve" => Some(self.params.amp_decay_curve.range()),
            _ => None,
        }
    }
}
