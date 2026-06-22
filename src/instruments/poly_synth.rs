use crate::effects::waveshaper::Waveshaper;
use crate::engine::Instrument;
use crate::envelope::{ADSRConfig, Envelope, EnvelopeCurve};
use crate::filters::StateVariableFilterTpt;
use crate::gen::polyblep::{polyblep_saw, polyblep_square};
use crate::music::note::midi_to_freq;
use crate::utils::{tuning_to_multiplier, Blendable, SmoothedParam, DEFAULT_SMOOTH_TIME_MS};
use std::f64::consts::TAU;

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
}

#[derive(Clone, Copy, Debug)]
pub struct PolySynthConfig {
    pub osc_shape: f32,
    pub sub_level: f32,
    pub osc_level: f32,
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
    pub overdrive: f32,
    pub volume: f32,
    pub tuning: f32,
}

impl PolySynthConfig {
    pub fn default() -> Self {
        Self {
            osc_shape: 0.0,
            sub_level: 0.3,
            osc_level: 0.7,
            filter_cutoff: 0.6,
            filter_resonance: 0.15,
            filter_env_amount: 0.3,
            amp_attack: 0.55,
            amp_decay: 0.7,
            amp_sustain: 0.7,
            amp_release: 0.8,
            filter_attack: 0.5,
            filter_decay: 0.65,
            filter_sustain: 0.4,
            filter_release: 0.75,
            overdrive: 0.0,
            volume: 0.7,
            tuning: 0.5,
        }
    }

    pub fn pad() -> Self {
        Self {
            osc_shape: 0.0,
            sub_level: 0.4,
            osc_level: 0.6,
            filter_cutoff: 0.45,
            filter_resonance: 0.2,
            filter_env_amount: 0.2,
            amp_attack: 0.8,
            amp_decay: 0.75,
            amp_sustain: 0.8,
            amp_release: 0.85,
            filter_attack: 0.75,
            filter_decay: 0.7,
            filter_sustain: 0.5,
            filter_release: 0.8,
            overdrive: 0.0,
            volume: 0.6,
            tuning: 0.5,
        }
    }

    pub fn pluck() -> Self {
        Self {
            osc_shape: 0.3,
            sub_level: 0.1,
            osc_level: 0.8,
            filter_cutoff: 0.7,
            filter_resonance: 0.25,
            filter_env_amount: 0.6,
            amp_attack: 0.0,
            amp_decay: 0.75,
            amp_sustain: 0.0,
            amp_release: 0.65,
            filter_attack: 0.0,
            filter_decay: 0.7,
            filter_sustain: 0.1,
            filter_release: 0.65,
            overdrive: 0.0,
            volume: 0.7,
            tuning: 0.5,
        }
    }

    pub fn keys() -> Self {
        Self {
            osc_shape: 0.5,
            sub_level: 0.25,
            osc_level: 0.8,
            filter_cutoff: 0.55,
            filter_resonance: 0.1,
            filter_env_amount: 0.4,
            amp_attack: 0.35,
            amp_decay: 0.7,
            amp_sustain: 0.5,
            amp_release: 0.75,
            filter_attack: 0.3,
            filter_decay: 0.65,
            filter_sustain: 0.3,
            filter_release: 0.7,
            overdrive: 0.0,
            volume: 0.7,
            tuning: 0.5,
        }
    }

    pub fn strings() -> Self {
        Self {
            osc_shape: 0.0,
            sub_level: 0.15,
            osc_level: 0.7,
            filter_cutoff: 0.5,
            filter_resonance: 0.1,
            filter_env_amount: 0.15,
            amp_attack: 0.85,
            amp_decay: 0.7,
            amp_sustain: 0.9,
            amp_release: 0.85,
            filter_attack: 0.8,
            filter_decay: 0.7,
            filter_sustain: 0.6,
            filter_release: 0.8,
            overdrive: 0.0,
            volume: 0.5,
            tuning: 0.5,
        }
    }
}

