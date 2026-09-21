//! Reusable monophonic control around the expressive [`PolySynth`].
//!
//! The wrapped synth keeps its existing stereo DSP and normalized parameter
//! namespace. `MonoSynth` adds ownership of one held note, including legato
//! retuning and note-off, so features do not need to repeat that state machine.

use crate::engine::Instrument;
use crate::frame::StereoFrame;

use super::{PolySynth, PolySynthConfig};

/// A one-held-note instrument backed by the existing expressive poly synth.
pub struct MonoSynth {
    synth: PolySynth,
    held_note: Option<u8>,
    held_velocity: f32,
    pending_note: Option<u8>,
}

impl MonoSynth {
    pub fn new(sample_rate: f32) -> Self {
        Self::with_config(sample_rate, PolySynthConfig::default())
    }

    pub fn with_config(sample_rate: f32, config: PolySynthConfig) -> Self {
        Self {
            synth: PolySynth::with_config(sample_rate, config),
            held_note: None,
            held_velocity: 1.0,
            pending_note: None,
        }
    }

    /// Begin a note, releasing any currently held note first.
    pub fn note_on(&mut self, note: u8, velocity: f32) {
        if self.held_note.is_some() {
            self.synth.release_all();
        }
        self.held_velocity = finite_velocity(velocity);
        self.synth.trigger_note(note, self.held_velocity);
        self.held_note = Some(note);
    }

    /// Move the held note without restarting its oscillators or envelopes.
    ///
    /// Returns false when there is no held note. If the wrapped voice has
    /// already disappeared, a replacement note is triggered so the public
    /// monophonic state remains audible and coherent.
    pub fn retune(&mut self, note: u8) -> bool {
        let Some(current) = self.held_note else {
            return false;
        };
        if current != note && !self.synth.retune_note(current, note) {
            self.synth.trigger_note(note, self.held_velocity);
        }
        self.held_note = Some(note);
        true
    }

    /// Release the held note while allowing its configured tail to finish.
    pub fn note_off(&mut self) {
        if let Some(note) = self.held_note.take() {
            self.synth.release_note(note);
        }
    }

    /// Release every wrapped voice and clear monophonic note ownership.
    pub fn release_all(&mut self) {
        self.synth.release_all();
        self.held_note = None;
    }

    pub fn current_note(&self) -> Option<u8> {
        self.held_note
    }

    pub fn set_param(&mut self, param: u32, value: f32) -> bool {
        self.synth.set_param(param, value)
    }

    pub fn param(&self, param: u32) -> Option<f32> {
        self.synth.param(param)
    }

    pub fn tick_frame(&mut self, current_time: f64) -> StereoFrame {
        self.synth.tick_frame(current_time)
    }
}

impl Instrument for MonoSynth {
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        let note = self.pending_note.take().unwrap_or(60);
        if self.held_note.is_some() {
            self.synth.release_all();
        }
        self.held_velocity = finite_velocity(velocity);
        self.synth.set_midi_note(note);
        self.synth.trigger_with_velocity(time, self.held_velocity);
        self.held_note = Some(note);
    }

    fn tick(&mut self, current_time: f64) -> f32 {
        self.synth.tick(current_time)
    }

    fn tick_stereo(&mut self, current_time: f64) -> Option<StereoFrame> {
        Some(self.tick_frame(current_time))
    }

    fn is_active(&self) -> bool {
        self.synth.is_active()
    }

    fn set_midi_note(&mut self, note: u8) {
        self.pending_note = Some(note);
    }
}

fn finite_velocity(velocity: f32) -> f32 {
    if velocity.is_finite() {
        velocity.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instruments::{POLY_PARAM_FILTER_CUTOFF, POLY_PARAM_VOLUME};

    #[test]
    fn owns_one_held_note_and_retargets_legato() {
        let mut synth = MonoSynth::with_config(44_100.0, PolySynthConfig::keys());

        synth.note_on(60, 0.8);
        assert_eq!(synth.current_note(), Some(60));
        for frame in 0..512 {
            let _ = synth.tick_frame(frame as f64 / 44_100.0);
        }
        assert!(synth.retune(64));
        assert_eq!(synth.current_note(), Some(64));

        synth.note_off();
        assert_eq!(synth.current_note(), None);
        assert!(!synth.retune(67));
        let release = synth.tick_frame(513.0 / 44_100.0);
        assert!(release.l.abs().max(release.r.abs()) > 0.0);
    }

    #[test]
    fn passes_parameters_through_to_the_wrapped_synth() {
        let mut synth = MonoSynth::new(44_100.0);

        assert!(synth.set_param(POLY_PARAM_FILTER_CUTOFF, 0.42));
        assert!(synth.set_param(POLY_PARAM_VOLUME, 0.25));
        assert_eq!(synth.param(POLY_PARAM_FILTER_CUTOFF), Some(0.42));
        assert_eq!(synth.param(POLY_PARAM_VOLUME), Some(0.25));
        assert!(!synth.set_param(u32::MAX, 0.5));
    }

    #[test]
    fn implements_instrument_with_the_requested_midi_note() {
        fn require_instrument<T: Instrument>() {}
        require_instrument::<MonoSynth>();

        let mut synth = MonoSynth::new(44_100.0);
        synth.set_midi_note(72);
        synth.trigger_with_velocity(0.0, 0.7);
        assert_eq!(synth.current_note(), Some(72));
        assert!(synth.is_active());
        assert!(synth.tick_stereo(0.001).is_some());
    }

    #[test]
    fn stereo_render_matches_the_wrapped_poly_synth() {
        let config = PolySynthConfig::keys();
        let mut mono = MonoSynth::with_config(44_100.0, config);
        let mut poly = PolySynth::with_config(44_100.0, config);
        mono.note_on(67, 0.8);
        poly.trigger_note(67, 0.8);

        for frame in 0..256 {
            let time = frame as f64 / 44_100.0;
            assert_eq!(mono.tick_frame(time), poly.tick_frame(time));
        }
    }
}
