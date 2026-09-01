# Expose a piano chord velocity slider through the C FFI

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. This document is maintained in accordance with `.agent/PLANS.md` from the repository root.

## Purpose / Big Picture

An iOS or other C-ABI host can already strike individual keys on the multi-sampled piano with a normalized velocity, but it must calculate every MIDI note and every per-note velocity itself. After this change, the host can describe a diatonic seventh chord using the same key, scale, degree, voicing, and octave values accepted by the poly synth and provide one base velocity. The instrument distributes that base velocity across the chord according to one normalized slider property: 0.0 emphasizes the lowest note, 0.5 strikes all notes evenly, and 1.0 emphasizes the highest note.

The weighting is continuous, symmetric, and deterministic. At either extreme the emphasized edge remains at the requested base velocity while the opposite edge reaches 66%; intermediate slider positions interpolate toward the even center. A newly registered piano defaults to 0.0 because the original musical requirement favored stronger lower notes.

## Progress

- [x] (2026-08-31 13:46Z) Inspected the multi-sampled piano, chord dynamics, existing poly chord FFI, generated header flow, and integration tests.
- [x] (2026-08-31 13:55Z) Implemented and validated the first profile-plus-humanize FFI design.
- [x] (2026-08-31 17:24Z) Replaced the profile-plus-humanize design with one continuous instrument-owned velocity mode.
- [x] (2026-08-31 17:27Z) Added instrument and FFI tests for low, center, high, clamping, defaults, chord triggering, partial maps, and overlap.
- [x] (2026-08-31 17:30Z) Ran final formatting, the full suite, the iOS-feature build, and generated-header inspection after the slider revision.
- [x] (2026-08-31 17:32Z) Recorded the revision in git and updated pull request #233.

## Surprises & Discoveries

- Observation: The original piano commit already contained a discrete Rust-side `ChordDynamics` model for examples, but the revised app interface needs neither its profile enum nor its humanize control.
  Evidence: The new deterministic slider weighting can live directly on `MultiSampleInstrument`; `src/music/dynamics.rs` remains unchanged for existing Rust callers.

- Observation: The existing `velocity_track` piano parameter is distinct from chord velocity mode.
  Evidence: `velocity_track` changes amplitude inside a selected sample layer, while velocity mode assigns a different attack velocity to each note in a chord.

- Observation: A strict repository-wide clippy run is not currently a usable acceptance gate because the baseline has unrelated denied warnings.
  Evidence: `cargo clippy --lib --no-default-features --features ios -- -D warnings` stopped on 58 existing findings in unrelated files and pre-existing portions of `src/ffi.rs`.

## Decision Log

- Decision: Mirror the harmonic arguments of `gooey_engine_poly_trigger_chord`, without a piano preset argument.
  Rationale: The host can reuse its existing root, scale, degree, voicing, and octave values while piano presets remain independently controllable through `gooey_engine_piano_set_preset`.
  Date/Author: 2026-08-31 / Codex and user

- Decision: Supersede the three-profile and humanize FFI with one normalized velocity mode property stored on `MultiSampleInstrument`.
  Rationale: The app needs one UI slider, and ownership by the instrument keeps the setting with the piano it affects.
  Date/Author: 2026-08-31 / Codex and user

- Decision: Map 0.0 to low emphasis, 0.5 to even velocity, and 1.0 to high emphasis using a symmetric linear tilt.
  Rationale: The endpoints and center have obvious meanings, every intermediate value changes smoothly, and the single control has no hidden discrete transitions.
  Date/Author: 2026-08-31 / Codex

- Decision: Keep the maximum attenuation at 34%, matching the earlier bass-led profile's 1.0-to-0.66 range, and make the result deterministic.
  Rationale: This preserves the approved strength of the natural weighting while removing the second humanize control and hidden random state.
  Date/Author: 2026-08-31 / Codex

- Decision: Default new pianos to low-weighted mode 0.0.
  Rationale: The original request specifically preferred stronger lower notes, and the center remains one slider movement away.
  Date/Author: 2026-08-31 / Codex

- Decision: Do not release prior notes in the chord trigger.
  Rationale: The user selected overlapping chords. Shared keys use the piano's existing self-masking behavior, and hosts can call the existing release APIs.
  Date/Author: 2026-08-31 / Codex and user

- Decision: Sound every mapped note but return false if any requested note lacks a zone.
  Rationale: A partially mapped keyboard remains playable while the boolean tells the host that the chord was incomplete.
  Date/Author: 2026-08-31 / Codex and user

## Outcomes & Retrospective

The C FFI now exposes one low/center/high velocity mode slider, its getter, and the theory-based piano chord trigger. The mode is owned by each `MultiSampleInstrument`, defaults to low-weighted, is deterministic, and preserves the requested overlap and partial-map behavior. The superseded profile constants, humanize setter, random state, and engine-owned dynamics array are absent.

The focused instrument tests pass 2/2, the piano FFI integration suite passes 16/16, the full suite passes with 376 library tests plus all integration suites, and `cargo build --no-default-features --features ios` succeeds. The generated header contains the setter, getter, and trigger and no longer contains the superseded profile API.

## Context and Orientation

