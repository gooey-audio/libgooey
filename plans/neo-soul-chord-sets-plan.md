# Add selectable chord sets to the chord engine, starting with Neo Soul

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. This document must be maintained in accordance with `.agent/PLANS.md` from the repository root.

## Purpose / Big Picture

A chord app built on libgooey lays out seven pads and plays one chord per pad. Before this change every pad press, on both the poly synth and the sampled piano, produced one of the seven *diatonic seventh chords* of a major or natural-minor key — the chords you get by stacking four scale notes in thirds on each degree. That is all the C interface could express, so a player could change key, voicing, octave and sound, but never the harmonic flavour. The library already knew about triads, ninths, elevenths and thirteenths, but none of them reached the C interface.

After this change the host picks a **chord set**: a named seven-pad harmonic palette. The five diatonic levels (triads, 7ths, 9ths, 11ths, 13ths) became chord sets, and one stylistic palette shipped: **Neo Soul**, the D'Angelo / Robert Glasper flavour — lush ninths, a suspended dominant that floats instead of resolving, a lydian IV chord, a secondary dominant with a sharp ninth, and a borrowed flat-seventh chord that is not in the key at all. A host can now enumerate the sets, read each pad's label and chord symbol for its UI, trigger pads from any set, and record and replay a performance that remembers which palette it was played from.

You can hear it by running the terminal chord explorer, pressing `>` until the header reads `Set: Neo Soul`, and playing the pads: pad 3 (`III7#9`) pulls hard toward pad 6 (`vi9`), and pad 7 (`bVII9`) sounds borrowed from outside the key.

## Progress

- [x] (2026-09-16 13:33Z) Surveyed the chord path end to end: `src/music/`, the four hard-wired `Key::diatonic_sevenths()` call sites in `src/ffi.rs`, `src/performance/mod.rs`, and the two terminal examples.
- [x] (2026-09-16 13:33Z) Milestone 1 — new `ChordQuality` variants (16 of them), `ChordQuality::ALL`, `suffix()`/`suffix_str()`, the `cstr!` macro, `src/music/chord_set.rs` with the Neo Soul tables, and `Key::diatonic_*` delegating to `ChordSet`.
- [x] (2026-09-16 13:33Z) Milestone 2 — `CHORD_SET_*` constants, `resolve_chord`, the `_set` trigger variants, the six metadata getters, `chord_set` persisted on `ChordClipEvent`, and `gooey_engine_perf_get_event_chord_set`.
- [x] (2026-09-16 13:33Z) Milestone 4 — `examples/chords.rs` and `examples/piano.rs` cycle `ChordSet::ALL` and print pad labels from the set; this plan written.
- [ ] Milestone 3 (optional, deliberately deferred) — Gospel and Lo-fi chord sets as ids 6 and 7. Every chord quality they need already exists; each is a seven-row table per scale type. See "Deferred palettes" below.

## Surprises & Discoveries

- Observation: Rust's `c"..."` C-string literals cannot be used anywhere in this crate's `src/`, even though the toolchain is rustc 1.96 and supports them.
  Evidence: `build.rs` runs cbindgen 0.26, which parses the crate source with `syn` 1.x. The first build after introducing them failed with

      thread 'main' panicked at syn-1.0.109/src/lit.rs:1020:13:
      Unrecognized literal: `c""`

  The fix was `src/music/cstr.rs`, a `cstr!("text")` macro that builds a `&'static CStr` through a `const` item. The `const` forces the nul-check to happen at compile time, so it is as safe as a real `c"..."` literal, and `syn` 1.x can tokenize it because it is an ordinary string literal inside a macro call. Bumping cbindgen to 0.29 (which uses `syn` 2) would also work and is the better long-term fix, but it was out of scope here and would change the generated header for the iOS host without warning.

- Observation: `gooey_engine_piano_active_voices` does not reflect a chord trigger until the next render.
  Evidence: asserting the voice count immediately after `gooey_engine_piano_trigger_chord_set` reported 0 instead of 5; moving the assertion after `render(engine, 512)` reported 5. The pre-existing piano tests in `tests/multisample_piano.rs` already render first, which is why this was not visible before.

