use crate::engine::Instrument;
use crate::envelope::{ADSRConfig, Envelope, EnvelopeCurve};
use crate::filters::StateVariableFilterTpt;
use crate::gen::polyblep::{polyblep_saw, polyblep_square};
use crate::music::note::midi_to_freq;
use crate::utils::SmoothedParam;

mod ranges {
    pub fn filter_cutoff_hz(normalized: f32) -> f32 {
        // Exponential mapping: 0.0 = 20 Hz, 1.0 = 18000 Hz
        20.0 * (18000.0_f32 / 20.0).powf(normalized)
    }

    pub fn filter_resonance_q(normalized: f32) -> f32 {
        // 0.0 = 0.5 (gentle), 1.0 = 15.0 (screaming)
        0.5 + normalized * 14.5
    }

    pub fn env_time(normalized: f32) -> f32 {
        // Exponential mapping: 0.0 = 0.001s, 1.0 = 5.0s
        0.001 * (5000.0_f32).powf(normalized)
    }

    pub fn detune_ratio(normalized: f32) -> f64 {
        // 0.0 = no detune, 1.0 = ~30 cents
        1.0 + normalized as f64 * 0.0175
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PolySynthConfig {
    pub osc_shape: f32,
    pub detune_amount: f32,
    pub filter_cutoff: f32,
    pub filter_resonance: f32,
    pub filter_env_amount: f32,
    pub amp_attack: f32,
    pub amp_decay: f32,
    pub amp_sustain: f32,
    pub amp_release: f32,
    pub filter_attack: f32,
    pub filter_decay: f32,
    pub filter_sustain: f32,
    pub filter_release: f32,
    pub volume: f32,
}

impl PolySynthConfig {
    pub fn default() -> Self {
        Self {
            osc_shape: 0.0,
            detune_amount: 0.2,
            filter_cutoff: 0.6,
            filter_resonance: 0.15,
            filter_env_amount: 0.3,
            amp_attack: 0.55, // ~100ms
            amp_decay: 0.7,   // ~370ms
            amp_sustain: 0.7,
            amp_release: 0.8, // ~890ms
            filter_attack: 0.5,
            filter_decay: 0.65,
            filter_sustain: 0.4,
            filter_release: 0.75,
            volume: 0.7,
        }
    }

    pub fn pad() -> Self {
        Self {
            osc_shape: 0.0,
            detune_amount: 0.4,
            filter_cutoff: 0.45,
            filter_resonance: 0.2,
            filter_env_amount: 0.2,
            amp_attack: 0.8, // ~890ms
            amp_decay: 0.75, // ~530ms
            amp_sustain: 0.8,
            amp_release: 0.85, // ~1.5s
            filter_attack: 0.75,
            filter_decay: 0.7,
            filter_sustain: 0.5,
            filter_release: 0.8,
            volume: 0.6,
        }
    }

    pub fn pluck() -> Self {
        Self {
            osc_shape: 0.3,
            detune_amount: 0.1,
            filter_cutoff: 0.7,
            filter_resonance: 0.25,
            filter_env_amount: 0.6,
            amp_attack: 0.0,
            amp_decay: 0.75, // ~530ms
            amp_sustain: 0.0,
            amp_release: 0.65, // ~280ms
            filter_attack: 0.0,
            filter_decay: 0.7,
            filter_sustain: 0.1,
            filter_release: 0.65,
            volume: 0.7,
        }
    }

    pub fn keys() -> Self {
        Self {
            osc_shape: 0.5,
            detune_amount: 0.15,
            filter_cutoff: 0.55,
            filter_resonance: 0.1,
            filter_env_amount: 0.4,
            amp_attack: 0.35, // ~20ms
            amp_decay: 0.7,   // ~370ms
            amp_sustain: 0.5,
            amp_release: 0.75, // ~530ms
            filter_attack: 0.3,
            filter_decay: 0.65,
            filter_sustain: 0.3,
            filter_release: 0.7,
            volume: 0.7,
        }
    }

    pub fn strings() -> Self {
        Self {
            osc_shape: 0.0,
            detune_amount: 0.5,
            filter_cutoff: 0.5,
            filter_resonance: 0.1,
            filter_env_amount: 0.15,
            amp_attack: 0.85, // ~1.5s
            amp_decay: 0.7,   // ~370ms
            amp_sustain: 0.9,
            amp_release: 0.85, // ~1.5s
            filter_attack: 0.8,
            filter_decay: 0.7,
            filter_sustain: 0.6,
            filter_release: 0.8,
            volume: 0.5,
        }
    }
}

pub struct PolySynthParams {
    pub osc_shape: SmoothedParam,
    pub detune_amount: SmoothedParam,
    pub filter_cutoff: SmoothedParam,
    pub filter_resonance: SmoothedParam,
    pub filter_env_amount: SmoothedParam,
    pub amp_attack: SmoothedParam,
    pub amp_decay: SmoothedParam,
    pub amp_sustain: SmoothedParam,
    pub amp_release: SmoothedParam,
    pub filter_attack: SmoothedParam,
    pub filter_decay: SmoothedParam,
    pub filter_sustain: SmoothedParam,
    pub filter_release: SmoothedParam,
    pub volume: SmoothedParam,
}

impl PolySynthParams {
    pub fn from_config(config: &PolySynthConfig, sample_rate: f32) -> Self {
        Self {
            osc_shape: SmoothedParam::new_normalized(config.osc_shape, sample_rate),
            detune_amount: SmoothedParam::new_normalized(config.detune_amount, sample_rate),
            filter_cutoff: SmoothedParam::new_normalized(config.filter_cutoff, sample_rate),
            filter_resonance: SmoothedParam::new_normalized(config.filter_resonance, sample_rate),
            filter_env_amount: SmoothedParam::new_normalized(config.filter_env_amount, sample_rate),
            amp_attack: SmoothedParam::new_normalized(config.amp_attack, sample_rate),
            amp_decay: SmoothedParam::new_normalized(config.amp_decay, sample_rate),
            amp_sustain: SmoothedParam::new_normalized(config.amp_sustain, sample_rate),
            amp_release: SmoothedParam::new_normalized(config.amp_release, sample_rate),
            filter_attack: SmoothedParam::new_normalized(config.filter_attack, sample_rate),
            filter_decay: SmoothedParam::new_normalized(config.filter_decay, sample_rate),
            filter_sustain: SmoothedParam::new_normalized(config.filter_sustain, sample_rate),
            filter_release: SmoothedParam::new_normalized(config.filter_release, sample_rate),
            volume: SmoothedParam::new_normalized(config.volume, sample_rate),
        }
    }

    pub fn tick(&mut self) {
        self.osc_shape.tick();
        self.detune_amount.tick();
        self.filter_cutoff.tick();
        self.filter_resonance.tick();
        self.filter_env_amount.tick();
        self.amp_attack.tick();
        self.amp_decay.tick();
        self.amp_sustain.tick();
        self.amp_release.tick();
        self.filter_attack.tick();
        self.filter_decay.tick();
        self.filter_sustain.tick();
        self.filter_release.tick();
        self.volume.tick();
    }

    pub fn snap_all(&mut self) {
        self.osc_shape.snap();
        self.detune_amount.snap();
        self.filter_cutoff.snap();
        self.filter_resonance.snap();
        self.filter_cutoff.snap();
        self.filter_resonance.snap();
        self.filter_env_amount.snap();
        self.amp_attack.snap();
        self.amp_decay.snap();
        self.amp_sustain.snap();
        self.amp_release.snap();
        self.filter_attack.snap();
        self.filter_decay.snap();
        self.filter_sustain.snap();
        self.filter_release.snap();
        self.volume.snap();
    }
}

const NUM_VOICES: usize = 6;

struct Voice {
    midi_note: u8,
    frequency: f64,
    phase_a: f64,
    phase_b: f64,
    amp_envelope: Envelope,
    filter_envelope: Envelope,
    filter: StateVariableFilterTpt,
    velocity: f32,
    active: bool,
    trigger_order: u64,
}

impl Voice {
    fn new(sample_rate: f32) -> Self {
        Self {
            midi_note: 0,
            frequency: 440.0,
            phase_a: 0.0,
            phase_b: 0.0,
            amp_envelope: Envelope::new(),
            filter_envelope: Envelope::new(),
            filter: StateVariableFilterTpt::new(sample_rate, 1000.0, 1.0),
            velocity: 1.0,
            active: false,
            trigger_order: 0,
        }
    }
}

pub struct PolySynth {
    sample_rate: f32,
    pub params: PolySynthParams,
    voices: Vec<Voice>,
    trigger_counter: u64,
    pending_note: Option<u8>,
    /// Tracks the latest audio clock time from tick() so that
    /// trigger_note/release_note called from the UI thread can
    /// use the real audio time instead of a stale value.
    current_time: f64,
}

impl PolySynth {
    pub fn new(sample_rate: f32) -> Self {
        Self::with_config(sample_rate, PolySynthConfig::default())
    }

    pub fn with_config(sample_rate: f32, config: PolySynthConfig) -> Self {
        let params = PolySynthParams::from_config(&config, sample_rate);
        let voices = (0..NUM_VOICES).map(|_| Voice::new(sample_rate)).collect();

        Self {
            sample_rate,
            params,
            voices,
            trigger_counter: 0,
            pending_note: None,
            current_time: 0.0,
        }
    }

    pub fn set_config(&mut self, config: PolySynthConfig) {
        self.params.osc_shape.set_target(config.osc_shape);
        self.params.detune_amount.set_target(config.detune_amount);
        self.params.filter_cutoff.set_target(config.filter_cutoff);
        self.params
            .filter_resonance
            .set_target(config.filter_resonance);
        self.params
            .filter_env_amount
            .set_target(config.filter_env_amount);
        self.params.amp_attack.set_target(config.amp_attack);
        self.params.amp_decay.set_target(config.amp_decay);
        self.params.amp_sustain.set_target(config.amp_sustain);
        self.params.amp_release.set_target(config.amp_release);
        self.params.filter_attack.set_target(config.filter_attack);
        self.params.filter_decay.set_target(config.filter_decay);
        self.params.filter_sustain.set_target(config.filter_sustain);
        self.params.filter_release.set_target(config.filter_release);
        self.params.volume.set_target(config.volume);
    }

    pub fn snap_params(&mut self) {
        self.params.snap_all();
    }

    /// Trigger a specific MIDI note with velocity.
    /// Uses the current audio clock time tracked from tick().
    pub fn trigger_note(&mut self, note: u8, velocity: f32) {
        let time = self.current_time;
        let voice_idx = self.allocate_voice();
        let voice = &mut self.voices[voice_idx];

        voice.midi_note = note;
        voice.frequency = midi_to_freq(note);
        voice.phase_a = 0.0;
        voice.phase_b = 0.0;
        voice.velocity = velocity;
        voice.active = true;
        voice.trigger_order = self.trigger_counter;
        self.trigger_counter += 1;

        // Configure amp envelope
        let amp_config = ADSRConfig::new(
            ranges::env_time(self.params.amp_attack.get()),
            ranges::env_time(self.params.amp_decay.get()),
            self.params.amp_sustain.get(),
            ranges::env_time(self.params.amp_release.get()),
        )
        .with_decay_curve(EnvelopeCurve::Exponential(0.5));

        voice.amp_envelope.set_config(amp_config);
        voice.amp_envelope.trigger(time);

        // Configure filter envelope
        let filt_config = ADSRConfig::new(
            ranges::env_time(self.params.filter_attack.get()),
            ranges::env_time(self.params.filter_decay.get()),
            self.params.filter_sustain.get(),
            ranges::env_time(self.params.filter_release.get()),
        )
        .with_decay_curve(EnvelopeCurve::Exponential(0.5));

        voice.filter_envelope.set_config(filt_config);
        voice.filter_envelope.trigger(time);

        // Reset filter state
        voice.filter.reset();
    }

    /// Release a specific MIDI note
    pub fn release_note(&mut self, note: u8) {
        let time = self.current_time;
        for voice in &mut self.voices {
            if voice.active
                && voice.midi_note == note
                && voice.amp_envelope.release_time_start.is_none()
            {
                voice.amp_envelope.release(time);
                voice.filter_envelope.release(time);
            }
        }
    }

    /// Release all active voices
    pub fn release_all(&mut self) {
        let time = self.current_time;
        for voice in &mut self.voices {
            if voice.active {
                voice.amp_envelope.release(time);
                voice.filter_envelope.release(time);
            }
        }
    }

    // Individual parameter setters (normalized 0-1)

    pub fn set_osc_shape(&mut self, value: f32) {
        self.params.osc_shape.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_detune_amount(&mut self, value: f32) {
        self.params.detune_amount.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_filter_cutoff(&mut self, value: f32) {
        self.params.filter_cutoff.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_filter_resonance(&mut self, value: f32) {
        self.params
            .filter_resonance
            .set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_filter_env_amount(&mut self, value: f32) {
        self.params
            .filter_env_amount
            .set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_amp_attack(&mut self, value: f32) {
        self.params.amp_attack.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_amp_decay(&mut self, value: f32) {
        self.params.amp_decay.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_amp_sustain(&mut self, value: f32) {
        self.params.amp_sustain.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_amp_release(&mut self, value: f32) {
        self.params.amp_release.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_volume(&mut self, value: f32) {
        self.params.volume.set_target(value.clamp(0.0, 1.0));
    }

    fn allocate_voice(&self) -> usize {
        // Prefer an inactive voice
        if let Some(idx) = self.voices.iter().position(|v| !v.active) {
            return idx;
        }

        // Steal the oldest active voice
        self.voices
            .iter()
            .enumerate()
            .min_by_key(|(_, v)| v.trigger_order)
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    fn generate_voice(&mut self, voice_idx: usize, current_time: f64) -> f32 {
        let osc_shape = self.params.osc_shape.get();
        let detune = self.params.detune_amount.get();
        let cutoff_norm = self.params.filter_cutoff.get();
        let resonance_norm = self.params.filter_resonance.get();
        let filter_env_amount = self.params.filter_env_amount.get();
        let volume = self.params.volume.get();

        let voice = &mut self.voices[voice_idx];

        if !voice.active {
            return 0.0;
        }

        // Amp envelope
        let amp_env = voice.amp_envelope.get_amplitude(current_time);
        if !voice.amp_envelope.is_active {
            voice.active = false;
            return 0.0;
        }

        // Filter envelope
        let filter_env = voice.filter_envelope.get_amplitude(current_time);

        // Oscillators
        let freq = voice.frequency;
        let detune_ratio = ranges::detune_ratio(detune);
        let dt = 1.0 / self.sample_rate as f64;

        let phase_inc_a = freq * dt;
        let phase_inc_b = freq * detune_ratio * dt;

        // Generate oscillator A
        let saw_a = polyblep_saw(voice.phase_a, phase_inc_a);
        let square_a = polyblep_square(voice.phase_a, phase_inc_a);
        let osc_a = saw_a * (1.0 - osc_shape) + square_a * osc_shape;

        // Generate oscillator B (detuned)
        let saw_b = polyblep_saw(voice.phase_b, phase_inc_b);
        let square_b = polyblep_square(voice.phase_b, phase_inc_b);
        let osc_b = saw_b * (1.0 - osc_shape) + square_b * osc_shape;

        // Mix oscillators
        let osc_mix = (osc_a + osc_b) * 0.5;

        // Advance phases
        voice.phase_a += phase_inc_a;
        voice.phase_a -= voice.phase_a.floor();
        voice.phase_b += phase_inc_b;
        voice.phase_b -= voice.phase_b.floor();

        // Filter with envelope modulation
        let base_cutoff = ranges::filter_cutoff_hz(cutoff_norm);
        let max_cutoff = 18000.0_f32;
        let modulated_cutoff =
            base_cutoff + filter_env_amount * filter_env * (max_cutoff - base_cutoff);
        let q = ranges::filter_resonance_q(resonance_norm);

        voice
            .filter
            .set_params(modulated_cutoff.clamp(20.0, 18000.0), q);
        let (filtered, _, _) = voice.filter.process_all(osc_mix);

        // Apply amplitude envelope and velocity
        filtered * amp_env * voice.velocity.sqrt() * volume
    }
}

impl Instrument for PolySynth {
    fn trigger_with_velocity(&mut self, _time: f64, velocity: f32) {
        let note = self.pending_note.unwrap_or(60);
        self.trigger_note(note, velocity);
        self.pending_note = None;
    }

    fn tick(&mut self, current_time: f64) -> f32 {
        self.current_time = current_time;
        self.params.tick();

        let mut output = 0.0;
        for i in 0..NUM_VOICES {
            output += self.generate_voice(i, current_time);
        }

        // Scale by inverse of voice count to prevent clipping with chords
        let active = self.voices.iter().filter(|v| v.active).count().max(1) as f32;
        output / active
    }

    fn is_active(&self) -> bool {
        self.voices.iter().any(|v| v.active)
    }

    fn set_midi_note(&mut self, note: u8) {
        self.pending_note = Some(note);
    }

    fn get_frequency(&self) -> Option<f32> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_poly_synth_creation() {
        let synth = PolySynth::new(44100.0);
        assert!(!synth.is_active());
    }

    #[test]
    fn test_poly_synth_trigger() {
        let mut synth = PolySynth::new(44100.0);
        synth.trigger_note(60, 1.0);
        assert!(synth.is_active());
    }

    #[test]
    fn test_poly_synth_produces_audio() {
        let mut synth = PolySynth::new(44100.0);
        synth.trigger_note(60, 1.0);

        let mut energy = 0.0_f64;
        let dt = 1.0 / 44100.0;
        for i in 0..4410 {
            let t = i as f64 * dt;
            let s = synth.tick(t) as f64;
            energy += s * s;
        }
        assert!(energy > 0.1, "synth should produce audible output");
    }

    #[test]
    fn test_poly_synth_six_voices() {
        let mut synth = PolySynth::new(44100.0);
        for note in 60..66 {
            synth.trigger_note(note, 1.0);
        }

        let active_count = synth.voices.iter().filter(|v| v.active).count();
        assert_eq!(active_count, 6);
    }

    #[test]
    fn test_poly_synth_voice_stealing() {
        let mut synth = PolySynth::new(44100.0);
        // Fill all 6 voices
        for note in 60..66 {
            synth.trigger_note(note, 1.0);
        }
        // Trigger a 7th - should steal oldest
        synth.trigger_note(66, 1.0);

        let active_count = synth.voices.iter().filter(|v| v.active).count();
        assert_eq!(active_count, 6);
        assert!(synth.voices.iter().any(|v| v.midi_note == 66));
    }

    #[test]
    fn test_poly_synth_release() {
        let mut synth = PolySynth::new(44100.0);
        synth.trigger_note(60, 1.0);
        synth.release_note(60);

        // Advance time past release
        let dt = 1.0 / 44100.0;
        for i in 0..441000 {
            let t = i as f64 * dt;
            synth.tick(t);
        }
        assert!(!synth.is_active());
    }

    #[test]
    fn test_presets() {
        let sample_rate = 44100.0;
        let _pad = PolySynth::with_config(sample_rate, PolySynthConfig::pad());
        let _pluck = PolySynth::with_config(sample_rate, PolySynthConfig::pluck());
        let _keys = PolySynth::with_config(sample_rate, PolySynthConfig::keys());
        let _strings = PolySynth::with_config(sample_rate, PolySynthConfig::strings());
    }
}
