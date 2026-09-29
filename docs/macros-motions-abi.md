# Macros and Motions C ABI

Macros and motions let a host add movement to a performance without a timeline.

- A **macro** is a single 0–1 control connected to up to 16 parameters. Each connection, called a mapping, stores the parameter's value at macro 0 (`from`) and at macro 1 (`to`). A host can show a tray of macro knobs and play them by hand.
- A **motion** automates one macro's value once per trigger. It ramps the macro to a target value over a duration in beats or milliseconds, following a curve, and then stops. Motions can be retriggered, and many can run at the same time.
- A **macro LFO** cycles one macro's value continuously between 0 and 1 at a tempo-synced rate, so the macro's mappings sweep back and forth without the host sending values. Every macro has its own LFO.

Everything is addressed by index: 16 macros (`MACRO_COUNT`, one LFO each) and 32 motion slots (`MOTION_SLOT_COUNT`). The functions are declared in `include/gooey.h`. A runnable demo is `cargo run --example macro_motion --features native,crossterm`.

## Parameter targets

A mapping names its parameter with three numbers: `(kind, index, param)`. They use the same constants as the existing setters.

| kind | index | param | values |
|---|---|---|---|
| `PARAM_TARGET_POLY` | 0 | `POLY_PARAM_*` | normalized 0–1 |
| `PARAM_TARGET_DRUM` | channel (`INSTRUMENT_KICK`..`INSTRUMENT_TOM`) | the `*_PARAM_*` constant of the channel's current instrument | normalized 0–1 |
| `PARAM_TARGET_GLOBAL_EFFECT` | `EFFECT_*` | the effect's `*_PARAM_*` constant | same units as `gooey_engine_set_global_effect_param` |

Only continuous parameters can be mapped. `gooey_engine_macro_add_mapping` returns false for any of these:

- the poly oscillator waveform selectors
- `SNARE_PARAM_FILTER_TYPE`
- bass channels
- delay timing and ping-pong
- both waveshapers

Poly envelope and curve parameters can be mapped, but the synth only reads them when a note starts. Moving them affects the next note, not notes already sounding.

## Capture workflow

1. `gooey_engine_macro_capture_begin(engine)` snapshots every mappable parameter.
2. The user adjusts parameters with the normal setters. `gooey_engine_macro_capture_get_change_count` reports how many have changed so far.
3. `gooey_engine_macro_capture_commit(engine, macro, mode)` registers the changes, with `mode` set to `MACRO_CAPTURE_REPLACE` or `MACRO_CAPTURE_MERGE`:
   - Each changed parameter is mapped `from` its pre-capture value `to` its current value.
   - The changed parameters revert to their pre-capture values.
   - The macro is set to 0, and any motion or LFO on that macro stops.
   - `MACRO_CAPTURE_MERGE` adds the changes to the macro's existing mappings. A parameter the macro already drives keeps its `from` value and takes the new `to` value.
   - It returns the macro's mapping count, or one of these errors:
     - `MACRO_CAPTURE_ERROR_INVALID`: bad arguments, or no capture in progress.
     - `MACRO_CAPTURE_ERROR_TOO_MANY`: more than 16 mappings would result. The capture stays open.
4. `gooey_engine_macro_capture_cancel(engine, revert)` ends a capture without registering it. It restores the changed parameters when `revert` is true.

Mappings can also be edited directly:

- `gooey_engine_macro_add_mapping` adds or updates a mapping, and `gooey_engine_macro_remove_mapping` / `gooey_engine_macro_clear` remove them.
- `gooey_engine_macro_get_mapping_count` and `gooey_engine_macro_get_mapping` read them back.

## Playing macros

`gooey_engine_macro_set_value(engine, macro, value)` moves a macro by hand. Any motion or LFO driving that macro stops, the way grabbing a fader overrides automation. `gooey_engine_macro_get_value` returns the macro's live position, which keeps changing while a motion or LFO runs.

A macro writes its parameters only when its value changes. If the host edits a mapped parameter directly, that edit stays until the macro moves again.

## Motions

`gooey_engine_motion_configure(engine, slot, macro, target)` points a slot at a macro and a target value. A slot that has never been configured gets these defaults:

| setting | default | setter | options |
|---|---|---|---|
| duration | 4 beats | `gooey_engine_motion_set_duration` | `MOTION_DURATION_BEATS` (follows tempo changes) or `MOTION_DURATION_MS` |
| curve | linear | `gooey_engine_motion_set_curve` | `MOTION_CURVE_LINEAR`, `EASE_IN`, `EASE_OUT`, `S_CURVE` |
| end mode | hold | `gooey_engine_motion_set_end_mode` | `MOTION_END_HOLD` stays at the target; `MOTION_END_RETURN` retraces back to the start over the same duration; `MOTION_END_SNAP_BACK` jumps back once the target is reached |
| start value | the macro's current value | `gooey_engine_motion_set_start` | a value in 0–1, or NaN for the current value. An explicit start makes a hold motion repeatable. |
| quantize | none | `gooey_engine_motion_set_quantize` | `MOTION_QUANTIZE_BEAT` or `MOTION_QUANTIZE_BAR` wait for the next beat or 4-beat bar while the transport runs. A seek while waiting re-aims at the next boundary from the new position. With the transport stopped, the motion starts immediately. |

Every setter has a matching getter. `gooey_engine_motion_clear` unconfigures a slot.

To run and watch motions:

- `gooey_engine_motion_trigger(engine, slot)` starts or restarts a slot at the next render buffer. Only one motion owns a macro at a time, so starting one stops any other motion, and the LFO, on the same macro. A quantized motion takes the macro when it is triggered, not when it begins.
- `gooey_engine_motion_stop` and `gooey_engine_motion_stop_all` freeze macros where they are.
- `gooey_engine_motion_get_state` returns one of `MOTION_STATE_IDLE`, `PENDING`, `RUNNING` or `RETURNING`.
- `gooey_engine_motion_get_progress` returns 0–1. For a return motion, the outbound leg covers 0–0.5.
- Both state and progress are published once per rendered buffer.

## Macro LFOs

Each macro has one LFO. Its output (0–1) becomes the macro's value, which then drives the macro's mappings exactly like a manual value: each parameter moves between its `from` and `to`.

| setting | default | setter | options |
|---|---|---|---|
| shape | sine | `gooey_engine_macro_lfo_set_shape` | `MACRO_LFO_SHAPE_SINE` (raised cosine 0 → 1 → 0), `TRIANGLE` (linear 0 → 1 → 0), `SAW` (ramp 0 → 1, then jump to 0), `SQUARE` (0 for the first half of the cycle, 1 for the second) |
| rate | one bar | `gooey_engine_macro_lfo_set_rate` | one cycle per `LFO_TIMING_SIXTEENTH`, `EIGHTH`, `QUARTER`, `HALF`, `ONE_BAR`, `TWO_BARS` or `FOUR_BARS` at the engine BPM. `LFO_TIMING_THIRTY_SECOND` is rejected. |

Both setters have matching getters, and the settings are independent per macro. Changing either on a running LFO takes effect at the next render buffer without resetting its phase.

- `gooey_engine_macro_lfo_start(engine, macro)` starts the LFO from phase 0 at the next render buffer. Every shape is 0 at phase 0, so the macro starts at 0 (each mapping at `from`). Calling it on a running LFO restarts the cycle and applies the latest settings.
- `gooey_engine_macro_lfo_reset_phase(engine, macro)` restarts a running LFO's cycle at phase 0 without changing anything else. It does nothing to a stopped LFO.
- `gooey_engine_macro_lfo_stop(engine, macro)` stops the LFO. The macro holds the value that `gooey_engine_macro_get_value` reported at the moment of the call (the value from the last rendered buffer), so the UI, the poly getters and the sound agree exactly. `gooey_engine_macro_lfo_stop_all` stops every LFO the same way. Stopping a stopped LFO succeeds and changes nothing.
- `gooey_engine_macro_lfo_get_state` returns `MACRO_LFO_STATE_RUNNING` or `MACRO_LFO_STATE_STOPPED`. It reflects start and stop calls, motion triggers and manual values immediately.
- `gooey_engine_macro_lfo_get_value` (0–1) and `gooey_engine_macro_lfo_get_phase` (0–1) are published once per rendered buffer. While the LFO runs, its value equals `gooey_engine_macro_get_value`.

Timing:

- Phase is free-running. The LFO keeps cycling while the transport is stopped, and starting, stopping or seeking the transport does not move it.
- A BPM change (`gooey_engine_set_bpm`) changes only the speed. The LFO continues from its current phase.
- LFOs advance with the other automation every 32 frames. A square wave's edges are therefore softened by the parameters' 10–15 ms smoothers.

## Ownership

A macro has one driver at a time. The newest one wins:

| action | effect on the macro's motion | effect on the macro's LFO |
|---|---|---|
| `gooey_engine_macro_lfo_start` | stops (including a pending, quantized one) | starts or restarts from 0 |
| `gooey_engine_motion_trigger` | starts or restarts | stops; the motion starts from the LFO's current value |
| `gooey_engine_macro_set_value` | stops | stops; the macro takes the manual value |
| `gooey_engine_macro_capture_commit` | stops | stops; the macro is set to 0 |

## Precedence

- Macros and motions update every 32 frames. The parameters' existing 10–15 ms smoothers fill in between updates.
- While a motion or macro LFO runs, it re-applies its macro on every update. It therefore overrides a poly preset re-apply and a sequencer blend snap.
- Routed LFOs (`gooey_engine_add_lfo_route`) are applied after macros, motions and macro LFOs. A routed LFO on the same drum parameter wins.
- When two macros map the same parameter, the higher-numbered macro owns it. The lower macro's mapping has no effect, whichever macro moved last.
- Poly parameter getters (`gooey_engine_poly_get_param`) show where a macro puts the parameter:
  - After a manual macro move, the getter reports the new value.
  - When a hold motion is triggered, the getter reports the motion's end value straight away.
  - When a macro LFO starts, the getter reports the LFO's start value (the mapping's `from`) until the LFO stops. On stop, it reports the held value.
  - This projection also keeps unrelated preset edits from snapping a macro-driven value back.
- Drum and effect getters read live values, so they animate while a motion or macro LFO runs.

## Threading

These calls use a render-boundary queue and atomics, so they are safe while rendering:

- mapping edits
- macro values
- motion and macro LFO settings
- trigger, start, stop and phase reset
- all getters

Calls that change a macro's owner (manual values, motion triggers, LFO start and stop) update the poly projection while holding the automation lock. Concurrent calls from different threads therefore leave the projection matching whichever command the render thread applies last.

The queue holds 128 commands. Consecutive manual values for the same macro are merged, so a fast knob drag doesn't fill it.

Capture begin, commit and cancel read and write parameters through the legacy setter paths. They must be serialized with rendering, exactly like `gooey_engine_set_*_param`.

The render thread never allocates, frees or blocks for macros, motions and macro LFOs; `tests/automation_alloc.rs` checks this.
