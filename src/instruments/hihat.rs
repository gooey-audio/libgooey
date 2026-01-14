use crate::envelope::{ADSRConfig, Envelope};
use crate::gen::oscillator::Oscillator;
use crate::gen::waveform::Waveform;

#[derive(Clone, Copy, Debug)]
pub struct HiHatConfig {
    pub base_frequency: f32, // Base frequency for filtering (6000-12000Hz typical)
    pub resonance: f32,      // Filter resonance (0.0-1.0)
    pub brightness: f32,     // High-frequency content (0.0-1.0)
    pub decay_time: f32,     // Decay length in seconds
    pub attack_time: f32,    // Attack time in seconds
    pub volume: f32,         // Overall volume (0.0-1.0)
    pub is_open: bool,       // true for open, false for closed
}

impl HiHatConfig {
    pub fn new(
        base_frequency: f32,
        resonance: f32,
        brightness: f32,
        decay_time: f32,
        attack_time: f32,
        volume: f32,
        is_open: bool,
    ) -> Self {
        Self {
            base_frequency: base_frequency.max(4000.0).min(16000.0), // Reasonable hi-hat range
            resonance: resonance.clamp(0.0, 1.0),
            brightness: brightness.clamp(0.0, 1.0),
            decay_time: decay_time.max(0.01).min(3.0), // Reasonable decay range
            attack_time: attack_time.max(0.001).min(0.1), // Quick attack for hi-hats
            volume: volume.clamp(0.0, 1.0),
            is_open,
        }
    }

    pub fn closed_default() -> Self {
        Self::new(8000.0, 0.7, 0.6, 0.1, 0.001, 0.8, false)
    }

    pub fn open_default() -> Self {
        Self::new(8000.0, 0.5, 0.8, 0.8, 0.001, 0.7, true)
    }

    pub fn closed_tight() -> Self {
        Self::new(10000.0, 0.8, 0.5, 0.05, 0.001, 0.9, false)
    }

    pub fn open_bright() -> Self {
        Self::new(12000.0, 0.4, 1.0, 1.2, 0.001, 0.8, true)
    }

    pub fn closed_dark() -> Self {
        Self::new(6000.0, 0.6, 0.3, 0.15, 0.002, 0.7, false)
    }

    pub fn open_long() -> Self {
        Self::new(7000.0, 0.3, 0.7, 2.0, 0.001, 0.6, true)
    }
}

pub struct HiHat {
    pub sample_rate: f32,
    pub config: HiHatConfig,

    // Noise oscillators for different frequency ranges
    pub noise_oscillator: Oscillator,      // Main noise source
    pub brightness_oscillator: Oscillator, // High-frequency emphasis

    // Amplitude envelope
    pub amplitude_envelope: Envelope,

    pub is_active: bool,

    // Velocity-responsive state
    /// Current trigger velocity (0.0-1.0), set on trigger
    current_velocity: f32,

    // Velocity scaling configuration
    // Note: Hi-hat velocity relationships are OPPOSITE to kick drum!
    // On real cymbals, harder hits = more resonance = longer decay
    /// How much velocity affects decay time (0.0-1.0)
    /// Higher velocity = LONGER decay (more cymbal ring)
    /// This is opposite to kick drum behavior
    velocity_to_decay: f32,

    /// How much velocity affects brightness (0.0-1.0)
    /// Higher velocity = brighter sound (more high-frequency content)
    velocity_to_brightness: f32,

    /// How much velocity affects resonance (0.0-1.0)
    /// Higher velocity = more resonant shimmer
    velocity_to_resonance: f32,
}

impl HiHat {
    pub fn new(sample_rate: f32) -> Self {
        let config = HiHatConfig::closed_default();
        Self::with_config(sample_rate, config)
    }

    pub fn new_closed(sample_rate: f32) -> Self {
        let config = HiHatConfig::closed_default();
        Self::with_config(sample_rate, config)
    }

    pub fn new_open(sample_rate: f32) -> Self {
        let config = HiHatConfig::open_default();
        Self::with_config(sample_rate, config)
    }

