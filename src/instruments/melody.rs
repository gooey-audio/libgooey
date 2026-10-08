//! Monophonic-control synth for chord-aware live melody.

use crate::frame::StereoFrame;
use crate::music::{quantize_note_to_chord, quantize_note_to_key, Chord, Key};

use super::{MonoSynth, PolySynthConfig};

/// Which pitches a held melody may land on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MelodyNotePool {
    /// Only the latched chord's own tones.
    Chord,
    /// Chord tones plus the key's tensions that do not clash with the chord.
    /// Falls back to [`MelodyNotePool::Chord`] when no key is latched.
    #[default]
    ChordAndKey,
}

/// A dedicated synth and the harmonic state used to quantize one held melody.
pub struct MelodyVoice {
    synth: MonoSynth,
    harmony: Option<Chord>,
    key: Option<Key>,
    note_pool: MelodyNotePool,
    held_input: Option<u8>,
    velocity: f32,
}

impl MelodyVoice {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            synth: MonoSynth::with_config(sample_rate, PolySynthConfig::keys()),
            harmony: None,
            key: None,
            note_pool: MelodyNotePool::default(),
            held_input: None,
            velocity: 1.0,
        }
    }

    /// Latch a chord with no key and immediately make a held gesture valid
    /// for it. Only chord tones are eligible until a key is latched.
    pub fn set_harmony(&mut self, chord: Chord) {
        self.harmony = Some(chord);
        self.key = None;
        self.retarget_held_note();
    }

    /// Latch a chord along with the key it was drawn from, so the melody can
    /// also reach that key's tensions.
    pub fn set_harmony_in_key(&mut self, chord: Chord, key: Key) {
        self.harmony = Some(chord);
        self.key = Some(key);
        self.retarget_held_note();
    }

    pub fn note_pool(&self) -> MelodyNotePool {
        self.note_pool
    }

    /// Choose the eligible pitches. A held gesture moves to the new pool.
    pub fn set_note_pool(&mut self, pool: MelodyNotePool) {
        self.note_pool = pool;
        self.retarget_held_note();
    }

    pub fn has_harmony(&self) -> bool {
        self.harmony.is_some()
    }

    /// Forget the harmonic context and end the complete gesture.
    pub fn clear_harmony(&mut self) {
        self.synth.release_all();
        self.harmony = None;
        self.key = None;
        self.held_input = None;
    }

    /// Begin a gesture. With no harmony the gesture remains held but silent.
    pub fn note_on(&mut self, input: u8, velocity: f32) -> Option<u8> {
        if self.synth.current_note().is_some() {
            self.synth.release_all();
        }
        self.held_input = Some(input);
        self.velocity = velocity.clamp(0.0, 1.0);
        self.retarget_held_note();
        self.synth.current_note()
    }

    /// Move an existing gesture to a new intended note.
    pub fn update_note(&mut self, input: u8) -> Option<u8> {
        self.held_input?;
        self.held_input = Some(input);
        self.retarget_held_note();
        self.synth.current_note()
    }

    pub fn note_off(&mut self) {
        self.held_input = None;
        self.synth.note_off();
    }

    pub fn sounding_note(&self) -> Option<u8> {
        self.synth.current_note()
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
        let sounding_note = self.synth.current_note();
        let quantized = match (self.note_pool, self.key.as_ref()) {
            (MelodyNotePool::ChordAndKey, Some(key)) => {
                quantize_note_to_key(input, chord, key, sounding_note)
            }
            _ => quantize_note_to_chord(input, chord, sounding_note),
        };

        match sounding_note {
            Some(current) if current == quantized => {}
            Some(_) => {
                _ = self.synth.retune(quantized);
            }
            None => {
                self.synth.note_on(quantized, self.velocity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::{ChordQuality, NoteName, ScaleType};

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

    #[test]
    fn a_key_opens_tensions_and_the_pool_can_narrow_back_to_the_chord() {
        let mut melody = MelodyVoice::new(44_100.0);
        let c_major = Key::new(NoteName::C, ScaleType::Major);
        melody.set_harmony_in_key(Chord::new(NoteName::C, ChordQuality::Major), c_major);
        // D is the 9th: chord-only quantization would have pulled it to C.
        assert_eq!(melody.note_on(62, 0.8), Some(62));

        melody.set_note_pool(MelodyNotePool::Chord);
        assert_eq!(melody.sounding_note(), Some(60));

        melody.set_note_pool(MelodyNotePool::ChordAndKey);
        assert_eq!(melody.sounding_note(), Some(62));

        // A keyless chord falls back to chord tones regardless of the pool.
        melody.set_harmony(Chord::new(NoteName::C, ChordQuality::Major));
        assert_eq!(melody.sounding_note(), Some(60));
    }
}
