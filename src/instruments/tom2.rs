//! Tom2 instrument with morph oscillator for A/B testing with Max/MSP
//!
//! This replicates the signal flow from a Max patch with morph oscillator:
//! - Single envelope controls both pitch and amplitude
//! - Morph oscillator combines sine and triangle sources (noise disabled for testing)
//! - Envelope: attack to 1.0 in 1ms (curve 0.8), decay to 0.0 in decay_ms (curve -0.83)
//!
//! Pitch formula (matching Max patch):
//! - frequency = (envelope × bend)² × tune_frequency
//! - tune_frequency = zmap(tune, 0, 100, 40, 600)
//! - bend is scaled 0-2 (like Max zmap 1 127 0 2), then (env × bend) is squared
//!
//! Output = morph_oscillator * envelope_value
//!
//! Parameters:
//! - tune: Base frequency (0-100 maps to 40-600 Hz)
//! - bend: Pitch envelope depth (0-100, scaled to 0-2, then (env×bend)² applied)
//! - tone: Mix control position (0-100, crossfades between ring mod, triangle+noise, noise+gated sine)
//! - color: Noise rand~ rate (0-100 → double-mtof chain → ~116-2794 Hz)
//! - decay: Envelope decay time (0-100 maps to 0.5-4000ms)

use crate::engine::Instrument;
use crate::filters::{BiquadBandpass, MembraneResonator};
use crate::gen::{ClickOsc, MorphOsc};
use crate::max_curve::MaxCurveEnvelope;
use crate::utils::Blendable;

/// Frequency range constants (from Max zmap 0 1 40 600)
const FREQ_MIN: f32 = 40.0;
const FREQ_MAX: f32 = 600.0;

/// Fade range to prevent clicks on cutoff
const FADE_START_FREQ: f32 = 40.0; // Start fading at 40 Hz
const MIN_AUDIBLE_FREQ: f32 = 20.0; // Full cutoff at 20 Hz

/// Decay range constants (from Max zmap 1 100 0.5 4000)
const DECAY_MIN_MS: f32 = 0.5;
const DECAY_MAX_MS: f32 = 4000.0;

/// Generate triangle wave from phase (0.0 to 1.0)
#[inline]
fn triangle(phase: f32) -> f32 {
    let t = phase.fract();
    if t < 0.5 {
        4.0 * t - 1.0
    } else {
        3.0 - 4.0 * t
    }
}

/// Advance a phase accumulator by frequency
#[inline]
fn advance_phase(phase: &mut f32, frequency: f32, sample_rate: f32) {
    *phase += frequency / sample_rate;
    if *phase >= 1.0 {
        *phase -= 1.0;
    }
}

/// Tom drum with morph oscillator for A/B testing with Max/MSP reference
pub struct Tom2 {
    sample_rate: f32,
    morph_osc: MorphOsc,
    click_osc: ClickOsc,
    bandpass_filter: BiquadBandpass,
    envelope: MaxCurveEnvelope,
    is_active: bool,
    #[allow(dead_code)]
    trigger_time: f32,

    // Standalone triangle oscillator phase (matches Max's tri~ outside morphoscillator)
    tri_phase: f32,

    // Track if we've passed the attack phase for early cutoff
    past_attack: bool,

    // Parameters (matching Max patch ranges)
    tune: f32,  // 0-100: maps to 40-600 Hz
    bend: f32,  // 0-100: pitch envelope depth
    tone: f32,  // 0-100: mix control (100% = silent, noise channel disabled)
    color: f32, // 0-100: double-mtof chain gives rand~ rate 116-2794 Hz AND filter cutoff
    decay: f32, // 0-100: maps to 0.5-4000ms via zmap

    // Toggle for standalone triangle oscillator (for A/B testing)
    triangle_enabled: bool,

    // Membrane resonator effect - uses tom sound as input, rings independently of VCA
    membrane_resonator: MembraneResonator,
    membrane: f32,     // 0-100: mix amount
    membrane_q: f32,   // 0-100: Q scale (maps to 0.005-0.02, centered at 0.01)

    // Track when main tom sound is done but membrane is still ringing
    main_sound_done: bool,
}

