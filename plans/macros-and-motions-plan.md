# Macros and motions: playable parameter groups with triggerable automation

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. It is maintained in accordance with `.agent/PLANS.md` at the repository root.

## Purpose / Big Picture

Host apps embed libgooey for music making. They want movement in a composition without building a DAW timeline. For example, they might want to gradually open the hi-hat filter as a song transition, or close a pad's filter over eight beats. After this change a host can do two things.

The first is to build a **macro**: one 0–1 control connected to several parameters. The user clicks "set macro point", turns knobs normally, and clicks "register". Each changed parameter becomes connected to the macro, running from its old value (macro at 0) to its new value (macro at 1). The knobs then revert. The macro can be played by hand from a tray of macro controls.

The second is to fire a **motion**: a one-shot automation of a macro's value. It ramps the macro to a target over a number of beats or milliseconds, along a chosen curve. When it finishes it holds, returns at the same speed, or snaps back, and then stops until it is triggered again. Many motions can run at once.

To see it working, run `cargo run --example macro_motion --features native,crossterm` from the repository root:

- Press `g` on macro 1 and hear the hi-hats brighten over eight beats.
- Press `2` then `g` to hear the pad filter close and come back.
- Press `c`, adjust parameters with `f`/`h`/`w`, then press `r` to register a new macro.

## Progress

- [x] (2026-09-23) Core types in `src/automation/`, covering macros, motions, and the host-to-render control queue. Includes unit tests.
- [x] (2026-09-23) Engine integration in `src/ffi.rs`:
  - global effect get/set refactored into methods
  - parameter whitelist and read/write dispatch
  - render-boundary drain, a control-rate tick before the LFO block, and per-buffer status publication
  - poly projection coherence via `PolySynthControl::set_projected_param`
- [x] (2026-09-23) C ABI: `gooey_engine_macro_*` (capture, mappings, values) and `gooey_engine_motion_*` (settings, trigger, stop, status).
- [x] (2026-09-23) Tests: `tests/macros_motions.rs` (FFI behavior), `tests/automation_alloc.rs` (no render-thread allocation), and `automation_tests` in `src/ffi.rs` (live poly ramp survives preset edits).
- [x] (2026-09-23) CLI example `examples/macro_motion.rs`, ABI doc `docs/macros-motions-abi.md`, and an AGENTS.md module map entry.
- [ ] Hear the example on real audio hardware and tune the seeded macro ranges by ear. The example was only smoke-tested headlessly: setup ran, then raw mode failed without a TTY.

## Surprises & Discoveries

- Observation: the existing FFI LFO overwrites drum parameters every sample instead of adding an offset. When an LFO and a macro target the same parameter, whichever writes last wins.
  Evidence: `ChannelInstrument::apply_modulation` in `src/ffi.rs` calls `SmoothedParam::set_bipolar`, which sets the target.
- Observation: a poly preset edit re-applies the whole preset config at the next buffer (`apply_poly_control_commands`), which would snap a macro-driven poly parameter back.
  Evidence: the in-crate test `poly_motion_ramps_live_synth_and_survives_preset_edits`. It is solved by running motions reasserting every tick and by projecting resting values into the preset bank.
- Observation: a motion waiting for a quantized start writes nothing, so the parameter keeps whatever value it had rather than the macro's 0 position.
  Evidence: `bar_quantized_motion_waits_for_the_next_bar` originally expected the macro's "from" value and saw the lowpass default of 20 kHz instead.

## Decision Log

- Decision: motions automate macros rather than parameters directly.
  Rationale: the user wanted a durable reference to a group of parameters that can be played by hand and extended. Motions layered on macros give both manual tray control and automation.
  Date/Author: 2026-09-23, user request.
- Decision: there are three end modes: hold, return (same speed, retracing the path), and snap back.
  Rationale: this was an explicit user choice.
  Date/Author: 2026-09-23, user.
