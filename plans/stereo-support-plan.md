# Stereo Support for libgooey — Multi-Pass Foundation

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. It must be maintained in accordance with `.agent/PLANS.md` (repository root).


## Purpose / Big Picture

libgooey was mono end-to-end: every `Instrument::tick` and `Effect::process` returns a single `f32`, both engines (the native `Engine` in `src/engine/mod.rs` and the FFI `GooeyEngine` in `src/ffi.rs`) sum to one mono sample, and the output layers duplicated that sample to every hardware channel. There was no stereo "currency" anywhere, so any future stereo feature (per-instrument panning, stereo reverb/delay/width) would require a cross-cutting refactor.

This work introduces a stereo **foundation** so those features become local changes later. After Pass 1, the engine produces a two-channel `StereoFrame` at a single conversion point (the "stereo seam"), the native CPAL output writes true left/right, and the iOS-facing FFI (`gooey_engine_render`) writes **interleaved stereo** (`frames * 2` floats, laid out `[L, R, L, R, ...]`). The signal path is still mono, so left and right are currently identical — but every consumer now treats output as stereo, which is what makes later passes cheap.

You can see it working by running `cargo test --test ffi_stereo` (asserts the buffer is `frames * 2`, that L == R for the mono path, and that a triggered kick produces audio on both channels) and by inspecting `include/gooey.h` for `#define GOOEY_OUTPUT_CHANNELS 2` and the interleaved-stereo contract on `gooey_engine_render`.


## Progress

- [x] (2026-06-10) Pass 1, Step 0: fast-forwarded `origin/main` (5 upstream PRs incl. FFI master-gain stage); re-verified green baseline.
- [x] (2026-06-10) Pass 1, Step 1: added `StereoFrame { l, r }` with `mono()`/`downmix()` in `src/frame.rs`; registered + re-exported in `src/lib.rs`.
- [x] (2026-06-10) Pass 1, Step 2: added `Engine::tick_stereo` seam in `src/engine/mod.rs` (mono `tick` untouched).
- [x] (2026-06-10) Pass 1, Step 3: native CPAL output writes true L/R in `src/engine/engine_output.rs` (new `write_stereo_frame` helper; viz buffer gets the downmix).
- [x] (2026-06-10) Pass 1, Step 4: `GooeyEngine::render` writes interleaved stereo (`chunks_mut(2)`, both lanes zeroed on silence/arm); `resolve_pending_arm` now receives the frame count; `gooey_engine_render` slices `frames * 2`; added `GOOEY_OUTPUT_CHANNELS`.
- [x] (2026-06-10) Pass 1, Step 5: updated all 9 FFI integration tests to the `frames * 2` buffer contract; added `tests/ffi_stereo.rs`; `StereoFrame` unit tests live in `src/frame.rs`.
- [x] (2026-06-10) Pass 1, Step 6: regenerated `include/gooey.h` (cbindgen); audited callers — only `gooey_engine_render` drives `GooeyEngine::render`; offline bounce stays mono (see Decision Log).
- [x] (2026-06-10) Pass 1, Step 7: this ExecPlan persisted.
- [ ] Pass 2: per-instrument pan (see roadmap).
- [ ] Pass 3: stereo-native effects (see roadmap).

