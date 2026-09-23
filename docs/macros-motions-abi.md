# Macros and Motions C ABI

Macros and motions let a host add movement to a performance without a timeline.

- A **macro** is a single 0–1 control connected to up to 16 parameters. Each connection, called a mapping, stores the parameter's value at macro 0 (`from`) and at macro 1 (`to`). A host can show a tray of macro knobs and play them by hand.
- A **motion** automates one macro's value once per trigger. It ramps the macro to a target value over a duration in beats or milliseconds, following a curve, and then stops. Motions can be retriggered, and many can run at the same time.

Everything is addressed by index: 16 macros (`MACRO_COUNT`) and 32 motion slots (`MOTION_SLOT_COUNT`). The functions are declared in `include/gooey.h`. A runnable demo is `cargo run --example macro_motion --features native,crossterm`.

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
   - The macro is set to 0, and any motion on that macro stops.
   - `MACRO_CAPTURE_MERGE` adds the changes to the macro's existing mappings. A parameter the macro already drives keeps its `from` value and takes the new `to` value.
   - It returns the macro's mapping count, or one of these errors:
     - `MACRO_CAPTURE_ERROR_INVALID`: bad arguments, or no capture in progress.
     - `MACRO_CAPTURE_ERROR_TOO_MANY`: more than 16 mappings would result. The capture stays open.
4. `gooey_engine_macro_capture_cancel(engine, revert)` ends a capture without registering it. It restores the changed parameters when `revert` is true.

Mappings can also be edited directly:

- `gooey_engine_macro_add_mapping` adds or updates a mapping, and `gooey_engine_macro_remove_mapping` / `gooey_engine_macro_clear` remove them.
- `gooey_engine_macro_get_mapping_count` and `gooey_engine_macro_get_mapping` read them back.

## Playing macros

`gooey_engine_macro_set_value(engine, macro, value)` moves a macro by hand. Any motion driving that macro stops, the way grabbing a fader overrides automation. `gooey_engine_macro_get_value` returns the macro's live position, which keeps changing while a motion runs.

A macro writes its parameters only when its value changes. If the host edits a mapped parameter directly, that edit stays until the macro moves again.

## Motions

`gooey_engine_motion_configure(engine, slot, macro, target)` points a slot at a macro and a target value. A slot that has never been configured gets these defaults:

| setting | default | setter | options |
|---|---|---|---|
| duration | 4 beats | `gooey_engine_motion_set_duration` | `MOTION_DURATION_BEATS` (follows tempo changes) or `MOTION_DURATION_MS` |
| curve | linear | `gooey_engine_motion_set_curve` | `MOTION_CURVE_LINEAR`, `EASE_IN`, `EASE_OUT`, `S_CURVE` |
| end mode | hold | `gooey_engine_motion_set_end_mode` | `MOTION_END_HOLD` stays at the target; `MOTION_END_RETURN` retraces back to the start over the same duration; `MOTION_END_SNAP_BACK` jumps back once the target is reached |
| start value | the macro's current value | `gooey_engine_motion_set_start` | a value in 0–1, or NaN for the current value. An explicit start makes a hold motion repeatable. |
| quantize | none | `gooey_engine_motion_set_quantize` | `MOTION_QUANTIZE_BEAT` or `MOTION_QUANTIZE_BAR` wait for the next beat or 4-beat bar while the transport runs. With the transport stopped, the motion starts immediately. |

Every setter has a matching getter. `gooey_engine_motion_clear` unconfigures a slot.

To run and watch motions:

- `gooey_engine_motion_trigger(engine, slot)` starts or restarts a slot at the next render buffer. Only one motion owns a macro at a time, so starting one stops any other motion on the same macro.
- `gooey_engine_motion_stop` and `gooey_engine_motion_stop_all` freeze macros where they are.
- `gooey_engine_motion_get_state` returns one of `MOTION_STATE_IDLE`, `PENDING`, `RUNNING` or `RETURNING`.
- `gooey_engine_motion_get_progress` returns 0–1. For a return motion, the outbound leg covers 0–0.5.
- Both state and progress are published once per rendered buffer.

## Precedence

- Macros and motions update every 32 frames. The parameters' existing 10–15 ms smoothers fill in between updates.
- While a motion runs, it re-applies its macro on every update. It therefore overrides a poly preset re-apply and a sequencer blend snap.
- LFOs are applied after macros. An LFO routed to the same drum parameter wins.
- When two macros map the same parameter, the higher-numbered macro wins.
- Poly parameter getters (`gooey_engine_poly_get_param`) show where a macro puts the parameter:
  - After a manual macro move, the getter reports the new value.
  - When a hold motion is triggered, the getter reports the motion's end value straight away.
  - This projection also keeps unrelated preset edits from snapping a macro-driven value back.
- Drum and effect getters read live values, so they animate while a motion runs.

## Threading

These calls use a render-boundary queue and atomics, so they are safe while rendering:

- mapping edits
- macro values
- motion settings
- trigger and stop
- all getters

The queue holds 128 commands. Consecutive manual values for the same macro are merged, so a fast knob drag doesn't fill it.

Capture begin, commit and cancel read and write parameters through the legacy setter paths. They must be serialized with rendering, exactly like `gooey_engine_set_*_param`.

The render thread never allocates, frees or blocks for macros and motions; `tests/automation_alloc.rs` checks this.
