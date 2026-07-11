# Add transport-synchronized clip-grid scheduling

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. Maintain this document in accordance with `.agent/PLANS.md` from the repository root.

## Purpose / Big Picture

After this work, an iOS or native host can preload a 4-column by 8-row grid of stereo audio clips and launch one clip, a whole scene row, or a column stop on a future musical boundary. Each column is mutually exclusive, so only one of its eight clips can play at a time. Clip playback follows the engine transport and BPM, including host-time armed starts and explicit transport seeks, while the host can query loaded, queued, and playing state to render an Ableton-style session grid.

The behavior is demonstrated through the public C FFI: tests load distinct buffers into slots, start transport, schedule transitions, render across the target beat, and observe that state and audio change on the intended sample. Existing `gooey_engine_loop_*` users continue working when they do not use the grid.

## Progress

- [x] (2026-07-10 19:07Z) Inspected the loop mixer, queued-buffer swap, sequencer transport, host-time arm, generated-header convention, and baseline tests.
- [x] (2026-07-10 19:07Z) Recorded product decisions for grid dimensions, quantization, scene behavior, transport stop, pitch preservation, relaunch, and active-slot replacement.
- [x] (2026-07-10 19:12Z) Implemented the clip-grid scheduler, monotonic transport, state queries, scene actions, hot replacement, seek alignment, and legacy detachment in the mixer; 40 mixer unit tests pass.
- [x] (2026-07-10 19:22Z) Wired BPM/start/stop/reset/seek/host-time arm and the full clip-grid control/state surface through C FFI.
- [x] (2026-07-10 19:22Z) Added 7 focused mixer unit tests and 10 end-to-end FFI integration tests; all new tests pass.
- [x] (2026-07-10 19:22Z) Completed validation: focused and full tests pass, changed Rust files are formatted, library/test clippy completes with the existing warning backlog, the iOS-feature build passes, and the generated header exposes the complete API. Repository-wide fmt and all-target clippy remain blocked by pre-existing unrelated files/examples documented below.

## Surprises & Discoveries

- Observation: `LoopChannel::queue_swap` is not a general clip-launch clock. It only fires when an already-playing channel cursor crosses a division, so it cannot launch an empty or stopped column or coordinate scenes against global transport.
  Evidence: `src/mixer/loop_channel.rs::maybe_swap_pending` derives its boundary from the active buffer cursor.

- Observation: `gooey_engine_sequencer_get_beat_position` reports the reference sequencer's position inside its 16-step pattern and therefore wraps every four quarter notes.
  Evidence: `src/ffi.rs::compute_beat_position` returns `(step + frac) / 4.0` using a wrapping `current_step`.

- Observation: generated `include/gooey.h` is intentionally untracked; `build.rs` regenerates it during Cargo builds.
  Evidence: only `include/module.modulemap` is tracked by Git.

- Observation: WSOLA correlation on constant audio has many equally valid offsets and can make its reported analysis cursor lag even when channels remain aligned.
  Evidence: the first tempo-alignment test used DC buffers and reported equal phases near 0.13 instead of the nominal 0.25; non-periodic buffers produce the expected transport phase and preserve equality across different source BPMs.

- Observation: Repository-wide formatting and all-target clippy were already not clean outside this feature.
  Evidence: `cargo fmt --check` requests formatting in the existing performance-recording example/module/test, while `cargo clippy --all-targets --all-features` fails because `examples/lfo_test.rs` imports the obsolete `libgooey` crate name and `examples/hihat.rs` references removed HiHat2 fields. `cargo clippy --lib --tests --all-features` completes, and none of its warnings point to the new clip-grid code.

## Decision Log

- Decision: The engine owns a fixed 4-column by 8-row grid.
  Rationale: Four columns map directly to the existing loop mixer and a fixed eight rows gives hosts a stable FFI contract.
  Date/Author: 2026-07-10 / user and Codex

