# Stereo Future Work: Bounce, Spread, and Modulation

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `.agent/PLANS.md`, and this document must be maintained in accordance with that file. If this plan is revised, keep it self-contained: a future contributor should be able to read only this file and the current working tree, then continue safely.

## Purpose / Big Picture

libgooey's realtime path is now fully stereo: `StereoFrame` is the output currency, every `Effect` is stereo-aware via `process_stereo`, the delay has a ping-pong mode, the reverb is decorrelated, and **per-instrument panning** is live in both engines (native `Engine` and the iOS-facing `GooeyEngine`). Pan is applied at the single "stereo seam" using an equal-power law (`StereoFrame::panned`, `src/frame.rs`): 0.0 = hard left, 0.5 = center, 1.0 = hard right.

What is still mono or fixed-center, and would benefit from stereo, is captured below. None of these is started; each is an independent milestone a future session can pick up on its own.

## Progress

- [x] Pass 1: `StereoFrame` + stereo seam, native CPAL + FFI interleaved output. (shipped, PR #201)
- [x] Pass 3: stereo-aware `Effect::process_stereo`, ping-pong delay, decorrelated reverb. (shipped, PRs #202–#203)
- [x] Per-instrument panning: `StereoFrame::panned`, `Engine::set_instrument_pan` / `instrument_pan`, FFI `gooey_engine_set_instrument_pan` / `gooey_engine_get_instrument_pan`, tests in `src/frame.rs`, `tests/panning.rs`, `tests/ffi_stereo.rs`.
- [ ] Milestone A: stereo offline bounce + 2-channel WAV export.
- [ ] Milestone B: poly-synth stereo spread (per-voice pan / unison spread).
- [ ] Milestone C: granulator per-grain panning.
- [ ] Milestone D: per-instrument pan for poly synth and granulator in the FFI engine (currently fixed-center).
- [ ] Milestone E: pan as an LFO modulation target (auto-pan).

## Deferred Opportunities (detail)

### Milestone A — Stereo offline bounce / WAV export (biggest payoff)
- **Where:** `src/bounce.rs`. `bounce_to_buffer` calls `Engine::tick()` (mono); WAV export writes `channels: 1`.
- **What:** Add a `tick_stereo`-based bounce that returns interleaved (or paired) stereo, and a 2-channel WAV writer. The mono `tick()` path deliberately ignores pan and stereo effects, so today's exports miss panning, ping-pong delay, and the decorrelated reverb that realtime already produces. This closes that gap.
- **Notes:** `prepare_for_bounce` already snaps smoothed params; mirror that for any new stereo state. Keep a mono bounce entry point for backward compatibility or downmix via `StereoFrame::downmix`.

### Milestone B — Poly-synth stereo spread
- **Where:** `src/instruments/poly_synth.rs` (`PolySynth`), summed to mono in the engine render loops.
- **What:** Spread held voices across the stereo field (per-voice pan, or unison detune + spread) so chords sound genuinely wide instead of collapsing to one point. Because instruments return mono from `Instrument::tick`, this needs either a stereo-capable instrument path or an internal per-voice pan that the synth resolves before returning — design decision to make at start.

### Milestone C — Granulator grain panning
- **Where:** `src/instruments/granulator.rs`.
- **What:** Randomize / spread per-grain pan for a classic wide granular cloud. Same mono-`tick` constraint as Milestone B applies.

### Milestone D — FFI pan for poly synth and granulator
- **Where:** `src/ffi.rs` `render_frames`. Poly synth and granulator are currently summed at fixed center (`StereoFrame::panned(poly + gran, 0.5)`).
- **What:** Once B/C land, give them the same smoothed `instrument_pans`-style control and C API as the drum channels.

### Milestone E — Pan as a modulation target (auto-pan)
- **Where:** the existing `Modulatable` trait + LFO routing in `src/engine/mod.rs` and the FFI LFO routes.
- **What:** Allow an LFO to drive an instrument's pan for auto-pan effects. Requires exposing pan as a routable parameter.

## Decision Log

- Pan law is **equal-power** (center = -3 dB per channel), chosen over a center-unity law: constant power across the sweep, at the cost of being quieter at center than the old dual-mono seam. Established when per-instrument panning shipped.
- Per-instrument panning is **stereo-path only**; the mono `Engine::tick` / offline bounce ignores it. Milestone A is what makes pan audible in exports.

## Surprises & Discoveries

- (none yet for this plan)

## Outcomes & Retrospective

- (pending — fill in as milestones complete)