/// Static configuration for Tom2 presets
/// All parameters use 0-100 ranges to match Max/MSP conventions
#[derive(Clone, Copy, Debug)]
pub struct Tom2Config {
    pub tune: f32,        // 0-100: maps to 40-600 Hz
    pub bend: f32,        // 0-100: pitch envelope depth
    pub tone: f32,        // 0-100: mix control
    pub color: f32,       // 0-100: noise rate / filter cutoff
    pub decay: f32,       // 0-100: maps to 0.5-4000ms
    pub membrane: f32,    // 0-100: membrane resonator mix
    pub membrane_q: f32,  // 0-100: membrane Q scale (maps to 0.005-0.02)
}

impl Tom2Config {
    /// "derp" preset - punchy mid tom
    pub fn derp() -> Self {
        Self {
            tune: 60.0,
            bend: 70.0,
            tone: 50.0,
            color: 0.0,
            decay: 20.0,
            membrane: 0.0,
            membrane_q: 50.0,
        }
    }

    /// "ring" preset - high, long decay
    pub fn ring() -> Self {
        Self {
            tune: 80.0,
            bend: 20.0,
            tone: 10.0,
            color: 0.0,
            decay: 100.0,
            membrane: 60.0,
            membrane_q: 70.0,
        }
    }

    /// "brush" preset - low, textured
    pub fn brush() -> Self {
        Self {
            tune: 40.0,
            bend: 20.0,
            tone: 10.0,
            color: 90.0,
            decay: 30.0,
            membrane: 0.0,
            membrane_q: 50.0,
        }
    }

    /// "void" preset - atmospheric, long
    pub fn void_preset() -> Self {
        Self {
            tune: 60.0,
            bend: 30.0,
            tone: 100.0,
            color: 50.0,
            decay: 90.0,
            membrane: 40.0,
            membrane_q: 80.0,
        }
    }

    /// Default preset
    pub fn default() -> Self {
        Self::derp()
    }
}

impl Blendable for Tom2Config {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let inv_t = 1.0 - t;
        Self {
            tune: self.tune * inv_t + other.tune * t,
            bend: self.bend * inv_t + other.bend * t,
            tone: self.tone * inv_t + other.tone * t,
            color: self.color * inv_t + other.color * t,
            decay: self.decay * inv_t + other.decay * t,
            membrane: self.membrane * inv_t + other.membrane * t,
            membrane_q: self.membrane_q * inv_t + other.membrane_q * t,
        }
    }
}

impl Tom2 {
    /// Create a new Tom2 with default Max patch settings
    pub fn new(sample_rate: f32) -> Self {
        // Create envelope matching Max patch: "1 1 0.8 0 2000 -0.83"
        // Segment 1: go to 1.0 in 1ms with curve 0.8
        // Segment 2: go to 0.0 in 2000ms with curve -0.83
        let envelope = MaxCurveEnvelope::new(vec![
            (1.0, 1.0, 0.8),      // Attack: value=1.0, time=1ms, curve=0.8
            (0.0, 2000.0, -0.83), // Decay: value=0.0, time=2000ms, curve=-0.83
        ]);

        let mut tom = Self {
            sample_rate,
            morph_osc: MorphOsc::new(sample_rate),
            click_osc: ClickOsc::new(),
            bandpass_filter: BiquadBandpass::new(sample_rate),
            envelope,
            is_active: false,
            trigger_time: 0.0,
            tri_phase: 0.0,
            past_attack: false,
            // Default parameter values
            tune: 50.0,   // 320 Hz (maps via zmap formula)
            bend: 30.0,   // Lower pitch envelope for testing
            tone: 50.0,   // Middle mix position
            color: 50.0,  // Middle rand~ rate AND filter cutoff (squared mapping)
            decay: 50.0,  // ~2000ms decay (maps via zmap 1 100 0.5 4000)
            triangle_enabled: true, // Standalone triangle on by default
            // Membrane resonator effect
            membrane_resonator: MembraneResonator::new(sample_rate),
            membrane: 0.0,      // Off by default
            membrane_q: 50.0,   // Middle Q scale
            main_sound_done: false,
        };
        tom.update_membrane_params();
        tom
    }

    /// Map tune parameter (0-100) to frequency (40-600 Hz)
    /// Max signal flow: tune → zmap 1 100 0 1 → pow 2 → zmap 0 1 40 600
    /// The tune value is SQUARED before mapping to frequency range
    #[inline]
    fn tune_to_freq(tune: f32) -> f32 {
        let normalized = tune / 100.0;
        let squared = normalized * normalized; // pow 2 like Max
        FREQ_MIN + squared * (FREQ_MAX - FREQ_MIN)
    }

