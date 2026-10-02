# Continuous mobile performance capture

This living ExecPlan follows `.agent/PLANS.md`.

## Purpose / Big Picture

Tide can capture three synchronized stereo instrument buses into one continuous take, then replay it through the current mixer. A bus is the sum of sources assigned to one mixer track. Recording before track effects/gain/pan/mute leaves those controls editable. This adds a C interface without depending on PR #267's desktop session layer.

## Progress

- [x] (2026-10-02) Inspect main at 5bc0dd4 and isolate `tide/studio-track-tape`.
- [x] (2026-10-02) Implement bounded capture/feed queues, render-owned commands, acknowledgements and dry mixer substitution.
- [x] (2026-10-02) Add queued studio edits, prepared percussion swaps, complete track racks and master ordering.
- [x] (2026-10-02) Pass eleven C integration tests including ten-minute capture and zero render allocations/frees.
- [x] (2026-10-02) Build device ARM64 and simulator ARM64/x86_64. Tide captures/replays three-minute takes on simulator.
- [ ] Finish physical-device audio, interruptions, route changes and sustained memory verification in Tide before final product acceptance.

## Surprises & Discoveries

Calling legacy transport setters from render entered a mutex-backed host queue and allocated a mutex. Apply sequencer/mixer transport and tempo directly from the queued edit handler. The allocation regression now observes zero allocations and frees. Exact replay gain assertions require warming the smoothed strip first. A full library test also encountered disk exhaustion writing its piano fixture; rerun after removing only workspace-generated build artifacts.

## Decision Log

Use one six-channel frame in each bounded queue and one host file: three stereo lanes always advance together, including partial-write recovery. Keep endpoint and engine lifetimes independent through shared ownership; workers must stop before endpoint free. Resolve next-bar arming on render after transport edits, since UI snapshots can lag. Reserve retirement credits for prepared voice swaps so concurrent production cannot overflow the return queue. Decisions made 2026-10-02 by Codex.

## Outcomes & Retrospective

The additive engine path is C-tested and used by Tide's file worker and actual simulator output callbacks. Hardware verification remains pending; this engine change intentionally defines no project storage, export or desktop session format.

## Context and Orientation

`src/ffi.rs` owns GooeyEngine and per-sample rendering. `src/mixer/graph.rs` accumulates source audio before track strips. `src/track_tape.rs` has two 131072-frame rings, bounded queues with one producer/consumer and acquire/release atomics. `src/live_control.rs` carries UI commands and returns retired DSP to control. `docs/track-tape.md` gives the complete host protocol and edit argument table. `tests/track_tape.rs` consumes the same C interface as Tide.

## Plan of Work

First add dry-frame reading/replacement and a recorder that copies only complete frames. Attach while audio is stopped. Commands apply at buffer boundaries; sample-by-sample recording starts at the requested transport bar. Overflow retains a synchronized prefix. Replay replaces live inputs; EOF/underrun continues silence through the existing effects for tails.

Next queue Tide runtime edits. Prepare voice/rack allocations before submission, swap on render and retire on control. Directly apply tempo/transport in the render handler rather than invoking the legacy queued host setters. Publish drum parameter targets for concurrent UI reads.

Finally exercise the C tests, library regressions and all Apple slices. From Tide point LIBGOOEY_PATH at this worktree, build locally and record/play chords and drums. An unused Bass lane must remain silent while all frame counts match. Hardware acceptance requires an unlocked paired device.

## Concrete Steps

From this repository root:

    cargo test --no-default-features --features ios --release --lib
    cargo test --no-default-features --features ios --release --test track_tape
    cargo build --release --lib --no-default-features --features ios --target aarch64-apple-ios
    cargo build --release --lib --no-default-features --features ios --target aarch64-apple-ios-sim
    cargo build --release --lib --no-default-features --features ios --target x86_64-apple-ios

From Tide set LIBGOOEY_PATH to this absolute worktree path, run `scripts/build_libgooey_xcframework.sh --force`, prepare its piano pack, generate via xcodegen and run TideTests on iOS Simulator. The override writes only Tide's local framework.

## Validation and Acceptance

Expect eleven C tests passing. Count-in captures zero frames until beat four, then synchronized lanes. Capture survives bus mute/zero gain and excludes the click. Replay applies current gain once, never feeds back into capture, and substitutes silence at EOF. Overflow retains exactly 131072 frames; underrun reports failure. Engine/endpoint cleanup works in either destruction order. Ten-minute capture checks finite samples, bounded queues and exact totals. Thread-local allocator accounting proves zero render allocations/frees including voice swaps, LFO edits, start, capture and playback. Library regressions and device/simulator slices must pass. Tide must additionally pass real file-worker capture/replay and physical route/interruption checks.

## Idempotence and Recovery

Without an attached tape the existing render ABI/default routing remain unchanged. Repeat builds safely. Stop/drain before discarding capture. Cancel the old feeder, issue Live, await acknowledgement, prime and Play. During interruption synchronously stop callbacks before flush_stopped; never flush concurrently with render. Retry storage only after keeping the complete captured prefix.

## Artifacts and Notes

    test result: ok. 11 passed; 0 failed

The paired phone was locked during initial validation, so no physical-audio claim is made.

## Interfaces and Dependencies

`gooey_engine_track_tape_new(engine,a,b,c)` attaches three distinct mixer tracks. Commands Live/Arm/Stop/Play/NextBar return generations; getters publish state, frames and applied generation. Drain/feed use aL,aR,bL,bR,cL,cR float samples. Free releases the worker endpoint; flush_stopped is for hosts that have stopped callbacks. Hosts own file storage and replay at the engine's original rate. Render does no file I/O, allocation or waiting in the new path.

Revision note (2026-10-02): record implementation, validation, the mutex discovery and remaining hardware acceptance.
