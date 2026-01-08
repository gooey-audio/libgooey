use crate::envelope::{ADSRConfig, Envelope};
use crate::filters::ResonantHighpassFilter;
use crate::gen::oscillator::Oscillator;
use crate::gen::waveform::Waveform;
use crate::instruments::fm_snap::FMSnapSynthesizer;
use crate::utils::{SmoothedParam, DEFAULT_SMOOTH_TIME_MS};

/// Static configuration for kick drum presets
/// Used to initialize a KickDrum with specific parameter values
#[derive(Clone, Copy, Debug)]
pub struct KickConfig {
    pub kick_frequency: f32, // Base frequency (40-80Hz typical)
    pub punch_amount: f32,   // Mid-frequency presence (0.0-1.0)
    pub sub_amount: f32,     // Sub-bass presence (0.0-1.0)
    pub click_amount: f32,   // High-frequency click (0.0-1.0)
    pub decay_time: f32,     // Overall decay length in seconds
    pub pitch_drop: f32,     // Frequency sweep amount (0.0-1.0)
    pub volume: f32,         // Overall volume (0.0-1.0)
}

impl KickConfig {
    pub fn new(
        kick_frequency: f32,
        punch_amount: f32,
        sub_amount: f32,
        click_amount: f32,
        decay_time: f32,
        pitch_drop: f32,
        volume: f32,
    ) -> Self {
        Self {
            kick_frequency: kick_frequency.max(20.0).min(200.0), // Reasonable kick range
            punch_amount: punch_amount.clamp(0.0, 1.0),
            sub_amount: sub_amount.clamp(0.0, 1.0),
            click_amount: click_amount.clamp(0.0, 1.0),
            decay_time: decay_time.max(0.01).min(5.0), // Reasonable decay range
            pitch_drop: pitch_drop.clamp(0.0, 1.0),
            volume: volume.clamp(0.0, 1.0),
        }
    }

    pub fn default() -> Self {
        Self::new(30.0, 0.80, 0.80, 0.20, 0.28, 0.20, 0.80)
    }

    pub fn punchy() -> Self {
        Self::new(60.0, 0.9, 0.6, 0.4, 0.6, 0.7, 0.85)
    }

    pub fn deep() -> Self {
        Self::new(45.0, 0.5, 1.0, 0.2, 1.2, 0.5, 0.9)
    }

    pub fn tight() -> Self {
        Self::new(70.0, 0.8, 0.7, 0.5, 0.4, 0.8, 0.8)
    }
}

/// Smoothed parameters for real-time control of the kick drum
/// These use one-pole smoothing to prevent clicks/pops during parameter changes
pub struct KickParams {
    pub frequency: SmoothedParam,   // Base frequency (20-200 Hz)
    pub punch: SmoothedParam,       // Mid-frequency presence (0-1)
    pub sub: SmoothedParam,         // Sub-bass presence (0-1)
    pub click: SmoothedParam,       // High-frequency click (0-1)
    pub decay: SmoothedParam,       // Decay time in seconds (0.01-5.0)
    pub pitch_drop: SmoothedParam,  // Pitch envelope amount (0-1)
    pub volume: SmoothedParam,      // Overall volume (0-1)
}

impl KickParams {
    /// Create new smoothed parameters from a config
    pub fn from_config(config: &KickConfig, sample_rate: f32) -> Self {
        Self {
            frequency: SmoothedParam::new(config.kick_frequency, 20.0, 200.0, sample_rate, DEFAULT_SMOOTH_TIME_MS),
            punch: SmoothedParam::new(config.punch_amount, 0.0, 1.0, sample_rate, DEFAULT_SMOOTH_TIME_MS),
            sub: SmoothedParam::new(config.sub_amount, 0.0, 1.0, sample_rate, DEFAULT_SMOOTH_TIME_MS),
            click: SmoothedParam::new(config.click_amount, 0.0, 1.0, sample_rate, DEFAULT_SMOOTH_TIME_MS),
            decay: SmoothedParam::new(config.decay_time, 0.01, 5.0, sample_rate, DEFAULT_SMOOTH_TIME_MS),
            pitch_drop: SmoothedParam::new(config.pitch_drop, 0.0, 1.0, sample_rate, DEFAULT_SMOOTH_TIME_MS),
            volume: SmoothedParam::new(config.volume, 0.0, 1.0, sample_rate, DEFAULT_SMOOTH_TIME_MS),
        }
    }