    pub fn with_config(sample_rate: f32, config: HiHatConfig) -> Self {
        let mut hihat = Self {
            sample_rate,
            config,
            noise_oscillator: Oscillator::new(sample_rate, config.base_frequency),
            brightness_oscillator: Oscillator::new(sample_rate, config.base_frequency * 2.0),
            amplitude_envelope: Envelope::new(),
            is_active: false,

            // Initialize velocity state
            current_velocity: 1.0,

            // Velocity scaling for natural cymbal behavior:
            // - Decay increases with velocity (0.5 = 50-100% range)
            velocity_to_decay: 0.5,
            // - Brightness increases strongly with velocity (0.6 = 40-100% range)
            velocity_to_brightness: 0.6,
            // - Resonance increases subtly with velocity (0.3 = 70-100% range)
            velocity_to_resonance: 0.3,
        };

        hihat.configure_oscillators();
        hihat
    }

    fn configure_oscillators(&mut self) {
        let config = self.config;

        // Main noise oscillator
        self.noise_oscillator.waveform = Waveform::Noise;
        self.noise_oscillator.frequency_hz = config.base_frequency;
        self.noise_oscillator.set_volume(config.volume);

        // Configure envelope based on open/closed type
        if config.is_open {
            // Open hi-hat: longer decay, more sustain
            self.noise_oscillator.set_adsr(ADSRConfig::new(
                config.attack_time,      // Quick attack
                config.decay_time * 0.3, // Medium decay
                0.3,                     // Some sustain for open sound
                config.decay_time * 0.7, // Longer release
            ));
        } else {
            // Closed hi-hat: very short decay, no sustain
            self.noise_oscillator.set_adsr(ADSRConfig::new(
                config.attack_time,      // Quick attack
                config.decay_time * 0.8, // Most of the decay
                0.0,                     // No sustain for closed sound
                config.decay_time * 0.2, // Short release
            ));
        }

        // Brightness oscillator for high-frequency emphasis
        self.brightness_oscillator.waveform = Waveform::Noise;
        self.brightness_oscillator.frequency_hz = config.base_frequency * 2.0;
        self.brightness_oscillator
            .set_volume(config.brightness * config.volume * 0.5);

        // Brightness has a shorter envelope for transient emphasis
        self.brightness_oscillator.set_adsr(ADSRConfig::new(
            config.attack_time,      // Quick attack
            config.decay_time * 0.3, // Shorter decay for brightness
            0.0,                     // No sustain
            config.decay_time * 0.1, // Very short release
        ));

        // Amplitude envelope for overall shaping
        if config.is_open {
            self.amplitude_envelope.set_config(ADSRConfig::new(
                config.attack_time,      // Quick attack
                config.decay_time * 0.4, // Medium decay
                0.2,                     // Low sustain
                config.decay_time * 0.6, // Longer release for open sound
            ));
        } else {
            self.amplitude_envelope.set_config(ADSRConfig::new(
                config.attack_time,      // Quick attack
                config.decay_time * 0.9, // Most of the decay
                0.0,                     // No sustain for closed sound
                config.decay_time * 0.1, // Very short release
            ));
        }
    }

    pub fn set_config(&mut self, config: HiHatConfig) {
        self.config = config;
        self.configure_oscillators();
    }

    /// Trigger at default velocity (convenience method)
    pub fn trigger(&mut self, time: f32) {
        self.trigger_with_velocity(time, 0.5);
    }

