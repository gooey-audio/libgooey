use crate::envelope::{ADSRConfig, Envelope};
use crate::gen::oscillator::Oscillator;
use crate::gen::waveform::Waveform;
use crate::utils::{SmoothedParam, DEFAULT_SMOOTH_TIME_MS};

#[derive(Clone, Copy, Debug)]
pub struct HiHatConfig {
    pub base_frequency: f32, // Base frequency for filtering (2000-18000Hz)
    pub resonance: f32,      // Filter resonance (0.0-1.0)
    pub brightness: f32,     // High-frequency content (0.0-1.0) - also boosts filter cutoff
    pub decay_time: f32,     // Decay length in seconds (0.005-2.0)
    pub volume: f32,         // Overall volume (0.0-1.0)
    pub is_open: bool,       // true for open, false for closed
}

impl HiHatConfig {
    pub fn new(
        base_frequency: f32,
        resonance: f32,
        brightness: f32,
        decay_time: f32,
        volume: f32,
        is_open: bool,
    ) -> Self {
        Self {
            base_frequency: base_frequency.clamp(2000.0, 18000.0), // Wide filter range
            resonance: resonance.clamp(0.0, 1.0),
            brightness: brightness.clamp(0.0, 1.0),
            decay_time: decay_time.clamp(0.005, 2.0), // Ultra-tight to open
            volume: volume.clamp(0.0, 1.0),
            is_open,
        }
    }

    pub fn closed_default() -> Self {
        Self::new(8000.0, 0.7, 0.5, 0.08, 0.8, false)
    }

    pub fn open_default() -> Self {
        Self::new(10000.0, 0.5, 0.7, 0.6, 0.7, true)
    }

    pub fn closed_tight() -> Self {
        Self::new(6000.0, 0.8, 0.3, 0.015, 0.9, false)
    }

    pub fn open_bright() -> Self {
        Self::new(14000.0, 0.4, 1.0, 1.2, 0.8, true)
    }

    pub fn closed_dark() -> Self {
        Self::new(4000.0, 0.6, 0.2, 0.1, 0.7, false)
    }

    pub fn open_long() -> Self {
        Self::new(8000.0, 0.3, 0.6, 1.8, 0.6, true)
    }
}

/// Smoothed parameters for real-time control of the hi-hat
/// These use one-pole smoothing to prevent clicks/pops during parameter changes
pub struct HiHatParams {
    pub frequency: SmoothedParam,  // Output filter cutoff (2000-18000 Hz)
    pub brightness: SmoothedParam, // High-frequency emphasis (0-1) - also boosts filter cutoff
    pub resonance: SmoothedParam,  // Filter resonance boost (0-1)
    pub decay: SmoothedParam,      // Decay time in seconds (0.005-2.0)
    pub volume: SmoothedParam,     // Overall volume (0-1)
}

