use crate::envelope::{ADSRConfig, Envelope, EnvelopeCurve};
use crate::filters::{ResonantHighpassFilter, ResonantLowpassFilter};
use crate::gen::oscillator::Oscillator;
use crate::gen::pink_noise::PinkNoise;
use crate::gen::waveform::Waveform;
use crate::instruments::fm_snap::{FMSnapSynthesizer, PhaseModulator};
use crate::utils::{SmoothedParam, DEFAULT_SMOOTH_TIME_MS};

/// Static configuration for kick drum presets
/// Used to initialize a KickDrum with specific parameter values
#[derive(Clone, Copy, Debug)]
pub struct KickConfig {
    pub kick_frequency: f32,     // Base frequency (30-80Hz typical)
    pub punch_amount: f32,       // Mid-frequency presence (0.0-1.0)
    pub sub_amount: f32,         // Sub-bass presence (0.0-1.0)
    pub click_amount: f32,       // High-frequency click (0.0-1.0)
    pub snap_amount: f32,        // FM snap transient/zap (0.0-1.0)
    pub decay_time: f32,         // Overall decay length in seconds
    pub pitch_envelope: f32,     // Frequency sweep amount (0.0-1.0)
    pub pitch_curve: f32,        // Pitch envelope decay curve (0.1-10.0, 1.0 = linear)
    pub volume: f32,             // Overall volume (0.0-1.0)
    pub pitch_start_ratio: f32,  // Starting pitch multiplier (1.0-10.0, default 3.0)
    pub phase_mod_enabled: bool, // Enable DS-style phase modulation for transient
    pub phase_mod_amount: f32,   // Phase modulation depth (0.0-1.0)
    pub noise_amount: f32,       // Pink noise layer amount (0.0-1.0)
    pub noise_cutoff: f32,       // Noise lowpass filter cutoff (20.0-20000.0 Hz)
    pub noise_resonance: f32,    // Noise lowpass filter resonance (0.0-10.0)
    // Master amplitude envelope parameters (DS Kick "p curvey" style)
    pub amp_attack: f32,         // Amplitude attack time in seconds (0.0005-0.4)
    pub amp_decay: f32,          // Amplitude decay time in seconds (0.0005-4.0)
    pub amp_attack_curve: f32,   // Attack curve (0.1-10.0, <1.0 = fast rise)
    pub amp_decay_curve: f32,    // Decay curve (0.1-10.0, <1.0 = natural decay)
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
        pitch_curve: f32,
        volume: f32,
    ) -> Self {
        Self {
            kick_frequency: kick_frequency.max(30.0).min(80.0), // Typical kick drum frequency range
            punch_amount: punch_amount.clamp(0.0, 1.0),
            sub_amount: sub_amount.clamp(0.0, 1.0),
            click_amount: click_amount.clamp(0.0, 1.0),
            snap_amount: snap_amount.clamp(0.0, 1.0),
            decay_time: decay_time.max(0.01).min(5.0), // Reasonable decay range
            pitch_envelope: pitch_envelope.clamp(0.0, 1.0),
            pitch_curve: pitch_curve.clamp(0.1, 10.0), // Curve exponent for pitch envelope decay
            volume: volume.clamp(0.0, 1.0),
            // Backward-compatible defaults for new DS Kick parameters
            pitch_start_ratio: 3.0,    // Default matches old behavior (1.0 + 1.0 * 2.0 = 3.0 max)
            phase_mod_enabled: false,  // Disabled by default for compatibility
            phase_mod_amount: 0.0,
            // Noise layer defaults (disabled by default)
            noise_amount: 0.0,
            noise_cutoff: 2000.0,
            noise_resonance: 2.0,
            // Master amplitude envelope defaults (matches oscillator envelope behavior)
            amp_attack: 0.001,   // 1ms instant attack
            amp_decay: 0.5,      // 500ms default decay
            amp_attack_curve: 1.0, // Linear attack (backward compatible)
            amp_decay_curve: 1.0,  // Linear decay (backward compatible)
        }
    }

    /// Create a KickConfig with all parameters including DS Kick features
    pub fn new_full(
        kick_frequency: f32,
        punch_amount: f32,
        sub_amount: f32,
        click_amount: f32,
        snap_amount: f32,
        decay_time: f32,
        pitch_envelope: f32,
        pitch_curve: f32,
        volume: f32,
        pitch_start_ratio: f32,
        phase_mod_enabled: bool,
        phase_mod_amount: f32,
        noise_amount: f32,
        noise_cutoff: f32,
        noise_resonance: f32,
        amp_attack: f32,
        amp_decay: f32,
        amp_attack_curve: f32,
        amp_decay_curve: f32,
    ) -> Self {
        Self {
            kick_frequency: kick_frequency.max(30.0).min(200.0), // Extended range for DS style
            punch_amount: punch_amount.clamp(0.0, 1.0),
            sub_amount: sub_amount.clamp(0.0, 1.0),
            click_amount: click_amount.clamp(0.0, 1.0),
            snap_amount: snap_amount.clamp(0.0, 1.0),
            decay_time: decay_time.max(0.01).min(5.0),
            pitch_envelope: pitch_envelope.clamp(0.0, 1.0),
            pitch_curve: pitch_curve.clamp(0.1, 10.0),
            volume: volume.clamp(0.0, 1.0),
            pitch_start_ratio: pitch_start_ratio.clamp(1.0, 10.0),
            phase_mod_enabled,
            phase_mod_amount: phase_mod_amount.clamp(0.0, 1.0),
            noise_amount: noise_amount.clamp(0.0, 1.0),
            noise_cutoff: noise_cutoff.clamp(20.0, 20000.0),
            noise_resonance: noise_resonance.clamp(0.0, 10.0),
            amp_attack: amp_attack.clamp(0.0005, 0.4),
            amp_decay: amp_decay.clamp(0.0005, 4.0),
            amp_attack_curve: amp_attack_curve.clamp(0.1, 10.0),
            amp_decay_curve: amp_decay_curve.clamp(0.1, 10.0),
        }
    }

    pub fn default() -> Self {
        // snap_amount defaults to 0.3 for subtle attack transient
        // pitch_curve 0.3 = aggressive exponential pitch drop (very punchy)
        Self::new(30.0, 0.80, 0.80, 0.20, 0.3, 0.28, 0.20, 0.3, 0.80)
    }

    pub fn punchy() -> Self {
        // punchy preset gets more snap for aggressive attack
        // pitch_curve 0.2 = very fast initial pitch drop for extreme 808-style sound
        Self::new(60.0, 0.9, 0.6, 0.4, 0.6, 0.6, 0.7, 0.2, 0.85)
    }

    pub fn deep() -> Self {
        // deep preset has less snap for smoother attack
        // pitch_curve 3.0 = very slow initial pitch drop for deeper, smoother sound
        Self::new(45.0, 0.5, 1.0, 0.2, 0.2, 1.2, 0.5, 3.0, 0.9)
    }

    pub fn tight() -> Self {
        // tight preset has moderate snap
        // pitch_curve 0.25 = aggressive pitch drop for tight, punchy sound
        Self::new(70.0, 0.8, 0.7, 0.5, 0.5, 0.4, 0.8, 0.25, 0.8)
    }

    /// DS Kick preset - Ableton Drum Synth style
    /// Single sine oscillator with 5:1 pitch ratio and phase modulation transient
    /// Based on analysis of the Ableton DS Kick Max MSP patch
    pub fn ds_kick() -> Self {
        Self::new_full(
            50.0,  // Base frequency - classic kick range
            0.0,   // Punch disabled - DS uses single oscillator
            1.0,   // Full sub (sine) output
            0.0,   // Click disabled - using phase mod instead
            0.0,   // Snap disabled - using phase mod instead
            0.5,   // 500ms decay (oscillator envelopes)
            1.0,   // Full pitch envelope
            0.25,  // Approximates Max's -0.83 convex curve
            0.85,  // Slight headroom
            5.0,   // DS signature: 5x pitch ratio
            true,  // Enable phase modulation
            0.7,   // Strong phase mod for transient snap
            0.07,  // Very subtle pink noise (7%) for warmth/body - matches Max patch 0.2 * 0.3 scaling
            100.0, // Very low cutoff (100 Hz) for subtle rumble - matches Max patch lores~ 100
            0.2,   // Minimal resonance (Q=0.2) for natural sound - matches Max patch
            // Master amplitude envelope - "p curvey" style
            0.001, // 1ms instant attack
            0.5,   // 500ms decay - matches Max patch default
            0.5,   // Attack curve: fast rise (approximates Max's -0.5)
            0.3,   // Decay curve: natural exponential (approximates Max's -0.8)
        )
    }
}

