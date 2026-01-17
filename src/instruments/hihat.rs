use crate::envelope::{ADSRConfig, Envelope};
use crate::gen::oscillator::Oscillator;
use crate::gen::waveform::Waveform;
use crate::utils::{SmoothedParam, DEFAULT_SMOOTH_TIME_MS};

#[derive(Clone, Copy, Debug)]
pub struct HiHatConfig {
    pub base_frequency: f32, // Base frequency for filtering (4000-16000Hz)
    pub filter: f32,         // Combined brightness + resonance control (0.0-1.0)
    pub decay_time: f32,     // Decay length in seconds (0.005-0.4)
    pub volume: f32,         // Overall volume (0.0-1.0)
    pub is_open: bool,       // true for open, false for closed
}

impl HiHatConfig {
    pub fn new(base_frequency: f32, filter: f32, decay_time: f32, volume: f32, is_open: bool) -> Self {
        Self {
            base_frequency: base_frequency.clamp(4000.0, 16000.0),
            filter: filter.clamp(0.0, 1.0),
            decay_time: decay_time.clamp(0.005, 0.4),
            volume: volume.clamp(0.0, 1.0),
            is_open,
        }
    }

    pub fn closed_default() -> Self {
        Self::new(8000.0, 0.6, 0.08, 0.8, false)
    }

    pub fn open_default() -> Self {
        Self::new(10000.0, 0.6, 0.4, 0.7, true)
    }

    pub fn closed_tight() -> Self {
        Self::new(6000.0, 0.55, 0.015, 0.9, false)
    }

    pub fn open_bright() -> Self {
        Self::new(14000.0, 0.7, 0.4, 0.8, true)
    }

    pub fn closed_dark() -> Self {
        Self::new(4000.0, 0.4, 0.1, 0.7, false)
    }

    pub fn open_long() -> Self {
        Self::new(8000.0, 0.45, 0.4, 0.6, true)
    }
}

/// Smoothed parameters for real-time control of the hi-hat
/// These use one-pole smoothing to prevent clicks/pops during parameter changes
pub struct HiHatParams {
    pub filter: SmoothedParam,    // Combined brightness + resonance (0-1)
    pub frequency: SmoothedParam, // Output filter cutoff (4000-16000 Hz)
    pub decay: SmoothedParam,     // Decay time in seconds (0.005-0.4)
    pub volume: SmoothedParam,    // Overall volume (0-1)
}

