# Performance Recording for Live Instruments (Chords First)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `.agent/PLANS.md`, and this document must be maintained in accordance with that file. If this plan is revised, keep it self-contained: a future contributor should be able to read only this file and the current working tree, then continue safely.

## Purpose / Big Picture

Tide can already sequence drums and bass on a shared 16-step grid and play chords live on the poly synth. What is missing is the ability to capture a free-hand chord pad performance into a looping clip that stays locked to the same transport as the drums.

After Stage 1 of this plan, a host can record-arm the performance clip, choose punch-out (auto-disarm after one loop) or continuous overdub, play chord pads while the transport runs, and hear those chords replay sample-accurately on subsequent loops through the same poly synth path used for live pads. Later stages add quantize-on-record, configurable loop length, parameter automation, and undo.

## Progress

- [x] (2026-07-09) Authored multi-stage plan from Tide/libgooey architecture review and product decisions.
- [x] (2026-07-09) Created git worktree at `../libgooey-worktrees/performance-recording` on branch `bhurlow/performance-recording`.
- [x] (2026-07-09) Implement Stage 1 core module `src/performance/` (clip, events, record modes, player).
- [x] (2026-07-09) Wire Stage 1 into `GooeyEngine` render + poly chord FFI (record on live pad, playback on clock).
- [x] (2026-07-09) Expose Stage 1 C FFI (`gooey_engine_perf_*`).
- [x] (2026-07-09) Add unit tests for punch-out, overdub cut-gate, loop wrap, empty clip (11 lib tests).
- [x] (2026-07-09) Add FFI integration tests (`tests/performance_recording.rs`, 5 tests).
- [ ] Stage 2: quantize options + editable event query for UI grid.
- [ ] Stage 3: configurable clip length (steps/bars).
- [ ] Stage 4: parameter automation recording/replay.
- [ ] Stage 5: undo / take stack.
- [x] (2026-07-09) Basic CLI demo: `examples/performance_record.rs` (FFI + cpal + crossterm).
- [ ] Tide host: transport record arm + mode toggle calling new FFI.

## Surprises & Discoveries

- Observation: Poly synth is not on a sequencer voice strip. Drums and bass use per-voice `Sequencer` inside `VoiceStrip`; poly is a free-standing `PolySynth` driven only by `gooey_engine_poly_trigger_chord` / `poly_release`. Chord recording cannot reuse monophonic `SequencerStep.note` without a new data model.
  Evidence: `src/ffi.rs` `GooeyEngine` fields; poly FFI near `gooey_engine_poly_trigger_chord`.

- Observation: Existing sequencer "arm" means scheduled transport start (Link/host time), not record-arm. Naming in FFI must use `perf` / `record` clearly to avoid confusion with `gooey_engine_sequencer_start_at_host_time`.
  Evidence: `ArmedStart` in `src/engine/sequencer.rs` and host-time arm in `src/ffi.rs`.

- Observation: `gooey_engine_sequencer_get_beat_position` already returns a fractional quarter-note position derived from the reference (kick) sequencer with swing-aware interpolation. That is the correct clock for stamping and replaying clip events.
  Evidence: `gooey_engine_sequencer_get_beat_position` in `src/ffi.rs`.

## Decision Log

- Decision: Store performances as timed event clips (sub-step ticks), not only a 16-step chord grid.
  Rationale: Free pad timing is the main value of live chord recording; quantize can be layered later without throwing away free-time data.
  Date/Author: 2026-07-09 / plan author

- Decision: Own record and sample-accurate replay in libgooey core, not only in Tide Swift.
  Rationale: One engine clock keeps chords locked to drums/bass; Nebula and other hosts can share the same FFI.
  Date/Author: 2026-07-09 / plan author

- Decision: Stage 1 chord payload is pad parameters (root, scale, degree, voicing, preset, octave, velocity), not expanded MIDI note lists.
  Rationale: Matches Tide/Nebula pad UX and existing `poly_trigger_chord` rebuild path; editing stays in musical terms.
  Date/Author: 2026-07-09 / plan author

- Decision: Stage 1 loop length is fixed at 16 sixteenth-note steps (one 4/4 bar on the current grid). Configurable length is Stage 3.
  Rationale: Matches current Tide drum/bass grid and minimizes API surface for MVP.
  Date/Author: 2026-07-09 / plan author

