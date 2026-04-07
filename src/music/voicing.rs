use std::fmt;

use super::chord::{Chord, ChordQuality};
use super::note::note_to_midi;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoicingType {
    RootPosition,
    FirstInversion,
    SecondInversion,
    ThirdInversion,
    OpenVoicing,
    Drop2,
    Drop3,
    Spread,
    /// Shell: root + 3rd + 7th only (strips the 5th and extensions)
    Shell,
    /// Rootless: drop the root, keep upper structure
    Rootless,
}

impl VoicingType {
    pub fn all() -> &'static [VoicingType] {
        &[
            VoicingType::RootPosition,
            VoicingType::FirstInversion,
            VoicingType::SecondInversion,
            VoicingType::ThirdInversion,
            VoicingType::OpenVoicing,
            VoicingType::Drop2,
            VoicingType::Drop3,
            VoicingType::Spread,
            VoicingType::Shell,
            VoicingType::Rootless,
        ]
    }
}

impl fmt::Display for VoicingType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VoicingType::RootPosition => write!(f, "Root"),
            VoicingType::FirstInversion => write!(f, "1st Inv"),
            VoicingType::SecondInversion => write!(f, "2nd Inv"),
            VoicingType::ThirdInversion => write!(f, "3rd Inv"),
            VoicingType::OpenVoicing => write!(f, "Open"),
            VoicingType::Drop2 => write!(f, "Drop 2"),
            VoicingType::Drop3 => write!(f, "Drop 3"),
            VoicingType::Spread => write!(f, "Spread"),
            VoicingType::Shell => write!(f, "Shell"),
            VoicingType::Rootless => write!(f, "Rootless"),
        }
    }
}

/// Returns the voicing types that are valid for a given chord quality
pub fn available_voicings(quality: &ChordQuality) -> Vec<VoicingType> {
    let note_count = quality.note_count();
    let mut voicings = vec![VoicingType::RootPosition, VoicingType::FirstInversion];

    if note_count >= 3 {
        voicings.push(VoicingType::SecondInversion);
        voicings.push(VoicingType::OpenVoicing);
        voicings.push(VoicingType::Spread);
        voicings.push(VoicingType::Rootless);
    }

    if note_count >= 4 {
        voicings.push(VoicingType::ThirdInversion);
        voicings.push(VoicingType::Drop2);
        voicings.push(VoicingType::Shell);
    }

    if note_count >= 5 {
        voicings.push(VoicingType::Drop3);
    }

    voicings
}