impl HiHatParams {
    /// Create new smoothed parameters from a config
    pub fn from_config(config: &HiHatConfig, sample_rate: f32) -> Self {
        Self {
            filter: SmoothedParam::new(
                config.filter,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            frequency: SmoothedParam::new(
                config.base_frequency,
                4000.0,
                16000.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            decay: SmoothedParam::new(
                config.decay_time,
                0.005,
                0.4,
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
        self.filter.tick();
        self.frequency.tick();
        self.decay.tick();
        self.volume.tick();

        // Return true if any smoother is still active
        !self.is_settled()
    }

    /// Check if all parameters have settled
    pub fn is_settled(&self) -> bool {
        self.filter.is_settled()
            && self.frequency.is_settled()
            && self.decay.is_settled()
            && self.volume.is_settled()
    }

    /// Get a snapshot of current values as a HiHatConfig (for reading back)
    pub fn to_config(&self, is_open: bool) -> HiHatConfig {
        HiHatConfig {
            base_frequency: self.frequency.get(),
            filter: self.filter.get(),
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

    // Filter envelope for subtle brightness decay (like pitch envelope but for filter cutoff)
    pub filter_envelope: Envelope,
    /// How much the filter sweeps down from initial bright state (subtle, fixed amount)
    filter_envelope_amount: f32,

    // Output lowpass filter state (one-pole, tames harshness)
    filter_state: f32,

    pub is_active: bool,

    // Velocity-responsive state
    /// Current trigger velocity (0.0-1.0), set on trigger
    current_velocity: f32,

    /// How much velocity affects decay time (0.0-1.0)
    /// Higher values = more velocity sensitivity (shorter decay at high velocity)
    velocity_to_decay: f32,

    /// How much velocity affects frequency/pitch (0.0-1.0)
    /// Higher values = more frequency boost at high velocity
    velocity_to_pitch: f32,

    /// Current velocity-based frequency boost multiplier (decays with filter envelope)
    velocity_freq_boost: f32,
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
            filter_envelope: Envelope::new(),
            filter_envelope_amount: 0.15, // Subtle 15% filter sweep (much lighter than kick's pitch)
            filter_state: 0.0,
            is_active: false,
            // Velocity sensitivity
            current_velocity: 1.0,
            velocity_to_decay: 0.4,  // Decay shortened by up to 40% at high velocity
            velocity_to_pitch: 0.3,  // Frequency boosted by up to 30% at high velocity
            velocity_freq_boost: 1.0, // No boost initially
        };

        hihat.configure_oscillators();
        hihat
    }

    /// Get current config snapshot (reads current smoothed values)
    pub fn config(&self) -> HiHatConfig {
        self.params.to_config(self.is_open)
    }

    /// Configure oscillators from current smoothed parameter values
    /// Called at initialization; trigger() handles per-hit envelope config
    /// Note: Volume is applied only at final output stage, not here
    fn configure_oscillators(&mut self) {
        let filter = self.params.filter.get();
        let decay = self.params.decay.get();

        // Fixed fast attack for hi-hats (always percussive)
        const ATTACK: f32 = 0.001;

        // Main noise oscillator (frequency doesn't affect noise, just a placeholder)
        self.noise_oscillator.waveform = Waveform::Noise;
        self.noise_oscillator.set_volume(1.0); // Full level; volume applied at output

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
        // Relative level based on filter param; volume applied at output
        self.brightness_oscillator.set_volume(filter * 0.8);

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
    /// Note: Volume is applied only at final output stage, not here
    #[inline]
    fn apply_params(&mut self) {
        let filter = self.params.filter.get();

        // Update brightness oscillator level (can change smoothly)
        // Note: frequency parameter controls the output lowpass filter cutoff
        // Note: volume is applied at final output, not here
        self.brightness_oscillator.set_volume(filter * 0.5);
    }

    pub fn set_config(&mut self, config: HiHatConfig) {
        // Set all parameter targets (will smooth to new values)
        self.params.filter.set_target(config.filter);
        self.params.frequency.set_target(config.base_frequency);
        self.params.decay.set_target(config.decay_time);
        self.params.volume.set_target(config.volume);
        self.is_open = config.is_open;

        // Reconfigure envelopes for new decay time
        self.configure_oscillators();
    }

    /// Trigger with default velocity (1.0)
    pub fn trigger(&mut self, time: f32) {
        self.trigger_with_velocity(time, 1.0);
    }

    /// Trigger with velocity sensitivity
    /// - High velocity: higher pitch, shorter decay, louder
    /// - Low velocity: lower pitch, longer decay, quieter
    pub fn trigger_with_velocity(&mut self, time: f32, velocity: f32) {
        self.is_active = true;
        self.current_velocity = velocity.clamp(0.0, 1.0);

        // Quadratic curve for natural acoustic-like response
        let vel = self.current_velocity;
        let vel_squared = vel * vel;

        // Read base parameters
        let base_decay = self.params.decay.get();
        let filter = self.params.filter.get();
        const ATTACK: f32 = 0.001;

        // High velocity = shorter decay (tighter, snappier sound)
        let decay_scale = 1.0 - (self.velocity_to_decay * vel_squared);
        let scaled_decay = base_decay * decay_scale;

        // High velocity = higher frequency (brighter, more cutting sound)
        // Store the boost factor - it will be applied transiently in tick() via filter envelope
        self.velocity_freq_boost = 1.0 + (self.velocity_to_pitch * vel_squared);

        // Configure and trigger noise oscillator envelope based on open/closed
        if self.is_open {
            self.noise_oscillator.set_adsr(ADSRConfig::new(
                ATTACK,
                scaled_decay * 0.2,
                0.4,
                scaled_decay * 0.8,
            ));
        } else {
            self.noise_oscillator.set_adsr(ADSRConfig::new(
                ATTACK,
                scaled_decay,
                0.0,
                scaled_decay * 0.1,
            ));
        }
        self.noise_oscillator.set_volume(1.0); // Full level; volume applied at output
        self.noise_oscillator.trigger(time);

        // Configure and trigger brightness oscillator (shorter envelope for transient sizzle)
        self.brightness_oscillator.set_adsr(ADSRConfig::new(
            ATTACK,
            scaled_decay * 0.2,
            0.0,
            scaled_decay * 0.05,
        ));
        self.brightness_oscillator.set_volume(filter * 0.8); // Relative level; volume applied at output
        self.brightness_oscillator.trigger(time);

        // Configure and trigger amplitude envelope
        if self.is_open {
            self.amplitude_envelope.set_config(ADSRConfig::new(
                ATTACK,
                scaled_decay * 0.3,
                0.3,
                scaled_decay * 0.7,
            ));
        } else {
            self.amplitude_envelope.set_config(ADSRConfig::new(
                ATTACK,
                scaled_decay,
                0.0,
                scaled_decay * 0.05,
            ));
        }
        self.amplitude_envelope.trigger(time);

        // Configure and trigger filter envelope (subtle brightness decay)
        // Filter envelope is faster than amplitude - decays to base before sound ends
        // This creates the characteristic "tsss" where brightness fades first
        let filter_decay = scaled_decay * 0.5; // 50% of amplitude decay
        self.filter_envelope.set_config(ADSRConfig::new(
            ATTACK,       // Instant attack (start bright)
            filter_decay, // Quick decay to base frequency
            0.0,          // No sustain - settle to base cutoff
            filter_decay * 0.1,
        ));
        self.filter_envelope.trigger(time);
    }

    pub fn release(&mut self, time: f32) {
        if self.is_active {
            self.noise_oscillator.release(time);
            self.brightness_oscillator.release(time);
            self.amplitude_envelope.release(time);
            self.filter_envelope.release(time);
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
        // Filter parameter controls both resonance and brightness
        let filter = self.params.filter.get();
        let resonance_factor = 1.0 + filter * 0.8;
        let resonant_output = final_output * resonance_factor;

        // Apply lowpass filter to tame harshness (one-pole filter)
        // Base cutoff from frequency parameter, boosted by filter
        let base_cutoff = self.params.frequency.get();

        // Apply filter envelope for subtle brightness decay
        // Envelope starts at 1.0 (bright) and decays to 0.0 (base cutoff)
        let filter_env = self.filter_envelope.get_amplitude(current_time);

        // Velocity frequency boost decays with filter envelope (transient, not permanent)
        let velocity_cutoff_boost = (self.velocity_freq_boost - 1.0) * filter_env * base_cutoff;

        // Filter envelope adds extra brightness at attack, then decays
        let envelope_boost = filter_env * self.filter_envelope_amount * base_cutoff;

        // Filter param adds up to 6kHz, plus transient velocity and envelope boosts
        let cutoff = (base_cutoff + filter * 6000.0 + envelope_boost + velocity_cutoff_boost)
            .min(self.sample_rate * 0.45);
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

        // Apply final volume control for direct, audible effect
        let volume = self.params.volume.get();

        // Apply velocity amplitude scaling (sqrt for perceptually linear loudness)
        let velocity_amplitude = self.current_velocity.sqrt();
        let final_output = self.filter_state * volume * velocity_amplitude;

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

    /// Set filter (combined brightness + resonance, smoothed)
    pub fn set_filter(&mut self, filter: f32) {
        self.params.filter.set_target(filter);
    }

    /// Set open/closed mode (reconfigures envelopes)
    pub fn set_open(&mut self, is_open: bool) {
        self.is_open = is_open;
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
        vec!["filter", "frequency", "decay", "volume"]
    }

    fn apply_modulation(&mut self, parameter: &str, value: f32) -> Result<(), String> {
        // value is -1.0 to 1.0 (bipolar), set_bipolar maps this to the param range
        match parameter {
            "filter" => {
                self.params.filter.set_bipolar(value);
                Ok(())
            }
            "frequency" => {
                self.params.frequency.set_bipolar(value);
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
            "filter" => Some(self.params.filter.range()),
            "frequency" => Some(self.params.frequency.range()),
            "decay" => Some(self.params.decay.range()),
            "volume" => Some(self.params.volume.range()),
            _ => None,
        }
    }
}
