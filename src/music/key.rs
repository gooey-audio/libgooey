use std::fmt;

use super::chord::{Chord, ChordQuality};
use super::note::NoteName;
use super::scale::ScaleType;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    pub root: NoteName,
    pub scale_type: ScaleType,
}

impl Key {
    pub fn new(root: NoteName, scale_type: ScaleType) -> Self {
        Self { root, scale_type }
    }

    /// Returns the note names of all 7 scale degrees
    pub fn scale_degrees(&self) -> Vec<NoteName> {
        self.scale_type
            .intervals()
            .iter()
            .map(|&offset| self.root.transpose(offset))
            .collect()
    }

    /// Returns the 7 diatonic triads in this key
    pub fn diatonic_triads(&self) -> Vec<Chord> {
        let degrees = self.scale_degrees();
        let triad_qualities = match self.scale_type {
            ScaleType::Major => [
                ChordQuality::Major,      // I
                ChordQuality::Minor,      // ii
                ChordQuality::Minor,      // iii
                ChordQuality::Major,      // IV
                ChordQuality::Major,      // V
                ChordQuality::Minor,      // vi
                ChordQuality::Diminished, // vii
            ],
            ScaleType::NaturalMinor => [
                ChordQuality::Minor,      // i
                ChordQuality::Diminished, // ii
                ChordQuality::Major,      // III
                ChordQuality::Minor,      // iv
                ChordQuality::Minor,      // v
                ChordQuality::Major,      // VI
                ChordQuality::Major,      // VII
            ],
        };

        degrees
            .iter()
            .zip(triad_qualities.iter())
            .map(|(&root, &quality)| Chord::new(root, quality))
            .collect()
    }

    /// Returns the 7 diatonic 7th chords in this key
    pub fn diatonic_sevenths(&self) -> Vec<Chord> {
        let degrees = self.scale_degrees();
        let seventh_qualities = match self.scale_type {
            ScaleType::Major => [
                ChordQuality::Major7,          // Imaj7
                ChordQuality::Minor7,          // ii7
                ChordQuality::Minor7,          // iii7
                ChordQuality::Major7,          // IVmaj7
                ChordQuality::Dominant7,       // V7
                ChordQuality::Minor7,          // vi7
                ChordQuality::HalfDiminished7, // viim7b5
            ],
            ScaleType::NaturalMinor => [
                ChordQuality::Minor7,          // i7
                ChordQuality::HalfDiminished7, // iim7b5
                ChordQuality::Major7,          // IIImaj7
                ChordQuality::Minor7,          // iv7
                ChordQuality::Minor7,          // v7
                ChordQuality::Major7,          // VImaj7
                ChordQuality::Dominant7,       // VII7
            ],
        };

        degrees
            .iter()
            .zip(seventh_qualities.iter())
            .map(|(&root, &quality)| Chord::new(root, quality))
            .collect()
    }

    /// Returns the 7 diatonic 9th chords in this key
    pub fn diatonic_ninths(&self) -> Vec<Chord> {
        let degrees = self.scale_degrees();
        let ninth_qualities = match self.scale_type {
            ScaleType::Major => [
                ChordQuality::Major9,    // Imaj9
                ChordQuality::Minor9,    // ii9
                ChordQuality::Minor9,    // iii9
                ChordQuality::Major9,    // IVmaj9
                ChordQuality::Dominant9, // V9
                ChordQuality::Minor9,    // vi9
                ChordQuality::Minor9,    // vii9 (simplified)
            ],
            ScaleType::NaturalMinor => [
                ChordQuality::Minor9,    // i9
                ChordQuality::Minor9,    // ii9 (simplified)
                ChordQuality::Major9,    // III9
                ChordQuality::Minor9,    // iv9
                ChordQuality::Minor9,    // v9
                ChordQuality::Major9,    // VI9
                ChordQuality::Dominant9, // VII9
            ],
        };

        degrees
            .iter()
            .zip(ninth_qualities.iter())
            .map(|(&root, &quality)| Chord::new(root, quality))
            .collect()
    }

