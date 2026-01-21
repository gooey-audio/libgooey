use crate::envelope::{ADSRConfig, Envelope};
use crate::filters::StateVariableFilter;
use crate::gen::oscillator::Oscillator;
use crate::gen::waveform::Waveform;
use crate::instruments::fm_snap::PhaseModulator;
use crate::utils::{SmoothedParam, DEFAULT_SMOOTH_TIME_MS};

#[derive(Clone, Copy, Debug)]
pub struct SnareConfig {
    pub snare_frequency: f32, // Base frequency (150-400Hz typical)
    pub tonal_amount: f32,    // Tonal component presence (0.0-1.0)
    pub noise_amount: f32,    // Noise component presence (0.0-1.0)
    pub crack_amount: f32,    // High-frequency crack (0.0-1.0)
    pub decay_time: f32,      // Overall decay length in seconds
    pub pitch_drop: f32,      // Frequency sweep amount (0.0-1.0)
    pub volume: f32,          // Overall volume (0.0-1.0)

    // DS-style parameters
    pub tonal_decay: f32,       // Separate tonal decay (0.004-0.7s)
    pub noise_decay: f32,       // Noise envelope decay (0.004-3.5s)
    pub noise_tail_decay: f32,  // Noise tail decay (0.004-3.5s)
    pub noise_color: f32,       // Noise frequency/color (400-1500 Hz)
    pub filter_cutoff: f32,     // SVF filter cutoff (100-20000 Hz)
    pub filter_resonance: f32,  // SVF filter resonance (0.5-10.0)
    pub filter_type: u8,        // 0=LP, 1=BP, 2=HP, 3=notch
    pub xfade: f32,             // Tonal/noise crossfade (0.0-1.0)
    pub click_enabled: bool,    // Enable click transient
    pub phase_mod_enabled: bool, // DS-style phase modulation
    pub phase_mod_amount: f32,  // Phase mod depth (0.0-1.0)
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
            snare_frequency: snare_frequency.max(100.0).min(600.0),
            tonal_amount: tonal_amount.clamp(0.0, 1.0),
            noise_amount: noise_amount.clamp(0.0, 1.0),
            crack_amount: crack_amount.clamp(0.0, 1.0),
            decay_time: decay_time.max(0.01).min(2.0),
            pitch_drop: pitch_drop.clamp(0.0, 1.0),
            volume: volume.clamp(0.0, 1.0),
            // DS parameters with defaults (disabled for backward compatibility)
            tonal_decay: decay_time * 0.8,
            noise_decay: decay_time * 0.6,
            noise_tail_decay: decay_time * 1.5,
            noise_color: 1000.0,
            filter_cutoff: 5000.0,
            filter_resonance: 1.0,
            filter_type: 1, // Bandpass default
            xfade: 0.5,
            click_enabled: false,
            phase_mod_enabled: false,
            phase_mod_amount: 0.0,
        }
    }

    /// Create a SnareConfig with all parameters including DS Snare features
    #[allow(clippy::too_many_arguments)]
    pub fn new_full(
        snare_frequency: f32,
        tonal_amount: f32,
        noise_amount: f32,
        crack_amount: f32,
        decay_time: f32,
        pitch_drop: f32,
        volume: f32,
        tonal_decay: f32,
        noise_decay: f32,
        noise_tail_decay: f32,
        noise_color: f32,
        filter_cutoff: f32,
        filter_resonance: f32,
        filter_type: u8,
        xfade: f32,
        click_enabled: bool,
        phase_mod_enabled: bool,
        phase_mod_amount: f32,
    ) -> Self {
        Self {
            snare_frequency: snare_frequency.max(80.0).min(600.0),
            tonal_amount: tonal_amount.clamp(0.0, 1.0),
            noise_amount: noise_amount.clamp(0.0, 1.0),
            crack_amount: crack_amount.clamp(0.0, 1.0),
            decay_time: decay_time.max(0.01).min(3.5),
            pitch_drop: pitch_drop.clamp(0.0, 1.0),
            volume: volume.clamp(0.0, 1.0),
            tonal_decay: tonal_decay.clamp(0.004, 0.7),
            noise_decay: noise_decay.clamp(0.004, 3.5),
            noise_tail_decay: noise_tail_decay.clamp(0.004, 3.5),
            noise_color: noise_color.clamp(400.0, 1500.0),
            filter_cutoff: filter_cutoff.clamp(100.0, 20000.0),
            filter_resonance: filter_resonance.clamp(0.5, 10.0),
            filter_type: filter_type.min(3),
            xfade: xfade.clamp(0.0, 1.0),
            click_enabled,
            phase_mod_enabled,
            phase_mod_amount: phase_mod_amount.clamp(0.0, 1.0),
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

    /// DS Snare preset - Ableton Drum Synth style
    /// Features: phase modulation transient, SVF-filtered noise, tonal/noise crossfade
    pub fn ds_snare() -> Self {
        Self::new_full(
            200.0, // Snare frequency (higher than kick)
            0.3,   // Reduced tonal (DS style)
            0.8,   // Strong noise
            0.0,   // Crack disabled (using phase mod)
            0.15,  // Overall decay
            0.3,   // Pitch drop
            0.85,  // Volume
            // DS parameters:
            0.05,   // Short tonal decay (50ms)
            0.12,   // Noise decay (120ms)
            0.3,    // Noise tail decay (300ms)
            1000.0, // Noise color frequency
            3000.0, // Filter cutoff
            2.0,    // Filter resonance
            1,      // Bandpass filter
            0.4,    // 40% tonal / 60% noise
            true,   // Click enabled
            true,   // Phase mod enabled
            0.5,    // Moderate phase mod
        )
    }
}

/// Smoothed parameters for real-time control of the snare drum
/// These use one-pole smoothing to prevent clicks/pops during parameter changes
pub struct SnareParams {
    pub frequency: SmoothedParam,        // Base frequency (100-600 Hz)
    pub decay: SmoothedParam,            // Decay time in seconds (0.01-2.0)
    pub brightness: SmoothedParam,       // Snap/crack tone amount (0-1)
    pub volume: SmoothedParam,           // Overall volume (0-1)
    pub tonal: SmoothedParam,            // Tonal component amount (0-1)
    pub noise: SmoothedParam,            // Noise component amount (0-1)
    pub pitch_drop: SmoothedParam,       // Pitch drop amount (0-1)

    // DS-style smoothed parameters
    pub tonal_decay: SmoothedParam,      // Tonal envelope decay (0.004-0.7s)
    pub noise_decay: SmoothedParam,      // Noise envelope decay (0.004-3.5s)
    pub noise_tail_decay: SmoothedParam, // Noise tail decay (0.004-3.5s)
    pub noise_color: SmoothedParam,      // Noise oscillator frequency (400-1500 Hz)
    pub filter_cutoff: SmoothedParam,    // SVF filter cutoff (100-20000 Hz)
    pub filter_resonance: SmoothedParam, // SVF filter resonance (0.5-10.0)
    pub filter_type: u8,                 // 0=LP, 1=BP, 2=HP, 3=notch (not smoothed)
    pub xfade: SmoothedParam,            // Tonal/noise crossfade (0-1)
    pub click_enabled: bool,             // Enable click transient (not smoothed)
    pub phase_mod_enabled: bool,         // Enable phase modulation (not smoothed)
    pub phase_mod_amount: SmoothedParam, // Phase mod depth (0-1)
}

impl SnareParams {
    /// Create new smoothed parameters from a config
    pub fn from_config(config: &SnareConfig, sample_rate: f32) -> Self {
        Self {
            frequency: SmoothedParam::new(
                config.snare_frequency,
                100.0,
                600.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            decay: SmoothedParam::new(
                config.decay_time,
                0.01,
                3.5,
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
            // DS-style parameters
            tonal_decay: SmoothedParam::new(
                config.tonal_decay,
                0.004,
                0.7,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            noise_decay: SmoothedParam::new(
                config.noise_decay,
                0.004,
                3.5,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            noise_tail_decay: SmoothedParam::new(
                config.noise_tail_decay,
                0.004,
                3.5,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            noise_color: SmoothedParam::new(
                config.noise_color,
                400.0,
                1500.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            filter_cutoff: SmoothedParam::new(
                config.filter_cutoff,
                100.0,
                20000.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            filter_resonance: SmoothedParam::new(
                config.filter_resonance,
                0.5,
                10.0,
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
            click_enabled: config.click_enabled,
            phase_mod_enabled: config.phase_mod_enabled,
            phase_mod_amount: SmoothedParam::new(
                config.phase_mod_amount,
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
        self.decay.tick();
        self.brightness.tick();
        self.volume.tick();
        self.tonal.tick();
        self.noise.tick();
        self.pitch_drop.tick();
        self.tonal_decay.tick();
        self.noise_decay.tick();
        self.noise_tail_decay.tick();
        self.noise_color.tick();
        self.filter_cutoff.tick();
        self.filter_resonance.tick();
        self.xfade.tick();
        self.phase_mod_amount.tick();
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
            && self.noise_decay.is_settled()
            && self.noise_tail_decay.is_settled()
            && self.noise_color.is_settled()
            && self.filter_cutoff.is_settled()
            && self.filter_resonance.is_settled()
            && self.xfade.is_settled()
            && self.phase_mod_amount.is_settled()
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

    /// Click envelope (short impulse at transient)
    click_envelope: Envelope,

    /// Noise tail envelope (separate from main noise)
    noise_tail_envelope: Envelope,

    /// Tonal-specific envelope (DS-style separate decay)
    tonal_envelope: Envelope,

    /// Main noise envelope (DS-style)
    main_noise_envelope: Envelope,
}

impl SnareDrum {
    pub fn new(sample_rate: f32) -> Self {
        let config = SnareConfig::default();
        Self::with_config(sample_rate, config)
    }

    pub fn with_config(sample_rate: f32, config: SnareConfig) -> Self {
        let params = SnareParams::from_config(&config, sample_rate);
        let mut snare = Self {
            sample_rate,
            config,
            params,
            tonal_oscillator: Oscillator::new(sample_rate, config.snare_frequency),
            noise_oscillator: Oscillator::new(sample_rate, config.snare_frequency * 8.0),
            crack_oscillator: Oscillator::new(sample_rate, config.snare_frequency * 25.0),
            pitch_envelope: Envelope::new(),
            base_frequency: config.snare_frequency,
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
                config.filter_cutoff,
                config.filter_resonance,
            ),
            phase_modulator: PhaseModulator::new(sample_rate),
            click_envelope: Envelope::new(),
            noise_tail_envelope: Envelope::new(),
            tonal_envelope: Envelope::new(),
            main_noise_envelope: Envelope::new(),
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
        self.base_frequency = config.snare_frequency;
        self.pitch_start_multiplier = 1.0 + config.pitch_drop * 1.5;
        // Update smoothed params to match new config
        self.params.frequency.set_target(config.snare_frequency);
        self.params.decay.set_target(config.decay_time);
        self.params.brightness.set_target(config.crack_amount);
        self.params.volume.set_target(config.volume);
        self.params.tonal.set_target(config.tonal_amount);
        self.params.noise.set_target(config.noise_amount);
        self.params.pitch_drop.set_target(config.pitch_drop);
        // DS parameters
        self.params.tonal_decay.set_target(config.tonal_decay);
        self.params.noise_decay.set_target(config.noise_decay);
        self.params.noise_tail_decay.set_target(config.noise_tail_decay);
        self.params.noise_color.set_target(config.noise_color);
        self.params.filter_cutoff.set_target(config.filter_cutoff);
        self.params.filter_resonance.set_target(config.filter_resonance);
        self.params.filter_type = config.filter_type;
        self.params.xfade.set_target(config.xfade);
        self.params.click_enabled = config.click_enabled;
        self.params.phase_mod_enabled = config.phase_mod_enabled;
        self.params.phase_mod_amount.set_target(config.phase_mod_amount);
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

        // Get current smoothed parameter values
        let base_freq = self.params.frequency.get();
        let base_decay = self.params.decay.get();
        let volume = self.params.volume.get();
        let brightness = self.params.brightness.get();
        let tonal_amount = self.params.tonal.get();
        let noise_amount = self.params.noise.get();
        let pitch_drop = self.params.pitch_drop.get();

        // Get DS-style parameters
        let tonal_decay = self.params.tonal_decay.get();
        let noise_decay = self.params.noise_decay.get();
        let noise_tail_decay = self.params.noise_tail_decay.get();

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

        // Configure tonal oscillator envelope with velocity-scaled decay
        self.tonal_oscillator.frequency_hz = base_freq;
        self.tonal_oscillator.set_volume(tonal_amount * volume);
        self.tonal_oscillator.set_adsr(ADSRConfig::new(
            0.001,                  // Very fast attack
            scaled_decay * 0.8,     // Main decay (velocity-scaled)
            0.0,                    // No sustain
            scaled_decay * 0.4,     // Medium release
        ));

        // Configure noise oscillator envelope with velocity-scaled decay
        self.noise_oscillator.frequency_hz = base_freq * 8.0;
        self.noise_oscillator.set_volume(noise_amount * volume * 0.8);
        self.noise_oscillator.set_adsr(ADSRConfig::new(
            0.001,                  // Very fast attack
            scaled_decay * 0.6,     // Shorter decay for noise (velocity-scaled)
            0.0,                    // No sustain
            scaled_decay * 0.3,     // Quick release
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

        // Tonal envelope (DS-style separate decay)
        let scaled_tonal_decay = tonal_decay * decay_scale;
        self.tonal_envelope.set_config(ADSRConfig::new(
            0.001,                       // Fast attack
            scaled_tonal_decay,          // DS-style tonal decay
            0.0,                         // No sustain
            scaled_tonal_decay * 0.2,    // Short release
        ));

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

        // Click envelope (very short impulse)
        if self.params.click_enabled {
            self.click_envelope.set_config(ADSRConfig::new(
                0.0005,   // Very fast attack (0.5ms)
                0.005,    // Very short decay (5ms)
                0.0,      // No sustain
                0.002,    // Very short release
            ));
            self.click_envelope.trigger(time);
        }

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

        // Trigger phase modulator if enabled
        if self.params.phase_mod_enabled {
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
            self.click_envelope.release(time);
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

        // Use smoothed frequency for pitch calculations
        let base_frequency = self.params.frequency.get();

        // Calculate pitch modulation from envelope
        let pitch_envelope_value = self.pitch_envelope.get_amplitude(current_time);
        let mut frequency_multiplier =
            1.0 + (self.pitch_start_multiplier - 1.0) * pitch_envelope_value;

        // Apply phase modulation if enabled (DS-style transient snap)
        if self.params.phase_mod_enabled {
            let phase_mod = self.phase_modulator.tick(current_time);
            let phase_mod_amount = self.params.phase_mod_amount.get();
            // Phase mod adds brief frequency boost (multiplier of up to 2x at full amount)
            frequency_multiplier *= 1.0 + (phase_mod * phase_mod_amount * 1.0);
        }

        // Apply pitch envelope to tonal oscillator only
        self.tonal_oscillator.frequency_hz = base_frequency * frequency_multiplier;

        // Update noise oscillator frequency from noise_color parameter
        let noise_color = self.params.noise_color.get();
        self.noise_oscillator.frequency_hz = noise_color;

        // Update filter parameters
        let filter_cutoff = self.params.filter_cutoff.get();
        let filter_resonance = self.params.filter_resonance.get();
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

        // --- Generate click transient ---
        let click_output = if self.params.click_enabled {
            let click_env = self.click_envelope.get_amplitude(current_time);
            // Click is a short burst at high frequency
            click_env * 0.3 * self.params.volume.get()
        } else {
            0.0
        };

        // Sum all components
        let total_output = tonal_output + noise_output + crack_output + click_output;

        // Apply velocity amplitude scaling (sqrt for perceptually linear loudness)
        let velocity_amplitude = self.current_velocity.sqrt();
        let final_output = total_output * velocity_amplitude;

        // Check if snare is still active
        let classic_active = self.tonal_oscillator.envelope.is_active
            || self.noise_oscillator.envelope.is_active
            || self.crack_oscillator.envelope.is_active;
        let ds_active = self.tonal_envelope.is_active
            || self.main_noise_envelope.is_active
            || self.noise_tail_envelope.is_active
            || self.click_envelope.is_active
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

    /// Set volume (smoothed)
    pub fn set_volume(&mut self, volume: f32) {
        self.params.volume.set_target(volume);
    }

    /// Set base frequency (smoothed)
    pub fn set_frequency(&mut self, frequency: f32) {
        self.params.frequency.set_target(frequency);
    }

    /// Set decay time (smoothed)
    /// Envelope will be reconfigured on next trigger
    pub fn set_decay(&mut self, decay_time: f32) {
        self.params.decay.set_target(decay_time);
    }

    /// Set brightness/snap amount (smoothed)
    pub fn set_brightness(&mut self, brightness: f32) {
        self.params.brightness.set_target(brightness);
    }

    /// Set tonal amount (smoothed)
    pub fn set_tonal(&mut self, tonal_amount: f32) {
        self.params.tonal.set_target(tonal_amount);
    }

    /// Set noise amount (smoothed)
    pub fn set_noise(&mut self, noise_amount: f32) {
        self.params.noise.set_target(noise_amount);
    }

    /// Set crack amount (alias for set_brightness)
    pub fn set_crack(&mut self, crack_amount: f32) {
        self.set_brightness(crack_amount);
    }

    /// Set pitch drop amount (smoothed)
    pub fn set_pitch_drop(&mut self, pitch_drop: f32) {
        self.params.pitch_drop.set_target(pitch_drop);
    }

    // --- DS-style parameter setters ---

    /// Set tonal decay time (smoothed)
    pub fn set_tonal_decay(&mut self, decay: f32) {
        self.params.tonal_decay.set_target(decay);
    }

    /// Set noise decay time (smoothed)
    pub fn set_noise_decay(&mut self, decay: f32) {
        self.params.noise_decay.set_target(decay);
    }

    /// Set noise tail decay time (smoothed)
    pub fn set_noise_tail_decay(&mut self, decay: f32) {
        self.params.noise_tail_decay.set_target(decay);
    }

    /// Set noise color/frequency (smoothed)
    pub fn set_noise_color(&mut self, color: f32) {
        self.params.noise_color.set_target(color);
    }

    /// Set filter cutoff frequency (smoothed)
    pub fn set_filter_cutoff(&mut self, cutoff: f32) {
        self.params.filter_cutoff.set_target(cutoff);
    }

    /// Set filter resonance (smoothed)
    pub fn set_filter_resonance(&mut self, resonance: f32) {
        self.params.filter_resonance.set_target(resonance);
    }

    /// Set filter type (0=LP, 1=BP, 2=HP, 3=notch)
    pub fn set_filter_type(&mut self, filter_type: u8) {
        self.params.filter_type = filter_type.min(3);
    }

    /// Set tonal/noise crossfade (smoothed)
    /// 0.0 = all tonal, 1.0 = all noise
    pub fn set_xfade(&mut self, xfade: f32) {
        self.params.xfade.set_target(xfade);
    }

    /// Enable/disable click transient
    pub fn set_click_enabled(&mut self, enabled: bool) {
        self.params.click_enabled = enabled;
    }

    /// Enable/disable phase modulation
    pub fn set_phase_mod_enabled(&mut self, enabled: bool) {
        self.params.phase_mod_enabled = enabled;
    }

    /// Set phase modulation amount (smoothed)
    pub fn set_phase_mod_amount(&mut self, amount: f32) {
        self.params.phase_mod_amount.set_target(amount);
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
            "noise_decay",
            "noise_tail_decay",
            "noise_color",
            "filter_cutoff",
            "filter_resonance",
            "xfade",
            "phase_mod_amount",
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
            "noise_decay" => {
                self.params.noise_decay.set_bipolar(value);
                Ok(())
            }
            "noise_tail_decay" => {
                self.params.noise_tail_decay.set_bipolar(value);
                Ok(())
            }
            "noise_color" => {
                self.params.noise_color.set_bipolar(value);
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
            "noise_decay" => Some(self.params.noise_decay.range()),
            "noise_tail_decay" => Some(self.params.noise_tail_decay.range()),
            "noise_color" => Some(self.params.noise_color.range()),
            "filter_cutoff" => Some(self.params.filter_cutoff.range()),
            "filter_resonance" => Some(self.params.filter_resonance.range()),
            "xfade" => Some(self.params.xfade.range()),
            "phase_mod_amount" => Some(self.params.phase_mod_amount.range()),
            _ => None,
        }
    }
}
