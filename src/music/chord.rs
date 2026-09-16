use std::ffi::CStr;
use std::fmt;

use super::cstr::cstr;
use super::interval::Interval;
use super::note::{note_to_midi, NoteName};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChordQuality {
    Major,
    Minor,
    Diminished,
    Augmented,
    Sus2,
    Sus4,
    Major6,
    Minor6,
    Add9,
    MinorAdd9,
    Major7,
    Minor7,
    Dominant7,
    Diminished7,
    HalfDiminished7,
    MinorMajor7,
    Dominant7Sus4,
    Dominant7Flat9,
    Dominant7Sharp9,
    Dominant7Sharp5,
    Major7Sharp11,
    Major9,
    Minor9,
    Dominant9,
    Dominant9Sus4,
    Minor9Flat5,
    MinorMajor9,
    Major69,
    Minor69,
    Major11,
    Minor11,
    Dominant11,
    Major13,
    Minor13,
    Dominant13,
}

impl ChordQuality {
    /// Every quality, for exhaustive tests and host enumeration.
    pub const ALL: [ChordQuality; 35] = [
        ChordQuality::Major,
        ChordQuality::Minor,
        ChordQuality::Diminished,
        ChordQuality::Augmented,
        ChordQuality::Sus2,
        ChordQuality::Sus4,
        ChordQuality::Major6,
        ChordQuality::Minor6,
        ChordQuality::Add9,
        ChordQuality::MinorAdd9,
        ChordQuality::Major7,
        ChordQuality::Minor7,
        ChordQuality::Dominant7,
        ChordQuality::Diminished7,
        ChordQuality::HalfDiminished7,
        ChordQuality::MinorMajor7,
        ChordQuality::Dominant7Sus4,
        ChordQuality::Dominant7Flat9,
        ChordQuality::Dominant7Sharp9,
        ChordQuality::Dominant7Sharp5,
        ChordQuality::Major7Sharp11,
        ChordQuality::Major9,
        ChordQuality::Minor9,
        ChordQuality::Dominant9,
        ChordQuality::Dominant9Sus4,
        ChordQuality::Minor9Flat5,
        ChordQuality::MinorMajor9,
        ChordQuality::Major69,
        ChordQuality::Minor69,
        ChordQuality::Major11,
        ChordQuality::Minor11,
        ChordQuality::Dominant11,
        ChordQuality::Major13,
        ChordQuality::Minor13,
        ChordQuality::Dominant13,
    ];

