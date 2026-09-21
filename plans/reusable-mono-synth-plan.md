# Extract the Jam lead into a reusable mono synth instrument

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. It is maintained in accordance with `.agent/PLANS.md`.

## Purpose / Big Picture

The chord-aware melody feature currently owns a raw `PolySynth` and repeats the state needed to treat it as a single held lead. This change creates a reusable `MonoSynth` instrument that preserves the existing sound while centralizing note ownership, legato retuning, release, parameters, and rendering. Nebula can then expose a lead-volume control through the existing melody parameter ABI without changing chord volume.

## Progress

- [x] (2026-09-18) Confirmed that `MelodyVoice` repeats monophonic state around `PolySynth` and that `POLY_PARAM_VOLUME` is already independently smoothed.
- [x] (2026-09-18) Added the public `MonoSynth`, refactored `MelodyVoice`, and added focused unit and FFI integration coverage.
- [x] (2026-09-21) Ran formatting, focused and full tests, clippy, and the iOS-feature build; recorded the results below.

## Surprises & Discoveries

- Observation: The melody source shares the default Synth mixer track with chord playback, so a track fader would change both sounds.
  Evidence: `src/mixer/graph.rs` routes both `SOURCE_POLYSYNTH` and `SOURCE_MELODY` to track 2.

## Decision Log

- Decision: Wrap `PolySynth` rather than introduce new oscillator, filter, or envelope DSP.
  Rationale: The Jam lead must keep its current sound and normalized `POLY_PARAM_*` namespace.
  Date/Author: 2026-09-18 / user and Codex.

- Decision: Preserve every existing `gooey_engine_melody_*` symbol and `SOURCE_MELODY` value.
  Rationale: This refactor should be source- and ABI-compatible for current hosts.
  Date/Author: 2026-09-18 / Codex.

## Outcomes & Retrospective

Implementation and validation are complete. `cargo fmt --all -- --check`, the focused mono-synth, chord-aware melody, and mixer-graph suites, the full `cargo test --verbose` suite, and `cargo build --no-default-features --features ios` all pass. `cargo clippy --lib --tests --all-features` also passes with the repository's existing warnings; the stricter `-D warnings` form remains blocked by 60-plus pre-existing warnings outside this change, and the new mono-synth code introduces no reported warning. Nebula's generated XCFramework was rebuilt from this branch and its iOS unit-test scheme passes all 116 tests.

## Context and Orientation

`src/instruments/poly_synth.rs` contains the expressive six-voice stereo synthesizer used by both chords and the Jam lead. `src/instruments/melody.rs` adds chord-tone quantization to an independent copy. `src/ffi.rs` owns that melody voice and exposes it to C hosts. `tests/chord_aware_melody.rs` verifies the public C behavior end to end.

A monophonic instrument means one note is considered held at a time. Release tails may continue after note-off, matching the pre-refactor behavior, but a held note can be retuned without restarting its oscillators or envelopes.

## Plan of Work

Add `src/instruments/mono_synth.rs` with a public `MonoSynth` that wraps `PolySynth`, owns the held and pending notes, forwards normalized parameters, renders the same stereo frame, and implements `crate::engine::Instrument`. Refactor `MelodyVoice` to keep only the intended controller note, velocity, and latched chord while delegating sounding-note state to `MonoSynth`. Add unit tests for note ownership, retuning, parameters, trait behavior, and sample-identical stereo output. Extend the FFI integration suite to prove that melody volume attenuates or mutes only the lead while a chord remains audible.

## Concrete Steps

From the libgooey repository root, run:

    cargo fmt --all -- --check
    cargo test --lib mono_synth
    cargo test --test chord_aware_melody
    cargo test --test mixer_graph
    cargo test --verbose
    cargo clippy --lib --tests --all-features -- -D warnings
    cargo build --no-default-features --features ios

The focused mono synth suite should report four passing tests, the chord-aware suite should report six passing tests, and all existing tests must remain green.

## Validation and Acceptance

Acceptance requires `MonoSynth` to be usable anywhere an `Instrument` is accepted, to report and retune one held note, and to produce the same stereo samples as a directly configured `PolySynth`. Through the C API, a melody volume of zero must silence the melody source while chord playback remains audible. No C symbol or numeric mixer source ID may change.

## Idempotence and Recovery

All changes are additive or internal refactors. Test and formatting commands are safe to repeat. If the wrapper changes samples, compare its trigger and tick order against the direct `PolySynth` test before changing DSP code.

## Artifacts and Notes

Validation transcripts will be summarized in `Outcomes & Retrospective` after the commands pass.

## Interfaces and Dependencies

Export `instruments::MonoSynth` with `new`, `with_config`, `note_on`, `retune`, `note_off`, `release_all`, `current_note`, `set_param`, `param`, and `tick_frame`. Implement `engine::Instrument`, including `set_midi_note` for generic sequencer-style triggering. Add no dependency and make no C ABI addition.

Revision note (2026-09-18): Created during implementation to capture the user-approved wrapper design and required validation.

Revision note (2026-09-21): Marked validation complete and recorded the strict-clippy baseline limitation.
