# Chord-Aware Melody Quantization

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept current as implementation proceeds. This file follows `.agent/PLANS.md` and is self-contained so a contributor can resume the work from this file and the repository alone.

## Purpose / Big Picture

libgooey can already record a loop of chord-pad presses and replay those chords sample-accurately, but a performer still has to choose correct melody notes. This change adds a controller-independent melody input: the host sends an intended MIDI note, libgooey snaps it to a tone of the most recently triggered chord, and a dedicated synth plays it without interfering with the accompaniment. A host can therefore map an XY pad, keyboard, or other controller to melody pitch while every sounded note follows the chord loop.

The observable result is that MIDI input 66 sounds G4 over Cmaj7 and changes legato to F4 when the active chord becomes Dm7. The accompaniment and melody have separate synth instances, so releasing or editing one never releases or edits the other.

## Progress

- [x] (2026-09-17) Researched the chord recorder, chord-set resolver, poly synth, piano chord path, and mixer source layout.
- [x] (2026-09-17) Decided the public behavior and additive FFI surface with the user.
- [x] (2026-09-17) Added the pure chord-tone quantizer and its unit tests.
- [x] (2026-09-17) Added legato PolySynth retuning and the dedicated melody voice.
- [x] (2026-09-17) Wired audible poly, replayed, and piano chord paths into a latched harmony context.
- [x] (2026-09-17) Added mixer source 11 and the complete melody C FFI.
- [x] (2026-09-17) Added integration coverage and completed native, iOS, test, format, focused-clippy, and header validation.
- [x] (2026-09-17) Added an audible CLI demo that records a sample-accurate I-IV-ii-V loop and maps the computer keyboard to chord-quantized melody input.

## Surprises & Discoveries

- Observation: the five legacy mixer source IDs are followed by four sampler slots and two piano slots. The melody source must therefore be ID 11 rather than increasing `SOURCE_COUNT`, because increasing that constant would renumber the existing sampler and piano ABI.
  Evidence: `src/mixer/graph.rs` defines source IDs 0 through 10 and derives the dynamic ranges from the legacy count of five.

- Observation: chord playback owns a free-standing `PolySynth` and calls `release_all` on every chord change. A melody sharing that instance would be cut off by accompaniment changes.
  Evidence: `GooeyEngine::trigger_poly_chord_from_event` and `gooey_engine_poly_trigger_chord_set` in `src/ffi.rs` both release all poly voices before triggering a chord.

- Observation: `cargo clippy --all-targets --all-features` still cannot complete because the pre-existing `examples/hihat.rs` targets removed `HiHat2` fields, setters, and preset constructors. The library and all tests pass clippy; none of the warnings or errors names the new melody or quantizer code.
  Evidence: the all-target run reports fourteen `E0599`/`E0609` errors in `examples/hihat.rs`; `cargo clippy --lib --tests --message-format short` exits successfully.

## Decision Log

- Decision: accept integer MIDI notes rather than encoding an XY surface in the API.
  Rationale: the quantizer remains useful to keyboards, touch surfaces, and other controllers; hosts retain control of gesture geometry.
  Date/Author: 2026-09-17 / user and Codex.

- Decision: use every pitch class in the concrete chord quality, including extensions and alterations, repeated across MIDI 0 through 127.
  Rationale: the melody must follow stylistic chord sets such as Neo Soul rather than falling back to the diatonic key.
  Date/Author: 2026-09-17 / user and Codex.

- Decision: latch the most recent successfully sounded chord and retune a held melody legato when it changes.
  Rationale: chord release gaps should not create melody dropouts, and a held gesture should remain harmonically valid across loop boundaries.
  Date/Author: 2026-09-17 / user and Codex.

- Decision: render through an independent `PolySynth`, routed as source 11 to the existing Synth track by default.
  Rationale: independent note ownership and parameters are required, while appending the source preserves all published IDs and the default four-track layout.
  Date/Author: 2026-09-17 / user and Codex.

- Decision: melody recording and rhythmic quantization are not part of this change.
  Rationale: the requested first version is a live pitch-quantized performance path; it should not expand the existing monophonic chord clip data model.
  Date/Author: 2026-09-17 / user and Codex.

## Outcomes & Retrospective

The feature is complete. libgooey now quantizes arbitrary MIDI-note intentions against the full active chord, keeps harmony latched through accompaniment releases, and retunes a held melody without restarting its envelope. The dedicated synth renders through stable mixer source 11 and exposes independent real-time parameters through eight new C functions.

`cargo build`, the iOS feature build, `cargo test --verbose`, formatting, diff checks, focused clippy, and generated-header checks pass. The library suite reports 431 passing tests, the new integration suite reports five passing tests, and all existing integration suites remain green. The only incomplete validation command is all-target clippy because of the unrelated, previously documented stale `examples/hihat.rs` API usage.

An interactive `chord_aware_melody` example now makes the feature directly audible: it builds a one-bar I-IV-ii-V seventh-chord clip with the existing performance recorder, then routes a chromatic computer-keyboard layout through the independent melody input. Its two example-level tests verify the chromatic mapping and immediate four-chord loop playback. Deferred work remains unchanged: melody recording, rhythmic quantization, MIDI-event export, portamento, and melody presets are not included.