    /// Map frequency (40-600 Hz) to tune parameter (0-100)
    /// Inverse of tune_to_freq: freq → normalized → sqrt → tune
    #[inline]
    fn freq_to_tune(freq: f32) -> f32 {
        let clamped = freq.clamp(FREQ_MIN, FREQ_MAX);
        let squared = (clamped - FREQ_MIN) / (FREQ_MAX - FREQ_MIN);
        squared.sqrt() * 100.0 // sqrt to invert the pow 2
    }

    /// Map decay parameter (0-100) to milliseconds (0.5-4000)
    /// Based on Max zmap: 1 100 0.5 4000
    #[inline]
    fn decay_to_ms(decay: f32) -> f32 {
        let normalized = decay / 100.0;
        DECAY_MIN_MS + normalized * (DECAY_MAX_MS - DECAY_MIN_MS)
    }

    /// Map milliseconds (0.5-4000) to decay parameter (0-100)
    #[inline]
    #[allow(dead_code)]
    fn ms_to_decay(ms: f32) -> f32 {
        let clamped = ms.clamp(DECAY_MIN_MS, DECAY_MAX_MS);
        ((clamped - DECAY_MIN_MS) / (DECAY_MAX_MS - DECAY_MIN_MS)) * 100.0
    }

    /// Set tune parameter (0-100): maps to 40-600 Hz
    pub fn set_tune(&mut self, tune: f32) {
        self.tune = tune.clamp(0.0, 100.0);
    }

    /// Get tune parameter (0-100)
    pub fn tune(&self) -> f32 {
        self.tune
    }

    /// Set the base frequency directly (40-600 Hz, clamped)
    pub fn set_frequency(&mut self, frequency: f32) {
        self.tune = Self::freq_to_tune(frequency);
    }

    /// Get the base frequency in Hz
    pub fn frequency(&self) -> f32 {
        Self::tune_to_freq(self.tune)
    }

    /// Set bend parameter (0-100): pitch envelope depth
    pub fn set_bend(&mut self, bend: f32) {
        self.bend = bend.clamp(0.0, 100.0);
    }

    /// Get bend parameter
    pub fn bend(&self) -> f32 {
        self.bend
    }

    /// Set tone parameter (0-100): mix control
    /// Note: at tone=100%, output is silent (noise channel disabled)
    pub fn set_tone(&mut self, tone: f32) {
        self.tone = tone.clamp(0.0, 100.0);
    }

    /// Get tone parameter
    pub fn tone(&self) -> f32 {
        self.tone
    }

    /// Set color parameter (0-100): controls rand~ rate
    /// Maps via zmap 1 200 30 50 to MIDI 30-50, then mtof for frequency
    pub fn set_color(&mut self, color: f32) {
        self.color = color.clamp(0.0, 100.0);
    }

    /// Get color parameter
    pub fn color(&self) -> f32 {
        self.color
    }

    /// Set decay parameter (0-100): maps to 0.5-4000ms
    pub fn set_decay(&mut self, decay: f32) {
        self.decay = decay.clamp(0.0, 100.0);
    }

    /// Get decay parameter (0-100)
    pub fn decay(&self) -> f32 {
        self.decay
    }

    /// Get the decay time in milliseconds
    pub fn decay_ms(&self) -> f32 {
        Self::decay_to_ms(self.decay)
    }

    /// Enable or disable the standalone triangle oscillator
    pub fn set_triangle_enabled(&mut self, enabled: bool) {
        self.triangle_enabled = enabled;
    }

    /// Check if triangle oscillator is enabled
    pub fn triangle_enabled(&self) -> bool {
        self.triangle_enabled
    }

    /// Set membrane mix amount (0-100)
    pub fn set_membrane(&mut self, membrane: f32) {
        self.membrane = membrane.clamp(0.0, 100.0);
    }

    /// Get membrane mix amount (0-100)
    pub fn membrane(&self) -> f32 {
        self.membrane
    }

    /// Set membrane Q scale (0-100): maps to 0.005-0.02 internally
    pub fn set_membrane_q(&mut self, membrane_q: f32) {
        self.membrane_q = membrane_q.clamp(0.0, 100.0);
        self.update_membrane_params();
    }