    /// Trigger with velocity (0.0-1.0)
    ///
    /// Velocity affects hi-hat characteristics naturally:
    /// - Higher velocity = LONGER decay (more cymbal ring/resonance)
    /// - Higher velocity = brighter sound (more high-frequency content)
    /// - Higher velocity = more resonant shimmer
    ///
    /// Note: This is OPPOSITE to kick drum behavior! Real cymbals
    /// ring longer when struck harder because more energy excites
    /// more resonant modes.
    pub fn trigger_with_velocity(&mut self, time: f32, velocity: f32) {
        self.current_velocity = velocity.clamp(0.0, 1.0);
        self.is_active = true;

        let vel = self.current_velocity;

        // Quadratic curve for natural acoustic-like response
        let vel_squared = vel * vel;

        // --- Decay time scaling (OPPOSITE to kick!) ---
        // Higher velocity = LONGER decay (more cymbal ring)
        // Scale: 0.5 at vel=0 up to 1.0 at vel=1
        // This gives soft hits 50% of the decay, hard hits full decay
        let decay_scale = (1.0 - self.velocity_to_decay) + (self.velocity_to_decay * vel_squared);

        // Get velocity-scaled decay time
        let config = self.config;
        let scaled_decay = config.decay_time * decay_scale;

        // Configure noise oscillator envelope with velocity-scaled decay
        if config.is_open {
            // Open hi-hat: longer decay, some sustain
            self.noise_oscillator.set_adsr(ADSRConfig::new(
                config.attack_time,
                scaled_decay * 0.3,
                0.3,
                scaled_decay * 0.7,
            ));
        } else {
            // Closed hi-hat: shorter decay, no sustain
            self.noise_oscillator.set_adsr(ADSRConfig::new(
                config.attack_time,
                scaled_decay * 0.8,
                0.0,
                scaled_decay * 0.2,
            ));
        }

        // Configure brightness oscillator with velocity-scaled (shorter) decay
        // Brightness always has a faster decay to emphasize the transient
        let brightness_decay = scaled_decay * 0.3;
        self.brightness_oscillator.set_adsr(ADSRConfig::new(
            config.attack_time,
            brightness_decay,
            0.0,
            brightness_decay * 0.3,
        ));

        // Configure amplitude envelope with velocity-scaled decay
        if config.is_open {
            self.amplitude_envelope.set_config(ADSRConfig::new(
                config.attack_time,
                scaled_decay * 0.4,
                0.2,
                scaled_decay * 0.6,
            ));
        } else {
            self.amplitude_envelope.set_config(ADSRConfig::new(
                config.attack_time,
                scaled_decay * 0.9,
                0.0,
                scaled_decay * 0.1,
            ));
        }

        // Trigger all oscillators
        self.noise_oscillator.trigger(time);
        self.brightness_oscillator.trigger(time);

        // Trigger amplitude envelope
        self.amplitude_envelope.trigger(time);
    }

    pub fn release(&mut self, time: f32) {
        if self.is_active {
            self.noise_oscillator.release(time);
            self.brightness_oscillator.release(time);
            self.amplitude_envelope.release(time);
        }
    }

    pub fn tick(&mut self, current_time: f32) -> f32 {
        if !self.is_active {
            return 0.0;
        }

        let vel = self.current_velocity;
        let vel_squared = vel * vel;

        // --- Velocity-responsive brightness scaling ---
        // Higher velocity = more brightness contribution
        // Scale: (1 - velocity_to_brightness) at vel=0 up to 1.0 at vel=1
        let brightness_scale =
            (1.0 - self.velocity_to_brightness) + (self.velocity_to_brightness * vel_squared);

        // Get outputs from oscillators
        let noise_output = self.noise_oscillator.tick(current_time);
        let brightness_output = self.brightness_oscillator.tick(current_time) * brightness_scale;

        // Combine oscillator outputs
        let combined_output = noise_output + brightness_output;

        // Apply amplitude envelope
        let amplitude = self.amplitude_envelope.get_amplitude(current_time);
        let enveloped_output = combined_output * amplitude;

        // --- Velocity-responsive resonance scaling ---
        // Higher velocity = more resonant shimmer
        // Base resonance + velocity-scaled additional resonance
        let resonance_scale =
            (1.0 - self.velocity_to_resonance) + (self.velocity_to_resonance * vel_squared);
        let effective_resonance = self.config.resonance * resonance_scale;
        let resonance_factor = 1.0 + effective_resonance * 0.5;
        let resonant_output = enveloped_output * resonance_factor;

        // --- Velocity amplitude scaling (sqrt for perceptual loudness) ---
        let velocity_amplitude = vel.sqrt();
        let final_output = resonant_output * velocity_amplitude;

        // Check if hi-hat is still active
        if !self.noise_oscillator.envelope.is_active
            && !self.brightness_oscillator.envelope.is_active
            && !self.amplitude_envelope.is_active
        {
            self.is_active = false;
        }

        final_output
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.config.volume = volume.clamp(0.0, 1.0);
        self.configure_oscillators();
    }

