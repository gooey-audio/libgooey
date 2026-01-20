use crate::effects::{Effect, SoftSaturation};
use crate::envelope::{ADSRConfig, Envelope};
use crate::filters::ResonantHighpassFilter;
use crate::gen::oscillator::Oscillator;
use crate::gen::waveform::Waveform;
use crate::instruments::fm_snap::FMSnapSynthesizer;
use crate::utils::{SmoothedParam, DEFAULT_SMOOTH_TIME_MS};

/// Effects rack for the kick drum
///
/// Holds all insert effects that can be applied to the kick drum output.
/// Each effect has an enable flag for bypassing. Effects are applied in order.
pub struct KickEffectsRack {
    /// Soft saturation effect for warmth and harmonics
    pub soft_saturation: SoftSaturation,
    /// Whether soft saturation is enabled
    pub soft_saturation_enabled: bool,
}

impl KickEffectsRack {
    /// Create a new effects rack with default settings
    pub fn new() -> Self {
        Self {
            soft_saturation: SoftSaturation::new(1.0), // threshold=1 means bypass
            soft_saturation_enabled: true,             // enabled by default when saturation > 0
        }
    }

    /// Process audio through all enabled effects
    #[inline]
    pub fn process(&self, input: f32, saturation_amount: f32) -> f32 {
        let mut output = input;

        // Apply soft saturation if enabled and amount > 0
        if self.soft_saturation_enabled && saturation_amount > 0.0 {
            // Map saturation amount to threshold: sat=0 -> a=1 (bypass), sat=1 -> a=0 (max)
            self.soft_saturation.set_threshold(1.0 - saturation_amount);
            output = self.soft_saturation.process(output);
        }

        output
    }

    /// Set soft saturation enabled state
    pub fn set_soft_saturation_enabled(&mut self, enabled: bool) {
        self.soft_saturation_enabled = enabled;
    }

    /// Get soft saturation enabled state
    pub fn is_soft_saturation_enabled(&self) -> bool {
        self.soft_saturation_enabled
    }
}

impl Default for KickEffectsRack {
    fn default() -> Self {
        Self::new()
    }
}

/// Static configuration for kick drum presets
/// Used to initialize a KickDrum with specific parameter values
#[derive(Clone, Copy, Debug)]
pub struct KickConfig {
    pub kick_frequency: f32, // Base frequency (30-80Hz typical)
    pub punch_amount: f32,   // Mid-frequency presence (0.0-1.0)
    pub sub_amount: f32,     // Sub-bass presence (0.0-1.0)
    pub click_amount: f32,   // High-frequency click (0.0-1.0)
    pub snap_amount: f32,    // FM snap transient/zap (0.0-1.0)
    pub decay_time: f32,     // Overall decay length in seconds
    pub pitch_envelope: f32, // Frequency sweep amount (0.0-1.0)
    pub volume: f32,         // Overall volume (0.0-1.0)
    pub saturation: f32,     // Soft saturation amount (0.0-1.0)
}

impl KickConfig {
    pub fn new(
        kick_frequency: f32,
        punch_amount: f32,
        sub_amount: f32,
        click_amount: f32,
        snap_amount: f32,
        decay_time: f32,
        pitch_envelope: f32,
        volume: f32,
        saturation: f32,
    ) -> Self {
        Self {
            kick_frequency: kick_frequency.max(30.0).min(80.0), // Typical kick drum frequency range
            punch_amount: punch_amount.clamp(0.0, 1.0),
            sub_amount: sub_amount.clamp(0.0, 1.0),
            click_amount: click_amount.clamp(0.0, 1.0),
            snap_amount: snap_amount.clamp(0.0, 1.0),
            decay_time: decay_time.max(0.01).min(5.0), // Reasonable decay range
            pitch_envelope: pitch_envelope.clamp(0.0, 1.0),
            volume: volume.clamp(0.0, 1.0),
            saturation: saturation.clamp(0.0, 1.0),
        }
    }

    pub fn default() -> Self {
        // snap_amount defaults to 0.3 for subtle attack transient
        // saturation defaults to 0.0 (no saturation)
        Self::new(30.0, 0.80, 0.80, 0.20, 0.3, 0.28, 0.20, 0.80, 0.0)
    }

    pub fn punchy() -> Self {
        // punchy preset gets more snap for aggressive attack
        // light saturation (0.3) for extra punch
        Self::new(60.0, 0.9, 0.6, 0.4, 0.6, 0.6, 0.7, 0.85, 0.3)
    }

    pub fn deep() -> Self {
        // deep preset has less snap for smoother attack
        // subtle saturation (0.1) for warmth
        Self::new(45.0, 0.5, 1.0, 0.2, 0.2, 1.2, 0.5, 0.9, 0.1)
    }

