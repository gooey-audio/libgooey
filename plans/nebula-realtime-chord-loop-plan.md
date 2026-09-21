# Add a real-time-safe chord-loop control path for Nebula

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This document must be maintained in accordance with `.agent/PLANS.md` from the repository root.

## Purpose / Big Picture

Nebula currently schedules chord changes from a UI-thread timer and writes many poly-synth parameters directly while audio may be rendering. After this work, a host can submit a complete chord loop or a live chord gesture through additive C functions, and the audio thread will apply it without waiting on the UI thread. Chord timing follows the mixer’s monotonic transport sample by sample, so UI stalls do not skip musical events. Poly preset edits become complete, coalesced render-boundary updates instead of concurrent mutation of live DSP.

The change is visible through the generated `include/gooey.h` and through `tests/chord_loop_control.rs`: clips of one through eight bars trigger poly and registered-piano chords on exact render samples, replacements commit at a loop boundary, and concurrent producer/render tests remain finite and correctly timed.

## Progress

- [x] (2026-09-21 21:27Z) Read `AGENTS.md`, `.agent/PLANS.md`, the supplied requirements, existing mixer/multisample control planes, performance player, poly/piano FFI, and focused tests.
- [x] (2026-09-21 21:27Z) Confirm baseline `origin/main` is `b26f399` and 40 focused performance/poly/piano/concurrency tests pass.
- [x] (2026-09-21 22:17Z) Added chord-loop and poly-preset control modules with atomic pending flags, render-side `try_lock`, projected preset state, bounded manual actions, coalesced edits, and producer-side snapshot retirement.
- [x] (2026-09-21 22:17Z) Extended the existing performance subsystem with targeted prepared events, variable lengths through `u32::MAX`, monotonic transport playback, cached sorted boundaries, seek reconciliation, and global-phase replacement.
- [x] (2026-09-21 22:17Z) Wired boundary ordering, owned-note piano release, melody harmony, preset application, six new C functions, four literal constants, and three C structs into `GooeyEngine` without changing legacy symbols.
- [x] (2026-09-21 22:17Z) Added private and FFI coverage, regenerated and inspected the C header, passed both full test matrices, and built both Apple iOS targets.
- [x] (2026-09-21 22:17Z) Ran the requested Clippy command; Rust 1.96 reports 56 pre-existing library errors and 62 pre-existing lib-test errors outside this change, while no new chord/poly-control finding remains. Recorded all validation evidence below.

## Surprises & Discoveries

- Observation: The existing performance player scans every chord event on every sample and derives phase from `compute_beat_position()`, which wraps every four beats.
  Evidence: `PerformanceRecorder::playback_action_at` linearly walks `self.events`, and `GooeyEngine::render` passes the reference sequencer’s wrapped position.
- Observation: The mixer already exposes the exact clock and discontinuity signal this feature needs.
  Evidence: `Mixer::transport_beat()` is monotonic while running, and `Mixer::transport_generation()` changes on start, seek, and reset.
- Observation: The generated header is ignored and produced by cbindgen 0.26; new structs must be added to `cbindgen.toml` and constants must be literals in `src/ffi.rs`.
  Evidence: `build.rs` writes `include/gooey.h` and watches `src/ffi.rs` plus `cbindgen.toml`.
- Observation: Repeated f64 beat accumulation can place an exact tick boundary a few ulps below its mathematical value at 48 kHz (observed at 60 BPM and tick 383).
  Evidence: The exact-sample piano test observed beat `3.989583333333234` immediately before tick 383; adding a `1e-9`-tick representation bias made all 1/2/4/8-bar cases exact at 60, 120, and 150 BPM without approaching a sample-sized interval.
- Observation: An audio-pending snapshot can be superseded before installation when retirement storage previously deferred it.
  Evidence: `newer_edit_retires_an_audio_pending_snapshot_without_dropping_it` now proves the newer edit replaces render scratch and the displaced Arc moves into producer-owned retirement storage.
- Observation: The requested repository-wide Clippy command is not green on `origin/main` under the installed Rust 1.96 toolchain.
  Evidence: The final run fails in unchanged files on lints including `empty_line_after_doc_comments` in `src/gen/polyblep.rs`, `derivable_impls` in `src/envelope.rs`, `misnamed_getters` in `src/engine/sequencer.rs`, and many older FFI functions missing safety docs. The new six unsafe functions have safety sections and no new control-module lint is reported.
- Observation: The workspace volume temporarily reached capacity during compilation.
  Evidence: Cargo reported `No space left on device`; cleaning only this workspace's disposable `target/` output and reusing an existing Cargo artifact directory allowed validation to continue without deleting source or another workspace's files.

## Decision Log

- Decision: Install a running replacement at the old clip’s next wrap, then derive the new clip’s phase from absolute transport ticks rather than re-anchoring it at zero.
  Rationale: This is the explicitly selected global-phase behavior; changing loop lengths may therefore enter the new clip mid-cycle.
  Date/Author: 2026-09-21 / user and Codex
