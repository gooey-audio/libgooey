# Add the Nebula mixer and drums live-control ABI

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept current in accordance with `.agent/PLANS.md`.

## Purpose / Big Picture

Nebula owns one Gooey engine and needs to change submix gains, calibrate individual sources, replace track insert racks, edit insert parameters, and replace the complete four-lane drum pattern while audio is rendering. This change adds a separate C control handle that copies and validates host data, publishes bounded commands to the render thread, and reports generations when commands become active. Existing stopped-render graph configuration and legacy sound remain unchanged.

## Progress

- [x] (2026-09-22) Confirmed the clean starting commit `06db257548d07a2347d3e3f16e337d9f356ef894`, current `origin/main`, existing mixer architecture, baseline tests, and absence of a matching Nexus task.
- [x] (2026-09-22) Implemented the fixed-capacity SPSC command/retirement queues, atomic lifecycle and acknowledgement state, strict validation, and control-thread rack preparation.
- [x] (2026-09-22) Integrated 15 ms per-source trim, atomic drum snapshots, render-boundary command application, and 10 ms rack replacement transitions with off-render retirement.
- [x] (2026-09-22) Exported the versioned POD ABI through cbindgen and documented ownership, ordering, rejection, generation, reclamation, and shutdown contracts.
- [x] (2026-09-22) Added focused graph, FFI, concurrency, allocation-counting, tail-preservation, and lifecycle coverage.
- [x] (2026-09-22) Passed formatting, the full library/integration suite, the iOS-feature release build, generated-header inspection, and `git diff --check`; re-confirmed the unrelated all-features example failure and re-queried Nexus with no matching task.

## Surprises & Discoveries

- Observation: The existing mixer graph is already render-owned, but its public track/rack FFI mutates `GooeyEngine` directly and is documented as host-serialized.
  Evidence: `src/mixer/graph.rs` owns track smoothers and effect chains; `src/ffi.rs` exposes direct `gooey_engine_mixer_*` and `gooey_engine_track_effect_*` mutations.
- Observation: The repository-wide `cargo test --all-features` gate fails before this work because `examples/hihat.rs` uses removed `HiHat2` fields, methods, and presets.
  Evidence: The clean baseline reports 14 compile errors, while `cargo test --all-features --lib --tests` passes 501 library tests with two ignored and every integration target.
- Observation: cbindgen preserves private Rust array-dimension constant names inside C declarations even when equivalent public constants are exported.
  Evidence: The first generated header used undefined `DRUM_LANE_COUNT` / `DRUM_STEP_COUNT`; literal Rust dimensions regenerate correctly as `GooeyDrumStep lanes[4][16]` alongside the public `GOOEY_*` macros.

## Decision Log

- Decision: Use `GooeyLiveControl` / `gooey_live_control_*` and one 64-entry SPSC queue.
  Rationale: The name separates concurrent controls from stopped-render configuration and the single producer contract matches Nebula's serialized control thread.
  Date/Author: 2026-09-22 / Codex
- Decision: A replacement command's submission generation is its rack generation, and all command generations share one monotonic sequence.
  Rationale: One acknowledgement stream is simple to poll while rack parameter calls can reject stale layouts precisely.
  Date/Author: 2026-09-22 / Codex
- Decision: Do not build or package an XCFramework.
  Rationale: The user explicitly narrowed delivery to code, generated-header verification, documentation, tests, and review-ready commits.
  Date/Author: 2026-09-22 / Codex

## Outcomes & Retrospective

The additive API is implemented without a new runtime dependency or changes to Nebula. The generated header exposes API version 1, a 64-command queue, fixed 4-by-16 drum PODs, explicit effect descriptors, all seven live-control functions, and the documented constants. Render-boundary application is allocation/deallocation-free, and replaced racks are returned through a second SPSC ring for control-thread destruction.