    pub fn tight() -> Self {
        // tight preset has moderate snap
        // moderate saturation (0.4) for more aggressive character
        Self::new(70.0, 0.8, 0.7, 0.5, 0.5, 0.4, 0.8, 0.8, 0.4)
    }
}

/// Smoothed parameters for real-time control of the kick drum
/// These use one-pole smoothing to prevent clicks/pops during parameter changes
pub struct KickParams {
    pub frequency: SmoothedParam,      // Base frequency (30-80 Hz)
    pub punch: SmoothedParam,          // Mid-frequency presence (0-1)
    pub sub: SmoothedParam,            // Sub-bass presence (0-1)
    pub click: SmoothedParam,          // High-frequency click (0-1)
    pub snap: SmoothedParam,           // FM snap transient/zap (0-1)
    pub decay: SmoothedParam,          // Decay time in seconds (0.01-5.0)
    pub pitch_envelope: SmoothedParam, // Pitch envelope amount (0-1)
    pub volume: SmoothedParam,         // Overall volume (0-1)
    pub saturation: SmoothedParam,     // Soft saturation amount (0-1)
}

impl KickParams {
    /// Create new smoothed parameters from a config
    pub fn from_config(config: &KickConfig, sample_rate: f32) -> Self {
        Self {
            frequency: SmoothedParam::new(
                config.kick_frequency,
                30.0,
                80.0,
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
            snap: SmoothedParam::new(
                config.snap_amount,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            decay: SmoothedParam::new(
                config.decay_time,
                0.01,
                5.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            pitch_envelope: SmoothedParam::new(
                config.pitch_envelope,
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
            saturation: SmoothedParam::new(
                config.saturation,
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
        self.snap.tick();
        self.decay.tick();
        self.pitch_envelope.tick();
        self.volume.tick();
        self.saturation.tick();

        // Return true if any smoother is still active
        !self.is_settled()
    }

    /// Check if all parameters have settled
    pub fn is_settled(&self) -> bool {
        self.frequency.is_settled()
            && self.punch.is_settled()
            && self.sub.is_settled()
            && self.click.is_settled()
            && self.snap.is_settled()
            && self.decay.is_settled()
            && self.pitch_envelope.is_settled()
            && self.volume.is_settled()
            && self.saturation.is_settled()
    }

    /// Get a snapshot of current values as a KickConfig (for reading back)
    pub fn to_config(&self) -> KickConfig {
        KickConfig {
            kick_frequency: self.frequency.get(),
            punch_amount: self.punch.get(),
            sub_amount: self.sub.get(),
            click_amount: self.click.get(),
            snap_amount: self.snap.get(),
            decay_time: self.decay.get(),
            pitch_envelope: self.pitch_envelope.get(),
            volume: self.volume.get(),
            saturation: self.saturation.get(),
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

    // Effects rack for insert effects
    pub effects: KickEffectsRack,

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
        // Initialize effects rack with saturation threshold derived from config
        // saturation=0 -> threshold=1 (bypass), saturation=1 -> threshold=0 (max sat)
        let effects = KickEffectsRack::new();
        effects.soft_saturation.set_threshold(1.0 - config.saturation);
        let mut kick = Self {
            sample_rate,
            params,
            sub_oscillator: Oscillator::new(sample_rate, config.kick_frequency),
            punch_oscillator: Oscillator::new(sample_rate, config.kick_frequency * 2.5),
            click_oscillator: Oscillator::new(sample_rate, config.kick_frequency * 40.0),
            pitch_envelope: Envelope::new(),
            pitch_start_multiplier: 1.0 + config.pitch_envelope * 2.0, // Start 1-3x higher
            click_filter: ResonantHighpassFilter::new(sample_rate, 8000.0, 4.0),
            fm_snap: FMSnapSynthesizer::new(sample_rate),
            effects,
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
        let decay = self.params.decay.get();

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
    }

    /// Apply current smoothed parameters to oscillators (called per-sample)
    #[inline]
    fn apply_params(&mut self) {
        let punch = self.params.punch.get();
        let sub = self.params.sub.get();
        let click = self.params.click.get();
        let pitch_envelope = self.params.pitch_envelope.get();
        let volume = self.params.volume.get();

        // Update pitch start multiplier
        self.pitch_start_multiplier = 1.0 + pitch_envelope * 2.0;

        // Light velocity scaling for click: range [0.6, 1.0]
        // Higher velocity = more click, lower velocity = less click
        let click_vel_scale = 0.6 + 0.4 * self.current_velocity;

        // Update oscillator volumes (these can change smoothly)
        self.sub_oscillator.set_volume(sub * volume);
        self.punch_oscillator.set_volume(punch * volume * 0.7);
        // Click reduced from 0.3 to 0.15, with velocity scaling
        self.click_oscillator
            .set_volume(click * volume * 0.15 * click_vel_scale);
    }

    pub fn set_config(&mut self, config: KickConfig) {
        // Set all parameter targets (will smooth to new values)
        self.params.frequency.set_target(config.kick_frequency);
        self.params.punch.set_target(config.punch_amount);
        self.params.sub.set_target(config.sub_amount);
        self.params.click.set_target(config.click_amount);
        self.params.snap.set_target(config.snap_amount);
        self.params.decay.set_target(config.decay_time);
        self.params.pitch_envelope.set_target(config.pitch_envelope);
        self.params.volume.set_target(config.volume);
        self.params.saturation.set_target(config.saturation);

        // Reconfigure envelopes for new decay time
        self.configure_oscillators();
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
        let pitch_decay_scale = 1.0 - (self.velocity_to_pitch * vel_squared);

        // Get base parameters
        let base_decay = self.params.decay.get() * decay_scale;
        // Pitch decay must be at most 60% of amplitude decay to ensure pitch settles
        // before sound ends (fixes "phantom pitch" at end of decay)
        let pitch_decay = (self.params.decay.get() * pitch_decay_scale).min(base_decay * 0.6);
        let base_freq = self.params.frequency.get();

        // Configure pitch envelope with velocity-scaled decay
        // High velocity = short pitch decay (sharp, punchy attack)
        // Low velocity = long pitch decay (smooth, subtle pitch sweep)
        // Pitch envelope always completes before amplitude to prevent pitch artifacts
        self.pitch_envelope
            .set_config(ADSRConfig::new(0.001, pitch_decay, 0.0, pitch_decay * 0.1));

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

        // Add FM snap for beater sound (controlled by snap_amount parameter)
        let fm_snap_output = self.fm_snap.tick(current_time);
        let snap = self.params.snap.get();

        let total_output = sub_output
            + punch_output
            + filtered_click_output
            + (fm_snap_output * snap * self.params.volume.get());

        // Apply effects rack (before velocity scaling)
        let saturation_amount = self.params.saturation.get();
        let saturated_output = self.effects.process(total_output, saturation_amount);

        // Apply velocity amplitude scaling (sqrt for perceptually linear loudness)
        let velocity_amplitude = self.current_velocity.sqrt();
        let final_output = saturated_output * velocity_amplitude;

        // Check if kick is still active
        if !self.sub_oscillator.envelope.is_active
            && !self.punch_oscillator.envelope.is_active
            && !self.click_oscillator.envelope.is_active
            && !self.fm_snap.is_active()
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

    /// Set snap amount (smoothed) - controls FM snap transient/zap
    pub fn set_snap(&mut self, snap_amount: f32) {
        self.params.snap.set_target(snap_amount);
    }

    /// Set pitch drop amount (smoothed)
    pub fn set_pitch_envelope(&mut self, pitch_envelope: f32) {
        self.params.pitch_envelope.set_target(pitch_envelope);
    }

    /// Set saturation amount (smoothed)
    /// 0.0 = no saturation (bypass), 1.0 = maximum saturation
    pub fn set_saturation(&mut self, saturation: f32) {
        self.params.saturation.set_target(saturation);
    }

    /// Set soft saturation effect enabled state
    pub fn set_saturation_enabled(&mut self, enabled: bool) {
        self.effects.set_soft_saturation_enabled(enabled);
    }

    /// Get soft saturation effect enabled state
    pub fn is_saturation_enabled(&self) -> bool {
        self.effects.is_soft_saturation_enabled()
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
impl crate::engine::Modulatable for KickDrum {
    fn modulatable_parameters(&self) -> Vec<&'static str> {
        vec![
            "frequency",
            "punch",
            "sub",
            "click",
            "snap",
            "decay",
            "pitch_envelope",
            "volume",
            "saturation",
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
            "snap" => {
                self.params.snap.set_bipolar(value);
                Ok(())
            }
            "decay" => {
                self.params.decay.set_bipolar(value);
                Ok(())
            }
            "pitch_envelope" => {
                self.params.pitch_envelope.set_bipolar(value);
                Ok(())
            }
            "volume" => {
                self.params.volume.set_bipolar(value);
                Ok(())
            }
            "saturation" => {
                self.params.saturation.set_bipolar(value);
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
            "snap" => Some(self.params.snap.range()),
            "decay" => Some(self.params.decay.range()),
            "pitch_envelope" => Some(self.params.pitch_envelope.range()),
            "volume" => Some(self.params.volume.range()),
            "saturation" => Some(self.params.saturation.range()),
            _ => None,
        }
    }
}
