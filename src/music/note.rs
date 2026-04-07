use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NoteName {
    C,
    Cs,
    D,
    Ds,
    E,
    F,
    Fs,
    G,
    Gs,
    A,
    As,
    B,
}

impl NoteName {
    pub const ALL: [NoteName; 12] = [
        NoteName::C,
        NoteName::Cs,
        NoteName::D,
        NoteName::Ds,
        NoteName::E,
        NoteName::F,
        NoteName::Fs,
        NoteName::G,
        NoteName::Gs,
        NoteName::A,
        NoteName::As,
        NoteName::B,
    ];

    pub fn from_index(index: u8) -> Self {
        Self::ALL[(index % 12) as usize]
    }

    pub fn to_index(self) -> u8 {
        match self {
            NoteName::C => 0,
            NoteName::Cs => 1,
            NoteName::D => 2,
            NoteName::Ds => 3,
            NoteName::E => 4,
            NoteName::F => 5,
            NoteName::Fs => 6,
            NoteName::G => 7,
            NoteName::Gs => 8,
            NoteName::A => 9,
            NoteName::As => 10,
            NoteName::B => 11,
        }
    }

    pub fn transpose(self, semitones: u8) -> Self {
        Self::from_index(self.to_index().wrapping_add(semitones) % 12)
    }
}

impl fmt::Display for NoteName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NoteName::C => write!(f, "C"),
            NoteName::Cs => write!(f, "C#"),
            NoteName::D => write!(f, "D"),
            NoteName::Ds => write!(f, "D#"),
            NoteName::E => write!(f, "E"),
            NoteName::F => write!(f, "F"),
            NoteName::Fs => write!(f, "F#"),
            NoteName::G => write!(f, "G"),
            NoteName::Gs => write!(f, "G#"),
            NoteName::A => write!(f, "A"),
            NoteName::As => write!(f, "A#"),
            NoteName::B => write!(f, "B"),
        }
    }
}

/// Convert a MIDI note number to frequency in Hz (A4 = 440 Hz, equal temperament)
pub fn midi_to_freq(note: u8) -> f64 {
    440.0 * 2.0_f64.powf((note as f64 - 69.0) / 12.0)
}

/// Convert a NoteName + octave to a MIDI note number
/// Octave 4 means middle C (C4 = MIDI 60)
pub fn note_to_midi(name: NoteName, octave: i8) -> u8 {
    ((octave as i16 + 1) * 12 + name.to_index() as i16).clamp(0, 127) as u8
}

/// Convert a MIDI note number to a (NoteName, octave) pair
pub fn midi_to_note(midi: u8) -> (NoteName, i8) {
    let name = NoteName::from_index(midi % 12);
    let octave = (midi / 12) as i8 - 1;
    (name, octave)
}

/// Format a MIDI note as a string like "C4", "F#3"
pub fn midi_to_string(midi: u8) -> String {
    let (name, octave) = midi_to_note(midi);
    format!("{}{}", name, octave)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_midi_to_freq_a4() {
        let freq = midi_to_freq(69);
        assert!((freq - 440.0).abs() < 0.001);
    }

    #[test]
    fn test_midi_to_freq_c4() {
        let freq = midi_to_freq(60);
        assert!((freq - 261.626).abs() < 0.01);
    }

    #[test]
    fn test_note_to_midi_c4() {
        assert_eq!(note_to_midi(NoteName::C, 4), 60);
    }

    #[test]
    fn test_note_to_midi_a4() {
        assert_eq!(note_to_midi(NoteName::A, 4), 69);
    }

    #[test]
    fn test_midi_roundtrip() {
        for midi in 0..=127u8 {
            let (name, octave) = midi_to_note(midi);
            assert_eq!(note_to_midi(name, octave), midi);
        }
    }

    #[test]
    fn test_transpose() {
        assert_eq!(NoteName::C.transpose(4), NoteName::E);
        assert_eq!(NoteName::A.transpose(3), NoteName::C);
    }
}