- Decision: Tick resolution is 96 pulses per quarter note (24 ticks per 16th step, 384 ticks per 16-step loop).
  Rationale: Fine enough for free pad timing, integer-friendly, common MIDI-style PPQN; avoids float event times in the clip.
  Date/Author: 2026-07-09 / plan author

- Decision: Record modes are Overdub (stay armed across loops) and PunchOut (auto-disarm after one full clip length of active recording).
  Rationale: Product request for punch-out vs continuous overdub from day one.
  Date/Author: 2026-07-09 / plan author

- Decision: Overdub overlap policy cuts the previous event gate at the new event start (tape-style), matching live `release_all` monophonic chord policy.
  Rationale: Stacked poly chords would require a different engine policy; cut-gate matches what the user hears live.
  Date/Author: 2026-07-09 / plan author

- Decision: Recording only stamps events from live poly FFI calls while armed and transport is running. Clip playback must not re-enter the recorder.
  Rationale: Prevents feedback loops and keeps overdub intentional (only new pad presses).
  Date/Author: 2026-07-09 / plan author

- Decision: When arming while transport is already running, the punch-out window and event acceptance begin at the next loop boundary (tick 0). When arming while stopped, recording becomes active when transport starts.
  Rationale: Clean one-bar takes without partial first loops; still simple to reason about.
  Date/Author: 2026-07-09 / plan author

## Outcomes & Retrospective

### Stage 1 core (2026-07-09)

Delivered a working chord performance clip in libgooey: timed pad-parameter events, punch-out and overdub modes, cut-gate overdub, loop-boundary arm quantize, sample-clocked playback in `gooey_engine_render`, and C FFI for hosts. Live pad monitoring still triggers immediately; newly stamped events are held out of playback until the next loop wrap (or punch-out complete) so they do not double-fire with the live press.

Remaining for product completeness: Tide UI wiring, quantize, configurable length, automation, undo. Full `cargo test` should be green on the branch after this pass.

## Context and Orientation

libgooey is a Rust real-time audio engine. iOS hosts talk to it only through C FFI in `src/ffi.rs`, generated into headers via cbindgen. The FFI-facing engine type is `GooeyEngine`.

Relevant layout:

- `src/ffi.rs` — `GooeyEngine`, render loop, poly chord FFI, sequencer transport FFI.
- `src/engine/sequencer.rs` — sample-accurate 16-step sequencer used by drum and bass voices.
- `src/instruments/poly_synth.rs` — six-voice poly synth with `trigger_note`, `release_note`, `release_all`.
- `src/music/` — key, scale, chord, voicing helpers used by `gooey_engine_poly_trigger_chord`.
- `src/mixer/graph.rs` — routes `SOURCE_POLYSYNTH` into the Synth track.

Plain-language terms used in this plan:

A clip is a looping container of timed musical events with a fixed length in ticks. A tick is one pulse on a fixed musical grid (96 per quarter note). Record-arm means the engine will write new events when the user plays while transport runs. Punch-out means record-arm turns off automatically after one full loop of active recording. Overdub means record-arm stays on and new events are merged into the clip using cut-gate overlap. Transport is the shared play/stop/BPM clock that advances drum sequencers and, after this work, the performance clip player.

Today chords are live only. Tide calls `gooey_engine_poly_trigger_chord` on pad down and `gooey_engine_poly_release` on pad up. Those calls do not store history. Drums and bass already store patterns as `SequencerStep` rows (enabled, velocity, optional blend, optional monophonic MIDI note).

There is no existing performance recorder, automation lane, or chord step grid in libgooey. Offline `bounce.rs` only replays the native mono `Engine` sequencers and does not capture live poly performance.

## Plan of Work

### Stage 1 — Chord clip record and replay (MVP)

Create module `src/performance/` with:

- Constants: `TICKS_PER_QUARTER = 96`, `DEFAULT_LENGTH_STEPS = 16`, `TICKS_PER_STEP = 24`, `DEFAULT_LENGTH_TICKS = 384`.
- `RecordMode::{Overdub, PunchOut}` mapped to C constants `PERF_RECORD_MODE_OVERDUB = 0`, `PERF_RECORD_MODE_PUNCH_OUT = 1`.
- `ChordClipEvent` with start_tick, duration_ticks, root, scale_type, degree, voicing, preset, octave, velocity.
- `PerformanceClip` owning length_ticks and a `Vec` of events (sorted by start_tick for stable queries).
- `PerformanceRecorder` owning arm state, mode, pending loop-boundary start, punch remaining ticks, optional open (unreleased) event, and the clip.
- `PerformancePlayer` tracking last loop tick and active event index for sample-accurate trigger/release while transport runs.