impl Blendable for PolySynthConfig {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let inv = 1.0 - t;
        Self {
            osc_shape: self.osc_shape * inv + other.osc_shape * t,
            sub_level: self.sub_level * inv + other.sub_level * t,
            osc_level: self.osc_level * inv + other.osc_level * t,
            filter_cutoff: self.filter_cutoff * inv + other.filter_cutoff * t,
            filter_resonance: self.filter_resonance * inv + other.filter_resonance * t,
            filter_env_amount: self.filter_env_amount * inv + other.filter_env_amount * t,
            amp_attack: self.amp_attack * inv + other.amp_attack * t,
            amp_decay: self.amp_decay * inv + other.amp_decay * t,
            amp_sustain: self.amp_sustain * inv + other.amp_sustain * t,
            amp_release: self.amp_release * inv + other.amp_release * t,
            filter_attack: self.filter_attack * inv + other.filter_attack * t,
            filter_decay: self.filter_decay * inv + other.filter_decay * t,
            filter_sustain: self.filter_sustain * inv + other.filter_sustain * t,
            filter_release: self.filter_release * inv + other.filter_release * t,
            overdrive: self.overdrive * inv + other.overdrive * t,
            volume: self.volume * inv + other.volume * t,
            tuning: self.tuning * inv + other.tuning * t,
        }
    }
}

pub struct PolySynthParams {
    pub osc_shape: SmoothedParam,
    pub sub_level: SmoothedParam,
    pub osc_level: SmoothedParam,
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
    pub overdrive: SmoothedParam,
    pub volume: SmoothedParam,
    pub tuning: SmoothedParam,
}

impl PolySynthParams {
    pub fn from_config(config: &PolySynthConfig, sample_rate: f32) -> Self {
        let make = |v: f32| SmoothedParam::new(v, 0.0, 1.0, sample_rate, DEFAULT_SMOOTH_TIME_MS);
        Self {
            osc_shape: make(config.osc_shape),
            sub_level: make(config.sub_level),
            osc_level: make(config.osc_level),
            filter_cutoff: make(config.filter_cutoff),
            filter_resonance: make(config.filter_resonance),
            filter_env_amount: make(config.filter_env_amount),
            amp_attack: make(config.amp_attack),
            amp_decay: make(config.amp_decay),
            amp_sustain: make(config.amp_sustain),
            amp_release: make(config.amp_release),
            filter_attack: make(config.filter_attack),
            filter_decay: make(config.filter_decay),
            filter_sustain: make(config.filter_sustain),
            filter_release: make(config.filter_release),
            overdrive: make(config.overdrive),
            volume: make(config.volume),
            tuning: make(config.tuning),
        }
    }

    pub fn tick(&mut self) {
        self.osc_shape.tick();
        self.sub_level.tick();
        self.osc_level.tick();
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
        self.overdrive.tick();
        self.volume.tick();
        self.tuning.tick();
    }

    pub fn snap_all(&mut self) {
        self.osc_shape.snap();
        self.sub_level.snap();
        self.osc_level.snap();
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
        self.overdrive.snap();
        self.volume.snap();
        self.tuning.snap();
    }
}

const NUM_VOICES: usize = 6;

struct Voice {
    midi_note: u8,
    frequency: f64,
    sub_phase: f64,
    phase_a: f64,
    amp_envelope: Envelope,
    filter_envelope: Envelope,
    filter: StateVariableFilterTpt,
    waveshaper: Waveshaper,
    velocity: f32,
    active: bool,
    trigger_order: u64,
}

