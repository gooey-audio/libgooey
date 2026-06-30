# Tempo Warp (Audio Clip BPM Change) — Research

## Summary

libgooey is well-prepared for tempo warping. The architecture already has the hook,
implementation surface is small (~50 lines).

## How BPM Works Today

The `Engine` (`src/engine/mod.rs`) stores a global `bpm` field (default 120). Calling
`set_bpm()` propagates the new tempo to:

- **Sequencers** — trigger timing and swing calculation
- **LFOs** — BPM-synced modulation rates (bars, beats, subdivisions)
- **Mixer** — re-tempos note-synced per-channel effects (e.g. delay)
- **Bounce** — bar/beat-to-sample conversion for offline rendering

The FFI layer (`src/ffi.rs`) also exposes external tempo source support (Ableton Link
integration) via `gooey_engine_set_external_tempo_source()` and
`gooey_engine_is_external_tempo_source()`.

## How Audio Clip Playback Works

`LoopChannel` (`src/mixer/loop_channel.rs`) plays back `StereoSampleBuffer` data.
Cursor advance per output sample is:

    cursor += speed * (source_sample_rate / engine_sample_rate)

Where `speed` is the user-controlled varispeed (`set_speed()`, range ±4x).

## The Tempo Warp Hook

In `src/mixer/loop_channel.rs` lines 7-8, the architecture explicitly documents the
intended approach:

> *this is the hook for the future tempo-warp phase — warping simply multiplies the
> advance by `engine_bpm / source_bpm` (see the plan's "Tempo warping" phase)*

## Implementation Plan

Three changes, all additive:

### 1. StereoSampleBuffer — store source BPM

In `src/mixer/stereo_buffer.rs`, add an optional `source_bpm` field:

```rust
pub struct StereoSampleBuffer {
    left: Arc<[f32]>,
    right: Arc<[f32]>,
    sample_rate: f32,
    source_bpm: Option<f32>,  // NEW
}
```

Add a `with_bpm()` builder or a `set_source_bpm()` method. When loading a loop from a
file, the host can optionally tag it with its original BPM.

### 2. LoopChannel — tempo warp toggle

In `src/mixer/loop_channel.rs`, add:

```rust
tempo_warp: bool,           // NEW field
engine_bpm: f32,            // NEW — cached from mixer/engine
```

Add public methods:

```rust
pub fn set_tempo_warp(&mut self, enabled: bool) { self.tempo_warp = enabled; }
pub fn tempo_warp(&self) -> bool { self.tempo_warp }
pub(crate) fn set_engine_bpm(&mut self, bpm: f32) { self.engine_bpm = bpm; }
```

### 3. LoopChannel::advance() — apply warp ratio

In the `advance()` method, when `tempo_warp` is enabled and a source BPM exists,
multiply the cursor advance by the warp ratio:

```rust
fn advance(&mut self, engine_sample_rate: f32) {
    // ... existing code ...

    let mut ratio = buffer.sample_rate() as f64 / engine_sample_rate.max(1.0) as f64;

    // NEW: tempo warp multiplier
    if self.tempo_warp {
        if let Some(source_bpm) = buffer.source_bpm {
            if source_bpm > 0.0 && self.engine_bpm > 0.0 {
                ratio *= self.engine_bpm as f64 / source_bpm as f64;
            }
        }
    }

    ratio *= self.speed as f64;
    self.cursor += ratio;
    // ... existing loop wrapping ...
}
```

The tempo warp ratio combines multiplicatively with the user's `speed` setting, so a
user can still apply additional varispeed on top of BPM-matched playback.

### Wiring

The `Mixer::set_bpm()` method already iterates channels to re-tempo effects. Extend it to
also call `channel.set_engine_bpm(bpm)`.

## Existing Test Coverage

- `tests/loop_mixer.rs` — loop channel playback
- `src/effects/delay.rs` — `test_delay_bpm_change_updates_time` (delay already handles BPM changes)
- `src/engine/sequencer.rs` — `test_swing_preserves_average_tempo`

New tests needed:
- Loop channel tempo warp: same buffer played at different engine BPMs produces different durations
- Tempo warp disabled: buffer plays at original speed regardless of engine BPM
- Tempo warp with no source BPM: falls back to normal playback
- Tempo warp × speed interaction: both factors combine correctly

## Files Touched

| File | Change |
|---|---|
| `src/mixer/stereo_buffer.rs` | Add `source_bpm` field + accessor |
| `src/mixer/loop_channel.rs` | Add `tempo_warp`, `engine_bpm` fields; update `advance()` |
| `src/mixer/mod.rs` | Wire `set_bpm()` to propagate to channels |
| `src/ffi.rs` | Optional: expose via C API |

## Complexity

**Low.** ~50 lines of new code, all additive. No breaking changes to existing APIs.
The architecture anticipated this feature.