- Decision: durations can be in beats (following BPM changes mid-motion) or milliseconds. Starts can optionally be quantized to the next beat or bar, which only applies while the transport runs.
  Rationale: musical use needs tempo sync, and bar-aligned transitions are the common case.
  Date/Author: 2026-09-23.
- Decision: capture, diff and revert live in libgooey. A manual mapping API is also provided.
  Rationale: every host would otherwise reimplement snapshotting across three parameter families.
  Date/Author: 2026-09-23.
- Decision: v1 targets are the poly synth, the drum voices, and the global effects, limited to continuous parameters. Bass (which has no getters), discrete selectors, and the unsmoothed waveshapers are excluded.
  Rationale: these are the parameters the user named. Discrete or unsmoothed parameters would click or be meaningless when swept.
  Date/Author: 2026-09-23.
- Decision: a macro writes its parameters only when its value changes. Running motions reassert every tick.
  Rationale: this lets a user tweak a mapped knob by hand without the macro fighting them, while motions still win against preset re-applies.
  Date/Author: 2026-09-23.
- Decision: a manual macro set stops motions on that macro. Starting a motion stops other motions on the same macro.
  Rationale: this matches fader-grab behavior and keeps one owner per macro.
  Date/Author: 2026-09-23.
- Decision: FFI names are `gooey_engine_macro_*` and `gooey_engine_motion_*`, not `gooey_macro_*`.
  Rationale: this matches the existing convention for functions that take an engine pointer. `gooey_live_control_*` takes a separate handle.
  Date/Author: 2026-09-23.
- Decision: automation runs at a 32-frame control rate, just before the LFO block in `GooeyEngine::render`.
  Rationale: parameter smoothers (10–15 ms) remove steps, and the cost stays negligible. Running before the LFO block preserves the existing LFO behavior.
  Date/Author: 2026-09-23.

## Outcomes & Retrospective

The feature is implemented end to end and every automated check passes. Remaining work is subjective: listening to the example and tuning its preset ranges. Possible follow-ups:

- per-mapping response curves, for example exponential Hz sweeps on effect cutoffs
- mapping bass, mixer track gains, and effect-rack parameters
- motions that drive several macros at once

## Context and Orientation

The host-facing engine is `GooeyEngine` in `src/ffi.rs`. Hosts call `#[no_mangle] extern "C"` functions on it, and `build.rs` generates `include/gooey.h` with cbindgen. `GooeyEngine::render` is the audio callback. It first drains control queues at the buffer boundary (`apply_*_control_commands`) and then loops over frames. Each frame it ticks sequencers, then LFOs, then instruments and effects.

Parameters are addressed by index constants:

- `POLY_PARAM_*` for the poly synth, normalized 0–1. The host edits a projected preset bank in `src/instruments/poly_synth_control.rs`, and the render thread applies it at buffer boundaries.
- `INSTRUMENT_*` channels with `KICK_PARAM_*`, `SNARE_PARAM_*`, `HIHAT_PARAM_*` or `TOM_PARAM_*` for drums. These are set directly through `ChannelInstrument::set_param` / `get_param`.
- `EFFECT_*` with per-effect `*_PARAM_*` constants for global effects, in engineering units.

`SmoothedParam` (`src/utils/smoother.rs`) smooths every target change over about 15 ms.

The new module `src/automation/` is engine-agnostic:

- `macros.rs` defines `ParamTarget`, `MacroMapping`, `MacroDefinition` (fixed capacity, `Copy`), and `MacroBank` (render-owned values and dirty flags).
- `motion.rs` defines `MotionDefinition` and its enums, plus `MotionRunner`, a fixed pool of 32 instances advanced by `advance(frames, clock, bank)`.
- `control.rs` defines `AutomationControl`. The host locks a mutex to edit projected definitions and push commands; the render thread only uses `try_lock` and drains into pre-reserved scratch. Macro values and motion status are published back through atomics.

## Plan of Work

All of the following is implemented; this section describes it for maintenance.