    pub fn set_frequency(&mut self, frequency: f32) {
        self.config.base_frequency = frequency.max(4000.0).min(16000.0);
        self.configure_oscillators();
    }

    pub fn set_decay(&mut self, decay_time: f32) {
        self.config.decay_time = decay_time.max(0.01).min(3.0);
        self.configure_oscillators();
    }

    pub fn set_brightness(&mut self, brightness: f32) {
        self.config.brightness = brightness.clamp(0.0, 1.0);
        self.configure_oscillators();
    }

    pub fn set_resonance(&mut self, resonance: f32) {
        self.config.resonance = resonance.clamp(0.0, 1.0);
        self.configure_oscillators();
    }

    pub fn set_attack(&mut self, attack_time: f32) {
        self.config.attack_time = attack_time.max(0.001).min(0.1);
        self.configure_oscillators();
    }

    pub fn set_open(&mut self, is_open: bool) {
        self.config.is_open = is_open;
        self.configure_oscillators();
    }
}

// Implement the Instrument trait for engine compatibility
impl crate::engine::Instrument for HiHat {
    fn trigger_with_velocity(&mut self, time: f32, velocity: f32) {
        HiHat::trigger_with_velocity(self, time, velocity);
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

// Implement modulation support for HiHat
impl crate::engine::Modulatable for HiHat {
    fn modulatable_parameters(&self) -> Vec<&'static str> {
        vec!["decay", "brightness", "resonance", "frequency", "attack"]
    }

    fn apply_modulation(&mut self, parameter: &str, value: f32) -> Result<(), String> {
        match parameter {
            "decay" => {
                // value is -1.0 to 1.0, map to decay range
                let range = self.parameter_range("decay").unwrap();
                self.config.decay_time = range.0 + (value + 1.0) * 0.5 * (range.1 - range.0);

                // Update the amplitude envelope decay time directly (efficient!)
                if self.config.is_open {
                    self.amplitude_envelope
                        .set_decay_time(self.config.decay_time * 0.4);
                    self.amplitude_envelope
                        .set_release_time(self.config.decay_time * 0.6);
                } else {
                    self.amplitude_envelope
                        .set_decay_time(self.config.decay_time * 0.9);
                    self.amplitude_envelope
                        .set_release_time(self.config.decay_time * 0.1);
                }
                Ok(())
            }
            "brightness" => {
                // Map -1.0 to 1.0 -> 0.0 to 1.0
                self.config.brightness = ((value + 1.0) * 0.5).clamp(0.0, 1.0);
                Ok(())
            }
            "resonance" => {
                self.config.resonance = ((value + 1.0) * 0.5).clamp(0.0, 1.0);
                Ok(())
            }
            "frequency" => {
                let range = self.parameter_range("frequency").unwrap();
                self.config.base_frequency = range.0 + (value + 1.0) * 0.5 * (range.1 - range.0);
                self.configure_oscillators();
                Ok(())
            }
            "attack" => {
                let range = self.parameter_range("attack").unwrap();
                self.config.attack_time = range.0 + (value + 1.0) * 0.5 * (range.1 - range.0);

                // Update envelope attack time directly
                self.amplitude_envelope
                    .set_attack_time(self.config.attack_time);
                Ok(())
            }
            _ => Err(format!("Unknown parameter: {}", parameter)),
        }
    }

    fn parameter_range(&self, parameter: &str) -> Option<(f32, f32)> {
        match parameter {
            "decay" => Some((0.02, 0.5)), // 20ms to 500ms
            "brightness" => Some((0.0, 1.0)),
            "resonance" => Some((0.0, 1.0)),
            "frequency" => Some((5000.0, 15000.0)),
            "attack" => Some((0.0001, 0.01)), // 0.1ms to 10ms
            _ => None,
        }
    }
}
