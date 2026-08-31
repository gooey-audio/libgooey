# Expose naturalized piano chord velocity through the C FFI

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. This document is maintained in accordance with `.agent/PLANS.md` from the repository root.

## Purpose / Big Picture

An iOS or other C-ABI host can already strike individual keys on the multi-sampled piano with a normalized velocity, but it must calculate every MIDI note and every per-note velocity itself. After this change, the host can describe a diatonic seventh chord using the same key, scale, degree, voicing, and octave values accepted by the poly synth, provide one base velocity, and let the engine distribute that velocity naturally across the chord. A host can select even, melody-led, or bass-led dynamics and control the amount of small repeat-to-repeat variation. A newly registered piano starts bass-led with humanize amount 0.35, and the host can obtain mechanical playback by selecting the even profile with amount zero.

The result is observable through the generated C header and the piano integration tests: one FFI call strikes a complete chord, bass-led dynamics emphasize its lower notes, repeated humanized hits vary deterministically as the stored random-number generator advances, and a second chord does not implicitly release non-shared notes from the first.

## Progress

- [x] (2026-08-31 13:46Z) Inspected the multi-sampled piano, chord dynamics, existing poly chord FFI, generated header flow, and integration tests.
- [x] (2026-08-31 13:49Z) Added persistent per-piano chord dynamics and the exported C constants and functions.
- [x] (2026-08-31 13:51Z) Added integration coverage for validation, chord construction, velocity behavior, partial maps, and overlap.
- [x] (2026-08-31 13:54Z) Ran formatting, focused tests, the full suite, the iOS-feature build, and inspected the generated header.
- [x] (2026-08-31 13:55Z) Updated this living plan with final evidence and outcomes.

## Surprises & Discoveries

- Observation: The recent piano commit already contains the complete Rust-side dynamics model and uses it in `examples/piano.rs`, but `src/ffi.rs` only exposes per-note velocity.
  Evidence: `ChordDynamics::velocities` implements profile weighting and seeded jitter, while the only piano strike export is `gooey_engine_piano_note_on`.

- Observation: The existing `velocity_track` piano parameter is distinct from chord naturalization.
  Evidence: `velocity_track` scales amplitude inside a selected sample layer; it does not assign different velocities to chord notes.

- Observation: A strict repository-wide clippy run is not currently a usable acceptance gate because the baseline has unrelated denied warnings.
  Evidence: `cargo clippy --lib --no-default-features --features ios -- -D warnings` stopped on 58 existing findings in files including `src/gen/polyblep.rs`, `src/dsl.rs`, `src/envelope.rs`, and pre-existing portions of `src/ffi.rs`; none identified the new chord functions.

## Decision Log

- Decision: Mirror the harmonic arguments of `gooey_engine_poly_trigger_chord`, without a piano preset argument.
  Rationale: The host can reuse its existing root, scale, degree, voicing, and octave values while piano presets remain independently controllable through `gooey_engine_piano_set_preset`.
  Date/Author: 2026-08-31 / Codex and user

- Decision: Expose three profile constants and one setter taking profile plus normalized humanize amount.
  Rationale: This preserves the existing Rust model and makes naturalization toggleable: bass-led plus 0.35 is naturalized, while even plus zero is mechanical.
  Date/Author: 2026-08-31 / Codex and user

- Decision: Initialize each piano's chord dynamics as bass-led with humanize 0.35.
  Rationale: Natural playback should be audible without extra setup, but the host retains explicit control.
  Date/Author: 2026-08-31 / Codex and user

- Decision: Do not release prior notes in the chord trigger.
  Rationale: The user selected overlapping chords. Shared keys still use the piano's existing self-masking behavior when restruck, and hosts can call the existing release APIs.
  Date/Author: 2026-08-31 / Codex and user

- Decision: Sound every mapped note but return false if any requested note lacks a zone.
  Rationale: A partially mapped keyboard remains playable while the boolean tells the host that the chord was incomplete.
  Date/Author: 2026-08-31 / Codex and user

## Outcomes & Retrospective

The C FFI now exposes all three chord velocity profiles, a per-piano dynamics setter, and a theory-based piano chord trigger. Each piano starts with persistent bass-led dynamics at humanize 0.35. Chord hits use the existing velocity-layered note path, preserve overlapping non-shared notes, self-mask restruck keys, and report incomplete sample maps without suppressing covered notes.

The focused dynamics tests remained 12/12, and the piano FFI integration suite grew from 11 to 16 passing tests. Formatting, the full test suite, and `cargo build --no-default-features --features ios` passed. The generated header contains the three constants and both function declarations. The existing performance recorder remains poly-synth-specific as intended.

## Context and Orientation

`src/music/dynamics.rs` defines `VelocityProfile` and `ChordDynamics`. A velocity profile assigns a fixed relative weight to each voice ordered from lowest to highest. The bass-led profile starts at full strength for the lowest note and thins upward. Humanizing adds seeded per-note jitter; storing one `ChordDynamics` per piano lets its random sequence advance on successive hits while remaining reproducible from a fresh engine.

`src/ffi.rs` owns the C-compatible `GooeyEngine`. It already contains up to two `MultiSampleInstrument` values in `pianos`, maps C constants to Rust music types, and implements `gooey_engine_poly_trigger_chord` using `Key::diatonic_sevenths` and `apply_voicing`. The new piano chord call will use exactly that harmony path, then call `ChordDynamics::velocities` and the piano's existing `note_on` method for sample-layer selection and playback.

