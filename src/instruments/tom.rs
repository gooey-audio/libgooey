use crate::envelope::{ADSRConfig, Envelope};
use crate::gen::oscillator::Oscillator;
use crate::gen::waveform::Waveform;

#[derive(Clone, Copy, Debug)]
pub struct TomConfig {
    pub tom_frequency: f32,  // Base frequency (80-300Hz typical for toms)
    pub tonal_amount: f32,   // Tonal component presence (0.0-1.0)
    pub punch_amount: f32,   // Attack/punch component presence (0.0-1.0)
    pub decay_time: f32,     // Overall decay length in seconds
    pub pitch_drop: f32,     // Frequency sweep amount (0.0-1.0)
    pub volume: f32,         // Overall volume (0.0-1.0)
}

impl TomConfig {
    pub fn new(
        tom_frequency: f32,
        tonal_amount: f32,
        punch_amount: f32,
        decay_time: f32,
        pitch_drop: f32,
        volume: f32,
    ) -> Self {
        Self {
            tom_frequency: tom_frequency.max(60.0).min(400.0), // Reasonable tom range
            tonal_amount: tonal_amount.clamp(0.0, 1.0),
            punch_amount: punch_amount.clamp(0.0, 1.0),
            decay_time: decay_time.max(0.05).min(3.0), // Reasonable decay range for toms
            pitch_drop: pitch_drop.clamp(0.0, 1.0),
            volume: volume.clamp(0.0, 1.0),
        }
    }

    pub fn default() -> Self {
        Self::new(120.0, 0.8, 0.4, 0.4, 0.3, 0.8)
    }

    pub fn high_tom() -> Self {
        Self::new(180.0, 0.9, 0.5, 0.3, 0.4, 0.85)
    }

    pub fn mid_tom() -> Self {
        Self::new(120.0, 0.8, 0.4, 0.4, 0.3, 0.8)
    }

    pub fn low_tom() -> Self {
        Self::new(90.0, 0.7, 0.3, 0.6, 0.2, 0.85)
    }

    pub fn floor_tom() -> Self {
        Self::new(70.0, 0.6, 0.2, 0.8, 0.15, 0.9)
    }
}

pub struct TomDrum {
    pub sample_rate: f32,
    pub config: TomConfig,

    // Two oscillators for tom character
    pub tonal_oscillator: Oscillator,  // Main tonal component (sine/triangle)
    pub punch_oscillator: Oscillator,  // Attack/punch component

    // Pitch envelope for frequency sweeping
    pub pitch_envelope: Envelope,
    pub base_frequency: f32,
    pub pitch_start_multiplier: f32,

    pub is_active: bool,
}

impl TomDrum {
    pub fn new(sample_rate: f32) -> Self {
        let config = TomConfig::default();
        Self::with_config(sample_rate, config)
    }

    pub fn with_config(sample_rate: f32, config: TomConfig) -> Self {
        let mut tom = Self {
            sample_rate,
            config,
            tonal_oscillator: Oscillator::new(sample_rate, config.tom_frequency),
            punch_oscillator: Oscillator::new(sample_rate, config.tom_frequency * 3.0),
            pitch_envelope: Envelope::new(),
            base_frequency: config.tom_frequency,
            pitch_start_multiplier: 1.0 + config.pitch_drop * 1.0, // More subtle pitch drop than snare
            is_active: false,
        };

        tom.configure_oscillators();
        tom
    }