- Decision: Individual clip launch, whole-row scene launch, and quantized column stop are all in the first implementation.
  Rationale: These operations form the minimum useful Ableton-style session workflow.
  Date/Author: 2026-07-10 / user and Codex

- Decision: Launch timing supports straight sixteenth, quarter, and four-beat bar quantization plus an exact absolute future beat; default quantization is one bar.
  Rationale: Common clicks stay simple while hosts and Link integrations can name an exact musical time.
  Date/Author: 2026-07-10 / user and Codex

- Decision: Every slot requires a source BPM and always launches in pitch-preserving mode.
  Rationale: Remaining synchronized is mandatory and pitch-preserving WSOLA already exists in the loop channel.
  Date/Author: 2026-07-10 / user and Codex

- Decision: Empty cells in a launched scene stop their columns. Transport stop freezes active clips and cancels pending actions; resume continues frozen phase unless a seek/reset realigns it.
  Rationale: A scene fully describes the resulting mix, and transport behavior is predictable without discarding active clip selection.
  Date/Author: 2026-07-10 / user and Codex

- Decision: Relaunch restarts a clip. Replacing an active slot schedules its new buffer with default quantization; unloading it schedules stop-and-remove.
  Rationale: Slot edits must not tear the sounding buffer in the middle of a frame.
  Date/Author: 2026-07-10 / user and Codex

## Outcomes & Retrospective

The requested 4×8 engine-owned clip grid is implemented in Rust and C FFI. It supports preloading, individual and atomic scene launch, quantized and exact beats, column stops, cancellation, state inspection, monotonic transport, host-time start, freeze/resume/seek, pitch-preserving source-tempo alignment, active replacement/unload, and coherent legacy-loop detachment.

All 274 library tests and the complete integration/doc test suite pass, including 10 new clip-grid FFI tests plus the existing 17 loop-mixer and 9 host-time-start regressions. The iOS-feature build succeeds, `include/gooey.h` contains every new constant and function, and `git diff --check` is clean. Repository-wide fmt/all-target clippy gaps are unrelated pre-existing maintenance issues and were intentionally not folded into this feature.

## Context and Orientation

`src/mixer/mod.rs` defines `Mixer`, which owns four `LoopChannel` values. Each `LoopChannel` in `src/mixer/loop_channel.rs` owns one active stereo sample buffer, playback cursor, tempo-warp mode, fader state, and effects. `StereoSampleBuffer` in `src/mixer/stereo_buffer.rs` uses reference-counted channel data, so cloning a clip for activation is cheap.

`src/ffi.rs` defines `GooeyEngine`, the public C-facing engine used by iOS. Its render loop advances drum sequencers once per sample and then calls `Mixer::tick`. Existing sequencer start, stop, reset, seek, BPM, and host-time-arm functions are the authoritative transport controls and must drive the new mixer transport too.

A clip slot is one grid coordinate holding a full-buffer stereo loop and its required source BPM. A scene is one row across all four columns. Quantization means rounding a request to a future straight musical boundary. The monotonic transport is an absolute quarter-note position that does not wrap every bar.

## Plan of Work

Add `src/mixer/clip_grid.rs` containing the fixed grid, launch quantization enum, slot-state bit constants, per-column active and pending state, and a monotonic transport clock. The clock advances by `bpm / (60 * sample_rate)` beats after each rendered frame while running. Scheduled actions become due before the frame at their target beat is rendered. Use a half-sample beat tolerance so floating-point accumulation cannot skip a boundary.

Embed the grid and clock in `Mixer`. Before summing channels, apply all due actions against the same current transport beat. Launch clones the selected slot buffer into its `LoopChannel`, sets full loop bounds, speed one, `PitchMode::PreservePitch`, starts playback, and records the launch beat. Stop clears the active row and stops playback. Scene launch computes one target and installs four pending actions at that target, using stop actions for empty cells. A later request replaces that column's pending action.

