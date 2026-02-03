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
use crate::gen::{ClickOsc, MorphOsc};
use crate::max_curve::MaxCurveEnvelope;

/// Frequency range constants (from Max zmap 0 1 40 600)
const FREQ_MIN: f32 = 40.0;
const FREQ_MAX: f32 = 600.0;

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

/// Decay range constants (from Max zmap 1 100 0.5 4000)
const DECAY_MIN_MS: f32 = 0.5;
const DECAY_MAX_MS: f32 = 4000.0;

/// Tom drum with morph oscillator for A/B testing with Max/MSP reference
pub struct Tom2 {
    sample_rate: f32,
    morph_osc: MorphOsc,
    click_osc: ClickOsc,
    envelope: MaxCurveEnvelope,
    is_active: bool,
    #[allow(dead_code)]
    trigger_time: f32,

    // Standalone triangle oscillator phase (separate from MorphOsc's internal tri)
    tri_phase: f32,

    // Parameters (matching Max patch ranges)
    tune: f32,  // 0-100: maps to 40-600 Hz
    bend: f32,  // 0-100: pitch envelope depth
    tone: f32,  // 0-100: mix control (100% = silent, noise channel disabled)
    color: f32, // 0-100: double-mtof chain gives rand~ rate 116-2794 Hz
    decay: f32, // 0-100: maps to 0.5-4000ms via zmap
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

        Self {
            sample_rate,
            morph_osc: MorphOsc::new(sample_rate),
            click_osc: ClickOsc::new(),
            envelope,
            is_active: false,
            trigger_time: 0.0,
            tri_phase: 0.0,
            // Default parameter values
            tune: 51.25,  // ~327 Hz (maps to 327 via zmap formula)
            bend: 100.0,  // Full pitch envelope by default
            tone: 50.0,   // Middle mix position
            color: 50.0,  // Middle rand~ rate (maps to MIDI 40 → ~82 Hz)
            decay: 50.0,  // ~2000ms decay (maps via zmap 1 100 0.5 4000)
        }
    }

    /// Map tune parameter (0-100) to frequency (40-600 Hz)
    /// Based on Max zmap: 0 1 40 600
    #[inline]
    fn tune_to_freq(tune: f32) -> f32 {
        let normalized = tune / 100.0;
        FREQ_MIN + normalized * (FREQ_MAX - FREQ_MIN)
    }

    /// Map frequency (40-600 Hz) to tune parameter (0-100)
    #[inline]
    fn freq_to_tune(freq: f32) -> f32 {
        let clamped = freq.clamp(FREQ_MIN, FREQ_MAX);
        ((clamped - FREQ_MIN) / (FREQ_MAX - FREQ_MIN)) * 100.0
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
}

impl Instrument for Tom2 {
    fn trigger_with_velocity(&mut self, time: f32, _velocity: f32) {
        self.is_active = true;
        self.trigger_time = time;
        self.morph_osc.reset(); // Reset oscillator phases on trigger
        self.click_osc.trigger(); // Start click impulse playback
        self.tri_phase = 0.0; // Reset triangle phase

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

        // Check if envelope is complete
        if self.envelope.is_complete() {
            self.is_active = false;
            return 0.0;
        }

        // Get base frequency from tune parameter
        let base_frequency = Self::tune_to_freq(self.tune);

        // Bend controls pitch envelope depth
        // Max signal flow: curve~ → *~ bend → pow~ 2 → *~ tune_freq
        // frequency = (envelope × bend)² × tune_frequency
        // bend is 0-100, Max uses zmap 1 127 0 2, so scale to 0-2 range
        let bend_scaled = (self.bend / 100.0) * 2.0;
        let pitch_env = (env_value * bend_scaled).powi(2); // SQUARED like Max
        let modulated_freq = base_frequency * pitch_env;

        // === Path 1: Click oscillator ===
        // click~ output scaled by 1.1 (from Max patch)
        let click_output = self.click_osc.tick() * 1.1;

        // === Path 2: Standalone Triangle oscillator ===
        // tri~ at modulated frequency, scaled by 0.5
        let tri_output = triangle(self.tri_phase) * 0.5;
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

        // === Mixing ===
        // All three paths are summed, then go through biquad filter (to be added)
        // For now: sum all paths with envelope applied to morph
        let morph_vca = morph_output * env_value;

        // Sum all three oscillator outputs
        // TODO: Route through biquad bandpass filter before output
        click_output + tri_output + morph_vca
    }

    fn is_active(&self) -> bool {
        self.is_active
    }

    fn as_modulatable(&mut self) -> Option<&mut dyn crate::engine::Modulatable> {
        None
    }
}