    /// Tick all smoothers and return whether any are still smoothing
    #[inline]
    pub fn tick(&mut self) -> bool {
        self.frequency.tick();
        self.punch.tick();
        self.sub.tick();
        self.click.tick();
        self.decay.tick();
        self.pitch_drop.tick();
        self.volume.tick();
        
        // Return true if any smoother is still active
        !self.is_settled()
    }

    /// Check if all parameters have settled
    pub fn is_settled(&self) -> bool {
        self.frequency.is_settled() && self.punch.is_settled() && 
        self.sub.is_settled() && self.click.is_settled() &&
        self.decay.is_settled() && self.pitch_drop.is_settled() &&
        self.volume.is_settled()
    }

    /// Get a snapshot of current values as a KickConfig (for reading back)
    pub fn to_config(&self) -> KickConfig {
        KickConfig {
            kick_frequency: self.frequency.get(),
            punch_amount: self.punch.get(),
            sub_amount: self.sub.get(),
            click_amount: self.click.get(),
            decay_time: self.decay.get(),
            pitch_drop: self.pitch_drop.get(),
            volume: self.volume.get(),
        }
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
    pitch_start_multiplier: f32,

    // High-pass filter for click oscillator
    pub click_filter: ResonantHighpassFilter,

    // FM snap synthesizer for beater sound
    pub fm_snap: FMSnapSynthesizer,

    pub is_active: bool,
}

impl KickDrum {
    pub fn new(sample_rate: f32) -> Self {
        let config = KickConfig::default();
        Self::with_config(sample_rate, config)
    }

    pub fn with_config(sample_rate: f32, config: KickConfig) -> Self {
        let params = KickParams::from_config(&config, sample_rate);
        let mut kick = Self {
            sample_rate,
            params,
            sub_oscillator: Oscillator::new(sample_rate, config.kick_frequency),
            punch_oscillator: Oscillator::new(sample_rate, config.kick_frequency * 2.5),
            click_oscillator: Oscillator::new(sample_rate, config.kick_frequency * 40.0),
            pitch_envelope: Envelope::new(),
            pitch_start_multiplier: 1.0 + config.pitch_drop * 2.0, // Start 1-3x higher
            click_filter: ResonantHighpassFilter::new(sample_rate, 8000.0, 4.0),
            fm_snap: FMSnapSynthesizer::new(sample_rate),
            is_active: false,
        };

        kick.configure_oscillators();
        kick
    }

    /// Configure oscillators from current smoothed parameter values
    /// Called once at initialization and when decay changes significantly
    fn configure_oscillators(&mut self) {
        let decay = self.params.decay.get();

        // Sub oscillator: Deep sine wave
        self.sub_oscillator.waveform = Waveform::Sine;
        self.sub_oscillator.set_adsr(ADSRConfig::new(
            0.001,            // Very fast attack
            decay,            // Synchronized decay time
            0.0,              // No sustain
            decay * 0.2,      // Synchronized release
        ));

        // Punch oscillator: Triangle for mid-range impact
        self.punch_oscillator.waveform = Waveform::Triangle;
        self.punch_oscillator.set_adsr(ADSRConfig::new(
            0.001,            // Very fast attack
            decay,            // Synchronized decay time
            0.0,              // No sustain
            decay * 0.2,      // Synchronized release
        ));

        // Click oscillator: High-frequency filtered noise transient
        self.click_oscillator.waveform = Waveform::Noise;
        self.click_oscillator.set_adsr(ADSRConfig::new(
            0.001,             // Very fast attack
            decay * 0.2,       // Much shorter decay for click
            0.0,               // No sustain
            decay * 0.02,      // Extremely short release
        ));

        // Pitch envelope: Fast attack, synchronized decay for frequency sweeping
        self.pitch_envelope.set_config(ADSRConfig::new(
            0.001,            // Instant attack
            decay,            // Synchronized decay time
            0.0,              // Drop to base frequency
            decay * 0.2,      // Synchronized release
        ));
    }

    /// Apply current smoothed parameters to oscillators (called per-sample)
    #[inline]
    fn apply_params(&mut self) {
        let punch = self.params.punch.get();
        let sub = self.params.sub.get();
        let click = self.params.click.get();
        let pitch_drop = self.params.pitch_drop.get();
        let volume = self.params.volume.get();

        // Update pitch start multiplier
        self.pitch_start_multiplier = 1.0 + pitch_drop * 2.0;

        // Update oscillator volumes (these can change smoothly)
        self.sub_oscillator.set_volume(sub * volume);
        self.punch_oscillator.set_volume(punch * volume * 0.7);
        self.click_oscillator.set_volume(click * volume * 0.3);
    }

