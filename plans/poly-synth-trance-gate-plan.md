# Add a transport-synced poly-synth trance gate

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. This document must be maintained in accordance with `.agent/PLANS.md` from the repository root.

## Purpose / Big Picture

After this change, a host can select one of five curated one-bar rhythmic gates for each editable poly-synth preset. The gate follows the engine's sample-accurate transport, can range from subtle level movement to full silence on closed steps, and smooths edges by a tempo-relative amount. It processes only the poly source before that source enters the mixer graph, so delay and reverb tails continue naturally. Existing clients and sounds remain unchanged because every factory preset defaults to an off gate.

## Progress

- [x] (2026-09-22 01:15Z) Read the architecture, poly DSP/control/FFI paths, mixer transport, GUI lab, test suite, and Nexus guide; no matching Nexus task exists.
- [x] (2026-09-22 01:15Z) Record the approved behavior and interfaces in this living ExecPlan.
- [x] (2026-09-22 01:29Z) Implement and unit-test the reusable trance-gate DSP; eight focused gate tests pass.
- [x] (2026-09-22 01:29Z) Add per-preset gate state to the control handoff and FFI render path; 20 focused poly/control tests pass.
- [x] (2026-09-22 01:29Z) Add the public C configuration API and integration coverage; all 10 poly FFI tests pass.
- [x] (2026-09-22 01:29Z) Extend the poly-synth GUI laboratory with gate audition controls; the native visualization example compiles.
- [x] (2026-09-22 01:33Z) Run focused and full validation. All tests, builds, formatting, and targeted clippy checks pass; the requested all-target/all-feature clippy command reaches only the repository's pre-existing stale `examples/hihat.rs` errors.

## Surprises & Discoveries

- Observation: the FFI render loop already reads the mixer transport beat and running state before advancing the mixer for the current frame.
  Evidence: `GooeyEngine::render` captures `transport_beat` at the start of each frame and calls `mixer.tick` only after source rendering.
- Observation: the editable poly preset control already stages complete configs and coalesces writes without blocking the render thread.
  Evidence: `src/instruments/poly_synth_control.rs` stores projected and pending preset arrays behind a producer mutex and uses `try_lock` while draining.
- Observation: cbindgen does not emit public constants whose values use enum casts, even though Rust accepts them as const expressions.
  Evidence: the first generated header included only `POLY_GATE_PATTERN_COUNT`; changing the six stable pattern IDs to explicit numeric constants generated all seven required `POLY_GATE_PATTERN_*` macros.
- Observation: repository-wide clippy currently fails outside this feature because `examples/hihat.rs` targets an older hi-hat API.
  Evidence: `cargo clippy --all-targets --all-features` reports 14 missing-field/method errors in that example, while `cargo clippy --lib --features ios` and `cargo clippy --example polysynth_gui --features native,visualization` both pass with only existing warnings.

## Decision Log

- Decision: ship Off plus straight eighths, offbeat eighths, 2-on/2-off chopper, 3-on/1-off trance, and a syncopated `1011_0100_1011_0100` pattern.
  Rationale: this is a small but varied preset bank and avoids a custom-pattern ABI in v1.
  Date/Author: 2026-09-22 / user and Codex.
- Decision: store depth and smoothing with each editable poly preset, but keep the gate DSP separate from `PolySynth`.
  Rationale: recorded preset IDs should recall the complete sound while the processor remains reusable and does not imply that every standalone `PolySynth` owns transport state.
  Date/Author: 2026-09-22 / user and Codex.
- Decision: derive steps from the global transport, bypass while stopped, and insert the gate before mixer-track effects.
  Rationale: chords remain aligned through retriggers, tempo changes, and seeks; stopped audition stays natural; effect tails are not chopped.
  Date/Author: 2026-09-22 / user and Codex.

## Outcomes & Retrospective

The reusable processor, preset-local control handoff, render integration, additive C ABI, GUI controls, and requested coverage are complete. Factory presets remain audibly unchanged because the gate defaults Off. Full `cargo test`, both requested builds, formatting, focused suites, generated-header inspection, and feature-specific clippy checks pass. The only incomplete repository-wide check is `cargo clippy --all-targets --all-features`, which is blocked by the unrelated pre-existing `examples/hihat.rs` API drift described above.