- Observation: `ChordQuality::is_seventh_or_higher` had no callers anywhere in the crate. It is still public API, so it was updated for correctness (sus, 6th and add chords are not sevenths even when they carry four or five notes) rather than deleted.

## Decision Log

- Decision: a chord set is a per-trigger argument owned by the host, not engine state.
  Rationale: it matches how `voicing` and `preset` already work, and it means a recorded clip replays the palette it was played with regardless of what the UI shows later. The alternative — engine-level "current set" — would make old clips change sound when the user switches palette.
  Date/Author: 2026-09-16 / Claude.

- Decision: every set has exactly seven pads, so `degree` keeps meaning "pad index".
  Rationale: the whole existing ABI passes `degree` 0-6 and the host UI has seven pads. Non-diatonic chords such as `bVII9` simply occupy a pad rather than needing a new addressing scheme.
  Date/Author: 2026-09-16 / Claude.

- Decision: an unknown `chord_set` id is rejected outright — no sound, no recording, `false`, or a null pointer — rather than falling back to a default like `scale_from_id` does for scales.
  Rationale: a wrong scale shifts the key by a predictable amount and a musician hears something plausible; a wrong palette plays audibly wrong chords. Failing loudly during host development is better than shipping a subtly wrong sound.
  Date/Author: 2026-09-16 / Claude.

- Decision: every Neo Soul pad is capped at five notes.
  Rationale: `NUM_VOICES` in `src/instruments/poly_synth.rs` is 6, and each trigger calls `release_all()` before sounding the new chord. Capping at five leaves one voice to carry the previous chord's release tail, so pad-to-pad movement does not chop off the decay. The existing 11th and 13th sets use six notes and were left alone — changing them would alter sounds hosts already ship.
  Date/Author: 2026-09-16 / Claude.

- Decision: existing C functions keep their exact signatures and behaviour; new `_set` variants are added alongside and the old ones delegate with `CHORD_SET_SEVENTHS`.
  Rationale: the iOS host calls these positionally. `gooey_engine_perf_get_event` in particular is called with eleven positional arguments by `tests/performance_recording.rs` and by the host, so the recorded chord set is exposed through a separate `gooey_engine_perf_get_event_chord_set` rather than a twelfth out-parameter.
  Date/Author: 2026-09-16 / Claude.

- Decision: pad labels come from the chord set, not from `Key::roman_numeral`.
  Rationale: `roman_numeral` returns `vii` for pad 6 regardless of what is on it, which mislabels `bVII9` and `III7#9`. Each set carries its own static labels, so the UI is correct for stylistic palettes.
  Date/Author: 2026-09-16 / Claude.

- Decision: ship Neo Soul only; defer Gospel and Lo-fi.
  Rationale: the user asked for the Neo Soul part. The abstraction and all the chord qualities those palettes need are in place, so each is a table and an id away. The research is preserved below so the next contributor does not have to redo it.
  Date/Author: 2026-09-16 / Claude.

## Outcomes & Retrospective

The Neo Soul palette ships and is reachable from C, from the Rust API, and from the terminal examples. `cargo test --verbose` reports 422 library tests plus the integration suites passing, including 10 new tests in `tests/chord_set_ffi.rs`; `cargo fmt --all -- --check` and `cargo clippy --all-targets --all-features` are clean for every file touched; `include/gooey.h` contains `#define CHORD_SET_NEO_SOUL 5` and all nine new functions.

The design goal — that adding a palette is a seven-row table per scale type — held up. The largest single piece of work was not Neo Soul itself but the sixteen new `ChordQuality` variants, twelve of which exist purely so the deferred palettes are table-only later.

Two things remain. Milestone 3 (Gospel and Lo-fi) is unstarted by choice. And `examples/hihat.rs` does not compile against the current `HiHat2Config` API; this is pre-existing and unrelated, but it means `cargo clippy --all-targets` reports errors for that example. Nothing in this work touched it.

