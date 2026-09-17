//! Monophonic-control synth for chord-aware live melody.

use crate::frame::StereoFrame;
use crate::music::{quantize_note_to_chord, Chord};

use super::{PolySynth, PolySynthConfig};

/// A dedicated synth and the harmonic state used to quantize one held melody.
pub struct MelodyVoice {
    synth: PolySynth,
    harmony: Option<Chord>,
    held_input: Option<u8>,
    sounding_note: Option<u8>,
    velocity: f32,
}

impl MelodyVoice {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            synth: PolySynth::with_config(sample_rate, PolySynthConfig::keys()),
            harmony: None,
            held_input: None,
            sounding_note: None,
            velocity: 1.0,
        }
    }

    /// Latch a chord and immediately make a held gesture valid for it.
    pub fn set_harmony(&mut self, chord: Chord) {
        self.harmony = Some(chord);
        self.retarget_held_note();
    }

    pub fn has_harmony(&self) -> bool {
        self.harmony.is_some()
    }

    /// Forget the harmonic context and end the complete gesture.
    pub fn clear_harmony(&mut self) {
        self.synth.release_all();
        self.harmony = None;
        self.held_input = None;
        self.sounding_note = None;
    }

    /// Begin a gesture. With no harmony the gesture remains held but silent.
    pub fn note_on(&mut self, input: u8, velocity: f32) -> Option<u8> {
        if self.sounding_note.is_some() {
            self.synth.release_all();
        }
        self.held_input = Some(input);
        self.sounding_note = None;
        self.velocity = velocity.clamp(0.0, 1.0);
        self.retarget_held_note();
        self.sounding_note
    }

    /// Move an existing gesture to a new intended note.
    pub fn update_note(&mut self, input: u8) -> Option<u8> {
        self.held_input?;
        self.held_input = Some(input);
        self.retarget_held_note();
        self.sounding_note
    }

    pub fn note_off(&mut self) {
        self.held_input = None;
        if let Some(note) = self.sounding_note.take() {
            self.synth.release_note(note);
        }
    }

    pub fn sounding_note(&self) -> Option<u8> {
        self.sounding_note
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

    fn retarget_held_note(&mut self) {
        let (Some(input), Some(chord)) = (self.held_input, self.harmony.as_ref()) else {
            return;
        };
        let quantized = quantize_note_to_chord(input, chord, self.sounding_note);

        match self.sounding_note {
            Some(current) if current == quantized => {}
            Some(current) => {
                if !self.synth.retune_note(current, quantized) {
                    self.synth.trigger_note(quantized, self.velocity);
                }
                self.sounding_note = Some(quantized);
            }
            None => {
                self.synth.trigger_note(quantized, self.velocity);
                self.sounding_note = Some(quantized);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::{ChordQuality, NoteName};

    #[test]
    fn a_pre_harmony_gesture_starts_when_a_chord_arrives() {
        let mut melody = MelodyVoice::new(44_100.0);
        assert_eq!(melody.note_on(66, 0.8), None);
        melody.set_harmony(Chord::new(NoteName::C, ChordQuality::Major7));
        assert_eq!(melody.sounding_note(), Some(67));
    }

    #[test]
    fn a_held_note_retargets_and_harmony_latches() {
        let mut melody = MelodyVoice::new(44_100.0);
        melody.set_harmony(Chord::new(NoteName::C, ChordQuality::Major7));
        assert_eq!(melody.note_on(66, 0.8), Some(67));
        melody.set_harmony(Chord::new(NoteName::D, ChordQuality::Minor7));
        assert_eq!(melody.sounding_note(), Some(65));
        melody.note_off();
        assert!(melody.has_harmony());
        assert_eq!(melody.sounding_note(), None);
    }

    #[test]
    fn clearing_harmony_cancels_the_gesture() {
        let mut melody = MelodyVoice::new(44_100.0);
        melody.set_harmony(Chord::new(NoteName::C, ChordQuality::Major));
        assert_eq!(melody.note_on(64, 1.0), Some(64));
        melody.clear_harmony();
        assert!(!melody.has_harmony());
        assert_eq!(melody.update_note(67), None);
    }
}