Helpers:

- `beat_to_tick(beat_position: f64, length_ticks: u32) -> u32` using floor of `beat_position * TICKS_PER_QUARTER` modulo length.
- `tick_distance(start, end, length)` for gate lengths that may wrap.
- `cut_gates_at(clip, tick)` truncates any event whose gate covers `tick` so its end becomes `tick` (for monophonic overdub).

Wire into `GooeyEngine` in `src/ffi.rs`:

1. Add field `performance: PerformanceRecorder` (player state can live inside the same struct).
2. Each audio sample in `render`, after sequencers advance, if transport is running on the reference sequencer: compute beat position (same math as `gooey_engine_sequencer_get_beat_position`), advance the performance player, and apply Trigger/Release actions to `poly_synth` without recording.
3. On `gooey_engine_poly_trigger_chord`, after live trigger, if recorder is actively recording, cut previous gates, open a new event at current tick, and store pad params. Keep a `last_beat_position` updated every render sample for stamping when UI calls arrive between buffers.
4. On `gooey_engine_poly_release`, if an open recorded event exists, finalize duration from open start to current tick (minimum 1 tick).
5. On transport stop, finalize any open event and leave arm state as configured (armed may stay true for next play; punch remaining only counts while actively recording).
6. On transport reset / set_beat_position, reset player last-tick so replay does not miss or double-fire.

FFI surface (Stage 1):

- `gooey_engine_perf_set_record_armed(engine, bool)`
- `gooey_engine_perf_is_record_armed(engine) -> bool`
- `gooey_engine_perf_is_recording(engine) -> bool` (armed and actively capturing this loop window)
- `gooey_engine_perf_set_record_mode(engine, mode)`
- `gooey_engine_perf_get_record_mode(engine) -> u32`
- `gooey_engine_perf_clear_clip(engine)`
- `gooey_engine_perf_get_event_count(engine) -> u32`
- `gooey_engine_perf_get_event(engine, index, out fields...) -> bool`
- `gooey_engine_perf_get_length_ticks(engine) -> u32`
- `gooey_engine_perf_get_length_steps(engine) -> u32` (always 16 in Stage 1)

Register `pub mod performance` in `src/lib.rs`.

Tests in `src/performance/mod.rs` (unit) and `tests/performance_recording.rs` (FFI):

- Punch-out disarms after exactly one loop of active recording.
- Overdub keeps arm and appends a second-pass event; cut-gate shortens the earlier event.
- Empty clip produces no poly activity from the player.
- Clear empties events and open state.
- Replay at same BPM fires chord starts near recorded ticks (tolerance of a few samples via event query after simulated render, or pure unit player tests without audio).

### Stage 2 — Quantize and UI-facing edit helpers

Add record quantize modes Off / 16th / 8th / quarter that snap start_tick on note-on (and optionally duration on note-off). Add FFI to replace or delete an event by index for a simple chord strip UI. Tide can show recorded degrees on a 16-step strip even though storage remains free-time ticks.

### Stage 3 — Configurable loop length

Allow host to set length in steps (8/16/32) or bars. Punch-out length follows clip length. Document whether drum sequencers must match (default: independent clip length is allowed; hosts should set both when they want a single grid).

### Stage 4 — Parameter performance

Add a second event stream or unified timeline of `{tick, target_id, value}` for poly (then mixer/FX) parameters. Record while armed when host sets params; replay by setting smoothed targets. Requires stable param IDs already present for poly.

### Stage 5 — Undo / take stack

Keep a small ring of clip snapshots on punch-out complete, clear, or explicit commit. FFI undo/redo/can_undo.

## Concrete Steps

Work in the worktree:

    cd /Users/pretzel/code/gooey/gooey/libgooey-worktrees/performance-recording

Stage 1 implementation order:

1. Add `src/performance/mod.rs` with types and pure unit tests; `cargo test performance --lib`.
2. Export module from `src/lib.rs`.
3. Embed recorder in `GooeyEngine::new`, hook render + poly + transport FFI.
4. Add `tests/performance_recording.rs` using `gooey_engine_*` C API.
5. Run full `cargo test` and fix regressions.
6. Rebuild cbindgen headers if the project expects committed headers (this repo generates via `build.rs`; verify `include/` or build output as existing convention).