## Context and Orientation

libgooey is a real-time audio engine in Rust that targets desktop (via CPAL) and iOS (via a C interface generated with cbindgen). Read `AGENTS.md` at the repository root for the module map.

The music-theory code lives in `src/music/`. Terms used throughout, defined plainly:

A **note name** (`src/music/note.rs`) is one of the twelve pitch classes, spelled with sharps only: `C, Cs, D, Ds, E, F, Fs, G, Gs, A, As, B`. `NoteName::to_index` gives 0 through 11. There is no flat spelling anywhere in the library, which is why the C interface returns pad roots as numbers and lets the host spell them.

A **scale type** (`src/music/scale.rs`) is `Major` or `NaturalMinor`. Each returns seven semitone offsets from the key root: major is `[0, 2, 4, 5, 7, 9, 11]`, natural minor is `[0, 2, 3, 5, 7, 8, 10]`.

A **key** (`src/music/key.rs`) is a root note plus a scale type, for example C Major or A natural minor.

An **interval** (`src/music/interval.rs`) is a named distance in semitones, from `Unison` (0) up to `MajorThirteenth` (21). Note the ones the altered chords need: `MinorNinth` is 13 (a flat ninth), `MinorTenth` is 15 (used as a sharp ninth), `SharpEleventh` is 18.

A **chord quality** (`src/music/chord.rs`) is the shape of a chord independent of its root — `Major7`, `Minor9`, `Dominant7Sharp9` and so on. `ChordQuality::intervals()` returns its intervals from the root. A **chord** is a root note plus a quality; `Chord::midi_notes(octave)` turns it into MIDI note numbers.

A **voicing** (`src/music/voicing.rs`) rearranges a chord's notes into octaves — root position, inversions, drop 2, shell, rootless and so on. `apply_voicing(&chord, voicing, octave)` returns the final MIDI notes.

A **chord set** (`src/music/chord_set.rs`, added by this plan) is a named table of seven pad definitions. Each pad is a `ChordSetEntry`: a static display label, a root offset in semitones from the key root, and a chord quality.

The C interface is all of `src/ffi.rs`. `build.rs` runs cbindgen over that one file (it declares `cargo:rerun-if-changed=src/ffi.rs`) and writes `include/gooey.h`, which is gitignored and regenerated on every build. Because only `src/ffi.rs` is watched, **every C constant must be declared in `src/ffi.rs` as a literal** — a constant defined elsewhere and re-exported will not reliably appear in the header.

Performance recording lives in `src/performance/mod.rs`. A `PerformanceRecorder` captures chord pad presses into a looping clip locked to the engine transport, and replays them sample-accurately. Each press becomes a `ChordClipEvent` holding everything needed to reproduce it.

### The critical structural invariant

`ChordQuality::intervals()` must return intervals in a fixed structural order:

- index 0 is the root
- index 1 is the third, **or the 2nd/4th that substitutes for it in a suspended chord**
- index 2 is the fifth, possibly altered
- index 3, when present, is the seventh **or the sixth that replaces it in a 6 or 6/9 chord**
- anything after that is an extension, in ascending order

`VoicingType::Shell` in `src/music/voicing.rs` reads indices 0, 1 and 3 directly to build a root-third-seventh comping voicing. A new quality that breaks this layout will silently produce a wrong shell voicing rather than failing to compile. The test `all_qualities_keep_the_structural_interval_order` in `src/music/chord.rs` enforces it across `ChordQuality::ALL`.

## Milestones

### Milestone 1 — Chord qualities and the `ChordSet` abstraction

At the end of this milestone the library can name and spell every chord the stylistic palettes need, and `ChordSet` exists with Neo Soul in it, but nothing has reached the C interface yet.