## Context and Orientation

The music-theory types are in `src/music/`. A `Chord` combines a root pitch class with a `ChordQuality`; `ChordQuality::intervals()` lists every chord member as a semitone interval from the root. Chord sets resolve a key and pad index into a concrete chord.

The FFI engine is `GooeyEngine` in `src/ffi.rs`. Its `performance` recorder replays `ChordClipEvent` values inside the per-sample render loop and calls `trigger_poly_chord_from_event`. Live poly chord calls use the same resolver. Multi-sample piano chord calls resolve chords independently near the end of `src/ffi.rs`.

The existing `PolySynth` in `src/instruments/poly_synth.rs` has six voices and public note-on, note-off, and parameter methods. The melody will use a separate instance under monophonic control. “Legato retuning” means changing the held voice's MIDI note, base frequency, and key-dependent modulation without restarting its oscillators or envelopes.

The mixer graph in `src/mixer/graph.rs` routes fixed engine sources into host-defined tracks. IDs 0 through 4 are legacy sources, 5 through 8 are sampler racks, and 9 through 10 are pianos. ID 11 is free and will become the permanently active melody source.

## Plan of Work

Add `src/music/quantizer.rs` with a public function that builds a twelve-entry pitch-class mask from a concrete chord, scans MIDI notes 0 through 127, and returns the nearest eligible note. At equal distance it keeps the preferred currently sounding note when that note is one of the tied candidates; otherwise it chooses the lower note. Export it from `src/music/mod.rs` and cover range boundaries, ties, duplicated pitch classes, and extended or altered chords with unit tests.

Extend `PolySynth` with a legato retune method that selects the most recently triggered held voice for the old MIDI note. It must change the note, frequency, and modulation array while leaving oscillator phases, envelope state, velocity, active voice count, and trigger order unchanged. Add `src/instruments/melody.rs` as the stateful wrapper that owns an independent keys-configured synth, the latched chord, held input, velocity, and sounding output.

Embed the melody voice in `GooeyEngine`. When a valid poly chord becomes audible, when recorded playback triggers one, or when at least one note of a piano chord finds a mapped zone, update the melody harmony. Invalid or wholly silent trigger attempts leave the existing harmony untouched. Chord releases do not clear it. Render the melody synth once per sample and scatter it through source 11.

Expose note-on, pitch-update, note-off, current-note, harmony-state, clear-harmony, and independent parameter functions from `src/ffi.rs`. Reuse the existing `POLY_PARAM_*` numeric namespace. Invalid queries return `-1` for notes and NaN for parameters, following current FFI conventions.

## Concrete Steps

Work from `/Users/pretzel/conductor/workspaces/libgooey/chisinau`.

1. Add and export the quantizer, melody wrapper, and PolySynth retune behavior. Run `cargo test --lib quantiz` and the PolySynth/melody unit tests.
2. Append `SOURCE_MELODY` to the mixer graph without moving any current constant. Route it to the Synth track in `with_default_layout`.
3. Add `MelodyVoice` to `GooeyEngine`, render it, and update it from all successful chord paths.
4. Add the C functions and an integration suite in `tests/chord_aware_melody.rs`.
5. Regenerate the C header through the normal build and run the validation commands below.

## Validation and Acceptance

Run:

    cargo build
    cargo build --no-default-features --features ios
    cargo test --test chord_aware_melody
    cargo test --test mixer_graph
    cargo test --verbose
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features
    rg "SOURCE_MELODY|gooey_engine_melody_" include/gooey.h

Acceptance requires every command to pass, except that any already-known unrelated all-target clippy failure must be identified precisely rather than hidden. Integration tests must demonstrate input 66 resolving to 67 over Cmaj7 and to 65 over Dm7, harmony latching after chord release, a held note following replayed chord changes, independent poly/melody releases and parameters, piano chord participation, stable source IDs, and safe invalid inputs.

## Idempotence and Recovery

All changes are additive source edits. Re-running builds and tests is safe. The generated `include/gooey.h` is gitignored and can be regenerated by touching `src/ffi.rs` and running `cargo build`. No recorded performance data is migrated because the melody is live-only and adds no fields to `ChordClipEvent`.

## Interfaces and Dependencies

No dependency is added. The public C surface is:

    gooey_engine_melody_note_on(engine, input_note, velocity) -> i32
    gooey_engine_melody_update_note(engine, input_note) -> i32
    gooey_engine_melody_note_off(engine)
    gooey_engine_melody_get_note(engine) -> i32
    gooey_engine_melody_has_harmony(engine) -> bool
    gooey_engine_melody_clear_harmony(engine)
    gooey_engine_melody_set_param(engine, param, value) -> bool
    gooey_engine_melody_get_param(engine, param) -> f32

`input_note` must be 0 through 127. Velocity is finite and clamped to 0 through 1. Note-returning functions use `-1` for no sounding note. Melody parameters use the existing normalized `POLY_PARAM_*` identifiers and never alter the accompaniment synth.
