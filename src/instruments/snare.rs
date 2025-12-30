use crate::envelope::{ADSRConfig, Envelope};
use crate::gen::oscillator::Oscillator;
use crate::gen::waveform::Waveform;

#[derive(Clone, Copy, Debug)]
pub struct SnareConfig {
    pub snare_frequency: f32, // Base frequency (150-400Hz typical)
    pub tonal_amount: f32,    // Tonal component presence (0.0-1.0)
    pub noise_amount: f32,    // Noise component presence (0.0-1.0)
    pub crack_amount: f32,    // High-frequency crack (0.0-1.0)
    pub decay_time: f32,      // Overall decay length in seconds
    pub pitch_drop: f32,      // Frequency sweep amount (0.0-1.0)
    pub volume: f32,          // Overall volume (0.0-1.0)
}

impl SnareConfig {
    pub fn new(
        snare_frequency: f32,
        tonal_amount: f32,
        noise_amount: f32,
        crack_amount: f32,
        decay_time: f32,
        pitch_drop: f32,
        volume: f32,
    ) -> Self {
        Self {
            snare_frequency: snare_frequency.max(100.0).min(600.0), // Reasonable snare range
            tonal_amount: tonal_amount.clamp(0.0, 1.0),
            noise_amount: noise_amount.clamp(0.0, 1.0),
            crack_amount: crack_amount.clamp(0.0, 1.0),
            decay_time: decay_time.max(0.01).min(2.0), // Reasonable decay range for snare
            pitch_drop: pitch_drop.clamp(0.0, 1.0),
            volume: volume.clamp(0.0, 1.0),
        }
    }

    pub fn default() -> Self {
        Self::new(200.0, 0.4, 0.7, 0.5, 0.15, 0.3, 0.8)
    }

    pub fn crispy() -> Self {
        Self::new(250.0, 0.3, 0.8, 0.7, 0.12, 0.4, 0.85)
    }

    pub fn deep() -> Self {
        Self::new(180.0, 0.6, 0.6, 0.3, 0.2, 0.2, 0.9)
    }

    pub fn tight() -> Self {
        Self::new(220.0, 0.3, 0.8, 0.8, 0.08, 0.5, 0.8)
    }

    pub fn fat() -> Self {
        Self::new(160.0, 0.7, 0.5, 0.4, 0.25, 0.1, 0.9)
    }
}

pub struct SnareDrum {
    pub sample_rate: f32,
    pub config: SnareConfig,

    // Three oscillators for different components
    pub tonal_oscillator: Oscillator,  // Tonal component (triangle/sine)
    pub noise_oscillator: Oscillator,  // Main noise component
    pub crack_oscillator: Oscillator,  // High-frequency crack

    // Pitch envelope for frequency sweeping
    pub pitch_envelope: Envelope,
    pub base_frequency: f32,
    pub pitch_start_multiplier: f32,

    pub is_active: bool,
}

impl SnareDrum {
    pub fn new(sample_rate: f32) -> Self {
        let config = SnareConfig::default();
        Self::with_config(sample_rate, config)
    }

    pub fn with_config(sample_rate: f32, config: SnareConfig) -> Self {
        let mut snare = Self {
            sample_rate,
            config,
            tonal_oscillator: Oscillator::new(sample_rate, config.snare_frequency),
            noise_oscillator: Oscillator::new(sample_rate, config.snare_frequency * 8.0),
            crack_oscillator: Oscillator::new(sample_rate, config.snare_frequency * 25.0),
            pitch_envelope: Envelope::new(),
            base_frequency: config.snare_frequency,
            pitch_start_multiplier: 1.0 + config.pitch_drop * 1.5, // Start 1-2.5x higher
            is_active: false,
        };

        snare.configure_oscillators();
        snare
    }

