//! Chord-tone pitch quantization for live melodic input.

use super::{Chord, Key};

/// Snap an intended MIDI note to the nearest pitch belonging to `chord`.
///
/// Every pitch class named by the chord quality is eligible in every octave.
/// When two candidates are equally close, `preferred` wins if it is one of
/// those candidates; otherwise the lower note wins. The result is always in
/// the MIDI range 0..=127.
pub fn quantize_note_to_chord(input: u8, chord: &Chord, preferred: Option<u8>) -> u8 {
    let chord_tones = chord_pitch_classes(chord);
    quantize_to_pitch_classes(input, &chord_tones, &chord_tones, preferred)
}

/// Snap an intended MIDI note to the chord plus the key's available tensions.
///
/// Chord tones are always eligible. A diatonic note joins them unless it would
/// clash with the chord while held: a half step above a chord tone (the 4th
/// over a major third, for example), or a half step from a chromatic chord tone
/// it has been altered away from (G under E7's G-sharp in C major).
///
/// Ties prefer `preferred`, then a chord tone, then the lower note, so a
/// chromatic input between a chord tone and a tension leans into the chord.
pub fn quantize_note_to_key(input: u8, chord: &Chord, key: &Key, preferred: Option<u8>) -> u8 {
    let chord_tones = chord_pitch_classes(chord);
    let allowed = key_pitch_classes(&chord_tones, key);
    quantize_to_pitch_classes(input, &allowed, &chord_tones, preferred)
}

fn quantize_to_pitch_classes(
    input: u8,
    allowed: &[bool; 12],
    chord_tones: &[bool; 12],
    preferred: Option<u8>,
) -> u8 {
    let is_chord_tone = |note: u8| chord_tones[(note % 12) as usize];
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
            if candidate_is_preferred
                || (!best_is_preferred && is_chord_tone(note) && !is_chord_tone(best_note))
            {
                best_note = note;
            }
            // Otherwise the lower note, already in `best_note`, stands.
        }
    }

    best_note
}

fn chord_pitch_classes(chord: &Chord) -> [bool; 12] {
    let mut allowed = [false; 12];
    let root = chord.root.to_index();
    for interval in chord.quality.interval_slice() {
        let pitch_class = root.wrapping_add(interval.semitones()) % 12;
        allowed[pitch_class as usize] = true;
    }
    allowed
}

fn key_pitch_classes(chord_tones: &[bool; 12], key: &Key) -> [bool; 12] {
    let mut diatonic = [false; 12];
    let root = key.root.to_index();
    for offset in key.scale_type.intervals() {
        diatonic[((root + offset) % 12) as usize] = true;
    }

    let mut allowed = *chord_tones;
    for pitch_class in 0..12 {
        if !diatonic[pitch_class] || chord_tones[pitch_class] {
            continue;
        }
        let below = (pitch_class + 11) % 12;
        let above = (pitch_class + 1) % 12;
        let avoid_note = chord_tones[below];
        let displaced = chord_tones[above] && !diatonic[above];
        allowed[pitch_class] = !avoid_note && !displaced;
    }
    allowed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::{ChordQuality, NoteName, ScaleType};

    fn chord(root: NoteName, quality: ChordQuality) -> Chord {
        Chord::new(root, quality)
    }

    fn c_major() -> Key {
        Key::new(NoteName::C, ScaleType::Major)
    }

    fn pitch_classes(allowed: [bool; 12]) -> Vec<u8> {
        (0..12).filter(|&pc| allowed[pc as usize]).collect()
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

    #[test]
    fn key_adds_tensions_but_not_avoid_notes() {
        let key = c_major();
        let cases = [
            // Triads gain the 9th and 6th; F is a half step over E.
            (
                chord(NoteName::C, ChordQuality::Major),
                vec![0, 2, 4, 7, 9, 11],
            ),
            (
                chord(NoteName::D, ChordQuality::Minor7),
                vec![0, 2, 4, 5, 7, 9, 11],
            ),
            // C is a half step over G7's B.
            (
                chord(NoteName::G, ChordQuality::Dominant7),
                vec![2, 4, 5, 7, 9, 11],
            ),
            // Lydian #11 is fine over IV.
            (
                chord(NoteName::F, ChordQuality::Major),
                vec![0, 2, 4, 5, 7, 9, 11],
            ),
            // Phrygian iii loses both its b9 (F) and b13 (C).
            (
                chord(NoteName::E, ChordQuality::Minor),
                vec![2, 4, 7, 9, 11],
            ),
        ];
        for (chord, expected) in cases {
            let allowed = key_pitch_classes(&chord_pitch_classes(&chord), &key);
            assert_eq!(pitch_classes(allowed), expected, "{}", chord.display_name());
        }
    }

    #[test]
    fn chromatic_chord_tones_displace_the_diatonic_neighbor() {
        // E7 in C major: G-sharp replaces G, and A sits a half step above it.
        let e_seven = chord(NoteName::E, ChordQuality::Dominant7);
        let allowed = key_pitch_classes(&chord_pitch_classes(&e_seven), &c_major());
        assert_eq!(pitch_classes(allowed), vec![2, 4, 8, 11]);
        assert_eq!(quantize_note_to_key(67, &e_seven, &c_major(), None), 68);
    }

    #[test]
    fn key_quantization_reaches_tensions_and_ties_lean_into_the_chord() {
        let c_major_triad = chord(NoteName::C, ChordQuality::Major);
        let key = c_major();
        // D and A are reachable, which chord-only quantization would replace.
        assert_eq!(quantize_note_to_key(62, &c_major_triad, &key, None), 62);
        assert_eq!(quantize_note_to_key(69, &c_major_triad, &key, None), 69);
        assert_eq!(quantize_note_to_chord(62, &c_major_triad, None), 60);
        // F is an avoid note over the major third, so it falls back to E.
        assert_eq!(quantize_note_to_key(65, &c_major_triad, &key, None), 64);
        // C-sharp ties C and D: the chord tone wins over the lower note.
        let d_minor = chord(NoteName::D, ChordQuality::Minor);
        assert_eq!(quantize_note_to_key(61, &d_minor, &key, None), 62);
        // ...unless the sounding note is the other tied candidate.
        assert_eq!(quantize_note_to_key(61, &d_minor, &key, Some(60)), 60);
    }

    #[test]
    fn every_key_result_is_in_range_and_eligible() {
        let key = Key::new(NoteName::Fs, ScaleType::NaturalMinor);
        let chord = chord(NoteName::Cs, ChordQuality::Dominant7Sharp9);
        let allowed = key_pitch_classes(&chord_pitch_classes(&chord), &key);
        for input in 0..=127_u8 {
            let output = quantize_note_to_key(input, &chord, &key, None);
            assert!(output <= 127);
            assert!(allowed[(output % 12) as usize]);
        }
    }
}