    fn configure_oscillators(&mut self) {
        let config = self.config;

        // Tonal oscillator: Sine wave for body/tone
        self.tonal_oscillator.waveform = Waveform::Sine;
        self.tonal_oscillator.frequency_hz = config.tom_frequency;
        self.tonal_oscillator
            .set_volume(config.tonal_amount * config.volume);
        self.tonal_oscillator.set_adsr(ADSRConfig::new(
            0.001,                    // Very fast attack
            config.decay_time * 0.9,  // Main decay
            0.0,                      // No sustain - drums should decay to silence
            config.decay_time * 0.3,  // Medium release
        ));

        // Punch oscillator: Triangle wave for attack character
        self.punch_oscillator.waveform = Waveform::Triangle;
        self.punch_oscillator.frequency_hz = config.tom_frequency * 3.0;
        self.punch_oscillator
            .set_volume(config.punch_amount * config.volume * 0.6);
        self.punch_oscillator.set_adsr(ADSRConfig::new(
            0.001,                    // Very fast attack
            config.decay_time * 0.3,  // Short decay for punch
            0.0,                      // No sustain for punch
            config.decay_time * 0.1,  // Quick release
        ));

        // Pitch envelope: Fast attack, medium decay for frequency sweeping
        self.pitch_envelope.set_config(ADSRConfig::new(
            0.001,                    // Instant attack
            config.decay_time * 0.4,  // Medium pitch drop
            0.0,                      // Drop to base frequency
            config.decay_time * 0.2,  // Medium release
        ));
    }

    pub fn set_config(&mut self, config: TomConfig) {
        self.config = config;
        self.base_frequency = config.tom_frequency;
        self.pitch_start_multiplier = 1.0 + config.pitch_drop * 1.0;
        self.configure_oscillators();
    }

    pub fn trigger(&mut self, time: f32) {
        self.is_active = true;

        // Trigger both oscillators
        self.tonal_oscillator.trigger(time);
        self.punch_oscillator.trigger(time);

        // Trigger pitch envelope
        self.pitch_envelope.trigger(time);
    }

    pub fn release(&mut self, time: f32) {
        if self.is_active {
            self.tonal_oscillator.release(time);
            self.punch_oscillator.release(time);
            self.pitch_envelope.release(time);
        }
    }

    pub fn tick(&mut self, current_time: f32) -> f32 {
        if !self.is_active {
            return 0.0;
        }

        // Calculate pitch modulation
        let pitch_envelope_value = self.pitch_envelope.get_amplitude(current_time);
        let frequency_multiplier = 1.0 + (self.pitch_start_multiplier - 1.0) * pitch_envelope_value;

        // Apply pitch envelope to tonal oscillator
        self.tonal_oscillator.frequency_hz = self.base_frequency * frequency_multiplier;

        // Punch oscillator gets a more subtle pitch modulation
        self.punch_oscillator.frequency_hz = self.base_frequency * 3.0 * (1.0 + (frequency_multiplier - 1.0) * 0.5);

        // Sum oscillator outputs
        let tonal_output = self.tonal_oscillator.tick(current_time);
        let punch_output = self.punch_oscillator.tick(current_time);

        let total_output = tonal_output + punch_output;

        // Check if tom is still active
        if !self.tonal_oscillator.envelope.is_active
            && !self.punch_oscillator.envelope.is_active
        {
            self.is_active = false;
        }

        total_output
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.config.volume = volume.clamp(0.0, 1.0);
        self.configure_oscillators();
    }

    pub fn set_frequency(&mut self, frequency: f32) {
        self.config.tom_frequency = frequency.max(60.0).min(400.0);
        self.base_frequency = self.config.tom_frequency;
        self.configure_oscillators();
    }

    pub fn set_decay(&mut self, decay_time: f32) {
        self.config.decay_time = decay_time.max(0.05).min(3.0);
        self.configure_oscillators();
    }

    pub fn set_tonal(&mut self, tonal_amount: f32) {
        self.config.tonal_amount = tonal_amount.clamp(0.0, 1.0);
        self.configure_oscillators();
    }

    pub fn set_punch(&mut self, punch_amount: f32) {
        self.config.punch_amount = punch_amount.clamp(0.0, 1.0);
        self.configure_oscillators();
    }

    pub fn set_pitch_drop(&mut self, pitch_drop: f32) {
        self.config.pitch_drop = pitch_drop.clamp(0.0, 1.0);
        self.pitch_start_multiplier = 1.0 + self.config.pitch_drop * 1.0;
    }
}