- Decision: Host replacement and chord-loop clear replace the complete shared performance timeline, including recorded sampler hits, and disarm/finalize legacy recording.
  Rationale: This prevents one-bar sampler data from becoming stale or invalid under a variable-length host clip.
  Date/Author: 2026-09-21 / user and Codex
- Decision: Move every editable poly-preset API, including active parameter and modulation-route APIs, behind one projected bank.
  Rationale: Leaving any preset mutator on live state would retain the same render race the feature is intended to remove.
  Date/Author: 2026-09-21 / user and Codex
- Decision: Preserve direct poly/piano trigger and note APIs as legacy single-thread calls, while all new concurrent interaction uses the queued chord functions.
  Rationale: Existing C ABI behavior remains available without making the render thread wait on host activity.
  Date/Author: 2026-09-21 / Codex
- Decision: Preserve the public Rust `ChordQuality::intervals() -> Vec<Interval>` API and add `interval_slice()` for the allocation-free render path.
  Rationale: Melody harmony reconciliation must not allocate in render, but avoiding an unnecessary Rust API break is still possible with a static-slice companion.
  Date/Author: 2026-09-21 / Codex
- Decision: Keep the Clippy cleanup limited to findings introduced by this implementation.
  Rationale: Fixing 56–62 toolchain-wide pre-existing errors would expand this feature into unrelated DSP, sequencer, DSL, and legacy-FFI refactors explicitly outside the task.
  Date/Author: 2026-09-21 / Codex

## Outcomes & Retrospective

The real-time chord-loop path is implemented as the existing performance recorder/player's host-snapshot mode rather than a second player. Producer work resolves harmony and fixed note arrays, validates cyclic non-overlap, and coalesces immutable snapshots; render work only performs bounded copies, cursor transitions, `try_lock` drains, and fixed-capacity retirement. Piano ownership is note-specific and honors sustain, while poly chord replacement retains `release_all` semantics. Editable presets and modulation routes now round-trip through projected state and land before chord commands at a buffer boundary.

The additive C ABI is present exactly in the generated header. Integration tests cover null/array validation, 3,072-tick support, sorted queries, exact tick starts for four loop lengths at three tempos, rests/releases in private player tests, poly and piano commands, partial ownership/damper behavior, harmony latching, running global-phase replacement, last-write-wins generations, clear behavior, recorder/sampler takeover, atomic batches, and concurrent staging/rendering.

Both full test matrices and both iOS release builds pass. The one incomplete repository-level gate is `cargo clippy ... -D warnings`, which fails solely on the pre-existing Rust 1.96 lint backlog described above; no unrelated cleanup was made.

## Context and Orientation

`src/ffi.rs` defines the opaque `GooeyEngine`, its per-buffer/per-sample render loop, and every exported C symbol. At the start of each render buffer it already drains mixer, sampler, and piano control queues; the new poly and chord queues join that ordering. `src/performance/mod.rs` owns the current one-bar recorder/player and must remain the only performance chord player. `src/mixer/control.rs` and `src/instruments/multisample_control.rs` demonstrate the project’s producer mutex, atomic pending flag, render-side `try_lock`, published atomics, and retired-object handoff patterns.

A control projection is the host-visible desired state stored separately from live DSP. Setters update it immediately under a producer mutex, getters read it back, and the render thread later installs a copied configuration. A clip snapshot is an immutable, sorted `Arc` containing a loop length and at most 512 normalized events. “Loop-owned” means notes triggered by performance playback; those notes may be released by a rest, transport stop, replacement, or clear without disturbing unrelated piano notes.

## Plan of Work

First add `src/instruments/poly_synth_control.rs`. It will own projected factory-derived preset copies, projected active preset, one pending complete config per preset, and a pending preset selection. Producer setters validate a temporary copy before committing, which makes bulk updates atomic and gives duplicate entries last-write-wins array order. The audio thread checks an atomic flag, uses `try_lock`, copies pending values into fixed render scratch, and applies them before chord commands. Getters read only projected state.

Add `src/performance/control.rs` for chord commands and immutable snapshots. Producer code converts FFI events into normalized internal events, resolves harmony and MIDI notes outside render, sorts by start tick, and rejects adjacent cyclic overlap with widened arithmetic. The shared queue contains a bounded manual-action FIFO, one last-write-wins clip edit, generation counters, piano-registration atomics, and fixed retirement slots. The render drain defers if either the mutex or retirement capacity is unavailable.

Refactor `src/performance/mod.rs` so the recorder stores targeted normalized chord events, accepts an immutable host snapshot, and advances a sorted cursor from absolute mixer ticks. Normal forward playback advances only at cached start/end boundaries. A transport-generation change performs one direct covering-event lookup; if the same event remains active, it does not retrigger. A pending replacement waits for the old loop’s global wrap while running, then reconciles the replacement at its global phase. Stopped/empty playback installs immediately at a render boundary. Replacement and clear finalize/disarm recording and clear both chord and sampler lanes.

