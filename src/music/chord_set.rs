//! Chord sets: named seven-pad harmonic palettes.
//!
//! A chord app lays out seven pads. A [`ChordSet`] decides what each pad plays
//! for a given key. The five diatonic *levels* (triads through 13ths) are chord
//! sets whose pads are simply the seven scale degrees stacked to a given height.
//! A *stylistic* set such as [`ChordSet::NeoSoul`] is free to reach outside the
//! key — its pad 6 is a borrowed bVII9 — because every pad is stored as an
//! explicit (root offset in semitones, chord quality, label) triple rather than
//! being derived from the scale.
//!
//! Adding a palette is therefore a seven-row table per scale type plus an id.

use std::ffi::CStr;

use super::chord::{Chord, ChordQuality};
use super::cstr::cstr;
use super::key::Key;
use super::scale::ScaleType;

/// Pads per chord set. Fixed at seven so a `degree` argument keeps meaning
/// "pad index" across every set, diatonic or not.
pub const PADS_PER_SET: usize = 7;

/// One pad in a chord set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChordSetEntry {
    /// Display label, e.g. `IVmaj7#11`. Static so the FFI can hand out the
    /// pointer without the host ever freeing it.
    pub label: &'static CStr,
    /// Semitones above the key root for this pad's chord root (0-11).
    pub root_offset: u8,
    pub quality: ChordQuality,
}

impl ChordSetEntry {
    const fn new(label: &'static CStr, root_offset: u8, quality: ChordQuality) -> Self {
        Self {
            label,
            root_offset,
            quality,
        }
    }

    /// The label as a Rust string slice (always valid ASCII).
    pub fn label_str(&self) -> &'static str {
        self.label.to_str().unwrap_or("")
    }
}

/// A named seven-pad harmonic palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChordSet {
    Triads,
    Sevenths,
    Ninths,
    Elevenths,
    Thirteenths,
    /// D'Angelo / Robert Glasper flavoured palette: lush ninths, a suspended
    /// dominant, a lydian IV and a borrowed bVII.
    NeoSoul,
}

impl ChordSet {
    /// Every set, in id order. Index equals [`ChordSet::as_id`].
    pub const ALL: [ChordSet; 6] = [
        ChordSet::Triads,
        ChordSet::Sevenths,
        ChordSet::Ninths,
        ChordSet::Elevenths,
        ChordSet::Thirteenths,
        ChordSet::NeoSoul,
    ];

    /// Map a stable numeric id (as used over the C FFI) to a set.
    /// Returns `None` for an unknown id — a wrong palette is audible, so
    /// callers reject rather than silently substitute one.
    pub fn from_id(id: u32) -> Option<Self> {
        Self::ALL.get(id as usize).copied()
    }

    /// The stable numeric id for this set.
    pub fn as_id(self) -> u32 {
        match self {
            ChordSet::Triads => 0,
            ChordSet::Sevenths => 1,
            ChordSet::Ninths => 2,
            ChordSet::Elevenths => 3,
            ChordSet::Thirteenths => 4,
            ChordSet::NeoSoul => 5,
        }
    }