Expected commands:

    cargo test --lib performance
    cargo test --test performance_recording
    cargo test

## Validation and Acceptance

Stage 1 is done when all of the following hold:

1. Unit tests prove punch-out disarms after one loop, overdub cut-gate works, and tick math wraps correctly.
2. An FFI test starts transport, arms punch-out, injects `poly_trigger_chord` / `poly_release` at known times via render advancement, then on the next loop observes either recorded event fields matching the pad params or audible poly activity aligned with those ticks.
3. Live pad calls still sound immediately while recording (monitor path unchanged).
4. Clip playback uses the same chord-building path as live (pad params → diatonic seventh → voicing → notes).
5. `cargo test` passes on the branch.

Manual Tide acceptance (after a thin host wrap, can be a follow-up commit): play drums, arm punch-out, play four chords in one bar, auto-disarm, hear the chord loop locked to the bar.

## Idempotence and Recovery

All Stage 1 code is additive. `clear_clip` and disarm are safe to call repeatedly. Tests must not depend on global process state. If a mid-implementation build fails, the performance field can be left disarmed by default so existing hosts that never call `perf_*` behave as today.

## Artifacts and Notes

Branch: `bhurlow/performance-recording`

Worktree path (from monorepo): `libgooey-worktrees/performance-recording` (sibling checkout of the libgooey submodule).

Primary new files:

- `plans/performance-recording-plan.md` (this file)
- `src/performance/mod.rs`
- `tests/performance_recording.rs`

## Interfaces and Dependencies

In `src/performance/mod.rs` define approximately:

    pub const TICKS_PER_QUARTER: u32 = 96;
    pub const DEFAULT_LENGTH_STEPS: u32 = 16;
    pub const TICKS_PER_STEP: u32 = TICKS_PER_QUARTER / 4;
    pub const DEFAULT_LENGTH_TICKS: u32 = DEFAULT_LENGTH_STEPS * TICKS_PER_STEP;

    pub const PERF_RECORD_MODE_OVERDUB: u32 = 0;
    pub const PERF_RECORD_MODE_PUNCH_OUT: u32 = 1;

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum RecordMode { Overdub, PunchOut }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct ChordClipEvent {
        pub start_tick: u32,
        pub duration_ticks: u32,
        pub root: u32,
        pub scale_type: u32,
        pub degree: u32,
        pub voicing: u32,
        pub preset: u32,
        pub octave: i32,
        pub velocity: f32,
    }

    pub struct PerformanceRecorder { /* arm, mode, clip, player, open event, punch state */ }

    pub enum PlayerAction {
        Trigger(ChordClipEvent),
        Release,
    }

    impl PerformanceRecorder {
        pub fn new() -> Self;
        pub fn set_armed(&mut self, armed: bool);
        pub fn is_armed(&self) -> bool;
        pub fn is_recording(&self) -> bool;
        pub fn set_mode(&mut self, mode: RecordMode);
        pub fn clear_clip(&mut self);
        pub fn on_transport_running(&mut self, running: bool, beat_position: f64);
        pub fn tick_playback(&mut self, beat_position: f64) -> Option<PlayerAction>;
        pub fn record_chord_on(&mut self, beat_position: f64, event_params: ...) -> bool;
        pub fn record_chord_off(&mut self, beat_position: f64) -> bool;
        pub fn event_count(&self) -> usize;
        pub fn event(&self, index: usize) -> Option<ChordClipEvent>;
        pub fn length_ticks(&self) -> u32;
        pub fn update_clock(&mut self, beat_position: f64, transport_running: bool);
    }

No new crates. Uses only `std` plus existing music helpers at the FFI boundary when applying a `PlayerAction::Trigger` (reuse the same key/voicing code path as `gooey_engine_poly_trigger_chord`).

## Stage narrative (user story)

Milestone A (this implementation pass): pure clip math and recorder state machine with tests.

Milestone B: engine integration so that arming and playing pads while the sequencer runs leaves a clip that replays on the next loop without Tide changes beyond optional UI (FFI-only proof is enough for libgooey acceptance).

Milestone C (host, optional same PR or follow-up in Tide): transport bar record button and mode toggle calling the new FFI.