impl Voice {
    fn new(sample_rate: f32) -> Self {
        Self {
            midi_note: 0,
            frequency: 440.0,
            sub_phase: 0.0,
            phase_a: 0.0,
            amp_envelope: Envelope::new(),
            filter_envelope: Envelope::new(),
            filter: StateVariableFilterTpt::new(sample_rate, 1000.0, 1.0),
            waveshaper: Waveshaper::new(1.0, 1.0),
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
        self.params.sub_level.set_target(config.sub_level);
        self.params.osc_level.set_target(config.osc_level);
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
        self.params.overdrive.set_target(config.overdrive);
        self.params.volume.set_target(config.volume);
        self.params.tuning.set_target(config.tuning);
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
        voice.sub_phase = 0.0;
        voice.phase_a = 0.0;
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

    pub fn set_sub_level(&mut self, value: f32) {
        self.params.sub_level.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_osc_level(&mut self, value: f32) {
        self.params.osc_level.set_target(value.clamp(0.0, 1.0));
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

    pub fn set_filter_attack(&mut self, value: f32) {
        self.params.filter_attack.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_filter_decay(&mut self, value: f32) {
        self.params.filter_decay.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_filter_sustain(&mut self, value: f32) {
        self.params.filter_sustain.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_filter_release(&mut self, value: f32) {
        self.params.filter_release.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_overdrive(&mut self, value: f32) {
        self.params.overdrive.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_volume(&mut self, value: f32) {
        self.params.volume.set_target(value.clamp(0.0, 1.0));
    }

    /// Set tuning offset (smoothed, 0-1: 0=-12 semitones, 0.5=neutral, 1=+12 semitones).
    /// Applied live to sustaining voices, so sweeping produces pitch glide.
    pub fn set_tuning(&mut self, value: f32) {
        self.params.tuning.set_target(value.clamp(0.0, 1.0));
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
        let sub_level = self.params.sub_level.get();
        let osc_level = self.params.osc_level.get();
        let cutoff_norm = self.params.filter_cutoff.get();
        let resonance_norm = self.params.filter_resonance.get();
        let filter_env_amount = self.params.filter_env_amount.get();
        let overdrive = self.params.overdrive.get();
        let volume = self.params.volume.get();
        let tuning_mult = tuning_to_multiplier(self.params.tuning.get()) as f64;

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
        let freq = voice.frequency * tuning_mult;
        let dt = 1.0 / self.sample_rate as f64;

        let phase_inc_main = freq * dt;

        // Sub sine (pure fundamental at the note's frequency)
        let sub_out = (voice.sub_phase * TAU).sin() as f32;

        // Generate oscillator A (main)
        let saw_a = polyblep_saw(voice.phase_a, phase_inc_main);
        let square_a = polyblep_square(voice.phase_a, phase_inc_main);
        let osc_a = saw_a * (1.0 - osc_shape) + square_a * osc_shape;

        // Mix oscillator layers with independent levels
        let mix = sub_out * sub_level + osc_a * osc_level;

        // Advance phases
        voice.sub_phase += phase_inc_main;
        voice.sub_phase -= voice.sub_phase.floor();
        voice.phase_a += phase_inc_main;
        voice.phase_a -= voice.phase_a.floor();

        // Pre-filter saturation
        voice.waveshaper.set_drive(1.0 + overdrive * 9.0);
        let saturated = if overdrive > 0.001 {
            voice.waveshaper.process(mix)
        } else {
            mix
        };

        // Filter with envelope modulation
        let base_cutoff = ranges::filter_cutoff_hz(cutoff_norm);
        let max_cutoff = 18000.0_f32;
        let modulated_cutoff =
            base_cutoff + filter_env_amount * filter_env * (max_cutoff - base_cutoff);
        let q = ranges::filter_resonance_q(resonance_norm);

        voice
            .filter
            .set_params(modulated_cutoff.clamp(20.0, 18000.0), q);
        let (filtered, _, _) = voice.filter.process_all(saturated);

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

    fn as_modulatable(&mut self) -> Option<&mut dyn crate::engine::Modulatable> {
        Some(self)
    }
}

impl crate::engine::Modulatable for PolySynth {
    fn modulatable_parameters(&self) -> Vec<&'static str> {
        vec![
            "osc_shape",
            "sub_level",
            "osc_level",
            "filter_cutoff",
            "filter_resonance",
            "filter_env_amount",
            "amp_attack",
            "amp_decay",
            "amp_sustain",
            "amp_release",
            "filter_attack",
            "filter_decay",
            "filter_sustain",
            "filter_release",
            "overdrive",
            "volume",
            "tuning",
        ]
    }

    fn apply_modulation(&mut self, parameter: &str, value: f32) -> Result<(), String> {
        match parameter {
            "osc_shape" => {
                self.params.osc_shape.set_bipolar(value);
                Ok(())
            }
            "sub_level" => {
                self.params.sub_level.set_bipolar(value);
                Ok(())
            }
            "osc_level" => {
                self.params.osc_level.set_bipolar(value);
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
            "filter_env_amount" => {
                self.params.filter_env_amount.set_bipolar(value);
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
            "amp_sustain" => {
                self.params.amp_sustain.set_bipolar(value);
                Ok(())
            }
            "amp_release" => {
                self.params.amp_release.set_bipolar(value);
                Ok(())
            }
            "filter_attack" => {
                self.params.filter_attack.set_bipolar(value);
                Ok(())
            }
            "filter_decay" => {
                self.params.filter_decay.set_bipolar(value);
                Ok(())
            }
            "filter_sustain" => {
                self.params.filter_sustain.set_bipolar(value);
                Ok(())
            }
            "filter_release" => {
                self.params.filter_release.set_bipolar(value);
                Ok(())
            }
            "overdrive" => {
                self.params.overdrive.set_bipolar(value);
                Ok(())
            }
            "volume" => {
                self.params.volume.set_bipolar(value);
                Ok(())
            }
            "tuning" => {
                self.params.tuning.set_bipolar(value);
                Ok(())
            }
            _ => Err(format!("Unknown parameter: {}", parameter)),
        }
    }

    fn parameter_range(&self, parameter: &str) -> Option<(f32, f32)> {
        match parameter {
            "osc_shape" => Some(self.params.osc_shape.range()),
            "sub_level" => Some(self.params.sub_level.range()),
            "osc_level" => Some(self.params.osc_level.range()),
            "filter_cutoff" => Some(self.params.filter_cutoff.range()),
            "filter_resonance" => Some(self.params.filter_resonance.range()),
            "filter_env_amount" => Some(self.params.filter_env_amount.range()),
            "amp_attack" => Some(self.params.amp_attack.range()),
            "amp_decay" => Some(self.params.amp_decay.range()),
            "amp_sustain" => Some(self.params.amp_sustain.range()),
            "amp_release" => Some(self.params.amp_release.range()),
            "filter_attack" => Some(self.params.filter_attack.range()),
            "filter_decay" => Some(self.params.filter_decay.range()),
            "filter_sustain" => Some(self.params.filter_sustain.range()),
            "filter_release" => Some(self.params.filter_release.range()),
            "overdrive" => Some(self.params.overdrive.range()),
            "volume" => Some(self.params.volume.range()),
            "tuning" => Some(self.params.tuning.range()),
            _ => None,
        }
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
        for cfg in [
            PolySynthConfig::default(),
            PolySynthConfig::pad(),
            PolySynthConfig::pluck(),
            PolySynthConfig::keys(),
            PolySynthConfig::strings(),
        ] {
            let mut synth = PolySynth::with_config(sample_rate, cfg);
            synth.trigger_note(60, 1.0);

            let mut energy = 0.0_f64;
            let dt = 1.0 / sample_rate as f64;
            for i in 0..4410 {
                let t = i as f64 * dt;
                let s = synth.tick(t) as f64;
                energy += s * s;
            }
            assert!(energy > 0.0001, "preset should produce audible output");
        }
    }

    fn zero_crossings(samples: &[f32]) -> usize {
        samples
            .windows(2)
            .filter(|w| (w[0] <= 0.0 && w[1] > 0.0) || (w[0] >= 0.0 && w[1] < 0.0))
            .count()
    }

    #[test]
    fn test_poly_synth_tuning_shifts_pitch() {
        let sample_rate = 44100.0;
        let dt = 1.0 / sample_rate as f64;

        let mut neutral = PolySynth::new(sample_rate);
        neutral.set_tuning(0.5);
        neutral.snap_params();
        neutral.trigger_note(60, 1.0);

        let mut up = PolySynth::new(sample_rate);
        up.set_tuning(1.0);
        up.snap_params();
        up.trigger_note(60, 1.0);

        // Skip the attack portion; sample a steady-state window.
        let prime = (sample_rate * 0.05) as usize;
        for i in 0..prime {
            let t = i as f64 * dt;
            neutral.tick(t);
            up.tick(t);
        }

        let window = 2048;
        let mut n_samples = Vec::with_capacity(window);
        let mut u_samples = Vec::with_capacity(window);
        for i in 0..window {
            let t = (prime + i) as f64 * dt;
            n_samples.push(neutral.tick(t));
            u_samples.push(up.tick(t));
        }

        let n_zc = zero_crossings(&n_samples);
        let u_zc = zero_crossings(&u_samples);
        // +12 semitones doubles pitch, expect roughly 2x zero crossings.
        assert!(
            u_zc > n_zc + n_zc / 4,
            "tuning=1.0 should raise pitch (neutral={}, up={})",
            n_zc,
            u_zc
        );
    }

    #[test]
    fn test_poly_synth_sub_layer_adds_energy() {
        let sample_rate = 44100.0;
        let dt = 1.0 / sample_rate as f64;

        let mut cfg = PolySynthConfig::default();
        cfg.sub_level = 0.0;
        cfg.osc_level = 0.0;
        let mut silent_cfg = cfg;
        silent_cfg.sub_level = 0.0;
        let mut sub_cfg = cfg;
        sub_cfg.sub_level = 1.0;

        let mut silent = PolySynth::with_config(sample_rate, silent_cfg);
        silent.snap_params();
        silent.trigger_note(60, 1.0);

        let mut with_sub = PolySynth::with_config(sample_rate, sub_cfg);
        with_sub.snap_params();
        with_sub.trigger_note(60, 1.0);

        let mut silent_energy = 0.0_f64;
        let mut sub_energy = 0.0_f64;
        let prime = (sample_rate * 0.02) as usize;
        for i in 0..(prime + 4096) {
            let t = i as f64 * dt;
            let s_sil = silent.tick(t) as f64;
            let s_sub = with_sub.tick(t) as f64;
            if i >= prime {
                silent_energy += s_sil * s_sil;
                sub_energy += s_sub * s_sub;
            }
        }
        assert!(
            sub_energy > silent_energy + 0.001,
            "sub layer should add energy (silent={}, sub={})",
            silent_energy,
            sub_energy
        );
    }

    #[test]
    fn test_poly_synth_overdrive_changes_signal() {
        let sample_rate = 44100.0;
        let dt = 1.0 / sample_rate as f64;

        let mut clean = PolySynth::new(sample_rate);
        clean.set_overdrive(0.0);
        clean.snap_params();
        clean.trigger_note(60, 1.0);

        let mut driven = PolySynth::new(sample_rate);
        driven.set_overdrive(0.9);
        driven.snap_params();
        driven.trigger_note(60, 1.0);

        let prime = (sample_rate * 0.02) as usize;
        for i in 0..prime {
            let t = i as f64 * dt;
            clean.tick(t);
            driven.tick(t);
        }

        let window = 4096;
        let mut diff = 0.0_f64;
        for i in 0..window {
            let t = (prime + i) as f64 * dt;
            let a = clean.tick(t) as f64;
            let b = driven.tick(t) as f64;
            diff += (a - b).abs();
        }
        assert!(
            diff > 1.0,
            "overdrive should change the waveform (diff={})",
            diff
        );
    }
}