    /// Get membrane Q scale (0-100)
    pub fn membrane_q(&self) -> f32 {
        self.membrane_q
    }

    /// Update membrane resonator parameters based on current membrane_q setting
    fn update_membrane_params(&mut self) {
        // Map 0-100 to Q scale range 0.005-0.02 (centered at 0.01 which matches membrane example)
        let q_scale = 0.005 + (self.membrane_q / 100.0) * 0.015;
        self.membrane_resonator.set_q_scale(q_scale);
        // Higher gain scale for tom input (lower energy than noise)
        self.membrane_resonator.set_gain_scale(0.003);
    }

    /// Apply a config to this Tom2 instance
    pub fn set_config(&mut self, config: Tom2Config) {
        self.tune = config.tune;
        self.bend = config.bend;
        self.tone = config.tone;
        self.color = config.color;
        self.decay = config.decay;
        self.membrane = config.membrane;
        self.membrane_q = config.membrane_q;
        self.update_membrane_params();
    }

    /// Get current parameters as a config snapshot
    pub fn config(&self) -> Tom2Config {
        Tom2Config {
            tune: self.tune,
            bend: self.bend,
            tone: self.tone,
            color: self.color,
            decay: self.decay,
            membrane: self.membrane,
            membrane_q: self.membrane_q,
        }
    }
}

impl Instrument for Tom2 {
    fn trigger_with_velocity(&mut self, time: f32, _velocity: f32) {
        self.is_active = true;
        self.trigger_time = time;
        self.past_attack = false; // Reset attack phase tracking
        self.morph_osc.reset(); // Reset oscillator phases on trigger
        self.click_osc.trigger(); // Start click impulse playback
        self.tri_phase = 0.0; // Reset standalone triangle phase
        self.bandpass_filter.reset(); // Clear filter state

        // Reset membrane resonator state
        self.membrane_resonator.reset();
        self.main_sound_done = false;

        // Rebuild envelope with current decay value (mapped from 0-100 to ms)
        let decay_ms = Self::decay_to_ms(self.decay);
        self.envelope = MaxCurveEnvelope::new(vec![
            (1.0, 1.0, 0.8),          // Attack: value=1.0, time=1ms, curve=0.8
            (0.0, decay_ms, -0.83),   // Decay: value=0.0, time=decay ms, curve=-0.83
        ]);
        self.envelope.trigger(time);
    }

