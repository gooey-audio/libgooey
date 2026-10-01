# Add loop-aware chord voicing transformations

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. This document is maintained in accordance with `.agent/PLANS.md`.

## Purpose / Big Picture

After this change, an embedded host can hand libgooey an ordered chord progression and ask for either the smoothest named voicings or a musically constrained randomized variation. Libgooey returns new `VOICING_*` identifiers while leaving chord identity, timing, instrument, preset, octave, velocity, and gate untouched. Nebula can therefore update both its sounding loop and its visible Loop Editor without duplicating libgooey's chord tables or voicing theory.

## Progress

- [x] (2026-10-01 22:07Z) Inspected Nebula's current chord-loop snapshot path, libgooey's chord event ABI, chord-set tables, named voicings, and release workflow.
- [x] (2026-10-01 22:12Z) Implemented the pure Rust progression optimizer and seeded, randomized near-optimal selector.
- [x] (2026-10-01 22:14Z) Exposed the additive, in-place C function over `GooeyChordEvent` arrays and regenerated `include/gooey.h`.
- [x] (2026-10-01 22:20Z) Added Rust unit and FFI integration coverage; formatting, 536 library tests, four FFI tests, the iOS release build, generated-header C syntax check, and diff checks pass.
- [x] (2026-10-01 22:24Z) Committed and pushed the engine change, then opened libgooey pull request #270.

## Surprises & Discoveries

- Observation: Nebula now submits immutable, sample-accurate `GooeyChordLoopEvent` snapshots to libgooey rather than scheduling loop playback on a Swift timer.
  Evidence: `Nebula/Audio/GooeyChordEngine.swift` calls `gooey_engine_chord_loop_replace`, and libgooey main exposes the corresponding v1.1.13 ABI.

- Observation: `GooeyChordEvent` already contains every musical input required by the optimizer, so a second public progression struct would duplicate ABI fields and invite drift.
  Evidence: the struct carries chord set, key root, scale, degree, current voicing, and octave alongside playback-only fields that can be preserved byte-for-byte.

- Observation: the dependent Nebula build initially exhausted the machine's free disk while Xcode duplicated its piano sample pack; this was environmental rather than a code or ABI failure.
  Evidence: Xcode reported `No space left on device` during the app-extension copy. Cleaning only Nebula DerivedData and libgooey's generated Cargo target freed enough space, after which all 194 Nebula tests passed.

## Decision Log

- Decision: Add a stateless function that mutates only the `voicing` field of a caller-owned `GooeyChordEvent` array, with no `GooeyEngine` pointer.
  Rationale: voice-leading is music theory, not render state. A pure API is deterministic, easy to test, callable by every host, and lets the host retain event IDs and rests.
  Date/Author: 2026-10-01 / Codex

- Decision: Consider every named voicing returned by `available_voicings`, but charge an intrinsic penalty for omitted chord tones and extreme span.
  Rationale: the requested randomizer should include Open, Drop, Spread, Shell, and Rootless colors without allowing sparse shapes to win merely because there are fewer voices to move.
  Date/Author: 2026-10-01 / Codex

- Decision: Score the progression as a cycle, including the final-to-first transition, and use exact dynamic programming for the best mode.
  Rationale: Nebula plays a loop, so optimizing a forward phrase while ignoring the wrap creates an audible seam.
  Date/Author: 2026-10-01 / Codex

- Decision: Make randomization seeded and select only near-optimal cycles that differ from the current sequence when an alternative exists.
  Rationale: seeded behavior is reproducible in tests, while the near-optimal bound gives the dice action variety without abandoning voice-leading constraints.
  Date/Author: 2026-10-01 / Codex

## Outcomes & Retrospective

