# Reverb Algorithms: Plate, FDN, and Freeverb with User Selection

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `.agent/PLANS.md`, and this document must be maintained in accordance with that file. If this plan is revised, keep it self-contained: a future contributor should be able to read only this file and the current working tree, then continue safely.

## Purpose / Big Picture

Before this work, libgooey shipped exactly one reverb: `SpringReverbEffect` (`src/effects/reverb.rs`), a chain of six Schroeder allpass filters per channel feeding a single damped feedback loop. It is a "spring/dispersion" color — the sproingy sound of a physical spring tank — not a dense studio reverb, and it exposes only decay, mix, and damping. The user wants richer, better-sounding reverbs, particularly a **plate** (the smooth, dense, bright sound of a vibrating metal sheet, historically an EMT 140).

The goal is three reverb algorithms the user can pick between at runtime, delivered one at a time so each can be auditioned and tuned before the next starts:

1. **Plate** — Jon Dattorro's figure-eight tank plate reverb (SHIPPED, this session).
2. **FDN** — an 8×8 feedback-delay-network hall/room (future session).
3. **Freeverb** — the classic 8-comb + 4-allpass topology (future session, gated on a listening decision).

"User selection" reuses the engine's existing model of one effect per ID in the reorderable global chain: each algorithm is its own `EFFECT_*` id with its own enable flag. The user enables whichever they want via the existing `gooey_engine_set_global_effect_enabled` FFI; algorithms can also be combined or reordered for free. No new "algorithm selector" parameter machinery is needed.

**What you can do now that you could not before:** run `cargo run --example reverb_lab --features native,crossterm`, press SPACE to start a snare pattern, and press TAB to A/B the spring reverb against a true studio plate — with predelay, stereo width, and a size knob that sweeps the plate from a small bright chamber to a long modulated wash.

## Progress

- [x] (2026-07-02) Milestone 1: Dattorro plate reverb `EFFECT_PLATE_REVERB = 9`, six params incl. size, full FFI + loop-mixer wiring, unit + integration tests, `examples/reverb_lab.rs` listening lab. All of `cargo build`, `cargo test`, `cargo fmt --check`, `cargo clippy` green; `include/gooey.h` regenerated. (SHIPPED)
- [ ] Milestone 2: 8×8 Householder FDN reverb `EFFECT_FDN_REVERB = 10`.
- [ ] Milestone 3: Freeverb — decision checkpoint first (replace spring internals vs. add a fourth id), then implement.

## Milestone 1 — Dattorro Plate Reverb (SHIPPED)

### What exists now

`src/effects/plate_reverb.rs` implements `PlateReverbEffect` following Jon Dattorro, "Effect Design Part 1" (JAES 1997). Signal flow, in order:

1. **Predelay** — a delay line (0–200 ms, `PLATE_PARAM_PREDELAY`), fractional read so the smoothed knob moves clicklessly.
2. **Input bandwidth lowpass** — a one-pole lowpass (fixed coefficient 0.9995) that tames the very top before the tank.
3. **Input diffusion** — four series Schroeder allpasses (gains 0.750, 0.750, 0.625, 0.625) that smear the input into a diffuse wavefront. These are fixed-length (not size-scaled).
4. **Figure-eight tank** — two cross-coupled branches. Each branch is: a delay-modulated allpass (gain 0.70, delay swept by a per-branch LFO) → delay → one-pole damping lowpass (`PLATE_PARAM_DAMPING`) → multiply by decay → second allpass (gain follows Dattorro's decay-diffusion-2 rule, `clamp(decay·0.95 + 0.15, 0.25, 0.50)`) → delay → multiply by decay → **cross-feed into the OTHER branch**. The cross-feed uses the previous sample's value (captured into `fb_a`/`fb_b` before either branch updates), which is what makes the recirculating loop causal.
5. **Output taps** — Dattorro's seven taps per channel, spread across BOTH branches (the left output reads mostly from tank branch B and the right from branch A). This cross-branch tapping is what gives a plate its wide, coherent stereo image from a mono-summed input.
6. **Width** — a wet-only mid/side scale (`PLATE_PARAM_WIDTH`; 0 = mono wet, 1 = full Dattorro taps).

**Term definitions.** A *Schroeder allpass* passes all frequencies at equal gain but disperses them in time — `H(z) = (g + z^-N) / (1 + g·z^-N)`; used here as a diffuser. *Decay diffusion* is Dattorro's name for the allpasses inside the tank loop (as opposed to the input-diffusion allpasses in front of it). The *figure-eight* is the two-branch cross-coupled loop: signal circulates through branch A, crosses to B, crosses back to A, tracing a figure-8.

### Design decisions (see Decision Log for rationale)

- **Single shared tank, not dual-mono.** Unlike the spring's `UnsafeCell<[State; 2]>`, the plate is `UnsafeCell<PlateState>` — one tank. `process_stereo` mono-sums the input (`0.5·(l+r)`) into the tank once per frame; stereo comes from the cross-branch output taps. The dry signal stays stereo. The mono `process()` path returns `0.5·(wet_l + wet_r)` mixed with dry.
- **Size knob included.** `PLATE_PARAM_SIZE` (0–1) rescales every tank delay and output tap from 0.25× to 2.0× via fractional reads over max-size buffers (allocated at 2.0× + LFO excursion + margin). 0.5 = the published Dattorro plate. Sweeping it pitch-bends the tail (intended, tape-style). Input-diffusion allpasses stay fixed-length. `size_to_scale` maps the knob exponentially in each half so equal knob moves feel like equal size ratios.
- **Modulation fixed internally.** Two free-running sine LFOs at 0.50 Hz and 0.71 Hz (deliberately non-harmonic so they never phase-lock), excursion 16 samples @ 29761 Hz (rescaled). Not exposed as params.
- **Stability margin.** `MAX_DECAY = 0.95`. A time-varying allpass is not exactly unity-gain, so the modulated tank could otherwise creep above unity; the cap plus the final `is_finite` guard (falling back to dry) keeps it bounded. The `stable_at_max_decay` soak test is the gate.

### Parameters

All are 0–1 knobs pushed through `SmoothedParam` (15 ms) via lock-free atomics, exactly like the spring:

| FFI id | Name | Default (global / channel) | Mapping |
|---|---|---|---|
| `PLATE_PARAM_DECAY` = 0 | decay | 0.5 | linear → tank decay gain ×0.95; also sets decay-diffusion-2 |
| `PLATE_PARAM_MIX` = 1 | mix | 0.0 / 0.3 | linear dry/wet crossfade |
| `PLATE_PARAM_DAMPING` = 2 | damping | 0.5 | one-pole coeff ×0.95 (capped so it never latches DC) |
| `PLATE_PARAM_PREDELAY` = 3 | predelay | 0.0 | linear → 0–200 ms |
| `PLATE_PARAM_WIDTH` = 4 | width | 1.0 | wet mid/side side-scale |
| `PLATE_PARAM_SIZE` = 5 | size | 0.5 | 0.25×–2.0× tank scale, exponential each half |

### Files changed

- `src/effects/plate_reverb.rs` (new) — the effect + eight unit tests.
- `src/effects/mod.rs` — `pub mod plate_reverb;` + re-export.
- `src/ffi.rs` — `EFFECT_PLATE_REVERB = 9`, `EFFECT_COUNT` 9→10, `REORDERABLE_EFFECT_COUNT` 8→9, `DEFAULT_EFFECT_ORDER` grows, `PLATE_PARAM_*` constants, engine field `plate_reverb` + `plate_reverb_enabled`, construction, dispatch arm in the render loop, `reset_effect_states`, param set/get arms, enable set/get arms, `is_reorderable_effect`.
- `src/mixer/effect_chain.rs` — `ChannelEffect::PlateReverb` variant + `from_id`/`effect_type`/`process_stereo`/`set_param`/`reset` arms, one extra assertion in `add_reports_slot_and_type`.
- `tests/effect_order.rs` — every hardcoded 8-element `[u32; N]` array grew to 9 with `EFFECT_PLATE_REVERB`; `reorderable_count_is_eight` → `reorderable_count_is_nine`.
- `tests/stereo_effects.rs` — new `plate_reverb_decorrelates_left_and_right`, updated the exclusion note.
- `examples/reverb_lab.rs` (new) + `Cargo.toml` example entry.
- `include/gooey.h` — regenerated by `build.rs` (cbindgen) on build.

### Validation (Milestone 1)

    cargo build
    cargo build --example reverb_lab --features native,crossterm
    cargo test                       # 242 lib + all integration tests pass
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features   # no new warnings from the plate

Unit tests in `plate_reverb.rs`: `stable_at_max_decay` (5 s impulse soak, tail must not grow), `decay_time_is_sane` (windowed-RMS −60 dB point lands 0.3–4 s), `decorrelates_left_and_right`, `zero_width_collapses_to_mono`, `nan_input_produces_finite_output`, `reset_clears_state`, `constructor_and_setters_clamp`, `constructs_and_runs_at_common_sample_rates` (22.05k–96k), `size_changes_the_tail`. Integration: `tests/stereo_effects.rs::plate_reverb_decorrelates_left_and_right` drives it through the real FFI render path.

Manual acceptance: run the lab, press SPACE, then TAB between SPRING and PLATE. The plate should be audibly denser and wider than the spring on the same snare hits. On PLATE, drop `size` toward 0 for a small bright flutter and toward 1 for a long modulated wash; raise `predelay` to hear the dry transient separate from the tail.

### Downstream (iOS) follow-up

`include/gooey.h` regenerates automatically, but the Swift consumer app (built via `scripts/build-ios.sh`) owns the `GlobalEffect` enum and per-effect param enums. It must add `plateReverb = 9` and a `PlateParam` enum (decay, mix, damping, predelay, width, size) to surface the plate in the iOS UI, and account for `EFFECT_COUNT` moving 9→10. Flag this in the PR.

## Milestone 2 — 8×8 Householder FDN Reverb (future session)

Add `EFFECT_FDN_REVERB = 10` as a new effect, mirroring the plate's integration steps exactly (constants, engine field + flag, dispatch arm, `reset_effect_states`, param set/get, enable set/get, `is_reorderable_effect`, loop-mixer variant, `REORDERABLE_EFFECT_COUNT` 9→10, `DEFAULT_EFFECT_ORDER` grows, and every `[u32; N]` array in `tests/effect_order.rs` grows again).

**Topology.** Eight parallel delay lines with mutually-prime lengths ≈ 30–80 ms, recirculated through a Householder feedback matrix `H = I − (2/N)·J` where J is the all-ones matrix (an O(N) operation: compute the mean of the eight taps, subtract twice the mean from each — no full matrix multiply). Each delay line has a one-pole damping lowpass in its feedback path. Put two input-diffusion allpasses in front (reuse the plate's `DelayLine::allpass`). Derive L/R from alternating ± sums of the eight taps. The Householder matrix is lossless at unity gain, so stability analysis is clean and `size` is natural (fractional reads on all eight lines). Params mirror the plate: decay, mix, damping, size, predelay, width. Estimated cost ≈ 1.5–2× the spring per stereo frame. This is the pick when the user wants a general-purpose *hall/room* with an independent size knob rather than a plate color.

The plate's building blocks are directly reusable: `DelayLine` (fractional `read_frac`/`tap_frac` + `allpass`), the `flush_denormal` helper, the `size_to_scale` curve, and the atomic-target + `SmoothedParam` parameter pattern.

## Milestone 3 — Freeverb (future session, decision checkpoint)

**Checkpoint first.** Before writing code, A/B the spring reverb in `reverb_lab` and decide: is the spring's dispersion color worth keeping? The user's stated preference is to rework `SpringReverbEffect`'s internals into Freeverb under the existing `EFFECT_REVERB = 6` (zero FFI churn — decay/mix/damping map 1:1, the `[State; 2]` dual-mono layout is unchanged). But if the spring color earns its keep, land Freeverb as a fourth id (`EFFECT_FREEVERB`) instead. **Record the decision in the Decision Log below before implementing.**

**Topology (Freeverb).** Per channel: eight parallel lowpass-feedback comb filters (lengths 1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617 @ 44.1k, right channel offset by +23 samples for stereo spread) summed into four series allpasses (556, 441, 341, 225; gain 0.5). Decay → comb feedback (`roomsize`), damping → comb lowpass, mix → dry/wet. Estimated cost ≈ 2× the current spring. Known weakness: a metallic comb signature at short decays.

## Surprises & Discoveries

- Observation: with `width = 0` the wet output does not collapse to exactly mono until the width `SmoothedParam` fully settles.
  Evidence: `zero_width_collapses_to_mono` failed with `|L-R| = 4.2e-5` after only 100 ms of settling; the smoother's snap-to-target needed ~300 ms. Fixed by settling ~13k samples before asserting. This is inherent to the 15 ms one-pole smoother, not a topology bug.
- Observation: the `tests/effect_order.rs` arrays are `[u32; N]` sized by `REORDERABLE_EFFECT_COUNT`, so bumping the count to 9 turned every stale 8-element array into a **compile error**, not a silent wrong-length test. This is the intended loud-failure behavior — each array had to be hand-extended with `EFFECT_PLATE_REVERB`.

## Decision Log

- Decision: each reverb is its own `EFFECT_*` id rather than a mode of one reverb.
  Rationale: matches the codebase's one-effect-per-id model (own enable flag, own loop-mixer variant, reorderable independently), needs no new selector machinery, and lets the user stack/compare algorithms freely.
  Date/Author: 2026-07-02, Brian Hurlow (via planning)

- Decision: the plate uses a single shared tank (`UnsafeCell<PlateState>`), not dual-mono `[State; 2]`.
  Rationale: Dattorro's figure-eight derives its wide stereo image from cross-branch output taps on ONE tank fed a mono sum. Two independent tanks would break the tap symmetry and roughly double CPU/memory for a worse image.
  Date/Author: 2026-07-02

- Decision: include the size knob in v1 (deferred nothing).
  Rationale: user explicitly requested it. Implemented via fractional reads over max-size buffers so it is audio-thread-safe and runtime-sweepable; the resulting pitch-bend on sweep is a musical feature.
  Date/Author: 2026-07-02

- Decision: modulation rate/depth and input bandwidth are fixed internally.
  Rationale: they add parameter surface and a stability surface for marginal musical value; damping already provides tone control. Revisit only if users ask.
  Date/Author: 2026-07-02

- Decision (PENDING, Milestone 3): whether Freeverb replaces the spring internals or lands as a new id.
  Rationale: to be decided after A/B-ing the spring in the lab. Record the outcome here before implementing.

## Outcomes & Retrospective

- Milestone 1 (2026-07-02): Dattorro plate shipped end-to-end — new effect, full FFI + loop-mixer wiring, six params including a runtime size knob, eight unit tests + one integration test, and a TAB-to-compare listening lab. All validation gates green with no new clippy warnings. The plate slots into the reorderable chain exactly like the spring, so the "user selection = enable the id you want" model is proven and Milestones 2–3 can follow the same integration recipe. Remaining: FDN (M2), Freeverb decision + impl (M3), and the downstream Swift enum additions.
