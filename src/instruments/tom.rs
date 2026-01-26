use crate::envelope::{ADSRConfig, Envelope, EnvelopeCurve};
use crate::gen::oscillator::Oscillator;
use crate::gen::waveform::Waveform;
use crate::utils::smoother::{SmoothedParam, DEFAULT_SMOOTH_TIME_MS};

/// Normalization ranges for tom drum parameters
/// All external-facing parameters use 0.0-1.0 normalized values
pub(crate) mod ranges {
    /// Frequency: 0-1 maps to 60-300 Hz
    pub const FREQ_MIN: f32 = 60.0;
    pub const FREQ_MAX: f32 = 300.0;

    /// Oscillator decay: 0-1 maps to 0.05-2.0 seconds
    pub const DECAY_MIN: f32 = 0.05;
    pub const DECAY_MAX: f32 = 2.0;

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

/// Static configuration for tom drum presets
/// All parameters use normalized 0.0-1.0 values for easy integration with external systems.
#[derive(Clone, Copy, Debug)]
pub struct TomConfig {
    pub frequency: f32,       // Base frequency (0-1 -> 60-300Hz)
    pub tonal_amount: f32,    // Tonal component presence (0.0-1.0)
    pub punch_amount: f32,    // Attack/punch component presence (0.0-1.0)
    pub decay: f32,           // Oscillator decay time (0-1 -> 0.05-2.0s)
    pub pitch_drop: f32,      // Frequency sweep amount (0.0-1.0)
    pub volume: f32,          // Overall volume (0.0-1.0)
    pub amp_decay: f32,       // Master amplitude decay time (0-1 -> 0.0-4.0s)
    pub amp_decay_curve: f32, // Decay curve shape (0-1 -> 0.1-10.0, lower = steep-then-long)
}

impl TomConfig {
    pub fn new(
        frequency: f32,
        tonal_amount: f32,
        punch_amount: f32,
        decay: f32,
        pitch_drop: f32,
        volume: f32,
    ) -> Self {
        Self {
            frequency: frequency.clamp(0.0, 1.0),
            tonal_amount: tonal_amount.clamp(0.0, 1.0),
            punch_amount: punch_amount.clamp(0.0, 1.0),
            decay: decay.clamp(0.0, 1.0),
            pitch_drop: pitch_drop.clamp(0.0, 1.0),
            volume: volume.clamp(0.0, 1.0),
            // Default amp envelope settings
            amp_decay: 0.2,       // ~0.8s
            amp_decay_curve: 0.02, // ~0.3 (steep-then-long)
        }
    }

    /// Create a TomConfig with all parameters (all normalized 0-1)
    pub fn new_full(
        frequency: f32,
        tonal_amount: f32,
        punch_amount: f32,
        decay: f32,
        pitch_drop: f32,
        volume: f32,
        amp_decay: f32,
        amp_decay_curve: f32,
    ) -> Self {
        Self {
            frequency: frequency.clamp(0.0, 1.0),
            tonal_amount: tonal_amount.clamp(0.0, 1.0),
            punch_amount: punch_amount.clamp(0.0, 1.0),
            decay: decay.clamp(0.0, 1.0),
            pitch_drop: pitch_drop.clamp(0.0, 1.0),
            volume: volume.clamp(0.0, 1.0),
            amp_decay: amp_decay.clamp(0.0, 1.0),
            amp_decay_curve: amp_decay_curve.clamp(0.0, 1.0),
        }
    }

    // Helper methods to get actual (denormalized) values for audio processing

    /// Get actual frequency in Hz (60-300)
    #[inline]
    pub fn frequency_hz(&self) -> f32 {
        ranges::denormalize(self.frequency, ranges::FREQ_MIN, ranges::FREQ_MAX)
    }

    /// Get actual oscillator decay in seconds (0.05-2.0)
    #[inline]
    pub fn decay_secs(&self) -> f32 {
        ranges::denormalize(self.decay, ranges::DECAY_MIN, ranges::DECAY_MAX)
    }