`tests/multisample_piano.rs` drives the public FFI as a Swift or C host would. It can build deterministic in-memory sample maps, render buffers, and inspect public metering. The generated `include/gooey.h` is gitignored and is produced by cbindgen from the public constants and exported functions in `src/ffi.rs` during a Cargo build.

## Plan of Work

In `src/ffi.rs`, import `ChordDynamics` and `VelocityProfile`, add three public `PIANO_VELOCITY_PROFILE_*` constants, and add a `piano_dynamics` array beside the registered piano instruments. Initialize every entry with `ChordDynamics::new(VelocityProfile::BassLead, 0.35)` so registration needs no extra allocation or reset.

Add a private conversion from the public profile ID to `VelocityProfile`. Export `gooey_engine_piano_set_chord_dynamics`, rejecting a null or unregistered piano, an unknown profile, or a non-finite humanize amount. Valid finite amounts are clamped by `ChordDynamics::set_humanize` to the normalized range.

Export `gooey_engine_piano_trigger_chord`. Reject a non-finite velocity or invalid piano before advancing dynamics. Convert root, scale, degree, voicing, and octave using the same helpers and fallback rules as the poly chord call. Generate the selected diatonic seventh chord's sorted MIDI notes, generate matching per-note velocities from the persistent dynamics state, and strike each note without releasing existing voices. Attempt all notes and return true only if each call to `note_on` found a sample zone.

Extend `tests/multisample_piano.rs` with public-API integration tests. Use the existing two-layer map for base-velocity and voice-count coverage, and add narrowly mapped or distinguishable zones where necessary to demonstrate partial-map reporting and overlapping chords. Keep exact dynamics math covered by the existing `src/music/dynamics.rs` unit tests rather than duplicating its private random sequence in the FFI test.

## Concrete Steps

Work from `/Users/pretzel/conductor/workspaces/libgooey/kuala-lumpur`.

Edit the implementation and tests, then run:

    cargo fmt --all -- --check
    cargo test --lib music::dynamics
    cargo test --test multisample_piano
    cargo test
    cargo build --no-default-features --features ios
    rg -n "PIANO_VELOCITY_PROFILE|piano_set_chord_dynamics|piano_trigger_chord" include/gooey.h

The focused dynamics suite should retain its 12 passing tests. The piano integration suite should include the new chord cases with no failures. The full suite and iOS-feature build should exit successfully. The final `rg` output should show all three constants and both new exported function declarations.

## Validation and Acceptance

A registered piano with a committed map covering C3 through C5 must accept a C-major root-position seventh chord at octave 4 and publish four active voices after rendering. Changing the base velocity must select or audibly exercise different velocity layers. All three dynamics profiles must be accepted; an unknown profile, non-finite humanize amount, non-finite chord velocity, or invalid piano must return false. Finite humanize values outside zero through one must be accepted and clamped.

Bass-led dynamics must retain the ordering already pinned by `music::dynamics` tests, repeated naturalized calls must advance the stored random sequence, and even profile with zero humanize must remain equal and repeatable. Triggering two chords without a release must preserve non-shared voices from the first, while shared keys are replaced by the instrument's existing self-masking rather than doubled. A chord with one or more unmapped notes must play its mapped notes and return false.

No existing per-note piano function, velocity-layer selection rule, `velocity_track` parameter, poly synth chord behavior, or performance-recording behavior may change.

## Idempotence and Recovery

All source edits and tests are additive and can be rerun safely. Cargo may rewrite the gitignored generated header and build cache. If a validation step fails, fix the scoped source or test and rerun the same command; no database, network service, or destructive migration is involved.

## Artifacts and Notes

Before implementation, focused validation showed:

    music::dynamics: 12 passed; 0 failed
    tests/multisample_piano.rs: 11 passed; 0 failed

After implementation, validation showed:

    cargo fmt --all -- --check: passed
    music::dynamics: 12 passed; 0 failed
    tests/multisample_piano.rs: 16 passed; 0 failed
    cargo test: passed
    cargo build --no-default-features --features ios: passed
    generated include/gooey.h: three profile constants and both new functions present

The current branch started clean against `origin/main`.

Plan revision note (2026-08-31): Marked implementation and validation complete, recorded the strict-clippy baseline limitation, and added final test and generated-header evidence.

## Interfaces and Dependencies

The public C ABI will add:

    PIANO_VELOCITY_PROFILE_EVEN = 0
    PIANO_VELOCITY_PROFILE_MELODY_LEAD = 1
    PIANO_VELOCITY_PROFILE_BASS_LEAD = 2

    bool gooey_engine_piano_set_chord_dynamics(
        GooeyEngine *engine,
        uint32_t piano,
        uint32_t profile,
        float humanize);

    bool gooey_engine_piano_trigger_chord(
        GooeyEngine *engine,
        uint32_t piano,
        uint32_t root,
        uint32_t scale_type,
        uint32_t degree,
        uint32_t voicing,
        int32_t octave,
        float velocity);

No new dependency is required. The implementation reuses `crate::music::{ChordDynamics, VelocityProfile}`, the existing theory conversion helpers in `src/ffi.rs`, and `MultiSampleInstrument::note_on`.