impl HiHatParams {
    /// Create new smoothed parameters from a config
    pub fn from_config(config: &HiHatConfig, sample_rate: f32) -> Self {
        Self {
            frequency: SmoothedParam::new(
                config.base_frequency,
                2000.0,
                18000.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            brightness: SmoothedParam::new(
                config.brightness,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            resonance: SmoothedParam::new(
                config.resonance,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            decay: SmoothedParam::new(
                config.decay_time,
                0.005,
                2.0,
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

    /// Tick all smoothers and return whether any are still smoothing
    #[inline]
    pub fn tick(&mut self) -> bool {
        self.frequency.tick();
        self.brightness.tick();
        self.resonance.tick();
        self.decay.tick();
        self.volume.tick();

        // Return true if any smoother is still active
        !self.is_settled()
    }

    /// Check if all parameters have settled
    pub fn is_settled(&self) -> bool {
        self.frequency.is_settled()
            && self.brightness.is_settled()
            && self.resonance.is_settled()
            && self.decay.is_settled()
            && self.volume.is_settled()
    }

    /// Get a snapshot of current values as a HiHatConfig (for reading back)
    pub fn to_config(&self, is_open: bool) -> HiHatConfig {
        HiHatConfig {
            base_frequency: self.frequency.get(),
            resonance: self.resonance.get(),
            brightness: self.brightness.get(),
            decay_time: self.decay.get(),
            volume: self.volume.get(),
            is_open,
        }
    }
}

pub struct HiHat {
    pub sample_rate: f32,

    /// Smoothed parameters for click-free real-time control
    pub params: HiHatParams,

    /// Whether this is an open or closed hi-hat (affects envelope shape)
    pub is_open: bool,

    // Noise oscillators for different frequency ranges
    pub noise_oscillator: Oscillator,      // Main noise source
    pub brightness_oscillator: Oscillator, // High-frequency emphasis

    // Amplitude envelope
    pub amplitude_envelope: Envelope,

    // Output lowpass filter state (one-pole, tames harshness)
    filter_state: f32,

    pub is_active: bool,
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
        let params = HiHatParams::from_config(&config, sample_rate);
        let mut hihat = Self {
            sample_rate,
            params,
            is_open: config.is_open,
            noise_oscillator: Oscillator::new(sample_rate, config.base_frequency),
            brightness_oscillator: Oscillator::new(sample_rate, config.base_frequency * 2.0),
            amplitude_envelope: Envelope::new(),
            filter_state: 0.0,
            is_active: false,
        };

        hihat.configure_oscillators();
        hihat
    }

    /// Get current config snapshot (reads current smoothed values)
    pub fn config(&self) -> HiHatConfig {
        self.params.to_config(self.is_open)
    }

    /// Configure oscillators from current smoothed parameter values
    /// Called once at initialization and when decay changes significantly
    fn configure_oscillators(&mut self) {
        let brightness = self.params.brightness.get();
        let decay = self.params.decay.get();
        let volume = self.params.volume.get();

        // Fixed fast attack for hi-hats (always percussive)
        const ATTACK: f32 = 0.001;

        // Main noise oscillator (frequency doesn't affect noise, just a placeholder)
        self.noise_oscillator.waveform = Waveform::Noise;
        self.noise_oscillator.set_volume(volume);

        // Configure envelope based on open/closed type
        if self.is_open {
            // Open hi-hat: longer decay with sustain for that "wash" sound
            self.noise_oscillator.set_adsr(ADSRConfig::new(
                ATTACK,       // Instant attack
                decay * 0.2,  // Quick initial decay
                0.4,          // Sustain for open wash
                decay * 0.8,  // Long release
            ));
        } else {
            // Closed hi-hat: very short, punchy envelope
            self.noise_oscillator.set_adsr(ADSRConfig::new(
                ATTACK,       // Instant attack
                decay,        // Full decay time
                0.0,          // No sustain for tight closed sound
                decay * 0.1,  // Very short release
            ));
        }

        // Brightness oscillator for high-frequency transient emphasis
        self.brightness_oscillator.waveform = Waveform::Noise;
        // Significantly increase brightness oscillator contribution
        self.brightness_oscillator.set_volume(brightness * volume * 0.8);

        // Brightness has a shorter envelope for transient "sizzle"
        self.brightness_oscillator.set_adsr(ADSRConfig::new(
            ATTACK,       // Instant attack
            decay * 0.2,  // Shorter decay for brightness
            0.0,          // No sustain
            decay * 0.05, // Very short release
        ));

        // Amplitude envelope for overall shaping
        if self.is_open {
            self.amplitude_envelope.set_config(ADSRConfig::new(
                ATTACK,       // Instant attack
                decay * 0.3,  // Medium decay
                0.3,          // Sustain for open sound
                decay * 0.7,  // Longer release for open sound
            ));
        } else {
            self.amplitude_envelope.set_config(ADSRConfig::new(
                ATTACK,       // Instant attack
                decay,        // Full decay time
                0.0,          // No sustain for closed sound
                decay * 0.05, // Very short release
            ));
        }
    }

    /// Apply current smoothed parameters to oscillators (called per-sample)
    #[inline]
    fn apply_params(&mut self) {
        let brightness = self.params.brightness.get();
        let volume = self.params.volume.get();

        // Update oscillator volumes (these can change smoothly)
        // Note: frequency parameter now controls the output lowpass filter cutoff
        self.noise_oscillator.set_volume(volume);
        self.brightness_oscillator.set_volume(brightness * volume * 0.5);
    }

    pub fn set_config(&mut self, config: HiHatConfig) {
        // Set all parameter targets (will smooth to new values)
        self.params.frequency.set_target(config.base_frequency);
        self.params.brightness.set_target(config.brightness);
        self.params.resonance.set_target(config.resonance);
        self.params.decay.set_target(config.decay_time);
        self.params.volume.set_target(config.volume);
        self.is_open = config.is_open;

        // Reconfigure envelopes for new decay time
        self.configure_oscillators();
    }

    pub fn trigger(&mut self, time: f32) {
        self.is_active = true;

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
        // Always tick smoothers (even when not active, to settle values)
        self.params.tick();

        if !self.is_active {
            return 0.0;
        }

        // Apply smoothed parameters to oscillators
        self.apply_params();

        // Get outputs from oscillators
        let noise_output = self.noise_oscillator.tick(current_time);
        let brightness_output = self.brightness_oscillator.tick(current_time);

        // Combine oscillator outputs
        let combined_output = noise_output + brightness_output;

        // Apply amplitude envelope
        let amplitude = self.amplitude_envelope.get_amplitude(current_time);
        let final_output = combined_output * amplitude;

        // Apply simple resonance simulation by emphasizing certain frequencies
        let resonance_factor = 1.0 + self.params.resonance.get() * 0.8;
        let resonant_output = final_output * resonance_factor;

        // Apply lowpass filter to tame harshness (one-pole filter)
        // Base cutoff from frequency parameter, significantly boosted by brightness
        let base_cutoff = self.params.frequency.get();
        let brightness = self.params.brightness.get();
        // Brightness adds up to 8kHz to the cutoff (major impact on tone)
        let cutoff = (base_cutoff + brightness * 8000.0).min(self.sample_rate * 0.49);
        let normalized_freq = cutoff / self.sample_rate;
        // One-pole coefficient: g = 1 - e^(-2*pi*fc/fs)
        let g = 1.0 - (-2.0 * std::f32::consts::PI * normalized_freq).exp();
        let g = g.clamp(0.0, 1.0);

        // Apply filter: y[n] = y[n-1] + g * (x[n] - y[n-1])
        self.filter_state += g * (resonant_output - self.filter_state);

        // Flush denormals to zero
        if self.filter_state.abs() < 1e-15 {
            self.filter_state = 0.0;
        }

        let filtered_output = self.filter_state;

        // Check if hi-hat is still active
        if !self.noise_oscillator.envelope.is_active
            && !self.brightness_oscillator.envelope.is_active
            && !self.amplitude_envelope.is_active
        {
            self.is_active = false;
        }

        filtered_output
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    /// Set volume (smoothed)
    pub fn set_volume(&mut self, volume: f32) {
        self.params.volume.set_target(volume);
    }

    /// Set filter cutoff frequency (smoothed) - lower values tame harshness
    pub fn set_frequency(&mut self, frequency: f32) {
        self.params.frequency.set_target(frequency);
    }

    /// Set decay time (smoothed, takes effect on next trigger)
    pub fn set_decay(&mut self, decay_time: f32) {
        self.params.decay.set_target(decay_time);
    }

    /// Set brightness (smoothed)
    pub fn set_brightness(&mut self, brightness: f32) {
        self.params.brightness.set_target(brightness);
    }

    /// Set resonance (smoothed)
    pub fn set_resonance(&mut self, resonance: f32) {
        self.params.resonance.set_target(resonance);
    }

    /// Set open/closed mode (reconfigures envelopes)
    pub fn set_open(&mut self, is_open: bool) {
        self.is_open = is_open;
        self.configure_oscillators();
    }
}

// Implement the Instrument trait for engine compatibility
impl crate::engine::Instrument for HiHat {
    fn trigger_with_velocity(&mut self, time: f32, _velocity: f32) {
        // Velocity not yet implemented for hihat
        HiHat::trigger(self, time);
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
        vec!["frequency", "brightness", "resonance", "decay", "volume"]
    }

    fn apply_modulation(&mut self, parameter: &str, value: f32) -> Result<(), String> {
        // value is -1.0 to 1.0 (bipolar), set_bipolar maps this to the param range
        match parameter {
            "frequency" => {
                self.params.frequency.set_bipolar(value);
                Ok(())
            }
            "brightness" => {
                self.params.brightness.set_bipolar(value);
                Ok(())
            }
            "resonance" => {
                self.params.resonance.set_bipolar(value);
                Ok(())
            }
            "decay" => {
                self.params.decay.set_bipolar(value);
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
            "frequency" => Some(self.params.frequency.range()),
            "brightness" => Some(self.params.brightness.range()),
            "resonance" => Some(self.params.resonance.range()),
            "decay" => Some(self.params.decay.range()),
            "volume" => Some(self.params.volume.range()),
            _ => None,
        }
    }
}