    fn configure_oscillators(&mut self) {
        let config = self.config;

        // Tonal oscillator: Triangle wave for body
        self.tonal_oscillator.waveform = Waveform::Triangle;
        self.tonal_oscillator.frequency_hz = config.snare_frequency;
        self.tonal_oscillator
            .set_volume(config.tonal_amount * config.volume);
        self.tonal_oscillator.set_adsr(ADSRConfig::new(
            0.001,                   // Very fast attack
            config.decay_time * 0.8, // Main decay
            0.0,                     // No sustain - drums should decay to silence
            config.decay_time * 0.4, // Medium release
        ));

        // Noise oscillator: Main snare noise character
        self.noise_oscillator.waveform = Waveform::Noise;
        self.noise_oscillator.frequency_hz = config.snare_frequency * 8.0;
        self.noise_oscillator
            .set_volume(config.noise_amount * config.volume * 0.8);
        self.noise_oscillator.set_adsr(ADSRConfig::new(
            0.001,                   // Very fast attack
            config.decay_time * 0.6, // Shorter decay for noise
            0.0,                     // No sustain - drums should decay to silence
            config.decay_time * 0.3, // Quick release
        ));

        // Crack oscillator: High-frequency transient
        self.crack_oscillator.waveform = Waveform::Noise;
        self.crack_oscillator.frequency_hz = config.snare_frequency * 25.0;
        self.crack_oscillator
            .set_volume(config.crack_amount * config.volume * 0.4);
        self.crack_oscillator.set_adsr(ADSRConfig::new(
            0.001,                    // Very fast attack
            config.decay_time * 0.2,  // Very short decay for crack
            0.0,                      // No sustain
            config.decay_time * 0.1,  // Very short release
        ));

        // Pitch envelope: Fast attack, quick decay for frequency sweeping
        self.pitch_envelope.set_config(ADSRConfig::new(
            0.001,                   // Instant attack
            config.decay_time * 0.3, // Quick pitch drop
            0.0,                     // Drop to base frequency
            config.decay_time * 0.1, // Quick release
        ));
    }

    pub fn set_config(&mut self, config: SnareConfig) {
        self.config = config;
        self.base_frequency = config.snare_frequency;
        self.pitch_start_multiplier = 1.0 + config.pitch_drop * 1.5;
        self.configure_oscillators();
    }

    pub fn trigger(&mut self, time: f32) {
        self.is_active = true;

        // Trigger all oscillators
        self.tonal_oscillator.trigger(time);
        self.noise_oscillator.trigger(time);
        self.crack_oscillator.trigger(time);

        // Trigger pitch envelope
        self.pitch_envelope.trigger(time);
    }

    pub fn release(&mut self, time: f32) {
        if self.is_active {
            self.tonal_oscillator.release(time);
            self.noise_oscillator.release(time);
            self.crack_oscillator.release(time);
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

        // Apply pitch envelope to tonal oscillator only
        self.tonal_oscillator.frequency_hz = self.base_frequency * frequency_multiplier;

        // Noise components don't get pitch modulation to maintain their character

        // Sum all oscillator outputs
        let tonal_output = self.tonal_oscillator.tick(current_time);
        let noise_output = self.noise_oscillator.tick(current_time);
        let crack_output = self.crack_oscillator.tick(current_time);

        let total_output = tonal_output + noise_output + crack_output;

        // Check if snare is still active
        if !self.tonal_oscillator.envelope.is_active
            && !self.noise_oscillator.envelope.is_active
            && !self.crack_oscillator.envelope.is_active
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
        self.config.snare_frequency = frequency.max(100.0).min(600.0);
        self.base_frequency = self.config.snare_frequency;
        self.configure_oscillators();
    }

    pub fn set_decay(&mut self, decay_time: f32) {
        self.config.decay_time = decay_time.max(0.01).min(2.0);
        self.configure_oscillators();
    }

    pub fn set_tonal(&mut self, tonal_amount: f32) {
        self.config.tonal_amount = tonal_amount.clamp(0.0, 1.0);
        self.configure_oscillators();
    }

    pub fn set_noise(&mut self, noise_amount: f32) {
        self.config.noise_amount = noise_amount.clamp(0.0, 1.0);
        self.configure_oscillators();
    }

    pub fn set_crack(&mut self, crack_amount: f32) {
        self.config.crack_amount = crack_amount.clamp(0.0, 1.0);
        self.configure_oscillators();
    }

    pub fn set_pitch_drop(&mut self, pitch_drop: f32) {
        self.config.pitch_drop = pitch_drop.clamp(0.0, 1.0);
        self.pitch_start_multiplier = 1.0 + self.config.pitch_drop * 1.5;
    }
}