Add sixteen variants to `ChordQuality` in `src/music/chord.rs`. Four are required by Neo Soul: `Dominant7Sharp9` (root, major 3rd, perfect 5th, minor 7th, sharp 9th — the "Hendrix" chord), `Major7Sharp11` (the lydian major seventh), `Dominant9Sus4` (a ninth chord with the 4th in place of the 3rd), and `Minor9Flat5` (a half-diminished chord with a ninth on top). The other twelve exist so the deferred palettes need no new qualities: `Sus2`, `Sus4`, `Dominant7Sus4`, `Major6`, `Minor6`, `Major69`, `Minor69`, `Add9`, `MinorAdd9`, `Dominant7Flat9`, `Dominant7Sharp5`, `MinorMajor9`.

Add `ChordQuality::ALL`, an array of every variant, so tests can be exhaustive. Add `suffix(self) -> &'static CStr` returning the chord-symbol suffix (`"maj9"`, `"7#9"`, `"m7b5"`, empty for a plain major triad) and `suffix_str(self) -> &'static str`, with `Display` delegating to `suffix_str` so existing `display_name()` output is unchanged for existing qualities. Update `is_seventh_or_higher` so sus, 6th, 6/9 and add chords return false.

Add `src/music/cstr.rs` with the `cstr!` macro described under Surprises, and use it wherever a `&'static CStr` literal is needed.

Add `src/music/chord_set.rs` with `PADS_PER_SET = 7`, `ChordSetEntry { label, root_offset, quality }`, and `ChordSet` with six variants and the Neo Soul tables. Rewrite the five `Key::diatonic_*` methods in `src/music/key.rs` as one-line delegations to the matching set so every existing caller and test keeps working. Export the new names from `src/music/mod.rs`.

Acceptance: `cargo test --lib music::` passes. The new test `neo_soul_major_in_c` shows that `ChordSet::NeoSoul.chords(&Key::new(NoteName::C, ScaleType::Major))` spells `Cmaj9, Dm9, E7#9, Fmaj7#11, G9sus4, Am9, A#9`, and `diatonic_sets_match_the_key_helpers` shows the five diatonic sets are byte-identical to the old `Key::diatonic_*` output in four different keys.

### Milestone 2 — The C interface and performance persistence

At the end of this milestone a C host can enumerate chord sets, read pad metadata, trigger any pad of any set on the poly synth and the piano, and round-trip the set id through a recorded clip.

In `src/ffi.rs`, declare `CHORD_SET_TRIADS = 0` through `CHORD_SET_NEO_SOUL = 5`, plus `CHORD_SET_COUNT = 6` and `CHORD_SET_PAD_COUNT = 7`, as literals. Add a private `resolve_chord(chord_set, root, scale_type, degree) -> Option<Chord>` and route every chord lookup through it — there were four hard-wired `Key::diatonic_sevenths()` call sites: the poly trigger, the piano trigger, the voicing-count query, and the clip replay path `trigger_poly_chord_from_event`.

Add `gooey_engine_poly_trigger_chord_set` and `gooey_engine_piano_trigger_chord_set`, taking `chord_set` immediately before `root`, and make the existing `gooey_engine_poly_trigger_chord` and `gooey_engine_piano_trigger_chord` delegate to them with `CHORD_SET_SEVENTHS`. Add `gooey_chord_set_available_voicing_count` and make `gooey_engine_poly_available_voicing_count` delegate to it.

Add the UI metadata getters, all returning static pointers the caller must never free, mirroring the convention `gooey_engine_mixer_get_track_name` already uses: `gooey_chord_set_count`, `gooey_chord_set_pad_count`, `gooey_chord_set_name`, `gooey_chord_set_entry_label`, `gooey_chord_set_entry_quality_suffix`, `gooey_chord_set_entry_root` (a pitch class 0-11, so the host can choose flat spellings), and `gooey_chord_set_entry_note_count`.

In `src/performance/mod.rs`, add `pub chord_set: u32` to `ChordClipEvent` and to the private `OpenEvent`, and add `chord_set` as the first argument of `record_chord_on`. Add `gooey_engine_perf_get_event_chord_set` to read it back, returning `CHORD_SET_SEVENTHS` for a null engine or an out-of-range index. Leave `gooey_engine_perf_get_event` untouched.