    pub fn set_config(&mut self, config: KickConfig) {
        // Set all parameter targets (will smooth to new values)
        self.params.frequency.set_target(config.kick_frequency);
        self.params.punch.set_target(config.punch_amount);
        self.params.sub.set_target(config.sub_amount);
        self.params.click.set_target(config.click_amount);
        self.params.decay.set_target(config.decay_time);
        self.params.pitch_drop.set_target(config.pitch_drop);
        self.params.volume.set_target(config.volume);
        
        // Reconfigure envelopes for new decay time
        self.configure_oscillators();
    }
    
    /// Get current config snapshot (reads current smoothed values)
    pub fn config(&self) -> KickConfig {
        self.params.to_config()
    }

    pub fn trigger(&mut self, time: f32) {
        self.is_active = true;

        // Reconfigure envelopes with current decay value before triggering
        self.configure_oscillators();

        // Trigger all oscillators
        self.sub_oscillator.trigger(time);
        self.punch_oscillator.trigger(time);
        self.click_oscillator.trigger(time);

        // Trigger pitch envelope
        self.pitch_envelope.trigger(time);

        // Trigger FM snap for beater sound
        self.fm_snap.trigger(time);

        // Reset filter state for clean click transients
        self.click_filter.reset();
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

        // Apply smoothed parameters to oscillators
        self.apply_params();
        
        let base_frequency = self.params.frequency.get();

        // Calculate pitch modulation
        let pitch_envelope_value = self.pitch_envelope.get_amplitude(current_time);
        let frequency_multiplier = 1.0 + (self.pitch_start_multiplier - 1.0) * pitch_envelope_value;

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

        // Add FM snap for beater sound
        let fm_snap_output = self.fm_snap.tick(current_time);

        let total_output = sub_output
            + punch_output
            + filtered_click_output
            + (fm_snap_output * self.params.volume.get());

        // Check if kick is still active
        if !self.sub_oscillator.envelope.is_active
            && !self.punch_oscillator.envelope.is_active
            && !self.click_oscillator.envelope.is_active
            && !self.fm_snap.is_active()
        {
            self.is_active = false;
        }

        total_output
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    /// Set volume (smoothed)
    pub fn set_volume(&mut self, volume: f32) {
        self.params.volume.set_target(volume);
    }

    /// Set base frequency (smoothed)
    pub fn set_frequency(&mut self, frequency: f32) {
        self.params.frequency.set_target(frequency);
    }

    /// Set decay time (smoothed, takes effect on next trigger)
    pub fn set_decay(&mut self, decay_time: f32) {
        self.params.decay.set_target(decay_time);
    }

    /// Set punch amount (smoothed)
    pub fn set_punch(&mut self, punch_amount: f32) {
        self.params.punch.set_target(punch_amount);
    }

    /// Set sub amount (smoothed)
    pub fn set_sub(&mut self, sub_amount: f32) {
        self.params.sub.set_target(sub_amount);
    }

    /// Set click amount (smoothed)
    pub fn set_click(&mut self, click_amount: f32) {
        self.params.click.set_target(click_amount);
    }

    /// Set pitch drop amount (smoothed)
    pub fn set_pitch_drop(&mut self, pitch_drop: f32) {
        self.params.pitch_drop.set_target(pitch_drop);
    }
}

impl crate::engine::Instrument for KickDrum {
    fn trigger(&mut self, time: f32) {
        self.trigger(time);
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
impl crate::engine::Modulatable for KickDrum {
    fn modulatable_parameters(&self) -> Vec<&'static str> {
        vec!["frequency", "punch", "sub", "click", "decay", "pitch_drop", "volume"]
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
            _ => Err(format!("Unknown parameter: {}", parameter)),
        }
    }

    fn parameter_range(&self, parameter: &str) -> Option<(f32, f32)> {
        match parameter {
            "frequency" => Some(self.params.frequency.range()),
            "punch" => Some(self.params.punch.range()),
            "sub" => Some(self.params.sub.range()),
            "click" => Some(self.params.click.range()),
            "decay" => Some(self.params.decay.range()),
            "pitch_drop" => Some(self.params.pitch_drop.range()),
            "volume" => Some(self.params.volume.range()),
            _ => None,
        }
    }
}
