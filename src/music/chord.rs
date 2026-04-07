use std::fmt;

use super::interval::Interval;
use super::note::{note_to_midi, NoteName};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChordQuality {
    Major,
    Minor,
    Diminished,
    Augmented,
    Major7,
    Minor7,
    Dominant7,
    Diminished7,
    HalfDiminished7,
    MinorMajor7,
    Major9,
    Minor9,
    Dominant9,
    Major11,
    Minor11,
    Dominant11,
    Major13,
    Minor13,
    Dominant13,
}

impl ChordQuality {
    pub fn intervals(self) -> Vec<Interval> {
        use Interval::*;
        match self {
            // Triads
            ChordQuality::Major => vec![Unison, MajorThird, PerfectFifth],
            ChordQuality::Minor => vec![Unison, MinorThird, PerfectFifth],
            ChordQuality::Diminished => vec![Unison, MinorThird, Tritone],
            ChordQuality::Augmented => vec![Unison, MajorThird, MinorSixth],
            // 7th chords
            ChordQuality::Major7 => vec![Unison, MajorThird, PerfectFifth, MajorSeventh],
            ChordQuality::Minor7 => vec![Unison, MinorThird, PerfectFifth, MinorSeventh],
            ChordQuality::Dominant7 => vec![Unison, MajorThird, PerfectFifth, MinorSeventh],
            ChordQuality::Diminished7 => vec![Unison, MinorThird, Tritone, MajorSixth],
            ChordQuality::HalfDiminished7 => vec![Unison, MinorThird, Tritone, MinorSeventh],
            ChordQuality::MinorMajor7 => vec![Unison, MinorThird, PerfectFifth, MajorSeventh],
            // 9th chords
            ChordQuality::Major9 => {
                vec![Unison, MajorThird, PerfectFifth, MajorSeventh, MajorNinth]
            }
            ChordQuality::Minor9 => {
                vec![Unison, MinorThird, PerfectFifth, MinorSeventh, MajorNinth]
            }
            ChordQuality::Dominant9 => {
                vec![Unison, MajorThird, PerfectFifth, MinorSeventh, MajorNinth]
            }
            // 11th chords
            ChordQuality::Major11 => vec![
                Unison,
                MajorThird,
                PerfectFifth,
                MajorSeventh,
                MajorNinth,
                PerfectEleventh,
            ],
            ChordQuality::Minor11 => vec![
                Unison,
                MinorThird,
                PerfectFifth,
                MinorSeventh,
                MajorNinth,
                PerfectEleventh,
            ],
            ChordQuality::Dominant11 => vec![
                Unison,
                MajorThird,
                PerfectFifth,
                MinorSeventh,
                MajorNinth,
                PerfectEleventh,
            ],
            // 13th chords
            ChordQuality::Major13 => vec![
                Unison,
                MajorThird,
                PerfectFifth,
                MajorSeventh,
                MajorNinth,
                MajorThirteenth,
            ],
            ChordQuality::Minor13 => vec![
                Unison,
                MinorThird,
                PerfectFifth,
                MinorSeventh,
                MajorNinth,
                MajorThirteenth,
            ],
            ChordQuality::Dominant13 => vec![
                Unison,
                MajorThird,
                PerfectFifth,
                MinorSeventh,
                MajorNinth,
                MajorThirteenth,
            ],
        }
    }

    pub fn is_seventh_or_higher(self) -> bool {
        !matches!(
            self,
            ChordQuality::Major
                | ChordQuality::Minor
                | ChordQuality::Diminished
                | ChordQuality::Augmented
        )
    }

    pub fn note_count(self) -> usize {
        self.intervals().len()
    }
}

impl fmt::Display for ChordQuality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChordQuality::Major => write!(f, ""),
            ChordQuality::Minor => write!(f, "m"),
            ChordQuality::Diminished => write!(f, "dim"),
            ChordQuality::Augmented => write!(f, "aug"),
            ChordQuality::Major7 => write!(f, "maj7"),
            ChordQuality::Minor7 => write!(f, "m7"),
            ChordQuality::Dominant7 => write!(f, "7"),
            ChordQuality::Diminished7 => write!(f, "dim7"),
            ChordQuality::HalfDiminished7 => write!(f, "m7b5"),
            ChordQuality::MinorMajor7 => write!(f, "mMaj7"),
            ChordQuality::Major9 => write!(f, "maj9"),
            ChordQuality::Minor9 => write!(f, "m9"),
            ChordQuality::Dominant9 => write!(f, "9"),
            ChordQuality::Major11 => write!(f, "maj11"),
            ChordQuality::Minor11 => write!(f, "m11"),
            ChordQuality::Dominant11 => write!(f, "11"),
            ChordQuality::Major13 => write!(f, "maj13"),
            ChordQuality::Minor13 => write!(f, "m13"),
            ChordQuality::Dominant13 => write!(f, "13"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chord {
    pub root: NoteName,
    pub quality: ChordQuality,
}

impl Chord {
    pub fn new(root: NoteName, quality: ChordQuality) -> Self {
        Self { root, quality }
    }

    /// Returns MIDI note numbers for the chord in root position at the given octave
    pub fn midi_notes(&self, octave: i8) -> Vec<u8> {
        let root_midi = note_to_midi(self.root, octave);
        self.quality
            .intervals()
            .iter()
            .map(|interval| (root_midi + interval.semitones()).min(127))
            .collect()
    }

    /// Returns the note names in the chord
    pub fn note_names(&self) -> Vec<NoteName> {
        self.quality
            .intervals()
            .iter()
            .map(|interval| self.root.transpose(interval.semitones()))
            .collect()
    }

    pub fn display_name(&self) -> String {
        format!("{}{}", self.root, self.quality)
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_c_major_triad() {
        let chord = Chord::new(NoteName::C, ChordQuality::Major);
        let notes = chord.midi_notes(4);
        assert_eq!(notes, vec![60, 64, 67]); // C4, E4, G4
    }

    #[test]
    fn test_a_minor_triad() {
        let chord = Chord::new(NoteName::A, ChordQuality::Minor);
        let notes = chord.midi_notes(4);
        assert_eq!(notes, vec![69, 72, 76]); // A4, C5, E5
    }

    #[test]
    fn test_c_major7() {
        let chord = Chord::new(NoteName::C, ChordQuality::Major7);
        let notes = chord.midi_notes(4);
        assert_eq!(notes, vec![60, 64, 67, 71]); // C4, E4, G4, B4
    }

    #[test]
    fn test_display_name() {
        assert_eq!(
            Chord::new(NoteName::C, ChordQuality::Major).display_name(),
            "C"
        );
        assert_eq!(
            Chord::new(NoteName::D, ChordQuality::Minor).display_name(),
            "Dm"
        );
        assert_eq!(
            Chord::new(NoteName::G, ChordQuality::Dominant7).display_name(),
            "G7"
        );
    }
}