Transport stop sets grid-owned channels non-playing without clearing active rows and cancels pending actions. Start re-enables active channels. Seek sets the absolute beat and computes active phase as `(beat - launch_beat).rem_euclid(clip_length_beats) / clip_length_beats`, where clip length in beats is `(frames / sample_rate) * source_bpm / 60`. Reset seeks to zero and cancels pending actions.

Quantized requests made while running use the next strictly future boundary. While stopped, an exactly aligned current boundary is allowed and executes on the first frame after start. Exact targets must be finite, nonnegative, and not earlier than the current transport position; invalid requests leave existing pending state unchanged.

Make legacy buffer/playhead mutations detach clip-grid ownership for that column before applying. This includes direct load, playing, loop bounds, speed, restart, position, source BPM, pitch mode, and queued swap controls. Gain, mute/solo, and effect APIs remain shared column-strip controls and do not detach.

In `src/ffi.rs`, wire the mixer transport into BPM/start/stop/reset/seek/host-time-arm. Add constants and functions for slot load/unload/clear, clip and scene launch by quantization or exact beat, column stop, cancellation, default quantization, state queries, and absolute transport beat queries. FFI slot loading validates coordinates, audio input, sample rate, and source BPM before mutating the grid.

Add pure unit tests beside the clip-grid implementation and end-to-end tests in `tests/clip_grid.rs`. Keep old queued-swap APIs and tests unchanged.

## Concrete Steps

Work from `/Users/pretzel/conductor/workspaces/libgooey/rio-de-janeiro-v1`.

Implement and validate incrementally with:

    cargo test --lib mixer
    cargo test --test clip_grid
    cargo test --test loop_mixer --test sequencer_armed_start
    cargo test
    cargo fmt --check
    cargo clippy --all-targets --all-features
    cargo build --no-default-features --features ios

After the build, confirm the generated header contains `gooey_engine_clip_` and `gooey_engine_transport_get_beat_position` declarations. Do not commit `include/gooey.h` because it is generated and ignored.

## Validation and Acceptance

The feature is accepted when a clip in an empty column starts at the requested sample, a second row replaces it without overlapping the same column, and a scene changes all four columns on one sample while empty cells stop. State queries must distinguish loaded, playing, and queued, including the playing-and-queued combination used for active replacement.

Tests must prove sixteenth, quarter, bar, and exact-beat launches across render-buffer boundaries; last-request-wins cancellation; stopped transport freeze/resume; seek phase realignment; active replacement, relaunch, and unload; invalid-input rejection; pitch-preserving BPM adaptation; and legacy loop detachment. The full existing test suite, formatting, clippy, and iOS-feature build must remain green.

## Idempotence and Recovery

All changes are additive except small transport hooks and legacy-control detachment. Loading the same inactive slot replaces it safely. Cancel, clear, stop, and detach operations are idempotent. If an implementation step fails, unit tests can be run independently before FFI wiring; no migrations or external state changes are required.

The associated Nexus task is still `backlog`, so do not claim or update it during this implementation unless its status becomes `ready`.

## Artifacts and Notes

The primary implementation files are `src/mixer/clip_grid.rs`, `src/mixer/mod.rs`, `src/ffi.rs`, and `tests/clip_grid.rs`. This plan itself is committed at `plans/clip-scheduling-plan.md` and must be revised after each milestone and at completion.

## Interfaces and Dependencies

Expose Rust constants `CLIP_COLUMN_COUNT = 4`, `CLIP_ROW_COUNT = 8`, quantization IDs for sixteenth/quarter/bar, and bit flags for loaded/playing/queued. Expose an internal `LaunchQuantization` enum and `ClipGrid` methods mirrored by `Mixer`.

The C API uses `u32` column, row, quantization, and state values; `i32` row queries return `-1` for no row; scheduled-beat queries return `-1.0` when absent. Functions that schedule or mutate slots return `bool` for validation failure. No new crate dependency is needed.

Revision note (2026-07-10): Initial self-contained execution plan created from repository inspection and the user's approved implementation specification. Updated after implementation with delivered behavior, test evidence, the WSOLA test-material discovery, and pre-existing repository validation blockers.