`src/instruments/multisample.rs` defines `MultiSampleInstrument`, the velocity-layered sample player used for piano packs. It now owns `chord_velocity_mode` as a plain normalized value because the setting affects only future note attacks and does not need audio-rate smoothing. `chord_velocities` receives a base velocity and voice count, treats voice zero as the lowest note, and produces the per-note values used to select sample layers and scale attacks.

`src/ffi.rs` owns the C-compatible `GooeyEngine`, up to two registered pianos, and the existing music-theory helpers used by `gooey_engine_poly_trigger_chord`. The piano chord call uses the same `Key::diatonic_sevenths` and `apply_voicing` path, asks the selected piano for per-note velocities, and then calls that piano's existing `note_on` method.

`tests/multisample_piano.rs` drives the public FFI as a Swift or C host would. It builds deterministic in-memory sample maps, renders buffers, and inspects public metering. The generated `include/gooey.h` is gitignored and is produced by cbindgen during a Cargo build.

## Plan of Work

In `src/instruments/multisample.rs`, store a `chord_velocity_mode: f32` on every `MultiSampleInstrument`, default it to zero, and expose setter, getter, and per-chord velocity calculation methods. Clamp finite setter values to 0–1 and ignore non-finite values. For chords of two or more notes, apply a symmetric linear tilt whose maximum attenuation is 0.34; do not attenuate a single note.

In `src/ffi.rs`, export `gooey_engine_piano_set_velocity_mode` and `gooey_engine_piano_get_velocity_mode`. The setter returns false for a non-finite value or invalid piano, while the getter returns NaN for an invalid piano. Keep `gooey_engine_piano_trigger_chord`, but obtain its per-note velocities from the instrument property. Remove the superseded profile constants, profile conversion, humanize setter, and engine-owned dynamics array.

In the unit and integration tests, prove the continuous low-to-high ordering, exact even center, symmetry, clamping, default low mode, deterministic repeated hits, base velocity layer selection, partial-map reporting, and overlapping chord behavior.

## Concrete Steps

Work from `/Users/pretzel/conductor/workspaces/libgooey/kuala-lumpur` and run:

    cargo fmt --all -- --check
    cargo test --lib instruments::multisample::tests::chord_velocity_mode
    cargo test --test multisample_piano
    cargo test
    cargo build --no-default-features --features ios
    rg -n "piano_(set|get)_velocity_mode|piano_trigger_chord" include/gooey.h

The focused instrument suite should pass both slider tests. The piano integration suite should pass all 16 tests. The full suite and iOS-feature build should exit successfully. The generated header should contain the setter, getter, and chord trigger and should no longer contain the superseded velocity-profile constants or chord-dynamics setter.

## Validation and Acceptance

A new piano reports velocity mode 0.0. Setting 0.5 makes every note in a chord use the base velocity. At 0.0, per-note velocities descend from the lowest voice to the highest; at 1.0 they ascend by the symmetric amount. Values below zero or above one clamp, non-finite setters fail without changing the property, and invalid getters return NaN.

A registered piano with a committed map covering C3 through C5 accepts a C-major root-position seventh chord at octave 4 and publishes four active voices after rendering. Changing the base chord velocity reaches different velocity layers. Triggering two chords without a release preserves non-shared voices from the first, while shared keys are replaced by self-masking. A chord with an unmapped note plays its mapped notes and returns false.

No existing per-note piano function, `velocity_track` behavior, sample-layer rule, poly synth chord behavior, Rust `ChordDynamics` API, or performance-recording behavior changes.

## Idempotence and Recovery

All source edits and tests are additive or replace only the unmerged first design on this feature branch. Cargo may rewrite the gitignored generated header and build cache. If validation fails, fix the scoped source or test and rerun the same command; no database, network service, or destructive migration is involved.

## Artifacts and Notes

Before the slider revision, validation showed:

    cargo fmt --all -- --check: passed
    music::dynamics: 12 passed; 0 failed
    tests/multisample_piano.rs: 16 passed; 0 failed
    cargo test: passed
    cargo build --no-default-features --features ios: passed

After the slider code change, focused validation currently shows:

    instrument chord_velocity_mode tests: 2 passed; 0 failed
    tests/multisample_piano.rs: 16 passed; 0 failed
    cargo fmt --all -- --check: passed
    cargo test: passed (376 library tests plus all integration suites)
    cargo build --no-default-features --features ios: passed
    generated include/gooey.h: velocity mode setter/getter and chord trigger present; superseded profile API absent

Plan revision note (2026-08-31): Replaced the discrete profile-plus-humanize design with the requested single instrument-owned low/center/high slider and updated every section to describe the revised interface and validation.

Pull request update note (2026-08-31): Pushed commit `3cc5ef1` and retitled pull request #233 to “Add piano chord velocity balance slider to FFI,” with its description revised to match the final interface.

## Interfaces and Dependencies

The public C ABI adds:

    bool gooey_engine_piano_set_velocity_mode(
        GooeyEngine *engine,
        uint32_t piano,
        float mode);

    float gooey_engine_piano_get_velocity_mode(
        const GooeyEngine *engine,
        uint32_t piano);

    bool gooey_engine_piano_trigger_chord(
        GooeyEngine *engine,
        uint32_t piano,
        uint32_t root,
        uint32_t scale_type,
        uint32_t degree,
        uint32_t voicing,
        int32_t octave,
        float velocity);

No new dependency is required. The implementation reuses the existing music-theory helpers in `src/ffi.rs` and `MultiSampleInstrument::note_on`.