`cargo test --all-features --lib --tests` passes with 507 library tests, two private-pack tests ignored, and every integration test green. `cargo build --release --no-default-features --features ios`, `cargo fmt --all -- --check`, header inspection, and `git diff --check` pass. `cargo test --all-features` still stops on the same 14 pre-existing `examples/hihat.rs` API-drift errors observed at the starting commit; that example was intentionally left untouched. A second Nexus search returned no matching task, so there was nothing to claim or update.

## Context and Orientation

`src/ffi.rs` owns `GooeyEngine`, the C ABI, and the buffer render loop. `src/mixer/graph.rs` routes fixed engine source IDs into named tracks and processes each track's fader before its `EffectChain`. `src/mixer/effect_chain.rs` wraps existing filters, delay, and reverbs. The new control plane will live in `src/live_control.rs`; it will own only shared queue/status state and prepared racks, never live render DSP.

The generated `include/gooey.h` is intentionally ignored. `build.rs` invokes cbindgen, so ABI structures must be listed in `cbindgen.toml` and literal public constants must live in `src/ffi.rs`.

## Plan of Work

Implement a fixed SPSC ring using atomics plus `UnsafeCell<MaybeUninit<T>>`. The producer copies input and constructs effect chains before publishing commands. The render consumer moves commands into the graph at buffer boundaries. A second fixed ring returns replaced chains for destruction on the producer thread. Use atomics for generation acknowledgement, attachment, shutdown, and active-render accounting; `gooey_engine_free` waits off the render thread for users to detach.

Add unity source smoothers to the graph before routing accumulation. Add a ten-millisecond dual-rack crossfade after the existing track fader. Replace four 16-step drum lanes in place, clearing optional step metadata. Validate only low-pass, BPM delay, and spring reverb descriptors and their documented parameter ranges. Scalar commands mutate the existing effect instance and never rebuild it.

Expose the requested POD layouts, constants, creation/destruction functions, submissions, and applied-generation getter. Document pointer lifetimes, single-producer ownership, FIFO/capacity rules, stopped-render graph configuration, validation, rack generations, reclamation, and shutdown in `docs/live-control-abi.md`.

## Concrete Steps

Work from `/Users/pretzel/conductor/workspaces/libgooey/hamburg-v1`. Add the control module and focused tests, wire it through the mixer and FFI, then run:

    cargo fmt --all -- --check
    cargo test --all-features --lib --tests
    cargo build --release --no-default-features --features ios
    rg -n "GOOEY_LIVE_CONTROL_API_VERSION|GooeyLiveControl|GooeyDrumPattern|gooey_live_control_" include/gooey.h
    git diff --check

Also rerun `cargo test --all-features` and record the known unrelated example failure separately.

## Validation and Acceptance

Acceptance requires independent submix gain, isolated piano trim at 1.41062, unity compatibility, atomic empty-safe 4×16 replacement, stale-generation and full-queue rejection, concurrent finite rendering, allocation-free boundary application, ordered inserts, tail-preserving scalar changes, smooth rack replacement, and lifecycle waits. Every accepted generation must eventually appear as the applied generation after rendering; rejected calls must leave the prior projection unchanged.

## Idempotence and Recovery

Builds and tests may regenerate ignored `target/` files and `include/gooey.h`. Source edits are additive and can be reapplied. If validation fails, correct only the live-control, mixer, FFI, documentation, test, or plan changes; do not repair unrelated examples or edit Nebula.

## Artifacts and Notes

Starting commit: `06db257548d07a2347d3e3f16e337d9f356ef894`. API marker: `GOOEY_LIVE_CONTROL_API_VERSION = 1`. No XCFramework or checksum artifact is produced.

## Interfaces and Dependencies

No runtime dependency is added. `GooeyLiveControl` is an opaque single-producer handle. Commands return `u64`, with zero reserved for rejection. Drum and effect descriptor structs are `#[repr(C)]` PODs copied synchronously. Existing `EFFECT_*`, `*_PARAM_*`, and `DELAY_TIMING_*` numeric constants remain authoritative.