Acceptance: `cargo test --test chord_set_ffi` reports 10 passing tests, and `include/gooey.h` contains `#define CHORD_SET_NEO_SOUL 5`.

### Milestone 3 (deferred) — Gospel and Lo-fi

Not implemented. See "Deferred palettes".

### Milestone 4 — Examples and this plan

`examples/chords.rs` and `examples/piano.rs` each carried a private `ChordLevel` enum duplicating the diatonic levels. Replace both with `ChordSet`, cycling `ChordSet::ALL` on `<`/`>` (chords) and `,`/`.` (piano), and print `entries[i].label_str()` instead of `key.roman_numeral(i + 1)`.

Acceptance: both examples build, and cycling to `Neo Soul` in the chord explorer lists `Imaj9, ii9, III7#9, IVmaj7#11, V9sus4, vi9, bVII9` rather than seven roman numerals.

## The Neo Soul pad tables

In a major key, taking C major as the worked example. Each row is a pad index, its label, its root offset in semitones from the key root, its `ChordQuality`, and the notes it produces in C.

| Pad | Label | Offset | Quality | Notes in C |
|---|---|---|---|---|
| 0 | Imaj9 | 0 | `Major9` | C E G B D |
| 1 | ii9 | 2 | `Minor9` | D F A C E |
| 2 | III7#9 | 4 | `Dominant7Sharp9` | E G# B D G |
| 3 | IVmaj7#11 | 5 | `Major7Sharp11` | F A C E B |
| 4 | V9sus4 | 7 | `Dominant9Sus4` | G C D F A |
| 5 | vi9 | 9 | `Minor9` | A C E G B |
| 6 | bVII9 | 10 | `Dominant9` | A# D F G# C |

Pad 2 is a secondary dominant: it is the V7 of pad 5, with a sharp ninth for grit, so it pulls hard toward `vi9`. Pad 3 raises the IV chord's eleventh to get the lydian shimmer. Pad 4 suspends the dominant so it floats instead of resolving. Pad 6 is borrowed from the parallel minor and is not in the key at all.

In a natural-minor key, taking A minor as the worked example.

| Pad | Label | Offset | Quality | Notes in A minor |
|---|---|---|---|---|
| 0 | i9 | 0 | `Minor9` | A C E G B |
| 1 | iim9b5 | 2 | `Minor9Flat5` | B D F A C# |
| 2 | IIImaj9 | 3 | `Major9` | C E G B D |
| 3 | iv9 | 5 | `Minor9` | D F A C E |
| 4 | V7#9 | 7 | `Dominant7Sharp9` | E G# B D G |
| 5 | VImaj7#11 | 8 | `Major7Sharp11` | F A C E B |
| 6 | VII9 | 10 | `Dominant9` | G B D F A |

Pad 4 raises the natural minor's flat v to a true dominant with a sharp ninth, which is what makes the minor cadence land.

## Deferred palettes: research for the next contributor

The abstraction was built so that adding a palette is a seven-row table per scale type plus an id. This is the survey that informed it, so the work does not have to be redone. Tier B means the qualities already exist and only a table is needed. Tier C means something structural is missing first.