In `src/ffi.rs`, `GooeyEngine` gained these fields: `automation_control`, `automation_scratch`, `macros`, `motions`, `automation_frames`, and `macro_capture`. The render path changed as follows:

- `render` calls `apply_automation_commands` after `apply_chord_control_commands`.
- Inside the frame loop, just before the LFO block, it counts frames and calls `tick_automation` every `AUTOMATION_CONTROL_INTERVAL` (32) frames. That call advances motions and writes dirty macros through `automation_param_write`.
- `publish_automation_status` runs at the end of each buffer.

The bodies of `gooey_engine_set_global_effect_param` / `gooey_engine_get_global_effect_param` moved into `GooeyEngine::set_global_effect_param` / `global_effect_param` so macros can share them.

`automation_param_range` is the single whitelist of mappable parameters and their ranges. Supporting methods:

- `automation_targets` enumerates them for capture.
- `automation_param_read` is the host-visible read.
- `automation_param_write` is the render write, which re-validates in case a drum channel's instrument was swapped.
- `automation_param_host_write` is the capture revert. Poly goes through `PolySynthControl::set_param`.
- `project_poly_macro` mirrors macro positions into the poly preset projection via the new `PolySynthControl::set_projected_param`.

The C ABI section "Macros and motions" near the end of `src/ffi.rs` defines the constants and functions documented in `docs/macros-motions-abi.md`.

## Concrete Steps

From the repository root:

    cargo build
    cargo test --lib automation --verbose        # core + in-crate engine tests
    cargo test --test macros_motions --verbose   # FFI behavior
    cargo test --test automation_alloc --verbose # no render-thread allocation
    cargo build --example macro_motion --features native,crossterm
    cargo run --example macro_motion --features native,crossterm

Expected test summaries: `test result: ok. 23 passed` for the lib filter, `ok. 15 passed` for `macros_motions`, and `ok. 1 passed` for `automation_alloc`.

## Validation and Acceptance

In `tests/macros_motions.rs`, capturing a hi-hat tone change and a poly cutoff change into macro 0 returns 2. Both parameters read back their pre-capture values, and setting the macro to 0.5 puts both at their midpoints.

A motion over one beat at 120 BPM puts a 1–5 kHz lowpass mapping near 3 kHz after 0.25 s, and exactly 5 kHz after it completes. Return mode is back at 1 kHz after twice the duration, and snap-back is at 1 kHz right after completion. Bar quantization waits until beat 4.

In `tests/automation_alloc.rs`, rendering while motions start, run and complete performs zero allocations and deallocations. Finally, the full validation suite in CLAUDE.md passes.

## Idempotence and Recovery

All changes are additive, apart from moving the global-effect get/set bodies into methods, which kept their behavior. Tests create their own engines, so rerunning is safe.

## Artifacts and Notes

The render hook, from `GooeyEngine::render`:

    // Macros and motions run at control rate, before the LFOs so an
    // LFO routed to the same parameter still wins.
    self.automation_frames += 1;
    if self.automation_frames >= AUTOMATION_CONTROL_INTERVAL {
        self.tick_automation(transport_running, transport_beat);
    }

## Interfaces and Dependencies

`crate::automation` exports:

- `ParamTarget { kind, index, param }`, `MacroMapping { target, from, to }`, `MacroDefinition`, and `MacroBank`
- `MotionDefinition { macro_index, target, start: Option<f32>, duration, curve, end_mode, quantize }`
- `MotionCurve`, `MotionEndMode`, `MotionDuration`, `MotionQuantize`, `MotionPhase`, `MotionClock`, and `MotionRunner`
- `MACRO_COUNT = 16`, `MACRO_MAX_MAPPINGS = 16`, `MOTION_SLOT_COUNT = 32`

The crate-private `automation::control::AutomationControl` carries the `AutomationCommand` enum between threads. No new external dependencies were added.

---

Revision note (2026-09-23): initial plan, written alongside the implementation and recording its completed state and remaining listening check.
