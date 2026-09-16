use std::fmt;

use super::chord::Chord;
use super::chord_set::ChordSet;
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
        ChordSet::Triads.chords(self)
    }

    /// Returns the 7 diatonic 7th chords in this key
    pub fn diatonic_sevenths(&self) -> Vec<Chord> {
        ChordSet::Sevenths.chords(self)
    }

    /// Returns the 7 diatonic 9th chords in this key
    pub fn diatonic_ninths(&self) -> Vec<Chord> {
        ChordSet::Ninths.chords(self)
    }

    /// Returns the 7 diatonic 11th chords in this key
    pub fn diatonic_elevenths(&self) -> Vec<Chord> {
        ChordSet::Elevenths.chords(self)
    }

    /// Returns the 7 diatonic 13th chords in this key
    pub fn diatonic_thirteenths(&self) -> Vec<Chord> {
        ChordSet::Thirteenths.chords(self)
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