| Set | Character | Hallmark qualities | Non-diatonic pads | Tier |
|---|---|---|---|---|
| Gospel | Passing diminished chords, secondary dominants, a 6/9 tonic, flat-ninth dominants | 6/9, m9, dim7, 7b9, 9 | #ii°7, VI9, II9, bII9 | B |
| Lo-fi / Chillhop | Mellow sevenths with sprinkled ninths and sus, a borrowed bVIImaj7, a minor v with no leading tone | maj7, maj9, m7, m9, 7sus4 | bVIImaj7 | B |
| R&B / contemporary pop | Open add9 and sus2 colour, suspended dominants | add9, madd9, sus2, sus4, m7 | bVIIadd9 | B |
| Bossa / Latin jazz | maj7 and 6/9 tonic, ii-V with a flat ninth, minor 6/9 | maj9, 6/9, m6/9, 7b9, m7b5 | V7b9 in minor | B |
| Deep house / garage | Dorian minor 7th and 9th stabs | m7, m9, maj7, 9sus4 | dorian ii m7 at offset 2 in minor | B |
| Funk | Dominant vamps throughout | 9, 7#9, 13, 9sus4 | I as 9 or 7#9, bVII9 | B |
| Cinematic / ambient | Suspended and added-note chords with no thirds | sus2, sus4, add9, madd9 | bVIIsus2 | B |
| Blues | All-dominant I, IV and V with ninths | 7, 9, 7#9, dim7 | I7, IV7, #iv°7 | B |
| City pop / J-fusion | Royal-road IV-V-iii-vi with 6/9 and a III7 | maj9, 6/9, 9sus4, 7 | III7 | B, overlaps Neo Soul heavily |
| Dark / harmonic minor (trap) | Harmonic-minor triads including the augmented III and the fully diminished vii | aug, dim7, major V | III+, V major, vii°7 | B |
| Modal (Dorian, Lydian, Mixolydian) | Per-mode diatonic sets | as the diatonic levels | — | C: needs new `ScaleType` variants and `SCALE_*` ids |
| Slash / pedal bass (IV/5, I/3) | Gospel and R&B inversions with an explicit bass note | — | — | C: needs a bass-note dimension, which is a voicing-layer feature |
| Quartal (McCoy Tyner) | Stacked fourths | — | — | C: a new `VoicingType`, not a chord set |

Concrete starting tables for the two Tier B sets that were scoped for Milestone 3. Gospel in major: I6/9 (`Major69`), ii9 (`Minor9`), #ii°7 at offset 3 (`Diminished7`), IVmaj9 (`Major9`), V7b9 (`Dominant7Flat9`), VI9 at offset 9 (`Dominant9`, the V7 of ii), II9 at offset 2 (`Dominant9`, the V7 of V). Gospel in minor: i9, iiø7 (`HalfDiminished7`), III6/9 (`Major69`), iv9, V7b9 at offset 7, VImaj9, bII9 at offset 1 (`Dominant9`, a tritone substitute). Lo-fi in major: Imaj7, ii9, iiim7, IVmaj9, V7sus4 (`Dominant7Sus4`), vi9, bVIImaj7 at offset 10. Lo-fi in minor: i9, iiø7, IIImaj7, ivm7, v9 (`Minor9`, the natural v), VImaj9, VII7sus4.

Adding one of these means: a new `ChordSet` variant, an entry in `ChordSet::ALL` (append it, so existing ids do not move), an arm in `as_id`, a name in `name()`, two `const [ChordSetEntry; PADS_PER_SET]` tables, two early-return arms in `entries()`, one `CHORD_SET_*` literal in `src/ffi.rs`, and bumping `CHORD_SET_COUNT`. The generic tests in `src/music/chord_set.rs` and `tests/chord_set_ffi.rs` iterate `ChordSet::ALL` and `0..gooey_chord_set_count()`, so they cover a new set automatically; add a spelled-out `..._major_in_c` test like `neo_soul_major_in_c` to pin the actual chords.

## Concrete Steps

Work from the repository root. The exact validation sequence `CLAUDE.md` requires, plus the header check:

    cargo build
    cargo build --example kick --features native,crossterm
    cargo build --no-default-features --features ios
    cargo test --verbose
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features
    grep CHORD_SET include/gooey.h

`cargo test --verbose` should end with the library suite at 422 passed and `tests/chord_set_ffi.rs` at 10 passed, with no failures anywhere. `grep` should print the eight `#define CHORD_SET_*` lines.

Note that `cargo clippy --all-targets` reports pre-existing compile errors in `examples/hihat.rs` against the current `HiHat2Config` API. That example is unrelated to this work and was already broken; check the errors all name `HiHat2Config` or `HiHat2` before dismissing them.

To exercise the Neo Soul palette by ear:

    cargo run --example chords --features native,crossterm