    /// Intervals from the root, in a fixed structural order that voicings rely on:
    /// index 0 is the root, index 1 is the third **or its suspension substitute**
    /// (2nd/4th), index 2 is the fifth (possibly altered), index 3 (when present)
    /// is the seventh **or the sixth**, and any remaining entries are extensions
    /// in ascending order. `VoicingType::Shell` in `src/music/voicing.rs` reads
    /// indices 0, 1 and 3 directly, so new qualities must preserve this layout.
    pub fn intervals(self) -> Vec<Interval> {
        use Interval::*;
        match self {
            // Triads
            ChordQuality::Major => vec![Unison, MajorThird, PerfectFifth],
            ChordQuality::Minor => vec![Unison, MinorThird, PerfectFifth],
            ChordQuality::Diminished => vec![Unison, MinorThird, Tritone],
            ChordQuality::Augmented => vec![Unison, MajorThird, MinorSixth],
            ChordQuality::Sus2 => vec![Unison, MajorSecond, PerfectFifth],
            ChordQuality::Sus4 => vec![Unison, PerfectFourth, PerfectFifth],
            // 6ths and added-note chords (no 7th)
            ChordQuality::Major6 => vec![Unison, MajorThird, PerfectFifth, MajorSixth],
            ChordQuality::Minor6 => vec![Unison, MinorThird, PerfectFifth, MajorSixth],
            ChordQuality::Add9 => vec![Unison, MajorThird, PerfectFifth, MajorNinth],
            ChordQuality::MinorAdd9 => vec![Unison, MinorThird, PerfectFifth, MajorNinth],
            // 7th chords
            ChordQuality::Major7 => vec![Unison, MajorThird, PerfectFifth, MajorSeventh],
            ChordQuality::Minor7 => vec![Unison, MinorThird, PerfectFifth, MinorSeventh],
            ChordQuality::Dominant7 => vec![Unison, MajorThird, PerfectFifth, MinorSeventh],
            ChordQuality::Diminished7 => vec![Unison, MinorThird, Tritone, MajorSixth],
            ChordQuality::HalfDiminished7 => vec![Unison, MinorThird, Tritone, MinorSeventh],
            ChordQuality::MinorMajor7 => vec![Unison, MinorThird, PerfectFifth, MajorSeventh],
            ChordQuality::Dominant7Sus4 => vec![Unison, PerfectFourth, PerfectFifth, MinorSeventh],
            ChordQuality::Dominant7Sharp5 => vec![Unison, MajorThird, MinorSixth, MinorSeventh],
            // Altered / colour dominants and lydian major
            ChordQuality::Dominant7Flat9 => {
                vec![Unison, MajorThird, PerfectFifth, MinorSeventh, MinorNinth]
            }
            ChordQuality::Dominant7Sharp9 => {
                vec![Unison, MajorThird, PerfectFifth, MinorSeventh, MinorTenth]
            }
            ChordQuality::Major7Sharp11 => vec![
                Unison,
                MajorThird,
                PerfectFifth,
                MajorSeventh,
                SharpEleventh,
            ],
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
            ChordQuality::Dominant9Sus4 => {
                vec![
                    Unison,
                    PerfectFourth,
                    PerfectFifth,
                    MinorSeventh,
                    MajorNinth,
                ]
            }
            ChordQuality::Minor9Flat5 => {
                vec![Unison, MinorThird, Tritone, MinorSeventh, MajorNinth]
            }
            ChordQuality::MinorMajor9 => {
                vec![Unison, MinorThird, PerfectFifth, MajorSeventh, MajorNinth]
            }
            // 6/9 chords
            ChordQuality::Major69 => {
                vec![Unison, MajorThird, PerfectFifth, MajorSixth, MajorNinth]
            }
            ChordQuality::Minor69 => {
                vec![Unison, MinorThird, PerfectFifth, MajorSixth, MajorNinth]
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

    /// True when the quality contains a seventh (or a higher tertian extension
    /// built on one). Sixths, sus chords and add-chords are false: they have no
    /// seventh even though some of them carry four or five notes.
    pub fn is_seventh_or_higher(self) -> bool {
        !matches!(
            self,
            ChordQuality::Major
                | ChordQuality::Minor
                | ChordQuality::Diminished
                | ChordQuality::Augmented
                | ChordQuality::Sus2
                | ChordQuality::Sus4
                | ChordQuality::Major6
                | ChordQuality::Minor6
                | ChordQuality::Add9
                | ChordQuality::MinorAdd9
                | ChordQuality::Major69
                | ChordQuality::Minor69
        )
    }

    pub fn note_count(self) -> usize {
        self.intervals().len()
    }

    /// The chord-symbol suffix appended to the root name ("m7", "maj9", "9sus4").
    ///
    /// Returned as a C string so the FFI layer can hand a host a stable, static
    /// pointer it never has to free. `Display` writes the same text.
    pub fn suffix(self) -> &'static CStr {
        match self {
            ChordQuality::Major => cstr!(""),
            ChordQuality::Minor => cstr!("m"),
            ChordQuality::Diminished => cstr!("dim"),
            ChordQuality::Augmented => cstr!("aug"),
            ChordQuality::Sus2 => cstr!("sus2"),
            ChordQuality::Sus4 => cstr!("sus4"),
            ChordQuality::Major6 => cstr!("6"),
            ChordQuality::Minor6 => cstr!("m6"),
            ChordQuality::Add9 => cstr!("add9"),
            ChordQuality::MinorAdd9 => cstr!("madd9"),
            ChordQuality::Major7 => cstr!("maj7"),
            ChordQuality::Minor7 => cstr!("m7"),
            ChordQuality::Dominant7 => cstr!("7"),
            ChordQuality::Diminished7 => cstr!("dim7"),
            ChordQuality::HalfDiminished7 => cstr!("m7b5"),
            ChordQuality::MinorMajor7 => cstr!("mMaj7"),
            ChordQuality::Dominant7Sus4 => cstr!("7sus4"),
            ChordQuality::Dominant7Flat9 => cstr!("7b9"),
            ChordQuality::Dominant7Sharp9 => cstr!("7#9"),
            ChordQuality::Dominant7Sharp5 => cstr!("7#5"),
            ChordQuality::Major7Sharp11 => cstr!("maj7#11"),
            ChordQuality::Major9 => cstr!("maj9"),
            ChordQuality::Minor9 => cstr!("m9"),
            ChordQuality::Dominant9 => cstr!("9"),
            ChordQuality::Dominant9Sus4 => cstr!("9sus4"),
            ChordQuality::Minor9Flat5 => cstr!("m9b5"),
            ChordQuality::MinorMajor9 => cstr!("mMaj9"),
            ChordQuality::Major69 => cstr!("6/9"),
            ChordQuality::Minor69 => cstr!("m6/9"),
            ChordQuality::Major11 => cstr!("maj11"),
            ChordQuality::Minor11 => cstr!("m11"),
            ChordQuality::Dominant11 => cstr!("11"),
            ChordQuality::Major13 => cstr!("maj13"),
            ChordQuality::Minor13 => cstr!("m13"),
            ChordQuality::Dominant13 => cstr!("13"),
        }
    }

    /// The chord-symbol suffix as a Rust string slice.
    pub fn suffix_str(self) -> &'static str {
        // Every suffix above is an ASCII literal, so this never fails.
        self.suffix().to_str().unwrap_or("")
    }
}

impl fmt::Display for ChordQuality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.suffix_str())
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
            .map(|interval| (root_midi.saturating_add(interval.semitones())).min(127))
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

    fn notes_from_c(quality: ChordQuality) -> Vec<u8> {
        Chord::new(NoteName::C, quality).midi_notes(4)
    }

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

    #[test]
    fn neo_soul_qualities_have_expected_notes() {
        // C7#9: C E G Bb D# (the "Hendrix" chord)
        assert_eq!(
            notes_from_c(ChordQuality::Dominant7Sharp9),
            vec![60, 64, 67, 70, 75]
        );
        // C9sus4: C F G Bb D
        assert_eq!(
            notes_from_c(ChordQuality::Dominant9Sus4),
            vec![60, 65, 67, 70, 74]
        );
        // Cmaj7#11: C E G B F#(+octave)
        assert_eq!(
            notes_from_c(ChordQuality::Major7Sharp11),
            vec![60, 64, 67, 71, 78]
        );
        // Cm9b5: C Eb Gb Bb D
        assert_eq!(
            notes_from_c(ChordQuality::Minor9Flat5),
            vec![60, 63, 66, 70, 74]
        );
    }

    #[test]
    fn tier_b_qualities_have_expected_notes() {
        assert_eq!(notes_from_c(ChordQuality::Sus2), vec![60, 62, 67]);
        assert_eq!(notes_from_c(ChordQuality::Sus4), vec![60, 65, 67]);
        assert_eq!(
            notes_from_c(ChordQuality::Dominant7Sus4),
            vec![60, 65, 67, 70]
        );
        assert_eq!(notes_from_c(ChordQuality::Major6), vec![60, 64, 67, 69]);
        assert_eq!(notes_from_c(ChordQuality::Minor6), vec![60, 63, 67, 69]);
        assert_eq!(
            notes_from_c(ChordQuality::Major69),
            vec![60, 64, 67, 69, 74]
        );
        assert_eq!(
            notes_from_c(ChordQuality::Minor69),
            vec![60, 63, 67, 69, 74]
        );
        assert_eq!(notes_from_c(ChordQuality::Add9), vec![60, 64, 67, 74]);
        assert_eq!(notes_from_c(ChordQuality::MinorAdd9), vec![60, 63, 67, 74]);
        assert_eq!(
            notes_from_c(ChordQuality::Dominant7Flat9),
            vec![60, 64, 67, 70, 73]
        );
        assert_eq!(
            notes_from_c(ChordQuality::Dominant7Sharp5),
            vec![60, 64, 68, 70]
        );
        assert_eq!(
            notes_from_c(ChordQuality::MinorMajor9),
            vec![60, 63, 67, 71, 74]
        );
    }

    #[test]
    fn new_quality_suffixes() {
        assert_eq!(ChordQuality::Dominant7Sharp9.to_string(), "7#9");
        assert_eq!(ChordQuality::Major7Sharp11.to_string(), "maj7#11");
        assert_eq!(ChordQuality::Dominant9Sus4.to_string(), "9sus4");
        assert_eq!(ChordQuality::Minor9Flat5.to_string(), "m9b5");
        assert_eq!(ChordQuality::Major69.to_string(), "6/9");
        assert_eq!(
            Chord::new(NoteName::E, ChordQuality::Dominant7Sharp9).display_name(),
            "E7#9"
        );
    }

    #[test]
    fn all_qualities_are_unique_and_named() {
        for (i, a) in ChordQuality::ALL.iter().enumerate() {
            for b in ChordQuality::ALL.iter().skip(i + 1) {
                assert_ne!(a, b, "duplicate entry in ChordQuality::ALL");
            }
            // Only the plain major triad has an empty suffix.
            if *a != ChordQuality::Major {
                assert!(!a.suffix_str().is_empty(), "{a:?} has no suffix");
            }
            assert_eq!(a.suffix_str(), a.to_string(), "{a:?} suffix/Display differ");
        }
    }

    /// `VoicingType::Shell` reads intervals[1] as "the third" and intervals[3]
    /// as "the seventh". Every quality must keep that structural layout.
    #[test]
    fn all_qualities_keep_the_structural_interval_order() {
        for quality in ChordQuality::ALL {
            let intervals = quality.intervals();
            assert!(intervals.len() >= 3, "{quality:?} is too small");
            assert_eq!(intervals[0], Interval::Unison, "{quality:?} lacks a root");

            // Third, or a 2nd/4th substituting for it in a sus chord.
            let third = intervals[1].semitones();
            assert!(
                (2..=5).contains(&third),
                "{quality:?} has {third} semitones at the third slot"
            );

            if intervals.len() >= 4 {
                // Seventh, or the sixth that replaces it in 6 and 6/9 chords.
                let seventh = intervals[3].semitones();
                assert!(
                    matches!(seventh, 9 | 10 | 11 | 14),
                    "{quality:?} has {seventh} semitones at the seventh slot"
                );
            }

            // Extensions ascend.
            for pair in intervals.windows(2) {
                assert!(
                    pair[0].semitones() < pair[1].semitones(),
                    "{quality:?} intervals are not ascending"
                );
            }
        }
    }

    #[test]
    fn sixths_sus_and_add_chords_are_not_sevenths() {
        for quality in [
            ChordQuality::Sus2,
            ChordQuality::Sus4,
            ChordQuality::Major6,
            ChordQuality::Minor6,
            ChordQuality::Add9,
            ChordQuality::MinorAdd9,
            ChordQuality::Major69,
            ChordQuality::Minor69,
        ] {
            assert!(!quality.is_seventh_or_higher(), "{quality:?}");
        }
        for quality in [
            ChordQuality::Dominant7Sus4,
            ChordQuality::Dominant9Sus4,
            ChordQuality::Dominant7Sharp9,
            ChordQuality::Major7Sharp11,
            ChordQuality::Minor9Flat5,
        ] {
            assert!(quality.is_seventh_or_higher(), "{quality:?}");
        }
    }

    #[test]
    fn extreme_octaves_clamp_without_overflow() {
        for quality in ChordQuality::ALL {
            for octave in [0i8, 8] {
                for note in Chord::new(NoteName::B, quality).midi_notes(octave) {
                    assert!(note <= 127);
                }
            }
        }
    }
}