    /// Human-readable set name for a UI picker.
    pub fn name(self) -> &'static CStr {
        match self {
            ChordSet::Triads => cstr!("Triads"),
            ChordSet::Sevenths => cstr!("7ths"),
            ChordSet::Ninths => cstr!("9ths"),
            ChordSet::Elevenths => cstr!("11ths"),
            ChordSet::Thirteenths => cstr!("13ths"),
            ChordSet::NeoSoul => cstr!("Neo Soul"),
        }
    }

    /// The set name as a Rust string slice (always valid ASCII).
    pub fn name_str(self) -> &'static str {
        self.name().to_str().unwrap_or("")
    }

    /// The next set in [`ChordSet::ALL`], wrapping around.
    pub fn next(self) -> Self {
        Self::ALL[(self.as_id() as usize + 1) % Self::ALL.len()]
    }

    /// The previous set in [`ChordSet::ALL`], wrapping around.
    pub fn prev(self) -> Self {
        Self::ALL[(self.as_id() as usize + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    /// The seven pad definitions for this set in the given scale type.
    ///
    /// Stylistic sets return their table directly. Diatonic levels are built
    /// on the fly: pad `i` sits on scale degree `i`, so its root offset is the
    /// scale's own interval and its label is the plain roman numeral; only the
    /// chord quality varies from level to level.
    pub fn entries(self, scale: ScaleType) -> [ChordSetEntry; PADS_PER_SET] {
        use ChordQuality as Q;
        let qualities: [ChordQuality; PADS_PER_SET] = match (self, scale) {
            (ChordSet::NeoSoul, ScaleType::Major) => return NEO_SOUL_MAJOR,
            (ChordSet::NeoSoul, ScaleType::NaturalMinor) => return NEO_SOUL_MINOR,
            (ChordSet::Triads, ScaleType::Major) => [
                Q::Major,      // I
                Q::Minor,      // ii
                Q::Minor,      // iii
                Q::Major,      // IV
                Q::Major,      // V
                Q::Minor,      // vi
                Q::Diminished, // vii
            ],
            (ChordSet::Triads, ScaleType::NaturalMinor) => [
                Q::Minor,      // i
                Q::Diminished, // ii
                Q::Major,      // III
                Q::Minor,      // iv
                Q::Minor,      // v
                Q::Major,      // VI
                Q::Major,      // VII
            ],
            (ChordSet::Sevenths, ScaleType::Major) => [
                Q::Major7,          // Imaj7
                Q::Minor7,          // ii7
                Q::Minor7,          // iii7
                Q::Major7,          // IVmaj7
                Q::Dominant7,       // V7
                Q::Minor7,          // vi7
                Q::HalfDiminished7, // viim7b5
            ],
            (ChordSet::Sevenths, ScaleType::NaturalMinor) => [
                Q::Minor7,          // i7
                Q::HalfDiminished7, // iim7b5
                Q::Major7,          // IIImaj7
                Q::Minor7,          // iv7
                Q::Minor7,          // v7
                Q::Major7,          // VImaj7
                Q::Dominant7,       // VII7
            ],
            (ChordSet::Ninths, ScaleType::Major) => [
                Q::Major9,    // Imaj9
                Q::Minor9,    // ii9
                Q::Minor9,    // iii9
                Q::Major9,    // IVmaj9
                Q::Dominant9, // V9
                Q::Minor9,    // vi9
                Q::Minor9,    // vii9 (simplified)
            ],
            (ChordSet::Ninths, ScaleType::NaturalMinor) => [
                Q::Minor9,    // i9
                Q::Minor9,    // ii9 (simplified)
                Q::Major9,    // III9
                Q::Minor9,    // iv9
                Q::Minor9,    // v9
                Q::Major9,    // VI9
                Q::Dominant9, // VII9
            ],
            (ChordSet::Elevenths, ScaleType::Major) => [
                Q::Major11,    // Imaj11
                Q::Minor11,    // ii11
                Q::Minor11,    // iii11
                Q::Major11,    // IVmaj11
                Q::Dominant11, // V11
                Q::Minor11,    // vi11
                Q::Minor11,    // vii11
            ],
            (ChordSet::Elevenths, ScaleType::NaturalMinor) => [
                Q::Minor11,    // i11
                Q::Minor11,    // ii11
                Q::Major11,    // III11
                Q::Minor11,    // iv11
                Q::Minor11,    // v11
                Q::Major11,    // VI11
                Q::Dominant11, // VII11
            ],
            (ChordSet::Thirteenths, ScaleType::Major) => [
                Q::Major13,    // Imaj13
                Q::Minor13,    // ii13
                Q::Minor13,    // iii13
                Q::Major13,    // IVmaj13
                Q::Dominant13, // V13
                Q::Minor13,    // vi13
                Q::Minor13,    // vii13
            ],
            (ChordSet::Thirteenths, ScaleType::NaturalMinor) => [
                Q::Minor13,    // i13
                Q::Minor13,    // ii13
                Q::Major13,    // III13
                Q::Minor13,    // iv13
                Q::Minor13,    // v13
                Q::Major13,    // VI13
                Q::Dominant13, // VII13
            ],
        };

        let offsets = scale.intervals();
        let labels = roman_numerals(scale);
        std::array::from_fn(|i| ChordSetEntry::new(labels[i], offsets[i], qualities[i]))
    }

    /// One pad definition. `degree` wraps, so pad 7 is pad 0.
    pub fn entry(self, scale: ScaleType, degree: usize) -> ChordSetEntry {
        self.entries(scale)[degree % PADS_PER_SET]
    }

    /// The seven concrete chords this set produces in `key`.
    pub fn chords(self, key: &Key) -> Vec<Chord> {
        self.entries(key.scale_type)
            .iter()
            .map(|entry| Chord::new(key.root.transpose(entry.root_offset), entry.quality))
            .collect()
    }

    /// The concrete chord for one pad in `key`. `degree` wraps.
    pub fn chord(self, key: &Key, degree: usize) -> Chord {
        let entry = self.entry(key.scale_type, degree);
        Chord::new(key.root.transpose(entry.root_offset), entry.quality)
    }
}

fn roman_numerals(scale: ScaleType) -> &'static [&'static CStr; PADS_PER_SET] {
    match scale {
        ScaleType::Major => &[
            cstr!("I"),
            cstr!("ii"),
            cstr!("iii"),
            cstr!("IV"),
            cstr!("V"),
            cstr!("vi"),
            cstr!("vii"),
        ],
        ScaleType::NaturalMinor => &[
            cstr!("i"),
            cstr!("ii"),
            cstr!("III"),
            cstr!("iv"),
            cstr!("v"),
            cstr!("VI"),
            cstr!("VII"),
        ],
    }
}

/// Neo Soul in a major key. In C: Cmaj9, Dm9, E7#9, Fmaj7#11, G9sus4, Am9, A#9.
///
/// Pad 2 is a secondary dominant (V7#9 of vi) that pulls hard to pad 5; pad 3
/// raises the IV chord's eleventh to get the lydian shimmer; pad 4 suspends the
/// dominant so it floats instead of resolving; pad 6 is borrowed from the
/// parallel minor. Every entry is five notes so one of the poly synth's six
/// voices stays free to carry the previous chord's release tail.
const NEO_SOUL_MAJOR: [ChordSetEntry; PADS_PER_SET] = [
    ChordSetEntry::new(cstr!("Imaj9"), 0, ChordQuality::Major9),
    ChordSetEntry::new(cstr!("ii9"), 2, ChordQuality::Minor9),
    ChordSetEntry::new(cstr!("III7#9"), 4, ChordQuality::Dominant7Sharp9),
    ChordSetEntry::new(cstr!("IVmaj7#11"), 5, ChordQuality::Major7Sharp11),
    ChordSetEntry::new(cstr!("V9sus4"), 7, ChordQuality::Dominant9Sus4),
    ChordSetEntry::new(cstr!("vi9"), 9, ChordQuality::Minor9),
    ChordSetEntry::new(cstr!("bVII9"), 10, ChordQuality::Dominant9),
];

/// Neo Soul in a natural-minor key. In A minor: Am9, Bm9b5, Cmaj9, Dm9, E7#9,
/// Fmaj7#11, G9. Pad 4 raises the minor v to a dominant with a sharp ninth for
/// the classic cadence; pad 1 keeps the ninth on the half-diminished ii.
const NEO_SOUL_MINOR: [ChordSetEntry; PADS_PER_SET] = [
    ChordSetEntry::new(cstr!("i9"), 0, ChordQuality::Minor9),
    ChordSetEntry::new(cstr!("iim9b5"), 2, ChordQuality::Minor9Flat5),
    ChordSetEntry::new(cstr!("IIImaj9"), 3, ChordQuality::Major9),
    ChordSetEntry::new(cstr!("iv9"), 5, ChordQuality::Minor9),
    ChordSetEntry::new(cstr!("V7#9"), 7, ChordQuality::Dominant7Sharp9),
    ChordSetEntry::new(cstr!("VImaj7#11"), 8, ChordQuality::Major7Sharp11),
    ChordSetEntry::new(cstr!("VII9"), 10, ChordQuality::Dominant9),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::note::NoteName;
    use crate::music::voicing::{apply_voicing, VoicingType};

    fn c_major() -> Key {
        Key::new(NoteName::C, ScaleType::Major)
    }

    fn a_minor() -> Key {
        Key::new(NoteName::A, ScaleType::NaturalMinor)
    }

    #[test]
    fn id_roundtrip() {
        for (index, set) in ChordSet::ALL.iter().enumerate() {
            assert_eq!(set.as_id() as usize, index);
            assert_eq!(ChordSet::from_id(set.as_id()), Some(*set));
        }
        assert_eq!(ChordSet::from_id(ChordSet::ALL.len() as u32), None);
        assert_eq!(ChordSet::from_id(u32::MAX), None);
    }

    #[test]
    fn next_and_prev_wrap() {
        assert_eq!(ChordSet::Thirteenths.next(), ChordSet::NeoSoul);
        assert_eq!(ChordSet::NeoSoul.next(), ChordSet::Triads);
        assert_eq!(ChordSet::Triads.prev(), ChordSet::NeoSoul);
    }

    #[test]
    fn every_set_has_seven_well_formed_pads() {
        for set in ChordSet::ALL {
            assert!(!set.name_str().is_empty());
            for scale in [ScaleType::Major, ScaleType::NaturalMinor] {
                let entries = set.entries(scale);
                assert_eq!(entries.len(), PADS_PER_SET);
                for entry in entries {
                    assert!(
                        entry.root_offset < 12,
                        "{set:?} {scale} offset out of range"
                    );
                    assert!(!entry.label_str().is_empty(), "{set:?} {scale} empty label");
                }
                assert_eq!(
                    set.chords(&Key::new(NoteName::C, scale)).len(),
                    PADS_PER_SET
                );
            }
        }
    }

    #[test]
    fn diatonic_sets_match_the_key_helpers() {
        for key in [
            c_major(),
            a_minor(),
            Key::new(NoteName::Fs, ScaleType::Major),
            Key::new(NoteName::Ds, ScaleType::NaturalMinor),
        ] {
            assert_eq!(ChordSet::Triads.chords(&key), key.diatonic_triads());
            assert_eq!(ChordSet::Sevenths.chords(&key), key.diatonic_sevenths());
            assert_eq!(ChordSet::Ninths.chords(&key), key.diatonic_ninths());
            assert_eq!(ChordSet::Elevenths.chords(&key), key.diatonic_elevenths());
            assert_eq!(
                ChordSet::Thirteenths.chords(&key),
                key.diatonic_thirteenths()
            );
        }
    }

    #[test]
    fn neo_soul_major_in_c() {
        let names: Vec<String> = ChordSet::NeoSoul
            .chords(&c_major())
            .iter()
            .map(Chord::display_name)
            .collect();
        assert_eq!(
            names,
            vec!["Cmaj9", "Dm9", "E7#9", "Fmaj7#11", "G9sus4", "Am9", "A#9"]
        );
    }

    #[test]
    fn neo_soul_minor_in_a() {
        let names: Vec<String> = ChordSet::NeoSoul
            .chords(&a_minor())
            .iter()
            .map(Chord::display_name)
            .collect();
        assert_eq!(
            names,
            vec!["Am9", "Bm9b5", "Cmaj9", "Dm9", "E7#9", "Fmaj7#11", "G9"]
        );
    }

    #[test]
    fn neo_soul_pads_leave_a_voice_free_for_the_release_tail() {
        for scale in [ScaleType::Major, ScaleType::NaturalMinor] {
            for entry in ChordSet::NeoSoul.entries(scale) {
                assert!(
                    entry.quality.note_count() <= 5,
                    "{} has {} notes",
                    entry.label_str(),
                    entry.quality.note_count()
                );
            }
        }
    }

    #[test]
    fn chord_degree_wraps_past_the_last_pad() {
        let key = c_major();
        assert_eq!(
            ChordSet::NeoSoul.chord(&key, 7),
            ChordSet::NeoSoul.chord(&key, 0)
        );
        assert_eq!(
            ChordSet::NeoSoul.chord(&key, 13),
            ChordSet::NeoSoul.chord(&key, 6)
        );
    }

    #[test]
    fn every_voicing_of_every_pad_stays_in_midi_range() {
        for set in ChordSet::ALL {
            for scale in [ScaleType::Major, ScaleType::NaturalMinor] {
                for root in NoteName::ALL {
                    let key = Key::new(root, scale);
                    for chord in set.chords(&key) {
                        for voicing in VoicingType::all() {
                            for octave in [0i8, 4, 8] {
                                let notes = apply_voicing(&chord, *voicing, octave);
                                assert!(!notes.is_empty());
                                assert!(notes.iter().all(|&n| n <= 127));
                            }
                        }
                    }
                }
            }
        }
    }
}