Press `>` five times until the header reads `Set: Neo Soul`. The pad list should read `Imaj9 / ii9 / III7#9 / IVmaj7#11 / V9sus4 / vi9 / bVII9` with the chord names `Cmaj9 / Dm9 / E7#9 / Fmaj7#11 / G9sus4 / Am9 / A#9` beside them. Use Up and Down to select a pad and SPACE to play it. Playing pad 3 then pad 6 should sound like a resolution; pad 7 should sound borrowed. Press `TAB` to switch to minor and the list becomes `i9 / iim9b5 / IIImaj9 / iv9 / V7#9 / VImaj7#11 / VII9`.

## Validation and Acceptance

The behavioural claims this plan makes, and the test that proves each:

That every Neo Soul pad actually sounds, in both scale types, is proven by `every_neo_soul_pad_renders_audio_in_both_scales` in `tests/chord_set_ffi.rs`, which renders 1024 frames per pad and requires a peak above 0.001.

That the palette reaches outside the key is proven by `neo_soul_borrows_the_flat_seventh_and_names_its_pads`, which asserts `gooey_chord_set_entry_root(CHORD_SET_NEO_SOUL, 0, SCALE_MAJOR, 6) == 10` — ten semitones above C is A#, which is not in C major.

That an unknown palette is inert rather than wrong is proven by `an_unknown_set_sounds_nothing_and_records_nothing` (peak exactly 0.0, event count 0) and `metadata_rejects_an_unknown_set` (null pointers and zero counts).

That clips remember their palette is proven by `recorded_events_remember_their_chord_set`, which records one Neo Soul press and one legacy press and reads back `CHORD_SET_NEO_SOUL` and `CHORD_SET_SEVENTHS`, and by `a_recorded_neo_soul_pad_replays_from_the_clip`, which lets punch-out complete and the loop come back around, then requires audible output.

That nothing already shipped changed behaviour is proven by `diatonic_sets_match_the_key_helpers` in `src/music/chord_set.rs`, by `legacy_voicing_count_matches_the_sevenths_set`, by the legacy-path assertions inside `piano_chord_set_trigger_sounds_every_note_and_rejects_a_bad_set`, and by the pre-existing suites in `tests/poly_synth_params.rs`, `tests/performance_recording.rs` and `tests/multisample_piano.rs` continuing to pass untouched.

That new chord qualities are spelled correctly is proven by `neo_soul_qualities_have_expected_notes` and `tier_b_qualities_have_expected_notes` in `src/music/chord.rs`, which pin exact MIDI note numbers from C4 — for example `C7#9` is `[60, 64, 67, 70, 75]` and `Cmaj7#11` is `[60, 64, 67, 71, 78]`.

That the structural invariant holds is proven by `all_qualities_keep_the_structural_interval_order`, and that no voicing of any pad of any set in any key can leave MIDI range or panic is proven by `every_voicing_of_every_pad_stays_in_midi_range`, which sweeps all six sets across both scales, all twelve roots, all ten voicings and octaves 0, 4 and 8.

## Idempotence and Recovery

Every step is a source edit followed by a build; all of them are safe to repeat. `include/gooey.h` is gitignored and regenerated by `build.rs` on every build that touches `src/ffi.rs`, so it never needs manual cleanup — if it looks stale, `touch src/ffi.rs && cargo build`.

The one irreversible-looking change is the extra `chord_set` field on `ChordClipEvent`, but clips are in-memory only and are not serialized to disk anywhere in the crate, so there is no migration to perform and no stored data to invalidate.

## Interfaces and Dependencies

No dependency was added or changed. `Cargo.toml` still pins `cbindgen = "0.26"` as a build dependency; see the first entry under Surprises for why that matters.