The implementation provides a pure, bounded cyclic optimizer, deterministic seeded variation, and an atomic C ABI that changes only voicing IDs. All 536 libgooey library tests and four new FFI integration tests pass; the iOS-feature release build regenerates a C header accepted by clang. The engine work is published as [libgooey pull request #270](https://github.com/gooey-audio/libgooey/pull/270), ready for the dependent Nebula change to link.

## Context and Orientation

`src/music/voicing.rs` defines the ten `VoicingType` cases, lists which are valid for each `ChordQuality`, and expands a named voicing into sorted MIDI notes. `src/music/chord_set.rs` resolves one chord-set pad into a concrete `Chord`. `src/ffi.rs` defines stable numeric `VOICING_*` constants plus `GooeyChordEvent`, the C-compatible payload Nebula already uses for live and looping chord playback. `build.rs` runs cbindgen after Rust compilation and writes the generated public header to `include/gooey.h`.

Voice-leading here means choosing a named layout for every unchanged chord so consecutive sounding pitches move economically. A cycle is the progression plus its wrap from the last chord back to the first. A sparse voicing is a named layout such as Shell or Rootless that deliberately omits one or more chord tones.

## Plan of Work

Create `src/music/voice_leading.rs` and export it from `src/music/mod.rs`. Define a small internal progression-chord value containing the concrete `Chord`, octave, and current `VoicingType`; a public strategy enum for Best and Randomized-with-seed; and a transformation function that returns one valid `VoicingType` per input.

For each chord, enumerate `available_voicings` and expand each candidate through `apply_voicing`. Score a candidate with a small change penalty, ten points per omitted chord tone, and a penalty for spans wider than two octaves. Score a transition with a non-crossing edit-distance alignment over sorted MIDI pitches: matched voices pay their semitone distance plus an extra charge beyond a perfect fifth, while inserted or removed voices pay twelve points. Add explicit bass and soprano motion charges so a low total hidden inside an implausible outer-voice leap cannot dominate.

For Best, enumerate each possible first voicing and run dynamic programming through the remaining events, then add the closing last-to-first transition. Preserve deterministic tie-breaking by candidate order. For Randomized, first compute the best base score, then run seeded cost-perturbed searches and accept only differing cycles within thirty percent or eighteen points of the optimum. If no perturbed candidate qualifies, return the lowest-cost differing cycle; if no alternative exists, return the current sequence.

In `src/ffi.rs`, add stable constants `GOOEY_VOICE_LEADING_BEST` and `GOOEY_VOICE_LEADING_RANDOM`, plus `gooey_chord_progression_transform_voicings(events, event_count, strategy, seed)`. Accept null only when the count is zero, reject more than `GOOEY_CHORD_LOOP_MAX_EVENTS`, validate every musical field and current voicing before writing, transform a copied Rust representation, and then update only each source struct's `voicing`. Invalid input must return false with the whole array unchanged. Document that target, target ID, preset, velocity, and gate are preserved rather than used for scoring.

Add focused module tests for loop improvement, sparse-shape behavior, single-chord behavior, availability, deterministic best output, seeded random repeatability, diversity, and unchanged avoidance. Add `tests/voice_leading.rs` for the C contract, including null/count behavior, invalid strategy and musical fields, no-partial-write guarantees, preservation of every non-voicing field, and valid output IDs.

## Concrete Steps

Work from `/Users/pretzel/conductor/workspaces/nebula/douala/.context/libgooey-voice-leading`.

Implement the module and FFI, then run:

    cargo fmt --all -- --check
    cargo test --no-default-features voice_leading
    cargo test --no-default-features --test voice_leading
    cargo test --no-default-features --lib
    cargo build --release --no-default-features --features ios
    xcrun --sdk iphoneos clang -fsyntax-only -x c include/gooey.h

The focused tests must report all new tests passed. The library suite must have no regressions. The release build must regenerate a header containing the two strategy constants and `gooey_chord_progression_transform_voicings`, and clang must accept that header as C.

## Validation and Acceptance

The canonical C-major I-V-vi-IV seventh-chord loop must receive valid named voicings whose cyclic score is lower than all-root-position playback, with neither Shell nor Rootless selected simply to reduce note count. Running Best twice on the same input must produce byte-identical voicing IDs. Running Randomized twice with the same nonzero seed must produce the same changed result; a range of different seeds must yield more than one valid sequence while every result remains inside the documented near-optimal bound.

At the ABI boundary, an array with valid harmony must return true and preserve all fields except voicing. An invalid event anywhere in the array must return false and leave every input byte unchanged. Null with zero events is a successful no-op; null with a positive count, an unknown strategy, and an oversized count must fail.

## Idempotence and Recovery

The transformation is pure apart from the caller-owned output array and can be retried with no engine state. Best is idempotent. Randomized is repeatable for a fixed seed. If a build fails after header generation, rerunning Cargo regenerates the same header. No release tag is created from this branch; publishing remains a post-merge action from libgooey main.

## Artifacts and Notes

The dependent Nebula repository is `/Users/pretzel/conductor/workspaces/nebula/douala`. Its TestFlight workflow currently builds libgooey main, so the Nebula pull request can validate after this engine pull request merges without waiting for a separately packaged archive.

## Interfaces and Dependencies

The Rust interface in `src/music/voice_leading.rs` must expose a progression input type, a `VoiceLeadingStrategy`, and a transformation function returning `Option<Vec<VoicingType>>` or an equally explicit validation result. The C interface in `src/ffi.rs` must be exactly:

    pub const GOOEY_VOICE_LEADING_BEST: u32 = 0;
    pub const GOOEY_VOICE_LEADING_RANDOM: u32 = 1;

    pub unsafe extern "C" fn gooey_chord_progression_transform_voicings(
        events: *mut GooeyChordEvent,
        event_count: u32,
        strategy: u32,
        seed: u64,
    ) -> bool;

No random-number dependency is added. Use a small deterministic SplitMix64 generator local to the voice-leading module so native, iOS, and tests receive identical results.

Plan revision note (2026-10-01): Created after inspecting the current libgooey main and Nebula's post-v1.1.13 loop integration; it replaces the earlier assumption that Swift owned real-time loop scheduling.
