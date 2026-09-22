# Live-control C ABI

`GOOEY_LIVE_CONTROL_API_VERSION` is `1`. This additive interface lets one host
control thread safely submit mixer and drum changes while another thread calls
`gooey_engine_render`. It does not turn the mixer graph into a live graph
editor: create tracks, register and route sources, and set names only while
rendering is stopped, then attach one `GooeyLiveControl`.

## Ownership and lifetime

Call `gooey_engine_live_control_new(engine)` after stopped-render configuration
and before concurrent rendering. An engine permits one such attachment for its
lifetime. The returned handle is single-producer: serialize every
`gooey_live_control_*` submission on one control thread. The render callback is
the only consumer and the only owner of mutable DSP graph state.

Release the handle with `gooey_live_control_free`. Normally stop the audio
callback, release the handle, and then call `gooey_engine_free`. Engine free sets
a shutdown flag, rejects new submissions/renders, and waits for an in-flight
render and the attached control handle to detach. The host must prevent new raw
engine-pointer calls once free begins; no C API can make a call that starts
after the pointer was destroyed safe. A static entry counter is acquired before
render first dereferences the engine pointer, then handed off to a per-engine
active-render count. Lifecycle state lives in an allocation prefix separate
from mutable DSP state, so free cannot release either region during that entry
window or an active callback.

Caller-owned pattern and descriptor memory only needs to remain valid for the
duration of its submission call. The function copies every POD and constructs
effect graphs before returning. Nonzero pointer counts require nonnull pointers;
null plus a zero rack count means clear the rack.

## Ordering, capacity, and generations

The producer-to-render queue contains exactly
`GOOEY_LIVE_CONTROL_QUEUE_CAPACITY` (64) commands. Commands are FIFO, and one
render call containing at least one frame consumes the complete bounded queue
before producing its first sample. A zero-frame call is not a render boundary
and does not apply or acknowledge commands. Therefore every accepted command
published before a real boundary becomes active at that boundary. A full queue
rejects the new call without displacing or modifying earlier state.

Every accepted submission returns a unique monotonically increasing nonzero
`uint64_t`. Zero always means rejection. Gaps are possible after an internal
publication race and must not be interpreted as missing state. Poll
`gooey_live_control_get_last_applied_generation`; it advances with release/acquire
ordering after the whole command is installed. For a smoother, "applied" means
its new target is installed, not that the ramp has settled.

A rack replacement's submission generation is also that rack's identity.
`gooey_live_control_set_track_effect_param` must present this generation along
with track and slot. A stale, zero, or wrong generation is rejected, including
when a newer replacement is queued but not rendered. Parameter edits mutate the
existing effect and preserve its delay/reverb tail.

## Validation and audio behavior

Track gain and source trim accept finite linear values from `0.0` through `2.0`.
Track gain retains its existing 10 ms ramp. Each source has an independent 15 ms
pre-route trim, defaulting to exactly `1.0`; trim is applied before source
accumulation, then the track fader/balance runs before track inserts.

`GooeyDrumPattern` always contains four lanes by sixteen steps. `enabled` must be
exactly zero or one and velocity must be finite in `0.0...1.0`. Installation
replaces all 64 cells as one command and clears per-cell note/blend metadata.
Disabled or unused lanes consequently remain empty and cannot inherit a factory
pattern. Transport and sequencer running state are not changed.

A live rack has at most eight ordered effects and supports:

- `EFFECT_LOWPASS_FILTER`: cutoff `20...20000` Hz and resonance `0...0.95`;
- `EFFECT_DELAY`: an exact `DELAY_TIMING_*` value, feedback `0...0.95`, and mix
  `0...1`;
- `EFFECT_REVERB`: spring decay, mix, and damping in `0...1`.

Omitted parameters use the existing track-effect defaults: open low-pass;
quarter-note delay with feedback/mix `0.3` and internal cutoff 8 kHz; spring
decay `0.5`, mix `0.3`, and damping `0.5`. Duplicate, unknown, nonfinite, or
out-of-range parameters reject the complete replacement. Delay BPM is refreshed
when the prepared rack reaches render.

Rack replacement crossfades old and new post-fader output for 10 ms. A second
replacement for that track is rejected until the old rack has crossed out and
entered the retirement queue. Retired racks are reclaimed and destroyed by the
control thread on its next API call, never by render. Empty replacement racks
clear inserts with the same transition.

## Rejection and thread safety

All validation occurs before queue publication. Rejection returns zero and
leaves projected rack generations/layouts and render state unchanged. Besides
invalid values and queue capacity, calls reject an invalid/inactive source,
missing track/slot, stale rack generation, active rack transition, shutdown, or
malformed pointer/count pair.

The command and retirement paths use fixed SPSC rings with acquire/release
atomics. Render does not allocate or deallocate effect graphs, lock a mutex,
wait, or invoke host code while applying live controls. The applied-generation
getter reads shared atomic status and never dereferences live DSP or meter state.

Legacy `gooey_engine_mixer_*`, `gooey_engine_track_effect_*`, layout/routing
getters, and other direct engine APIs retain their historical host-serialized
contract. Do not call them concurrently with render merely because a live
control handle also exists.