/// Apply a voicing strategy to a chord, returning MIDI note numbers
pub fn apply_voicing(chord: &Chord, voicing: VoicingType, octave: i8) -> Vec<u8> {
    let root_midi = note_to_midi(chord.root, octave);
    let intervals: Vec<u8> = chord
        .quality
        .intervals()
        .iter()
        .map(|i| i.semitones())
        .collect();

    // Build close-voiced MIDI notes
    let mut notes: Vec<u8> = intervals.iter().map(|&i| root_midi + i).collect();

    match voicing {
        VoicingType::RootPosition => {
            // Already in root position
        }
        VoicingType::FirstInversion => {
            if !notes.is_empty() {
                notes[0] += 12;
                notes.sort();
            }
        }
        VoicingType::SecondInversion => {
            if notes.len() >= 2 {
                notes[0] += 12;
                notes[1] += 12;
                notes.sort();
            }
        }
        VoicingType::ThirdInversion => {
            if notes.len() >= 4 {
                notes[0] += 12;
                notes[1] += 12;
                notes[2] += 12;
                notes.sort();
            }
        }
        VoicingType::OpenVoicing => {
            for i in (1..notes.len()).step_by(2) {
                notes[i] += 12;
            }
            notes.sort();
        }
        VoicingType::Drop2 => {
            if notes.len() >= 4 {
                let idx = notes.len() - 2;
                notes[idx] = notes[idx].saturating_sub(12);
                notes.sort();
            }
        }
        VoicingType::Drop3 => {
            if notes.len() >= 5 {
                let idx = notes.len() - 3;
                notes[idx] = notes[idx].saturating_sub(12);
                notes.sort();
            }
        }
        VoicingType::Spread => {
            for (i, note) in notes.iter_mut().enumerate() {
                let octave_offset = (i / 2) as u8 * 12;
                *note = note.saturating_add(octave_offset);
            }
            notes.sort();
        }
        VoicingType::Shell => {
            // Keep root, 3rd, and 7th only (jazz comping voicing)
            if intervals.len() >= 4 {
                notes = vec![
                    root_midi + intervals[0], // root
                    root_midi + intervals[1], // 3rd
                    root_midi + intervals[3], // 7th
                ];
            }
            // For triads, just use root and 3rd with the 5th up an octave
            else if intervals.len() >= 3 {
                notes = vec![
                    root_midi + intervals[0],
                    root_midi + intervals[1],
                    root_midi + intervals[2] + 12,
                ];
            }
        }
        VoicingType::Rootless => {
            // Drop the root, shift everything else down into the pocket
            if notes.len() >= 3 {
                notes.remove(0);
                // Move the bottom note down an octave to fill the bass range
                notes[0] = notes[0].saturating_sub(12);
                notes.sort();
            }
        }
    }

    // Clamp all notes to valid MIDI range
    notes.iter().map(|&n| n.min(127)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::note::NoteName;

    #[test]
    fn test_root_position_c_major() {
        let chord = Chord::new(NoteName::C, ChordQuality::Major);
        let notes = apply_voicing(&chord, VoicingType::RootPosition, 4);
        assert_eq!(notes, vec![60, 64, 67]); // C4, E4, G4
    }

    #[test]
    fn test_first_inversion_c_major() {
        let chord = Chord::new(NoteName::C, ChordQuality::Major);
        let notes = apply_voicing(&chord, VoicingType::FirstInversion, 4);
        assert_eq!(notes, vec![64, 67, 72]); // E4, G4, C5
    }

    #[test]
    fn test_second_inversion_c_major() {
        let chord = Chord::new(NoteName::C, ChordQuality::Major);
        let notes = apply_voicing(&chord, VoicingType::SecondInversion, 4);
        assert_eq!(notes, vec![67, 72, 76]); // G4, C5, E5
    }

    #[test]
    fn test_available_voicings_triad() {
        let voicings = available_voicings(&ChordQuality::Major);
        assert!(voicings.contains(&VoicingType::RootPosition));
        assert!(voicings.contains(&VoicingType::OpenVoicing));
        assert!(voicings.contains(&VoicingType::Rootless));
        assert!(!voicings.contains(&VoicingType::ThirdInversion));
        assert!(!voicings.contains(&VoicingType::Drop2));
    }

    #[test]
    fn test_available_voicings_seventh() {
        let voicings = available_voicings(&ChordQuality::Major7);
        assert!(voicings.contains(&VoicingType::ThirdInversion));
        assert!(voicings.contains(&VoicingType::Drop2));
        assert!(voicings.contains(&VoicingType::Shell));
    }

    #[test]
    fn test_drop2_cmaj7() {
        let chord = Chord::new(NoteName::C, ChordQuality::Major7);
        let notes = apply_voicing(&chord, VoicingType::Drop2, 4);
        // Close voiced: C4(60), E4(64), G4(67), B4(71)
        // Drop2: move G4(67) down to G3(55)
        assert_eq!(notes, vec![55, 60, 64, 71]); // G3, C4, E4, B4
    }

    #[test]
    fn test_shell_cmaj7() {
        let chord = Chord::new(NoteName::C, ChordQuality::Major7);
        let notes = apply_voicing(&chord, VoicingType::Shell, 4);
        // root(C4=60), 3rd(E4=64), 7th(B4=71)
        assert_eq!(notes, vec![60, 64, 71]);
    }

    #[test]
    fn test_rootless_c_major() {
        let chord = Chord::new(NoteName::C, ChordQuality::Major);
        let notes = apply_voicing(&chord, VoicingType::Rootless, 4);
        // Remove root C4, left with E4(64), G4(67)
        // Move bottom (E4) down octave -> E3(52)
        assert_eq!(notes, vec![52, 67]); // E3, G4
    }

}