The Rust API that must exist at the end of this work, in `crate::music`:

    pub const PADS_PER_SET: usize = 7;

    pub struct ChordSetEntry {
        pub label: &'static CStr,
        pub root_offset: u8,
        pub quality: ChordQuality,
    }
    impl ChordSetEntry {
        pub fn label_str(&self) -> &'static str;
    }

    pub enum ChordSet { Triads, Sevenths, Ninths, Elevenths, Thirteenths, NeoSoul }
    impl ChordSet {
        pub const ALL: [ChordSet; 6];
        pub fn from_id(id: u32) -> Option<Self>;
        pub fn as_id(self) -> u32;
        pub fn name(self) -> &'static CStr;
        pub fn name_str(self) -> &'static str;
        pub fn next(self) -> Self;
        pub fn prev(self) -> Self;
        pub fn entries(self, scale: ScaleType) -> [ChordSetEntry; PADS_PER_SET];
        pub fn entry(self, scale: ScaleType, degree: usize) -> ChordSetEntry;
        pub fn chords(self, key: &Key) -> Vec<Chord>;
        pub fn chord(self, key: &Key, degree: usize) -> Chord;
    }

    impl ChordQuality {
        pub const ALL: [ChordQuality; 35];
        pub fn suffix(self) -> &'static CStr;
        pub fn suffix_str(self) -> &'static str;
    }

The C interface added to `include/gooey.h`, all additive:

    #define CHORD_SET_TRIADS 0
    #define CHORD_SET_SEVENTHS 1
    #define CHORD_SET_NINTHS 2
    #define CHORD_SET_ELEVENTHS 3
    #define CHORD_SET_THIRTEENTHS 4
    #define CHORD_SET_NEO_SOUL 5
    #define CHORD_SET_COUNT 6
    #define CHORD_SET_PAD_COUNT 7

    void gooey_engine_poly_trigger_chord_set(struct GooeyEngine *engine,
                                             uint32_t chord_set, uint32_t root,
                                             uint32_t scale_type, uint32_t degree,
                                             uint32_t voicing, uint32_t preset,
                                             int32_t octave, float velocity);

    bool gooey_engine_piano_trigger_chord_set(struct GooeyEngine *engine, uint32_t piano,
                                              uint32_t chord_set, uint32_t root,
                                              uint32_t scale_type, uint32_t degree,
                                              uint32_t voicing, int32_t octave,
                                              float velocity);

    uint32_t gooey_engine_perf_get_event_chord_set(const struct GooeyEngine *engine,
                                                   uint32_t index);

    uint32_t gooey_chord_set_count(void);
    uint32_t gooey_chord_set_pad_count(void);
    const char *gooey_chord_set_name(uint32_t chord_set);
    const char *gooey_chord_set_entry_label(uint32_t chord_set, uint32_t scale_type,
                                            uint32_t degree);
    const char *gooey_chord_set_entry_quality_suffix(uint32_t chord_set, uint32_t scale_type,
                                                     uint32_t degree);
    uint32_t gooey_chord_set_entry_root(uint32_t chord_set, uint32_t root,
                                        uint32_t scale_type, uint32_t degree);
    uint32_t gooey_chord_set_entry_note_count(uint32_t chord_set, uint32_t scale_type,
                                              uint32_t degree);
    uint32_t gooey_chord_set_available_voicing_count(uint32_t chord_set, uint32_t root,
                                                     uint32_t scale_type, uint32_t degree);

Every `const char *` above points at a static constant baked into the library. It stays valid for the lifetime of the process and the caller must never free it. All of them return null for an unknown `chord_set`; the `uint32_t` getters return 0.

## Files Touched

`src/music/chord.rs` (new qualities, `ALL`, `suffix`, the invariant comment and its test), `src/music/chord_set.rs` (new), `src/music/cstr.rs` (new), `src/music/mod.rs` (module declarations and re-exports), `src/music/key.rs` (`diatonic_*` delegating), `src/performance/mod.rs` (`chord_set` field and `record_chord_on` argument), `src/ffi.rs` (constants, `resolve_chord`, the new externs, the replay path, the delegations), `examples/chords.rs` and `examples/piano.rs` (use `ChordSet`), `tests/chord_set_ffi.rs` (new), and this plan.
