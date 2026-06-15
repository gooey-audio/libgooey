# Stereo Effects for libgooey — Pass 3 (Foundation + Ping-Pong Showcase)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. It must be maintained in accordance with `.agent/PLANS.md` (repository root). It continues the stereo effort begun in `plans/stereo-support-plan.md` (Pass 1).


## Purpose / Big Picture

libgooey shipped a stereo **foundation** in Pass 1 (`plans/stereo-support-plan.md`): a `StereoFrame { l, r }` currency, a single "stereo seam" (`StereoFrame::mono(x)` — the one place the mono signal path becomes two channels), interleaved FFI output, and true left/right from the native CPAL device. But that seam sits **after** the global effects chain in both engines, so every effect still only ever processes one mono sample — left and right are byte-identical.

Pass 3 makes the effects themselves stereo-aware. After this change a user can hear a genuinely stereo effect: enabling the delay's new **ping-pong** mode makes echoes bounce between the left and right speakers. The work has two layers:

1. **Foundation** — widen the `Effect` trait with a `process_stereo` method, move the seam to *before* the effects chain, and give every stateful effect genuine per-channel DSP state. Like Pass 1, this is behavior-preserving: with mono input (left == right) and identical per-channel state, the output stays left == right.
2. **Showcase** — make the delay genuinely stereo with an opt-in ping-pong mode (echoes alternate channels, producing audible left != right), proving the path end to end.

This applies to **both** engines: the FFI `GooeyEngine` (the iOS/host path driven by `gooey_engine_render`) and the native `Engine` (the desktop CPAL path). The offline mono `bounce` path stays mono and untouched.

You can see it working by running `cargo test --test stereo_effects`: with ping-pong off, an enabled effect keeps left == right for mono content; with ping-pong on, a triggered impulse makes left and right diverge.


## Progress

- [x] (2026-06-15) Step 1: widened the `Effect` trait with `process_stereo` (default impl, documented as stateless-only) in `src/effects/mod.rs`.
- [x] (2026-06-15) Step 2: per-channel state refactor for the 5 stateful effects + compressor (`[State; 2]` + extracted `process_one`/`process_inner` + `process_stereo` override; `reset()` clears both channels). All 64 effect unit tests stay green.
- [x] (2026-06-15) Step 3: moved the stereo seam before the effects chain in both engines (FFI `GooeyEngine::render`; native `Engine::tick_stereo` via a new `render_pre_effects` split keeping mono `tick()` byte-identical).
- [x] (2026-06-15) Step 4: ping-pong delay showcase — `DelayEffect` split into `step_read`/`step_write`, cross-channel feedback, dry injected left-only, `pingpong_target` atomic + `set_pingpong`/`get_pingpong`.
- [x] (2026-06-15) Step 5: FFI `DELAY_PARAM_PINGPONG = 4` const + set/get dispatch; `cargo build` regenerated `include/gooey.h` (const present, `GOOEY_OUTPUT_CHANNELS` still 2).
- [x] (2026-06-15) Step 6: added `tests/stereo_effects.rs` (3 tests: per-effect L==R foundation, ping-pong L≠R divergence, ping-pong-off dual-mono). Existing `ffi_stereo`, `effect_order`, `ffi_gain_staging`, `bounce` suites stay green.
- [x] (2026-06-15) Step 7: full validation — `cargo test` all suites pass; `cargo fmt --all -- --check` clean; `cargo clippy --lib --tests` introduced zero new warnings; `cargo build --example kick --features native,crossterm` builds.


## Surprises & Discoveries

- Observation: A genuinely-stereo ping-pong with a *centered (mono)* input cannot diverge by symmetric crossed feedback alone — swapping L↔R is a symmetry of the system when inputs are equal, so the channels would stay identical forever.
  Resolution: the dry input is injected only into the LEFT delay line (the right buffer is fed solely by the crossed feedback). That asymmetry is what makes the echoes bounce L→R→L from a centered impulse. Implemented via `step_write`'s separate `dry_input` (for the wet/dry output mix, kept on both channels so the dry stays centered) and `inject_input` (buffer injection, left-only in ping-pong).
- Observation: the delay's per-sample DSP had to be split into a read phase (`step_read`: advance smoothers + feedback-path filter, produce the tap) and a write phase (`step_write`: inject + feedback + buffer write). The split is safe because reads happen at `write_index - delay` and writes at `write_index` — different buffer slots — so computing both channels' taps before either write does not corrupt the buffers.
- Observation: clippy on `--lib --tests` is NOT clean on the current baseline (drifted since Pass 1: `Envelope`/`EngineOutput` Default impls, `seq_triggers` index loops, the relocated `saved_global_freq` contains_key+insert, the untouched `EFFECT_LIMITER` match arm). None of these are in this change; this work added zero new warnings.


## Decision Log

