# Anti-Aliasing Effects — Implementation Plan

## Background

The libgooey codebase already has anti-aliasing infrastructure:
- **`utils/oversampler.rs`** — 2x and 4x oversampling using polyphase IIR half-band filters
- **`TubeSaturation`** — uses `Oversampler` for the nonlinear `arctan` path
- **`Waveshaper`** — uses `Oversampler` for the `tanh` path
- **`FeedbackWaveshaper`** — uses `Oversampler` for the `tanh` path
- **`gen/polyblep.rs`** — PolyBLEP anti-aliasing for oscillators

## Gaps to Fill

1. **`TubeCompressor` lacks oversampling** — the compressor's nonlinear "tube coloring" (`atan` soft-clip applied when gain reduction is active) is processed at the engine sample rate, creating aliasing harmonics.

2. **No FFI/control-surface exposure** — the oversampling mode is adjustable per-effect struct but there are no `EFFECT_*` / `*_PARAM_*` constants in the FFI to let the UI or host change oversampling at runtime.

3. **No cross-effect alias validation tests** — existing alias tests only validate the raw `Oversampler`. There are no end-to-end tests confirming each effect reduces aliasing.

## Changes

### 1. Add oversampling to TubeCompressor

- Add `Oversampler` to the per-channel `CompressorState`
- Wrap the `atan` tube-coloring path with `oversampler.process(input, |x| x.atan() * FRAC_2_PI * 1.1)`
- Add `set_oversampling_mode` / `oversampling_mode` methods
- Add `oversampling_mode_target: AtomicU8` field to `TubeCompressor`
- Add unit tests validating alias reduction

**File:** `src/effects/compressor.rs`

### 2. Add oversampling mode to the FFI constants

- Add `EFFECT_COMPRESSOR_OVERSAMPLING_PARAM` (or similar) to `src/ffi.rs`
- Wire `set_param` in `ChannelEffect` to accept the new param
- This lets the UI surface oversampling as a controllable parameter

**File:** `src/ffi.rs`

### 3. Add cross-effect anti-alias tests

- In `src/effects/` or a dedicated test file, add spectral comparison tests for each nonlinear effect (saturation, waveshaper, feedback waveshaper, compressor) that verify alias power drops by at least 10 dB when oversampling is enabled vs. off.
- Use the same Goertzel/bin-power approach from `utils/oversampler.rs` tests.

**File:** `src/effects/mod.rs` (add a test module)

## Order of Work

1. Add oversampling to `TubeCompressor` (most impactful gap)
2. Expose oversampling via FFI constants
3. Add cross-effect alias validation tests
4. Run `cargo test` to verify everything passes
5. Commit and push

## Edge Cases

- Changing oversampling mode mid-stream resets filter history (already handled by `Oversampler::set_mode`)
- NaN/infinity protection already exists in each effect's hot path
- Compressor's `reset()` must also reset the new oversampler
