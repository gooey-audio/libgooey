# FFI Export for Effects — Implementation Plan

## Problem

Two effect structs exist in `src/effects/` but are not exposed through the FFI:

| Effect | Rust Struct | EFFECT_* constant | PARAM constants | ChannelEffect | Engine setter |
|---|---|---|---|---|---|
| **Waveshaper** | `Waveshaper` | ❌ | ❌ | ❌ | ❌ |
| **FeedbackWaveshaper** | `FeedbackWaveshaper` | ❌ | ❌ | ❌ | ❌ |
| (all others) | — | ✅ | ✅ | ✅ | ✅ |

## Changes (all in `src/ffi.rs` and `src/mixer/effect_chain.rs`)

### 1. EFFECT constants (`src/ffi.rs`)
- Add `EFFECT_WAVESHAPER = 7`
- Add `EFFECT_FEEDBACK_WAVESHAPER = 8`
- Update `EFFECT_COUNT` to 9
- Update `REORDERABLE_EFFECT_COUNT` to 8

### 2. PARAM constants (`src/ffi.rs`)
- `WAVESHAPER_PARAM_DRIVE = 0`
- `WAVESHAPER_PARAM_MIX = 1`
- `FEEDBACK_WAVESHAPER_PARAM_DRIVE = 0`
- `FEEDBACK_WAVESHAPER_PARAM_FEEDBACK = 1`
- `FEEDBACK_WAVESHAPER_PARAM_FILTER_CUTOFF = 2`
- `FEEDBACK_WAVESHAPER_PARAM_MIX = 3`

### 3. ChannelEffect variants (`src/mixer/effect_chain.rs`)
- Add `Waveshaper(Waveshaper)` variant
- Add `FeedbackWaveshaper(FeedbackWaveshaper)` variant
- Update `from_id()` for both
- Update `effect_type()` for both
- Update `process_stereo()` for both
- Update `set_param()` for both
- Update `reset()` for both

### 4. Engine param setter (`src/ffi.rs`)
- Add `EFFECT_WAVESHAPER` and `EFFECT_FEEDBACK_WAVESHAPER` cases in `gooey_engine_set_global_effect_param`

### 5. Default effect order
- Add to `DEFAULT_EFFECT_ORDER`

### 6. Tests
- Run `cargo test` to verify all 228+ tests still pass

## Edge Cases
- Waveshaper and FeedbackWaveshaper use `&mut self` (not `UnsafeCell`), so they need to be wrapped in `UnsafeCell` for the multi-channel stereo path in ChannelEffect, OR the ChannelEffect needs interior mutability
- Actually, looking at the existing ChannelEffect, each variant uses different approaches. I need to wrap in `Mutex` or similar since ChannelEffect uses `&self` in process_stereo