## Context and Orientation

`src/effects/trance_gate.rs` will own the reusable processor. A trance gate is an amplitude pattern: each sixteenth-note step is either open at unity or closed toward silence. `src/instruments/poly_synth_control.rs` owns the cross-thread editable preset projection. `src/ffi.rs` owns the dedicated poly synth, reads transport time, routes sources into `MixerGraph`, and exports the C ABI. `examples/polysynth_gui.rs` is the native visual laboratory used for audible poly-synth verification.

## Plan of Work

Create a transport-aware `TranceGate` with a validated normalized config and stable pattern enum. At each sample, derive the current sixteenth step from the supplied beat. When the desired gain changes, linearly ramp from the current gain over `smoothing * 45%` of a sixteenth at the supplied BPM. Off, zero depth, and stopped transport target unity. Process mono samples and stereo frames without changing stereo balance.

Wrap the existing synth config and new gate config in one crate-private preset record. Preserve the producer-side projection and render-side coalescing behavior while adding active/preset gate getters and setters. The FFI engine will own one render-side gate, update it when the active preset changes, and process the poly frame immediately before scattering it into the mixer graph.

Expose stable pattern constants, `GooeyPolyGateConfig`, and active/preset set/get functions. Invalid pointers, IDs, or non-finite values fail atomically; finite normalized values clamp. Keep `POLY_PARAM_COUNT` at 30.

Finally, add a Gate page to the poly-synth GUI. Its audio wrapper will process the synth through the reusable gate using an always-running 120 BPM audition clock, and its five local presets will retain independent gate configs.

## Concrete Steps

Work from `/Users/pretzel/conductor/workspaces/libgooey/lisbon`. After each milestone, run the narrowest relevant tests. Finish with:

    cargo test poly_synth --lib
    cargo test --test poly_synth_params
    cargo test
    cargo build --features ios
    cargo build --example polysynth_gui --features native,visualization
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features

## Validation and Acceptance

Unit tests must prove all masks and four-beat wrapping, exact bypass modes, depth scaling, stereo preservation, immediate zero smoothing, and equal fractional smoothing duration at multiple tempos. Control tests must prove gate edits coalesce, remain preset-local, and survive active-preset switching. FFI tests must round-trip active and inactive preset configs, reject invalid inputs atomically, restore factory-off state, carry gate settings through performance replay, bypass while stopped, and produce materially lower energy on a closed running step than an open step.

The baseline before changes is 18 passing focused poly/control library tests and 7 passing tests in `tests/poly_synth_params.rs`.

## Idempotence and Recovery

All changes are source-only and repeatable. Generated `include/gooey.h` is gitignored. Factory reset affects only the selected in-memory preset. If a milestone fails, keep the ExecPlan current and revert only that milestone's local edits rather than generated build output.

## Artifacts and Notes

The pattern strings list step zero at the left. The default config is pattern Off, depth `1.0`, smoothing `0.1`. Smoothing zero deliberately permits a hard edge; nonzero smoothing also handles configuration and transport-state transitions.

Validation evidence at completion:

    cargo test poly_synth --lib                         # 20 passed
    cargo test --test poly_synth_params                 # 10 passed
    cargo test                                          # 486 library tests plus all integration suites passed
    cargo build --features ios                          # passed
    cargo build --example polysynth_gui --features native,visualization  # passed
    cargo test --example polysynth_gui --features native,visualization   # 4 passed
    cargo fmt --all -- --check                          # passed
    cargo clippy --lib --features ios                   # passed with existing warnings
    cargo clippy --example polysynth_gui --features native,visualization # passed with existing warnings
    cargo clippy --all-targets --all-features           # pre-existing hihat example failure

## Interfaces and Dependencies

No dependency is added. Rust exposes `effects::trance_gate::{TranceGate, TranceGateConfig, TranceGatePattern}`. The C ABI adds `POLY_GATE_PATTERN_*`, `POLY_GATE_PATTERN_COUNT`, `GooeyPolyGateConfig`, `gooey_engine_poly_set_gate`, `gooey_engine_poly_get_gate`, `gooey_engine_poly_set_preset_gate`, and `gooey_engine_poly_get_preset_gate` without changing existing poly parameter IDs.