/// Smoothed parameters for real-time control of the kick drum
/// These use one-pole smoothing to prevent clicks/pops during parameter changes
pub struct KickParams {
    pub frequency: SmoothedParam,       // Base frequency (30-200 Hz)
    pub punch: SmoothedParam,           // Mid-frequency presence (0-1)
    pub sub: SmoothedParam,             // Sub-bass presence (0-1)
    pub click: SmoothedParam,           // High-frequency click (0-1)
    pub snap: SmoothedParam,            // FM snap transient/zap (0-1)
    pub decay: SmoothedParam,           // Decay time in seconds (0.01-5.0)
    pub pitch_envelope: SmoothedParam,  // Pitch envelope amount (0-1)
    pub pitch_curve: SmoothedParam,     // Pitch envelope decay curve (0.1-10.0)
    pub volume: SmoothedParam,          // Overall volume (0-1)
    pub pitch_start_ratio: SmoothedParam, // Starting pitch multiplier (1.0-10.0)
    pub phase_mod_enabled: bool,        // Enable DS-style phase modulation (not smoothed)
    pub phase_mod_amount: SmoothedParam,  // Phase modulation depth (0-1)
    pub noise_amount: SmoothedParam,    // Pink noise layer amount (0-1)
    pub noise_cutoff: SmoothedParam,    // Noise lowpass filter cutoff (20-20000 Hz)
    pub noise_resonance: SmoothedParam, // Noise lowpass filter resonance (0-10)
    // Master amplitude envelope parameters
    pub amp_attack: SmoothedParam,      // Amplitude attack time (0.0005-0.4s)
    pub amp_decay: SmoothedParam,       // Amplitude decay time (0.0005-4.0s)
    pub amp_attack_curve: SmoothedParam, // Attack curve (0.1-10.0)
    pub amp_decay_curve: SmoothedParam,  // Decay curve (0.1-10.0)
}