    fn tick(&mut self, current_time: f32) -> f32 {
        if !self.is_active {
            return 0.0;
        }

        // Get envelope value (0.0 to 1.0)
        let env_value = self.envelope.get_value(current_time);

        // Track when we've passed the attack phase (reached near-peak)
        if env_value > 0.9 {
            self.past_attack = true;
        }

        // Get base frequency from tune parameter
        let base_frequency = Self::tune_to_freq(self.tune);

        // Bend controls pitch envelope depth (how much pitch drops from peak to base)
        // bend=0: no pitch modulation, frequency stays at base_frequency
        // bend=100: maximum pitch modulation, frequency starts at 5× base and drops to base
        // Formula: frequency = base_frequency × (1 + (envelope × bend_scaled)²)
        // bend is 0-100, scaled to 0-2 range
        let bend_scaled = (self.bend / 100.0) * 2.0;
        let pitch_mod = (env_value * bend_scaled).powi(2);
        let raw_freq = base_frequency * (1.0 + pitch_mod);

        // Check if main tom sound should stop (envelope complete or pitch too low)
        // But DON'T deactivate yet - membrane may still be ringing
        let main_should_stop = self.envelope.is_complete()
            || (self.past_attack && raw_freq < MIN_AUDIBLE_FREQ);

        if main_should_stop {
            self.main_sound_done = true;
        }

        // If main sound done and membrane is quiet, fully deactivate
        if self.main_sound_done && !self.membrane_resonator.is_ringing() {
            self.is_active = false;
            return 0.0;
        }

        // Calculate fade factor for smooth cutoff (1.0 at 40Hz, 0.0 at 20Hz)
        let fade_factor = if self.past_attack && raw_freq < FADE_START_FREQ {
            (raw_freq - MIN_AUDIBLE_FREQ) / (FADE_START_FREQ - MIN_AUDIBLE_FREQ)
        } else {
            1.0
        };

        // Floor at FREQ_MIN (40Hz) for the oscillator
        let modulated_freq = raw_freq.max(FREQ_MIN);

        // === Path 1: Click oscillator ===
        // click~ output scaled by 1.1 (from Max patch)
        let click_output = self.click_osc.tick() * 1.1;

        // === Path 2: Standalone Triangle oscillator ===
        // tri~ at modulated frequency, scaled by 0.5 (matches Max's standalone tri~ outside morphosc)
        // Can be toggled off for A/B testing since it creates sub-bass at low frequencies
        let tri_output = if self.triangle_enabled {
            triangle(self.tri_phase) * 0.5
        } else {
            0.0
        };
        advance_phase(&mut self.tri_phase, modulated_freq, self.sample_rate);

        // === Path 3: MorphOsc ===
        // Map tone (0-100) to mix control (-1 to 1)
        let mix_control = (self.tone / 100.0) * 2.0 - 1.0;

        // Map color (0-100) to rand~ frequency via double mtof (matching Max patch)
        // Max chain: color → zmap 1 200 30 50 → mtof → sig~ → morphosc → mtof~ → rand~
        // First mtof: MIDI 30-50 → freq 46-147 Hz
        // Second mtof (in morphosc): treats freq as MIDI → 116-2794 Hz
        let color_midi = 30.0 + (self.color / 100.0) * 20.0;
        let color_freq_1 = 440.0 * 2.0_f32.powf((color_midi - 69.0) / 12.0); // First mtof

        // Generate morph oscillator output
        // Note: morph_osc applies second mtof internally to match Max's double-mtof chain
        let morph_output = self.morph_osc.tick(
            modulated_freq,
            mix_control,
            color_freq_1,
            self.tone,
        );

        // === Mixing (before filter, NO envelope yet!) ===
        // Max signal flow: click + tri + morphosc → biquad → *envelope → *0.5 → *1.4
        let mixed = click_output + tri_output + morph_output;

        // === Bandpass Filter ===
        // Filter frequency TRACKS THE PITCH (same formula as oscillator)
        // Max patch: curve~ × bend → pow~ 2 → *~ tune_freq → filtercoeff~
        // This centers the bandpass on the fundamental, attenuating noise
        let filter_freq = modulated_freq.max(20.0); // Same as oscillator pitch!

        // Q from color squared: zmap 0 1 1 2
        let color_norm = self.color / 100.0;
        let color_squared = color_norm * color_norm;
        let filter_q = 1.0 + color_squared;

        // Apply bandpass filter with gain 1.1 (from Max patch loadbang)
        self.bandpass_filter.set_params(filter_freq, filter_q, 1.1);
        let filtered = self.bandpass_filter.process(mixed);

        // === Membrane Resonator ===
        // Use the tom sound (pre-VCA) as input to excite the resonators
        // The resonators ring independently - no envelope on output so it sustains naturally
        let membrane_output = if self.membrane > 0.0 {
            // When main sound is done, feed zero input - filters ring out naturally
            let membrane_input = if self.main_sound_done {
                0.0
            } else {
                filtered * env_value
            };

            // Process through the membrane resonator effect
            self.membrane_resonator.process(membrane_input)
        } else {
            0.0
        };

        // If main sound is done, only output membrane ring (with fade for smooth ending)
        if self.main_sound_done {
            let membrane_mix = self.membrane / 100.0;
            let fade = self.membrane_resonator.fade_multiplier();
            return membrane_output * membrane_mix * fade * 0.7;
        }

        // Mix membrane with main signal
        // Dry signal has VCA envelope, membrane output rings freely (no additional envelope)
        let membrane_mix = self.membrane / 100.0;
        let dry_gain = 1.0 - membrane_mix;
        let wet_gain = membrane_mix;
        let dry_output = filtered * env_value;  // VCA envelope on dry
        let final_signal = dry_output * dry_gain + membrane_output * wet_gain;  // membrane rings independently

        // === Output gain stages ===
        // Combined gain: 0.5 * 1.4 = 0.7
        final_signal * fade_factor * 0.7
    }

    fn is_active(&self) -> bool {
        self.is_active
    }

    fn as_modulatable(&mut self) -> Option<&mut dyn crate::engine::Modulatable> {
        None
    }
}