Wire both controls into `GooeyEngine`. Boundary ordering is mixer transport, poly configs, sampler/piano swaps, then chord commands. Per sample, read mixer running/beat/generation before `Mixer::tick`, advance the performance player, apply any action, then render instruments. Track the active controlled chord and its owner. Poly releases use `release_all`; piano releases call `note_off` only for successfully sounded notes, preserving damper and unrelated-note behavior. Legacy recorder/direct-trigger calls remain callable only from the render-driving thread and receive explicit documentation.

Finally add the literal constants, three `repr(C)` structs, six requested functions, and cbindgen exports. A zero-count/null array is accepted, while nonzero/null and all specified invalid event/config cases are rejected without changing pending state. Add focused tests before running the complete validation matrix.

## Concrete Steps

Work from `/Users/pretzel/conductor/workspaces/libgooey/vilnius-v1`.

After each subsystem, run focused commands:

    cargo test --no-default-features --features ios performance::
    cargo test --no-default-features --features ios --test poly_synth_params
    cargo test --no-default-features --features ios --test chord_loop_control

At completion run:

    cargo fmt --all -- --check
    cargo test --no-default-features --features ios
    cargo test --verbose
    cargo clippy --all-targets --no-default-features --features ios -- -D warnings
    ./scripts/build-ios.sh
    rg -n "GOOEY_CHORD_|GooeyChord|GooeyPolyParamValue|gooey_engine_chord_|gooey_engine_poly_set_preset_params" include/gooey.h

Expected results are all tests passing, clippy reporting no warnings under the requested feature set, both iOS libraries building, and the generated header containing four exact constants, three exact structs, and six declarations.

## Validation and Acceptance

Private tests must prove sorting and wrapped-overlap validation, silent rests, full-loop events, generation publication only on installation, replacement coalescing, snapshot retirement, and immediate nonblocking deferral while a producer lock is deliberately held.

`tests/chord_loop_control.rs` must exercise one-, two-, four-, and eight-bar clips at 48 kHz and multiple tempos; poly and registered-piano events; wrapped gates and rests; global-phase replacement with differing lengths; clear without transport/metronome reset; zero/nonzero starts; forward/backward seek and same-event phase correction; a simulated UI stall; atomic bulk poly writes; immediate projected getters; recorder disarm/sampler clearing; and simultaneous staging/rendering without non-finite samples, missing onsets, or duplicates.

Existing performance recording, sampler, poly, armed-start, piano, metronome, mixer, and full repository tests must remain green. No Nebula repository, release tag, published artifact, or unrelated Nexus task is changed.

## Idempotence and Recovery

All source changes and tests are repeatable. Cargo may update ignored `target/` output and regenerate ignored `include/gooey.h`. If a focused test fails, correct its owning control/player integration before continuing; do not delete existing performance behavior or generated artifacts manually. Snapshot replacement is immutable and generation-based, so retrying an accepted host edit merely produces a newer last-write-wins generation.

## Artifacts and Notes

Baseline focused validation passed 40 tests: 6 performance-recording, 7 poly-synth, 19 piano, 1 mixer-concurrency, plus their associated focused selections.

Final command evidence:

    cargo fmt --all -- --check
    # PASS

    cargo test --no-default-features --features ios
    # PASS: 499 unit tests passed, 2 ignored; every integration and doc-test suite passed.

    cargo test --verbose
    # PASS: 474 native unit tests plus every integration and doc-test suite passed.

    cargo clippy --all-targets --no-default-features --features ios -- -D warnings
    # FAIL: 56 pre-existing lib errors / 62 lib-test errors under Rust 1.96,
    # all outside the new control modules and functions; representative paths
    # are recorded in Surprises & Discoveries.

    ./scripts/build-ios.sh
    # PASS: aarch64-apple-ios and aarch64-apple-ios-sim release libraries,
    # both 18 MiB, and include/gooey.h generated.

    rg -n "GOOEY_CHORD_|GooeyChord|GooeyPolyParamValue|gooey_engine_chord_|gooey_engine_poly_set_preset_params" include/gooey.h
    # PASS: exact literal constants at lines 993-1008, exact struct field order
    # at lines 1504-1526, and const-pointer declarations at lines 3729-3778.

## Interfaces and Dependencies

No dependency is added. `src/ffi.rs` will define literal `GOOEY_CHORD_TARGET_POLY`, `GOOEY_CHORD_TARGET_PIANO`, `GOOEY_CHORD_LOOP_TICKS_PER_QUARTER`, and `GOOEY_CHORD_LOOP_MAX_EVENTS`; C-compatible `GooeyChordEvent`, `GooeyChordLoopEvent`, and `GooeyPolyParamValue`; and the six requested `gooey_engine_chord_*`/bulk-poly functions with their supplied signatures. Existing exported function signatures and numeric IDs remain unchanged.

Plan revision note (2026-09-21): Created from the approved implementation plan after inspecting current `origin/main`, resolving global-phase, sampler-lane, and poly-scope decisions, and running the focused baseline. Updated at completion with implementation decisions, exact-sample and retirement discoveries, full validation results, and the pre-existing Rust 1.96 Clippy backlog.
