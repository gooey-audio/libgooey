//! Chord-tone pitch quantization for live melodic input.

use super::Chord;

/// Snap an intended MIDI note to the nearest pitch belonging to `chord`.
///
/// Every pitch class named by the chord quality is eligible in every octave.
/// When two candidates are equally close, `preferred` wins if it is one of
/// those candidates; otherwise the lower note wins. The result is always in
/// the MIDI range 0..=127.
pub fn quantize_note_to_chord(input: u8, chord: &Chord, preferred: Option<u8>) -> u8 {
    let allowed = chord_pitch_classes(chord);
    let mut best_note = 0_u8;
    let mut best_distance = u16::MAX;

    for note in 0..=127_u8 {
        if !allowed[(note % 12) as usize] {
            continue;
        }

        let distance = u16::from(note.abs_diff(input));
        if distance < best_distance {
            best_note = note;
            best_distance = distance;
            continue;
        }

        if distance == best_distance {
            let candidate_is_preferred = preferred == Some(note);
            let best_is_preferred = preferred == Some(best_note);
            if candidate_is_preferred || (!best_is_preferred && note < best_note) {
                best_note = note;
            }
        }
    }

    best_note
}

fn chord_pitch_classes(chord: &Chord) -> [bool; 12] {
    let mut allowed = [false; 12];
    let root = chord.root.to_index();
    for interval in chord.quality.intervals() {
        let pitch_class = root.wrapping_add(interval.semitones()) % 12;
        allowed[pitch_class as usize] = true;
    }
    allowed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::{ChordQuality, NoteName};

    fn chord(root: NoteName, quality: ChordQuality) -> Chord {
        Chord::new(root, quality)
    }

    #[test]
    fn nearest_chord_tone_wins_across_octaves() {
        let c_major_seven = chord(NoteName::C, ChordQuality::Major7);
        assert_eq!(quantize_note_to_chord(66, &c_major_seven, None), 67);
        assert_eq!(quantize_note_to_chord(1, &c_major_seven, None), 0);
        assert_eq!(quantize_note_to_chord(127, &c_major_seven, None), 127);
    }

    #[test]
    fn ties_are_lower_unless_the_sounding_note_is_tied() {
        let c_major = chord(NoteName::C, ChordQuality::Major);
        // D is equally far from C and E.
        assert_eq!(quantize_note_to_chord(62, &c_major, None), 60);
        assert_eq!(quantize_note_to_chord(62, &c_major, Some(64)), 64);
        assert_eq!(quantize_note_to_chord(62, &c_major, Some(67)), 60);
    }

    #[test]
    fn extensions_and_alterations_are_eligible_pitch_classes() {
        let c_nine = chord(NoteName::C, ChordQuality::Major9);
        assert_eq!(quantize_note_to_chord(62, &c_nine, None), 62);

        let e_sharp_nine = chord(NoteName::E, ChordQuality::Dominant7Sharp9);
        // G is the #9 of E and must remain available even though it is outside E major.
        assert_eq!(quantize_note_to_chord(67, &e_sharp_nine, None), 67);
    }

    #[test]
    fn octave_equivalent_intervals_do_not_change_the_result() {
        let c_sharp_nine = chord(NoteName::C, ChordQuality::Dominant7Sharp9);
        // E-flat/D-sharp appears as the sharp ninth pitch class; it is still one
        // candidate per MIDI note rather than a duplicated entry.
        assert_eq!(quantize_note_to_chord(63, &c_sharp_nine, None), 63);
    }

    #[test]
    fn every_result_is_in_range_and_belongs_to_the_chord() {
        let chord = chord(NoteName::As, ChordQuality::Major7Sharp11);
        let allowed = chord_pitch_classes(&chord);
        for input in 0..=127_u8 {
            let output = quantize_note_to_chord(input, &chord, None);
            assert!(output <= 127);
            assert!(allowed[(output % 12) as usize]);
        }
    }
}