    /// Returns the 7 diatonic 11th chords in this key
    pub fn diatonic_elevenths(&self) -> Vec<Chord> {
        let degrees = self.scale_degrees();
        let eleventh_qualities = match self.scale_type {
            ScaleType::Major => [
                ChordQuality::Major11,    // Imaj11
                ChordQuality::Minor11,    // ii11
                ChordQuality::Minor11,    // iii11
                ChordQuality::Major11,    // IVmaj11
                ChordQuality::Dominant11, // V11
                ChordQuality::Minor11,    // vi11
                ChordQuality::Minor11,    // vii11
            ],
            ScaleType::NaturalMinor => [
                ChordQuality::Minor11,    // i11
                ChordQuality::Minor11,    // ii11
                ChordQuality::Major11,    // III11
                ChordQuality::Minor11,    // iv11
                ChordQuality::Minor11,    // v11
                ChordQuality::Major11,    // VI11
                ChordQuality::Dominant11, // VII11
            ],
        };

        degrees
            .iter()
            .zip(eleventh_qualities.iter())
            .map(|(&root, &quality)| Chord::new(root, quality))
            .collect()
    }

    /// Returns the 7 diatonic 13th chords in this key
    pub fn diatonic_thirteenths(&self) -> Vec<Chord> {
        let degrees = self.scale_degrees();
        let thirteenth_qualities = match self.scale_type {
            ScaleType::Major => [
                ChordQuality::Major13,    // Imaj13
                ChordQuality::Minor13,    // ii13
                ChordQuality::Minor13,    // iii13
                ChordQuality::Major13,    // IVmaj13
                ChordQuality::Dominant13, // V13
                ChordQuality::Minor13,    // vi13
                ChordQuality::Minor13,    // vii13
            ],
            ScaleType::NaturalMinor => [
                ChordQuality::Minor13,    // i13
                ChordQuality::Minor13,    // ii13
                ChordQuality::Major13,    // III13
                ChordQuality::Minor13,    // iv13
                ChordQuality::Minor13,    // v13
                ChordQuality::Major13,    // VI13
                ChordQuality::Dominant13, // VII13
            ],
        };

        degrees
            .iter()
            .zip(thirteenth_qualities.iter())
            .map(|(&root, &quality)| Chord::new(root, quality))
            .collect()
    }

    /// Roman numeral for a given scale degree (1-based)
    pub fn roman_numeral(&self, degree: usize) -> &'static str {
        match self.scale_type {
            ScaleType::Major => match degree {
                1 => "I",
                2 => "ii",
                3 => "iii",
                4 => "IV",
                5 => "V",
                6 => "vi",
                7 => "vii",
                _ => "?",
            },
            ScaleType::NaturalMinor => match degree {
                1 => "i",
                2 => "ii",
                3 => "III",
                4 => "iv",
                5 => "v",
                6 => "VI",
                7 => "VII",
                _ => "?",
            },
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.root, self.scale_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_c_major_scale_degrees() {
        let key = Key::new(NoteName::C, ScaleType::Major);
        let degrees = key.scale_degrees();
        assert_eq!(
            degrees,
            vec![
                NoteName::C,
                NoteName::D,
                NoteName::E,
                NoteName::F,
                NoteName::G,
                NoteName::A,
                NoteName::B
            ]
        );
    }

    #[test]
    fn test_a_minor_scale_degrees() {
        let key = Key::new(NoteName::A, ScaleType::NaturalMinor);
        let degrees = key.scale_degrees();
        assert_eq!(
            degrees,
            vec![
                NoteName::A,
                NoteName::B,
                NoteName::C,
                NoteName::D,
                NoteName::E,
                NoteName::F,
                NoteName::G
            ]
        );
    }

    #[test]
    fn test_c_major_diatonic_triads() {
        let key = Key::new(NoteName::C, ScaleType::Major);
        let triads = key.diatonic_triads();
        assert_eq!(triads.len(), 7);
        assert_eq!(triads[0].display_name(), "C");
        assert_eq!(triads[1].display_name(), "Dm");
        assert_eq!(triads[2].display_name(), "Em");
        assert_eq!(triads[3].display_name(), "F");
        assert_eq!(triads[4].display_name(), "G");
        assert_eq!(triads[5].display_name(), "Am");
        assert_eq!(triads[6].display_name(), "Bdim");
    }

    #[test]
    fn test_c_major_diatonic_sevenths() {
        let key = Key::new(NoteName::C, ScaleType::Major);
        let sevenths = key.diatonic_sevenths();
        assert_eq!(sevenths[0].display_name(), "Cmaj7");
        assert_eq!(sevenths[4].display_name(), "G7");
        assert_eq!(sevenths[6].display_name(), "Bm7b5");
    }
}