- Decision: Scope Pass 3 as foundation + one genuinely-stereo showcase effect (ping-pong delay), across both engines.
  Rationale: User chose this explicitly; the foundation de-risks the cross-cutting trait change while the showcase proves audible stereo end to end.
  Date/Author: 2026-06-15, Brian (via planning Q&A).
- Decision: Keep mono `Engine::tick()` and the offline bounce byte-identical; introduce a `render_pre_effects` helper and run effects in stereo only inside `tick_stereo`.
  Rationale: `bounce.rs`, `tests/bounce.rs`, and `examples/bounce.rs` depend on mono `tick()`. The split isolates the stereo change to the realtime path.
  Date/Author: 2026-06-15, Claude.
- Decision: The `Effect::process_stereo` default impl (call `process` per channel) is correct only for stateless effects; every stateful effect overrides it with genuine per-channel state.
  Rationale: The default double-advances a single shared state per frame, corrupting filters/delays. Only `SoftLimiter`/`BrickWallLimiter` are stateless.
  Date/Author: 2026-06-15, Claude.
- Decision: The compressor sidechain detector stays mono, duplicated to both channels via `StereoFrame::mono`.
  Rationale: The sidechain source (`channel_outs[sc]`) is a single mono per-instrument sample; both channels should receive matching gain reduction.
  Date/Author: 2026-06-15, Claude.


## Context and Orientation

There are two engines. The native `Engine` (`src/engine/mod.rs`) owns a `HashMap` of instruments and a `global_effects: Vec<Box<dyn Effect>>`, driven by CPAL via `src/engine/engine_output.rs`. The FFI `GooeyEngine` (`src/ffi.rs`) is a self-contained struct with concrete effect fields (`delay`, `reverb`, `saturation`, `compressor`, `lowpass_filter`, `tilt_filter`, `limiter`), each paired with a `*_enabled: bool`, plus a reorderable `effect_order: [u32; 6]`. It is driven by the host through `gooey_engine_render`.

The `Effect` trait (`src/effects/mod.rs`) is `fn process(&self, input: f32) -> f32` — note `&self`; effects use interior mutability. Every **stateful** effect follows one pattern: tunable parameters live in `AtomicU32` fields (lock-free, shared across both channels), and per-sample DSP state (delay lines, filter memory, envelope followers, smoothers) lives in `state: UnsafeCell<XxxState>` with `unsafe impl Send/Sync`. `process()` does `let state = unsafe { &mut *self.state.get() };` and runs the DSP. The stateful effects are `TubeSaturation`, `LowpassFilterEffect`, `TiltFilterEffect`, `DelayEffect` (also has `process_with_sidechain` — no, that is the compressor), `TubeCompressor` (has `process_with_sidechain`), and `SpringReverbEffect`. The **stateless** effects are `SoftLimiter` and `BrickWallLimiter` (`src/effects/limiter.rs`): pure functions of input + immutable fields.

`StereoFrame` (`src/frame.rs`): `{ l: f32, r: f32 }`, with `mono(x) -> {l:x, r:x}` and `downmix() -> 0.5*(l+r)`. `include/gooey.h` is generated by cbindgen on `cargo build` — never hand-edit.


## Plan of Work

### Step 1 — Widen the `Effect` trait (`src/effects/mod.rs`)

Import `crate::frame::StereoFrame`. Add a default-provided `process_stereo(&self, StereoFrame) -> StereoFrame` whose default body calls `process` once per channel. Document loudly that the default is correct only for stateless effects (it advances shared state twice per frame); every stateful effect must override it. The two limiters keep the default.

### Step 2 — Per-channel state refactor (5 stateful effects + compressor)

Uniform mechanical pattern per effect:

1. `state: UnsafeCell<XxxState>` becomes `state: UnsafeCell<[XxxState; 2]>`, duplicating per-channel DSP state and the `SmoothedParam` smoothers (both channels read the same atomics, so with equal input they stay bit-identical).
2. Extract the current `process()` body into a private `fn process_one(&self, state: &mut XxxState, input: f32) -> f32`.
3. `process()` runs `process_one` on `states[0]` only (mono path byte-identical to today). `process_stereo()` runs it on `states[0]` for left and `states[1]` for right.
4. `reset()` clears both array elements.

Per-effect notes: `TubeSaturation` has two independent `Oversampler`s + DC blockers, and its NaN-path reset must be scoped to the passed-in channel state. `TiltFilterEffect` has two `StateVariableFilterTpt` instances. `SpringReverbEffect` doubles to 12 allpass buffers (~21 KB, acceptable). `TubeCompressor` gets `[CompressorState; 2]`, an extracted `process_inner(&mut CompressorState, input, sidechain)`, and a new `process_stereo_with_sidechain(StereoFrame, StereoFrame) -> StereoFrame`; its `Effect::process_stereo` runs per channel with `sidechain == input`. `DelayEffect` is handled in Step 4.

### Step 3 — Move the seam before the effects chain

