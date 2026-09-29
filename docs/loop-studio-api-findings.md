# APIs exercised by the loop-studio POC

This distinguishes implemented APIs from production work suggested by actual
prototype failures. See [usage](loop-studio.md), [evidence](loop-studio-verification.md)
and `tests/studio.rs`. Proposed interfaces below are **not** implemented.

## Existing engine capabilities reused

`src/mixer/graph.rs` already routes drums, bass, poly synth and audio-loop
submix into named strips with smoothed gain/balance, mute/solo, meters and
independent effect racks. The FFI engine sums strips into global effects and
a limiter. Track/master processing did not require another DSP engine or a
breaking C ABI change.

`gooey_engine_chord_loop_replace` installs prepared immutable chord clips.
The performance recorder captures held gates at 96 ticks per quarter note
and supports overdub. Drum/bass step patterns and PCM loops share the engine
transport; imported audio supports pitch-preserving stretch when tempos differ.

The single-producer endpoint created by `gooey_engine_live_control_new`
provides generation-tagged queued controls and retirement of prepared state.
It should underpin a production render/control split, rather than another
unbounded queue. The POC's serialized `Studio` does not integrate it yet.

## Implemented additive Rust session layer

Enable `studio` to use `gooey::studio::{Session, Studio, Control}`. The GUI,
headless `studio_render` and tests all use the same library API.

`Session` is a versioned, validated musical document: drum/bass rows and notes,
four strips, master controls, chord gates, recorded hits, automation lanes and
optional embedded stereo PCM. `save`, `load`, `import_wav` and `export_wav`
provide actual file workflows. Load validates before replacement. Save uses
a sibling temporary file and rename; WAV export streams fixed-size blocks
from a fresh engine.

`Studio` uniquely owns the engine, never exposes its raw pointer, and frees it
with the matching lifecycle function. Rendering/control require exclusive
mutable access. It may move between threads, but is not an independently
shared concurrent endpoint. Methods include `play`, `record`, `chord_on/off`,
`hit`, `set_step`, `set_control`, `set_tempo`, `rewind`, `render`,
`snapshot_session` and `replace`.

`Control` identifies strip gain/pan/mute/solo/filter/delay/reverb, master
gain/delay/reverb and bass/chord tone. Musical-tick `Lane` points replay held
targets through existing smoothing. An integer frame count determines the
cursor, and rendering splits buffers at tick boundaries. Recording uses a
tick-zero baseline, replaces writes at a touched tick, and suspends parameter
replay until recording is disarmed.

Finalized chord recordings become last-take-wins gates: newest takes own
covered ticks, older uncovered fragments remain, and seam-crossing fragments
join. The same canonical clip is installed into live replay and saved/exported.
This fixes overlaps exposed by real GUI overdubbing, not merely file validation.

## Production APIs made necessary by the POC

### One authoritative transport with pause/seek semantics

The legacy sequencer's Start schedules an immediate trigger; the original
studio cursor instead continued from its stopped beat. That is not a valid
pause/resume contract. The POC explicitly restarts at zero on Stop→Play and
active tempo edits. Per-bar reanchoring bounds legacy floating-point drift,
tested with fractional step sizes over 1001 bars. PCM phrase positions remain
intact during those anchors.

A production transport should own integer render-frame and musical positions,
publish them to every scheduler, and distinguish start-from-position, resume,
pause, stop and seek. Define whether seeks fire boundaries, release voices,
preserve tails, or retrigger sustained events. Tempo-map edits should retain
a defined musical position. Apply that contract to synthesis, PCM, automation,
live gestures, host sync and offline jobs together.

### Stable identity, instrument instances and flexible clips

Four fixed source indices are not durable identifiers for reorderable tracks,
and bass/poly sources are single instances. A song model needs stable track,
instrument-instance, clip, effect-instance and parameter IDs independent of
strip/rack order. Prepare creation/removal/routing off render and install one
whole generation atomically so automation cannot retarget a reordered slot.

Clip APIs should expose length, launch quantization, source offset, repetition,
record mode and seam-gate policy. Today's four-beat/384-tick format is a POC
constraint, not a permanent file contract. An arrangement can then reference
clips instead of copying synthesis state into a second scheduler.

### Render-owned automation and performance transactions

Production recording needs timestamp/ordering semantics for queued UI/MIDI
gestures, bounded overflow reporting and completed-take snapshots without
copying PCM in a callback. Define additive/last-take overdub, replace ranges,
trim/clear, held seam gates, and automation read/touch/latch/write behavior.
Parameter metadata should carry units, ranges, scaling, smoothing and discrete
values rather than GUI-only labels.

Prepared immutable lanes/events should install as one transaction with a
generation acknowledgement. Reuse existing live-control retirement and
chord-loop replacement. Interpolation curves and tempo automation are future
additions, not promises made by the current step-held player.

### Nonblocking ownership and offline job contracts

The CPAL example shares a mutex with drawing, snapshot copies and some control
updates. Passing tests/soaks do not make that a deadline guarantee. The next
safe host adapter should separate a render owner, bounded producer endpoint,
and compact published cursor/meters; prepare/retire samples, racks and sessions
off render.

Offline jobs should reuse immutable song state but own their DSP history,
exposing progress/cancellation, overwrite policy, atomic completion, output
rate/format, tails, stems and clipping analysis. The POC demonstrates fresh
mixdown and same-build byte-identical re-export of a GUI-recorded song, not
cross-platform identity or matching arbitrary live DSP history.

## Suggested next milestones

First wrap existing queued live control and add render allocation/deadline
tests. Next unify pause/seek/tempo and remove host-side bar repairs. Generalize
IDs, instrument instances and clip lengths before adding an arrangement editor.
Keep the captured Save/Export/Load/replay scenario as an end-to-end regression:
its first run found a musical-state bug missed by compilation and fifteen tests.