    /// Get actual amp decay in seconds (0.0-4.0)
    #[inline]
    pub fn amp_decay_secs(&self) -> f32 {
        ranges::denormalize(self.amp_decay, ranges::AMP_DECAY_MIN, ranges::AMP_DECAY_MAX)
    }

    /// Get actual amp decay curve value (0.1-10.0)
    #[inline]
    pub fn amp_decay_curve_value(&self) -> f32 {
        ranges::denormalize(self.amp_decay_curve, ranges::AMP_DECAY_CURVE_MIN, ranges::AMP_DECAY_CURVE_MAX)
    }

    // Presets - using normalized 0-1 values
    // Frequency mapping: 60Hz=0.0, 120Hz=0.25, 180Hz=0.5, 240Hz=0.75, 300Hz=1.0

    pub fn default() -> Self {
        // 120Hz mid tom
        Self::new_full(
            0.25,  // frequency: 120Hz
            0.8,   // tonal
            0.4,   // punch
            0.18,  // decay: ~0.4s
            0.3,   // pitch_drop
            0.8,   // volume
            0.2,   // amp_decay: ~0.8s
            0.02,  // amp_decay_curve: ~0.3 (steep-then-long)
        )
    }

    pub fn high_tom() -> Self {
        // 180Hz high tom - brighter, shorter
        Self::new_full(
            0.5,   // frequency: 180Hz
            0.9,   // tonal
            0.5,   // punch
            0.13,  // decay: ~0.3s
            0.4,   // pitch_drop
            0.85,  // volume
            0.15,  // amp_decay: ~0.6s
            0.02,  // amp_decay_curve: ~0.3 (steep-then-long)
        )
    }

    pub fn mid_tom() -> Self {
        // 120Hz mid tom - same as default
        Self::default()
    }

    pub fn low_tom() -> Self {
        // 90Hz low tom - deeper, longer
        Self::new_full(
            0.125, // frequency: 90Hz
            0.7,   // tonal
            0.3,   // punch
            0.28,  // decay: ~0.6s
            0.2,   // pitch_drop
            0.85,  // volume
            0.3,   // amp_decay: ~1.2s
            0.02,  // amp_decay_curve: ~0.3 (steep-then-long)
        )
    }