impl KickParams {
    /// Create new smoothed parameters from a config
    pub fn from_config(config: &KickConfig, sample_rate: f32) -> Self {
        Self {
            frequency: SmoothedParam::new(
                config.kick_frequency,
                30.0,
                200.0, // Extended range for DS style
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
            pitch_curve: SmoothedParam::new(
                config.pitch_curve,
                0.1,
                10.0,
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
            pitch_start_ratio: SmoothedParam::new(
                config.pitch_start_ratio,
                1.0,
                10.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            phase_mod_enabled: config.phase_mod_enabled,
            phase_mod_amount: SmoothedParam::new(
                config.phase_mod_amount,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            noise_amount: SmoothedParam::new(
                config.noise_amount,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            noise_cutoff: SmoothedParam::new(
                config.noise_cutoff,
                20.0,
                20000.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            noise_resonance: SmoothedParam::new(
                config.noise_resonance,
                0.0,
                10.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            amp_attack: SmoothedParam::new(
                config.amp_attack,
                0.0005,
                0.4,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            amp_decay: SmoothedParam::new(
                config.amp_decay,
                0.0005,
                4.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            amp_attack_curve: SmoothedParam::new(
                config.amp_attack_curve,
                0.1,
                10.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            amp_decay_curve: SmoothedParam::new(
                config.amp_decay_curve,
                0.1,
                10.0,
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
        self.pitch_curve.tick();
        self.volume.tick();
        self.pitch_start_ratio.tick();
        self.phase_mod_amount.tick();
        self.noise_amount.tick();
        self.noise_cutoff.tick();
        self.noise_resonance.tick();
        self.amp_attack.tick();
        self.amp_decay.tick();
        self.amp_attack_curve.tick();
        self.amp_decay_curve.tick();

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
            && self.pitch_curve.is_settled()
            && self.volume.is_settled()
            && self.pitch_start_ratio.is_settled()
            && self.phase_mod_amount.is_settled()
            && self.noise_amount.is_settled()
            && self.noise_cutoff.is_settled()
            && self.noise_resonance.is_settled()
            && self.amp_attack.is_settled()
            && self.amp_decay.is_settled()
            && self.amp_attack_curve.is_settled()
            && self.amp_decay_curve.is_settled()
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
            pitch_curve: self.pitch_curve.get(),
            volume: self.volume.get(),
            pitch_start_ratio: self.pitch_start_ratio.get(),
            phase_mod_enabled: self.phase_mod_enabled,
            phase_mod_amount: self.phase_mod_amount.get(),
            noise_amount: self.noise_amount.get(),
            noise_cutoff: self.noise_cutoff.get(),
            noise_resonance: self.noise_resonance.get(),
            amp_attack: self.amp_attack.get(),
            amp_decay: self.amp_decay.get(),
            amp_attack_curve: self.amp_attack_curve.get(),
            amp_decay_curve: self.amp_decay_curve.get(),
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

    // DS Kick-style phase modulator for transient snap
    pub phase_modulator: PhaseModulator,

    // Pink noise layer with resonant lowpass filter (DS Kick-style)
    pub pink_noise: PinkNoise,
    pub noise_filter: ResonantLowpassFilter,
    pub noise_envelope: Envelope,

    // Master amplitude envelope (DS Kick "p curvey" style)
    // Applied multiplicatively on top of oscillator envelopes
    pub amplitude_envelope: Envelope,

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

        // Calculate pitch start multiplier from pitch_start_ratio and pitch_envelope
        // At pitch_envelope=1.0, we get the full pitch_start_ratio
        // At pitch_envelope=0.0, we get 1.0 (no pitch sweep)
        let pitch_start_multiplier =
            1.0 + (config.pitch_start_ratio - 1.0) * config.pitch_envelope;

        let mut kick = Self {
            sample_rate,
            params,
            sub_oscillator: Oscillator::new(sample_rate, config.kick_frequency),
            punch_oscillator: Oscillator::new(sample_rate, config.kick_frequency * 2.5),
            click_oscillator: Oscillator::new(sample_rate, config.kick_frequency * 40.0),
            pitch_envelope: Envelope::new(),
            pitch_start_multiplier,
            click_filter: ResonantHighpassFilter::new(sample_rate, 8000.0, 4.0),
            fm_snap: FMSnapSynthesizer::new(sample_rate),
            phase_modulator: PhaseModulator::new(sample_rate),
            pink_noise: PinkNoise::new(),
            noise_filter: ResonantLowpassFilter::new(sample_rate, config.noise_cutoff, config.noise_resonance),
            noise_envelope: Envelope::new(),
            amplitude_envelope: Envelope::new(),
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

        // Noise envelope: Synchronized with amplitude envelope for consistent body
        self.noise_envelope.set_config(ADSRConfig::new(
            0.001,       // Very fast attack
            decay,       // Synchronized decay time
            0.0,         // No sustain
            decay * 0.2, // Synchronized release
        ));
    }

    /// Apply current smoothed parameters to oscillators (called per-sample)
    #[inline]
    fn apply_params(&mut self) {
        let punch = self.params.punch.get();
        let sub = self.params.sub.get();
        let click = self.params.click.get();
        let pitch_envelope = self.params.pitch_envelope.get();
        let pitch_start_ratio = self.params.pitch_start_ratio.get();
        let volume = self.params.volume.get();

        // Update pitch start multiplier using configurable ratio
        // At pitch_envelope=1.0, we get the full pitch_start_ratio
        // At pitch_envelope=0.0, we get 1.0 (no pitch sweep)
        self.pitch_start_multiplier = 1.0 + (pitch_start_ratio - 1.0) * pitch_envelope;

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
        self.params.pitch_curve.set_target(config.pitch_curve);
        self.params.volume.set_target(config.volume);
        self.params.pitch_start_ratio.set_target(config.pitch_start_ratio);
        self.params.phase_mod_enabled = config.phase_mod_enabled;
        self.params.phase_mod_amount.set_target(config.phase_mod_amount);
        self.params.noise_amount.set_target(config.noise_amount);
        self.params.noise_cutoff.set_target(config.noise_cutoff);
        self.params.noise_resonance.set_target(config.noise_resonance);
        self.params.amp_attack.set_target(config.amp_attack);
        self.params.amp_decay.set_target(config.amp_decay);
        self.params.amp_attack_curve.set_target(config.amp_attack_curve);
        self.params.amp_decay_curve.set_target(config.amp_decay_curve);

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
        // NOTE: Currently unused - pitch envelope duration matches amplitude to prevent pops
        let _pitch_decay_scale = 1.0 - (self.velocity_to_pitch * vel_squared);

        // Get base parameters
        let base_decay = self.params.decay.get() * decay_scale;
        let base_freq = self.params.frequency.get();

        // Configure pitch envelope with same duration as amplitude envelope
        // The exponential curve will make the pitch sweep complete early,
        // but the envelope stays active (at sustain=0) to prevent artifacts
        // High velocity = short pitch decay (sharp, punchy attack)
        // Low velocity = long pitch decay (smooth, subtle pitch sweep)
        let pitch_curve_value = self.params.pitch_curve.get();
        let decay_curve = if (pitch_curve_value - 1.0).abs() < 0.01 {
            // Close enough to 1.0 = use linear for efficiency
            EnvelopeCurve::Linear
        } else {
            EnvelopeCurve::Exponential(pitch_curve_value)
        };

        // CRITICAL: Pitch envelope must have same total duration as amplitude envelope
        // to prevent phase discontinuities and pops at the end
        // The exponential curve ensures pitch sweep completes early (within ~60% of decay)
        // while the envelope stays active to keep frequency stable
        self.pitch_envelope.set_config(
            ADSRConfig::new(0.001, base_decay, 0.0, base_decay * 0.2)
                .with_decay_curve(decay_curve),
        );

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

        // Trigger phase modulator if enabled (DS Kick-style transient)
        if self.params.phase_mod_enabled {
            self.phase_modulator.trigger(time);
        }

        // Configure and trigger noise envelope with velocity-scaled decay
        self.noise_envelope.set_config(ADSRConfig::new(
            0.001,
            base_decay,
            0.0,
            base_decay * 0.2,
        ));
        self.noise_envelope.trigger(time);

        // Configure and trigger master amplitude envelope (DS Kick "p curvey" style)
        // This is applied multiplicatively on top of oscillator envelopes
        let amp_attack = self.params.amp_attack.get();
        let amp_decay = self.params.amp_decay.get() * decay_scale; // Velocity scales decay
        let amp_attack_curve_val = self.params.amp_attack_curve.get();
        let amp_decay_curve_val = self.params.amp_decay_curve.get();

        let amp_attack_curve = if (amp_attack_curve_val - 1.0).abs() < 0.01 {
            EnvelopeCurve::Linear
        } else {
            EnvelopeCurve::Exponential(amp_attack_curve_val)
        };
        let amp_decay_curve = if (amp_decay_curve_val - 1.0).abs() < 0.01 {
            EnvelopeCurve::Linear
        } else {
            EnvelopeCurve::Exponential(amp_decay_curve_val)
        };

        self.amplitude_envelope.set_config(
            ADSRConfig::new(amp_attack, amp_decay, 0.0, amp_decay * 0.2)
                .with_attack_curve(amp_attack_curve)
                .with_decay_curve(amp_decay_curve),
        );
        self.amplitude_envelope.trigger(time);

        // Reset filter states for clean transients
        self.click_filter.reset();
        self.noise_filter.reset();
        self.pink_noise.reset();
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

        // Calculate pitch modulation from envelope
        let pitch_envelope_value = self.pitch_envelope.get_amplitude(current_time);
        let mut frequency_multiplier =
            1.0 + (self.pitch_start_multiplier - 1.0) * pitch_envelope_value;

        // Apply phase modulation if enabled (DS Kick-style transient snap)
        // This adds a brief frequency boost at the attack for extra punch
        if self.params.phase_mod_enabled {
            let phase_mod = self.phase_modulator.tick(current_time);
            let phase_mod_amount = self.params.phase_mod_amount.get();
            // Phase mod adds brief frequency boost (multiplier of up to 3x at full amount)
            frequency_multiplier *= 1.0 + (phase_mod * phase_mod_amount * 2.0);
        }

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

        // Generate and process pink noise layer (DS Kick-style)
        let noise_amount = self.params.noise_amount.get();
        let noise_output = if noise_amount > 0.001 {
            // Generate pink noise sample
            let pink_noise_sample = self.pink_noise.tick();

            // Update filter parameters from smoothed params
            let noise_cutoff = self.params.noise_cutoff.get();
            let noise_resonance = self.params.noise_resonance.get();
            self.noise_filter.set_cutoff_freq(noise_cutoff);
            self.noise_filter.set_resonance(noise_resonance);

            // Apply resonant lowpass filter
            let filtered_noise = self.noise_filter.process(pink_noise_sample);

            // Apply noise envelope
            let noise_env = self.noise_envelope.get_amplitude(current_time);
            filtered_noise * noise_env * noise_amount * self.params.volume.get()
        } else {
            0.0
        };

        let total_output = sub_output
            + punch_output
            + filtered_click_output
            + (fm_snap_output * snap * self.params.volume.get())
            + noise_output;

        // Apply master amplitude envelope (DS Kick "p curvey" style)
        // Multiplicative with existing oscillator envelopes
        let amp_env = self.amplitude_envelope.get_amplitude(current_time);

        // Apply velocity amplitude scaling (sqrt for perceptually linear loudness)
        let velocity_amplitude = self.current_velocity.sqrt();
        let final_output = total_output * amp_env * velocity_amplitude;

        // Check if kick is still active
        // Master amplitude envelope controls overall activity
        if !self.amplitude_envelope.is_active {
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

    /// Set pitch envelope decay curve (smoothed)
    /// Values < 1.0: Fast initial pitch drop, slow settle (punchy 808-style)
    /// Value = 1.0: Linear pitch sweep
    /// Values > 1.0: Slow initial pitch drop, fast settle (softer)
    pub fn set_pitch_curve(&mut self, pitch_curve: f32) {
        self.params.pitch_curve.set_target(pitch_curve.clamp(0.1, 10.0));
    }

    /// Set pitch start ratio (smoothed)
    /// Controls how much higher the initial pitch is relative to the base frequency
    /// 1.0 = no pitch sweep, 3.0 = default, 5.0 = DS Kick style, up to 10.0
    pub fn set_pitch_start_ratio(&mut self, ratio: f32) {
        self.params.pitch_start_ratio.set_target(ratio.clamp(1.0, 10.0));
    }

    /// Enable/disable DS Kick-style phase modulation
    /// When enabled, adds a brief frequency burst at note onset for transient snap
    pub fn set_phase_mod_enabled(&mut self, enabled: bool) {
        self.params.phase_mod_enabled = enabled;
    }

    /// Set phase modulation amount (smoothed)
    /// Controls the intensity of the DS Kick-style transient snap
    /// 0.0 = no effect, 1.0 = maximum effect
    pub fn set_phase_mod_amount(&mut self, amount: f32) {
        self.params.phase_mod_amount.set_target(amount.clamp(0.0, 1.0));
    }

    /// Set noise layer amount (smoothed)
    /// Controls the mix level of the pink noise layer
    /// 0.0 = no noise, 1.0 = full noise
    pub fn set_noise_amount(&mut self, amount: f32) {
        self.params.noise_amount.set_target(amount.clamp(0.0, 1.0));
    }

    /// Set noise filter cutoff frequency (smoothed)
    /// Controls the lowpass filter cutoff for the noise layer
    /// 20.0-20000.0 Hz
    pub fn set_noise_cutoff(&mut self, cutoff: f32) {
        self.params.noise_cutoff.set_target(cutoff.clamp(20.0, 20000.0));
    }

    /// Set noise filter resonance (smoothed)
    /// Controls the resonance/Q of the lowpass filter
    /// 0.0-10.0, typical 0.5-4.0
    pub fn set_noise_resonance(&mut self, resonance: f32) {
        self.params.noise_resonance.set_target(resonance.clamp(0.0, 10.0));
    }

    /// Set amplitude envelope attack time (smoothed)
    /// Controls how fast the kick reaches full volume
    /// 0.0005-0.4 seconds (0.5ms - 400ms)
    pub fn set_amp_attack(&mut self, attack: f32) {
        self.params.amp_attack.set_target(attack.clamp(0.0005, 0.4));
    }

    /// Set amplitude envelope decay time (smoothed)
    /// Controls how long the kick sustains before fading
    /// 0.0005-4.0 seconds (0.5ms - 4000ms)
    pub fn set_amp_decay(&mut self, decay: f32) {
        self.params.amp_decay.set_target(decay.clamp(0.0005, 4.0));
    }

    /// Set amplitude envelope attack curve (smoothed)
    /// Values < 1.0: Fast initial rise, slow approach to peak (punchy)
    /// Value = 1.0: Linear attack
    /// Values > 1.0: Slow initial rise, fast approach to peak (softer)
    pub fn set_amp_attack_curve(&mut self, curve: f32) {
        self.params.amp_attack_curve.set_target(curve.clamp(0.1, 10.0));
    }

    /// Set amplitude envelope decay curve (smoothed)
    /// Values < 1.0: Fast initial decay, slow tail (natural acoustic decay)
    /// Value = 1.0: Linear decay
    /// Values > 1.0: Slow initial decay, fast tail (unnatural)
    pub fn set_amp_decay_curve(&mut self, curve: f32) {
        self.params.amp_decay_curve.set_target(curve.clamp(0.1, 10.0));
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
            "pitch_curve",
            "volume",
            "pitch_start_ratio",
            "phase_mod_amount",
            "noise_amount",
            "noise_cutoff",
            "noise_resonance",
            "amp_attack",
            "amp_decay",
            "amp_attack_curve",
            "amp_decay_curve",
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
            "pitch_curve" => {
                self.params.pitch_curve.set_bipolar(value);
                Ok(())
            }
            "volume" => {
                self.params.volume.set_bipolar(value);
                Ok(())
            }
            "pitch_start_ratio" => {
                self.params.pitch_start_ratio.set_bipolar(value);
                Ok(())
            }
            "phase_mod_amount" => {
                self.params.phase_mod_amount.set_bipolar(value);
                Ok(())
            }
            "noise_amount" => {
                self.params.noise_amount.set_bipolar(value);
                Ok(())
            }
            "noise_cutoff" => {
                self.params.noise_cutoff.set_bipolar(value);
                Ok(())
            }
            "noise_resonance" => {
                self.params.noise_resonance.set_bipolar(value);
                Ok(())
            }
            "amp_attack" => {
                self.params.amp_attack.set_bipolar(value);
                Ok(())
            }
            "amp_decay" => {
                self.params.amp_decay.set_bipolar(value);
                Ok(())
            }
            "amp_attack_curve" => {
                self.params.amp_attack_curve.set_bipolar(value);
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
            "punch" => Some(self.params.punch.range()),
            "sub" => Some(self.params.sub.range()),
            "click" => Some(self.params.click.range()),
            "snap" => Some(self.params.snap.range()),
            "decay" => Some(self.params.decay.range()),
            "pitch_envelope" => Some(self.params.pitch_envelope.range()),
            "pitch_curve" => Some(self.params.pitch_curve.range()),
            "volume" => Some(self.params.volume.range()),
            "pitch_start_ratio" => Some(self.params.pitch_start_ratio.range()),
            "phase_mod_amount" => Some(self.params.phase_mod_amount.range()),
            "noise_amount" => Some(self.params.noise_amount.range()),
            "noise_cutoff" => Some(self.params.noise_cutoff.range()),
            "noise_resonance" => Some(self.params.noise_resonance.range()),
            "amp_attack" => Some(self.params.amp_attack.range()),
            "amp_decay" => Some(self.params.amp_decay.range()),
            "amp_attack_curve" => Some(self.params.amp_attack_curve.range()),
            "amp_decay_curve" => Some(self.params.amp_decay_curve.range()),
            _ => None,
        }
    }
}