FFI `GooeyEngine::render`: keep the pre-fire silence path (`frame.fill(0.0); continue;`). After `output *= self.master_gain.tick();`, build `let mut stereo = StereoFrame::mono(output);`, run each enabled effect as `stereo = self.<effect>.process_stereo(stereo);` (compressor uses `process_stereo_with_sidechain(stereo, StereoFrame::mono(channel_outs[sc]))` when a sidechain is set, else `process_stereo`), then limiter, then `frame[0] = stereo.l; if let Some(r) = frame.get_mut(1) { *r = stereo.r; }`.

Native `Engine`: factor the pre-effects work (instrument sum + LFO/sequencer/trigger + master gain) into `fn render_pre_effects(&mut self, t: f64) -> f32`. `tick()` = `render_pre_effects` + mono `process` loop (unchanged behavior). `tick_stereo()` = `StereoFrame::mono(render_pre_effects(t))` then `process_stereo` loop.

### Step 4 — Ping-pong delay showcase (`src/effects/delay.rs`)

`state` becomes `UnsafeCell<[DelayState; 2]>`. Add `pingpong_target: AtomicU32` (0=off default) + `set_pingpong`/`get_pingpong`. Generalize the write-back line `let write_sample = input + filtered_delay * feedback;` so the feedback tap can be the partner channel's `filtered_delay`. `process()` (mono) uses `states[0]` with its own tap (byte-identical). `process_stereo()`: ping-pong off = independent dual-mono (left == right for mono input); ping-pong on = compute each channel's filtered tap, then write left's buffer with right's tap and vice versa, so echoes alternate channels (audible left != right). `reset()` clears both.

### Step 5 — FFI surface (`src/ffi.rs`)

Add `pub const DELAY_PARAM_PINGPONG: u32 = 4;` near the delay param constants, wire it into the `EFFECT_DELAY` arms of `gooey_engine_set_global_effect_param` (`set_pingpong(value >= 0.5)`) and `..._get_...`. No new FFI function. `GOOEY_OUTPUT_CHANNELS` stays 2. `cargo build` regenerates `include/gooey.h`.

### Step 6 — Tests (`tests/stereo_effects.rs`, new)

Foundation: enable each reorderable effect (ping-pong off), trigger a kick, render `frames*2`, assert `frame[0] == frame[1]` for every frame. Showcase: enable delay only, set `DELAY_PARAM_PINGPONG = 1.0`, high feedback, mix = 1.0, trigger an impulse, render across >= 2 delay periods, assert some frames have `(l - r).abs() > epsilon` plus finiteness/stability. Existing suites (`ffi_stereo`, `effect_order`, `ffi_gain_staging`, `bounce`, per-effect unit tests) must stay green.


## Validation and Acceptance

Run from the repo root:

    cargo build                                            # regenerates include/gooey.h
    cargo test --verbose                                   # all suites incl. tests/stereo_effects.rs
    cargo build --example kick --features native,crossterm # examples typecheck
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features              # pre-existing example failures unrelated

Acceptance (behavior):
- Foundation: with ping-pong off and any/all effects enabled, the FFI render keeps `frame[0] == frame[1]` for mono content; mono `tick()` and bounce outputs are unchanged.
- Showcase: with `DELAY_PARAM_PINGPONG = 1` and an impulse, left and right diverge on echo frames and stay finite/stable.
- `include/gooey.h` contains `DELAY_PARAM_PINGPONG`; `GOOEY_OUTPUT_CHANNELS` is still 2.


## Idempotence and Recovery

All steps are additive. The per-channel refactor preserves mono behavior (mono `process` uses `states[0]` only), so a regression localizes to either the seam move or a specific effect's `process_stereo`. `cargo build` regenerates the header deterministically.


## Outcomes & Retrospective

Pass 3 complete. The `Effect` trait now carries `process_stereo`, the stereo seam moved ahead of the effects chain in both engines, and all six stateful effects (lowpass, tilt, saturation, reverb, delay, compressor) hold genuine per-channel state. The foundation is behavior-preserving: with mono content and no stereo behavior engaged, left == right (verified per-effect in `tests/stereo_effects.rs`). The showcase — the delay's ping-pong mode — makes a centered impulse audibly bounce between channels (verified by the L≠R divergence test), exposed to iOS through `DELAY_PARAM_PINGPONG` with no new FFI function.

The mono `Engine::tick()` and the offline bounce stayed byte-identical via the `render_pre_effects` split, so nothing downstream of them changed. The main lesson mirrors the ping-pong discovery: a stereo *plumbing* foundation is cheap and symmetry-preserving, but producing *audible* stereo from mono content requires an intentional asymmetry (here, left-only dry injection in ping-pong) — symmetric processing of equal inputs can never diverge.

What remains for future passes: stereo decorrelation for the reverb (its two tanks currently share allpass lengths, so reverb stays dual-mono even when engaged), per-instrument pan (Pass 2 in `plans/stereo-support-plan.md`), and a stereo offline bounce (still mono by design).