    pub fn floor_tom() -> Self {
        // 70Hz floor tom - deepest, longest
        Self::new_full(
            0.04,  // frequency: ~70Hz
            0.6,   // tonal
            0.2,   // punch
            0.38,  // decay: ~0.8s
            0.15,  // pitch_drop
            0.9,   // volume
            0.4,   // amp_decay: ~1.6s
            0.02,  // amp_decay_curve: ~0.3 (steep-then-long)
        )
    }
}

/// Smoothed parameters for real-time control of the tom drum
/// All parameters use normalized 0-1 values
pub struct TomParams {
    pub frequency: SmoothedParam,       // Base frequency (0-1)
    pub decay: SmoothedParam,           // Oscillator decay time (0-1)
    pub volume: SmoothedParam,          // Overall volume (0-1)
    pub tonal: SmoothedParam,           // Tonal component amount (0-1)
    pub punch: SmoothedParam,           // Punch component amount (0-1)
    pub pitch_drop: SmoothedParam,      // Pitch drop amount (0-1)
    pub amp_decay: SmoothedParam,       // Master amplitude decay (0-1)
    pub amp_decay_curve: SmoothedParam, // Decay curve shape (0-1)
}

impl TomParams {
    /// Create new smoothed parameters from a config
    pub fn from_config(config: &TomConfig, sample_rate: f32) -> Self {
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
            volume: SmoothedParam::new(config.volume, 0.0, 1.0, sample_rate, DEFAULT_SMOOTH_TIME_MS),
            tonal: SmoothedParam::new(
                config.tonal_amount,
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
            pitch_drop: SmoothedParam::new(
                config.pitch_drop,
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

    /// Tick all smoothers
    #[inline]
    pub fn tick(&mut self) {
        self.frequency.tick();
        self.decay.tick();
        self.volume.tick();
        self.tonal.tick();
        self.punch.tick();
        self.pitch_drop.tick();
        self.amp_decay.tick();
        self.amp_decay_curve.tick();
    }

    // Helper methods to get denormalized values

    /// Get actual frequency in Hz (60-300)
    #[inline]
    pub fn frequency_hz(&self) -> f32 {
        ranges::denormalize(self.frequency.get(), ranges::FREQ_MIN, ranges::FREQ_MAX)
    }

    /// Get actual oscillator decay in seconds (0.05-2.0)
    #[inline]
    pub fn decay_secs(&self) -> f32 {
        ranges::denormalize(self.decay.get(), ranges::DECAY_MIN, ranges::DECAY_MAX)
    }

    /// Get actual amp decay in seconds (0.0-4.0)
    #[inline]
    pub fn amp_decay_secs(&self) -> f32 {
        ranges::denormalize(self.amp_decay.get(), ranges::AMP_DECAY_MIN, ranges::AMP_DECAY_MAX)
    }

    /// Get actual amp decay curve value (0.1-10.0)
    #[inline]
    pub fn amp_decay_curve_value(&self) -> f32 {
        ranges::denormalize(self.amp_decay_curve.get(), ranges::AMP_DECAY_CURVE_MIN, ranges::AMP_DECAY_CURVE_MAX)
    }
}

pub struct TomDrum {
    pub sample_rate: f32,
    pub config: TomConfig,

    /// Smoothed parameters for click-free real-time control
    pub params: TomParams,

    // Two oscillators for tom character
    pub tonal_oscillator: Oscillator, // Main tonal component (sine/triangle)
    pub punch_oscillator: Oscillator, // Attack/punch component

    // Pitch envelope for frequency sweeping
    pub pitch_envelope: Envelope,
    pub base_frequency: f32,
    pub pitch_start_multiplier: f32,

    // Master amplitude envelope
    pub amplitude_envelope: Envelope,

    // Velocity tracking
    pub current_velocity: f32,

    pub is_active: bool,
}

impl TomDrum {
    pub fn new(sample_rate: f32) -> Self {
        let config = TomConfig::default();
        Self::with_config(sample_rate, config)
    }

    pub fn with_config(sample_rate: f32, config: TomConfig) -> Self {
        let params = TomParams::from_config(&config, sample_rate);
        let freq_hz = config.frequency_hz();
        let decay_secs = config.decay_secs();
        let pitch_drop = config.pitch_drop;

        let mut tom = Self {
            sample_rate,
            config,
            params,
            tonal_oscillator: Oscillator::new(sample_rate, freq_hz),
            punch_oscillator: Oscillator::new(sample_rate, freq_hz * 3.0),
            pitch_envelope: Envelope::new(),
            base_frequency: freq_hz,
            pitch_start_multiplier: 1.0 + pitch_drop * 1.0,
            amplitude_envelope: Envelope::new(),
            current_velocity: 1.0,
            is_active: false,
        };

        tom.configure_oscillators(freq_hz, decay_secs, pitch_drop);
        tom
    }

    fn configure_oscillators(&mut self, freq_hz: f32, decay_secs: f32, pitch_drop: f32) {
        let tonal_amount = self.params.tonal.get();
        let punch_amount = self.params.punch.get();
        let volume = self.params.volume.get();

        // Tonal oscillator: Sine wave for body/tone
        self.tonal_oscillator.waveform = Waveform::Sine;
        self.tonal_oscillator.frequency_hz = freq_hz;
        self.tonal_oscillator.set_volume(tonal_amount * volume);
        self.tonal_oscillator.set_adsr(ADSRConfig::new(
            0.001,               // Very fast attack
            decay_secs * 0.9,    // Main decay
            0.0,                 // No sustain - drums should decay to silence
            decay_secs * 0.3,    // Medium release
        ));

        // Punch oscillator: Triangle wave for attack character
        self.punch_oscillator.waveform = Waveform::Triangle;
        self.punch_oscillator.frequency_hz = freq_hz * 3.0;
        self.punch_oscillator.set_volume(punch_amount * volume * 0.6);
        self.punch_oscillator.set_adsr(ADSRConfig::new(
            0.001,               // Very fast attack
            decay_secs * 0.3,    // Short decay for punch
            0.0,                 // No sustain for punch
            decay_secs * 0.1,    // Quick release
        ));

        // Pitch envelope: Fast attack, medium decay for frequency sweeping
        self.pitch_envelope.set_config(ADSRConfig::new(
            0.001,               // Instant attack
            decay_secs * 0.4,    // Medium pitch drop
            0.0,                 // Drop to base frequency
            decay_secs * 0.2,    // Medium release
        ));

        self.base_frequency = freq_hz;
        self.pitch_start_multiplier = 1.0 + pitch_drop * 1.0;
    }

    pub fn set_config(&mut self, config: TomConfig) {
        self.config = config;

        // Update all params
        self.params.frequency.set_target(config.frequency);
        self.params.tonal.set_target(config.tonal_amount);
        self.params.punch.set_target(config.punch_amount);
        self.params.decay.set_target(config.decay);
        self.params.pitch_drop.set_target(config.pitch_drop);
        self.params.volume.set_target(config.volume);
        self.params.amp_decay.set_target(config.amp_decay);
        self.params.amp_decay_curve.set_target(config.amp_decay_curve);

        let freq_hz = config.frequency_hz();
        let decay_secs = config.decay_secs();
        self.configure_oscillators(freq_hz, decay_secs, config.pitch_drop);
    }

    pub fn trigger(&mut self, time: f32) {
        self.trigger_with_velocity_internal(time, 1.0);
    }

    fn trigger_with_velocity_internal(&mut self, time: f32, velocity: f32) {
        self.is_active = true;
        self.current_velocity = velocity.clamp(0.0, 1.0);

        // Get current parameter values
        let freq_hz = self.params.frequency_hz();
        let decay_secs = self.params.decay_secs();
        let pitch_drop = self.params.pitch_drop.get();

        // Velocity affects decay time (softer hits = shorter decay)
        let decay_scale = 0.5 + 0.5 * self.current_velocity;
        let scaled_decay = decay_secs * decay_scale;

        // Configure oscillators with current parameters
        self.configure_oscillators(freq_hz, scaled_decay, pitch_drop);

        // Trigger both oscillators
        self.tonal_oscillator.trigger(time);
        self.punch_oscillator.trigger(time);

        // Trigger pitch envelope
        self.pitch_envelope.trigger(time);

        // Configure and trigger master amplitude envelope
        const AMP_ATTACK: f32 = 0.001;
        const AMP_ATTACK_CURVE: f32 = 0.5;
        let amp_decay = self.params.amp_decay_secs() * decay_scale;
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
    }

    pub fn release(&mut self, time: f32) {
        if self.is_active {
            self.tonal_oscillator.release(time);
            self.punch_oscillator.release(time);
            self.pitch_envelope.release(time);
            self.amplitude_envelope.release(time);
        }
    }

    pub fn tick(&mut self, current_time: f32) -> f32 {
        // Tick smoothed parameters for click-free modulation
        self.params.tick();

        if !self.is_active {
            return 0.0;
        }

        // Get smoothed parameter values
        let frequency = self.params.frequency_hz();
        let volume = self.params.volume.get();
        let tonal_amount = self.params.tonal.get();
        let punch_amount = self.params.punch.get();
        let pitch_drop = self.params.pitch_drop.get();

        // Update pitch envelope multiplier from smoothed pitch_drop
        self.pitch_start_multiplier = 1.0 + pitch_drop * 1.0;

        // Calculate pitch modulation
        let pitch_envelope_value = self.pitch_envelope.get_amplitude(current_time);
        let frequency_multiplier = 1.0 + (self.pitch_start_multiplier - 1.0) * pitch_envelope_value;

        // Apply smoothed frequency with pitch envelope
        self.tonal_oscillator.frequency_hz = frequency * frequency_multiplier;
        self.tonal_oscillator.set_volume(tonal_amount * volume);

        // Punch oscillator gets a more subtle pitch modulation
        self.punch_oscillator.frequency_hz =
            frequency * 3.0 * (1.0 + (frequency_multiplier - 1.0) * 0.5);
        self.punch_oscillator.set_volume(punch_amount * volume * 0.6);

        // Sum oscillator outputs
        let tonal_output = self.tonal_oscillator.tick(current_time);
        let punch_output = self.punch_oscillator.tick(current_time);

        let total_output = tonal_output + punch_output;

        // Apply master amplitude envelope
        let amp_env = self.amplitude_envelope.get_amplitude(current_time);

        // Apply velocity amplitude scaling (sqrt for perceptually linear loudness)
        let velocity_amplitude = self.current_velocity.sqrt();
        let final_output = total_output * amp_env * velocity_amplitude;

        // Check if tom is still active (use amplitude envelope as master)
        if !self.amplitude_envelope.is_active {
            self.is_active = false;
        }

        final_output
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.params.volume.set_target(volume.clamp(0.0, 1.0));
    }

    pub fn set_frequency(&mut self, frequency: f32) {
        self.params.frequency.set_target(frequency.clamp(0.0, 1.0));
    }

    pub fn set_decay(&mut self, decay: f32) {
        self.params.decay.set_target(decay.clamp(0.0, 1.0));
    }

    pub fn set_tonal(&mut self, tonal_amount: f32) {
        self.params.tonal.set_target(tonal_amount.clamp(0.0, 1.0));
    }

    pub fn set_punch(&mut self, punch_amount: f32) {
        self.params.punch.set_target(punch_amount.clamp(0.0, 1.0));
    }

    pub fn set_pitch_drop(&mut self, pitch_drop: f32) {
        self.params.pitch_drop.set_target(pitch_drop.clamp(0.0, 1.0));
    }

    pub fn set_amp_decay(&mut self, amp_decay: f32) {
        self.params.amp_decay.set_target(amp_decay.clamp(0.0, 1.0));
    }

    pub fn set_amp_decay_curve(&mut self, amp_decay_curve: f32) {
        self.params.amp_decay_curve.set_target(amp_decay_curve.clamp(0.0, 1.0));
    }
}

impl crate::engine::Instrument for TomDrum {
    fn trigger_with_velocity(&mut self, time: f32, velocity: f32) {
        self.trigger_with_velocity_internal(time, velocity);
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

// Implement modulation support for TomDrum
impl crate::engine::Modulatable for TomDrum {
    fn modulatable_parameters(&self) -> Vec<&'static str> {
        vec!["frequency", "tonal", "punch", "decay", "pitch_drop", "volume", "amp_decay", "amp_decay_curve"]
    }

    fn apply_modulation(&mut self, parameter: &str, value: f32) -> Result<(), String> {
        // value is -1.0 to 1.0 (bipolar), set_bipolar maps this to the param range
        match parameter {
            "frequency" => {
                self.params.frequency.set_bipolar(value);
                Ok(())
            }
            "tonal" => {
                self.params.tonal.set_bipolar(value);
                Ok(())
            }
            "punch" => {
                self.params.punch.set_bipolar(value);
                Ok(())
            }
            "decay" => {
                self.params.decay.set_bipolar(value);
                Ok(())
            }
            "pitch_drop" => {
                self.params.pitch_drop.set_bipolar(value);
                Ok(())
            }
            "volume" => {
                self.params.volume.set_bipolar(value);
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
        // All parameters are normalized 0-1
        match parameter {
            "frequency" | "tonal" | "punch" | "decay" | "pitch_drop" | "volume" | "amp_decay" | "amp_decay_curve" => {
                Some((0.0, 1.0))
            }
            _ => None,
        }
    }
}