The foundation shipped in two commits on branch `bhurlow/libgooey-stereo-support` (PR #201): first the native-side foundation (StereoFrame + CPAL + `tick_stereo`), then the FFI interleaving + test updates.


## Surprises & Discoveries

- Observation: The `frames * 2` contract change touched **nine** integration test files, not the two named in the original plan. Every FFI test allocated a `frames`-length (mono) buffer; with the new contract each would have been a half-size out-of-bounds write.
  Evidence: `rg "gooey_engine_render" tests/` matched `channel_instrument_swap`, `effect_order`, `ffi_gain_staging`, `ffi_granulator`, `instrument_gain`, `mute_solo`, `sequencer_armed_start`, `sequencer_triggers_enabled`, `volume_zero_mute`.
- Observation: No test indexes the audio buffer positionally (only whole-buffer peak/energy and element-wise comparisons), so simply doubling each buffer to `frames * 2` preserves every assertion — L == R means peaks, sums, and per-index comparisons are unchanged.
  Evidence: `rg "buffer\[|audio\[|\.get\(" tests/` → no matches.
- Observation: Two call sites passed `buffer.len()` as the frame count (`effect_order.rs`, `ffi_granulator.rs`); under the new contract that would request `2 * buffer.len()` samples. Fixed by passing an explicit frame count.
- Observation: `clippy --all-targets --all-features` already fails on the post-merge baseline because some examples use a stale `HiHat2` API (`amp_decay`, `set_open`, `closed_default`). Pre-existing, unrelated to stereo; library + tests are clippy-clean.
  Evidence: stashing all stereo changes and rerunning clippy reproduced the example errors.


## Decision Log

- Decision: Ship stereo in passes; Pass 1 is I/O foundation only (StereoFrame currency + interleaved FFI/CPAL output, L == R), with instruments and effects untouched.
  Rationale: User chose this scope explicitly; it de-risks the cross-cutting work and integrates to iOS immediately.
  Date/Author: 2026-06-10, Brian (via planning Q&A).
- Decision: Replace `gooey_engine_render` to write interleaved stereo (`frames * 2`) rather than adding a parallel `_stereo` function.
  Rationale: Single consumer (the user's iOS app), single clean path; the user updates the Swift call site against the regenerated header.
  Date/Author: 2026-06-10, Brian (via planning Q&A).
- Decision: Keep changes in this repo only (FFI, header, tests); the Swift call site is wired up by the user in the separate app repo.
  Rationale: The iOS app is not in this repository; libgooey ships as `libgooey.a` + `include/gooey.h`.
  Date/Author: 2026-06-10, Brian (via planning Q&A).
- Decision: Leave the offline bounce (`bounce.rs` / `gooey_engine_bounce_to_buffer`) mono for now.
  Rationale: Bounce is a separate offline path driven by the mono `Engine::tick`; it remains internally consistent and its tests pass. Stereo-ifying bounce is deferred to a later pass to keep Pass 1 focused on the realtime/iOS path.
  Date/Author: 2026-06-10, Claude.


## Context and Orientation

There are two distinct engines. The native `Engine` (`src/engine/mod.rs`) owns a `HashMap` of instruments and is driven by CPAL via `src/engine/engine_output.rs`. The FFI `GooeyEngine` (`src/ffi.rs`) is a separate, self-contained struct with a fixed array of channels, driven by the host (iOS) through `gooey_engine_render`. They do not share a render loop.

The "stereo seam" is the single line where the mono signal becomes two channels: `StereoFrame::mono(output)` (`src/frame.rs`). In the native engine it lives in `Engine::tick_stereo`; in the FFI engine it lives at the end of the per-frame loop in `GooeyEngine::render`. "Interleaved stereo" means one frame occupies two consecutive buffer slots, `[left, right]`.

`include/gooey.h` is generated by cbindgen from `src/ffi.rs` during `cargo build` (`build.rs`); never hand-edit it.


## Plan of Work (as executed)

In `src/frame.rs` (new), define `StereoFrame { pub l: f32, pub r: f32 }` with `mono(x) -> {l:x, r:x}` and `downmix() -> 0.5*(l+r)`; register `pub mod frame;` + `pub use frame::StereoFrame;` in `src/lib.rs`.

In `src/engine/mod.rs`, add `pub fn tick_stereo(&mut self, t: f64) -> StereoFrame { StereoFrame::mono(self.tick(t)) }`.

In `src/engine/engine_output.rs`, replace the "copy to all channels" loop in both `process_frame` and `process_frame_no_viz` with `engine_guard.tick_stereo(...)` + `Self::write_stereo_frame(frame, stereo)`. The helper writes the downmix for 1 channel, L/R for ≥2 channels, and fills any extra surround channels with the downmix. The visualization buffer receives `stereo.downmix()`.

In `src/ffi.rs`: import `StereoFrame`; in `GooeyEngine::render` compute `frame_count = buffer.len() / 2`, pass it to `resolve_pending_arm`, iterate `buffer.chunks_mut(2)`, `frame.fill(0.0)` for pre-fire silence, and at the seam write `frame[0] = stereo.l; frame.get_mut(1) = stereo.r`. In `gooey_engine_render`, size both slices `frames * 2` and update the doc. Add `pub const GOOEY_OUTPUT_CHANNELS: u32 = 2;`.

In `tests/`, resize every FFI render buffer to `frames * 2`, replace the two `buffer.len()`-as-frames call sites with explicit frame counts, and add `tests/ffi_stereo.rs`.


## Validation and Acceptance

Run from the repo root:

    cargo build                       # regenerates include/gooey.h
    cargo test                        # expect all suites pass, incl. tests/ffi_stereo.rs (3 tests)
    cargo fmt --all -- --check        # expect exit 0
    cargo clippy --lib --tests --all-features   # expect warnings only (no errors)

Acceptance (behavior):
- `cargo test --test ffi_stereo` passes: render fills exactly `frames * 2`, L == R per frame, a triggered kick is audible on both channels, and a correctly sized buffer does not set the engine error flag.
- `include/gooey.h` contains `#define GOOEY_OUTPUT_CHANNELS 2` and documents the interleaved `frames * 2` contract on `gooey_engine_render`.

Known/pre-existing: `cargo clippy --all-targets --all-features` and `cargo build --example kick` fail on stale examples (outdated `HiHat2` API) that predate this work.


## Idempotence and Recovery

All steps are additive and re-runnable. `StereoFrame::mono` keeps L == R, so the change is behavior-preserving for current mono content — if a regression appears, the seam is the single place to inspect. The merge was a clean fast-forward; re-running `cargo build` regenerates the header deterministically.


## Pass 2 / Pass 3 Roadmap (why the seam is shaped this way)

Pass 2 — Per-instrument pan: add a per-channel pan `SmoothedParam` to `GooeyEngine` (and the native `Engine`), replace `StereoFrame::mono(output)` at the seam with a per-channel panned sum into a stereo bus, expose `gooey_engine_set_channel_pan(channel, pan)`, and add left/right output peak meters. Only the seam and a few FFI setters change.

Pass 3 — Stereo-native effects: widen the `Effect` trait with `fn process_stereo(&self, StereoFrame) -> StereoFrame` (effects gain per-channel state), enabling stereo reverb/delay/ping-pong and width. The Pass 1 output boundary (FFI interleave, CPAL write, tests) is reused unchanged.


## Interfaces and Dependencies

In `src/frame.rs`:

    pub struct StereoFrame { pub l: f32, pub r: f32 }
    impl StereoFrame {
        pub const fn mono(x: f32) -> Self;
        pub fn downmix(self) -> f32;
    }

In `src/engine/mod.rs`:

    impl Engine { pub fn tick_stereo(&mut self, current_time: f64) -> StereoFrame; }

In `src/ffi.rs` (C ABI, reflected in `include/gooey.h`):

    pub const GOOEY_OUTPUT_CHANNELS: u32 = 2;
    // buffer must hold `frames * GOOEY_OUTPUT_CHANNELS` interleaved [L, R] floats
    pub unsafe extern "C" fn gooey_engine_render(engine: *mut GooeyEngine, buffer: *mut f32, frames: u32);


## Outcomes & Retrospective

Pass 1 complete. libgooey now emits interleaved stereo from the FFI and true L/R from CPAL, with a single seam (`StereoFrame::mono`) as the only mono→stereo conversion point. The main lesson: the realtime FFI buffer contract is exercised by far more tests than expected (nine files), but because no test indexes audio positionally, the migration reduced to a mechanical buffer-size doubling plus two explicit frame-count fixes. The offline bounce remains mono and is the most likely first follow-up if stereo export is needed.
