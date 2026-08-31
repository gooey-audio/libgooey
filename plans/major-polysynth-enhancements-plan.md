# Rebuild the poly synth as an expressive stereo instrument

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. This document must be maintained in accordance with `.agent/PLANS.md` from the repository root.

## Purpose / Big Picture

After this work, libgooey's six-voice poly synth will be a configurable stereo instrument instead of a minimally editable chord sound. A host can edit two oscillator waveforms and levels, amp/pitch/filter envelopes, width, detune, saturation, and eight velocity/key modulation routes. Those edits persist in per-engine copies of the five factory presets and survive preset switching, chord retriggering, and recorded performance playback. The `chords` example will provide an audible, paged editor for verifying the complete behavior.

## Progress

- [x] (2026-08-31 15:38Z) Read the repository architecture, existing poly synth, stereo seam, FFI graph, envelope implementation, build instructions, and Nexus integration guide; confirmed the branch starts clean at `67a7292` and no matching Nexus task exists.
- [x] (2026-08-31 15:38Z) Record the approved design in this living ExecPlan.
- [x] (2026-08-31 15:52Z) Implement release curves and release-level capture in the shared envelope; focused envelope tests pass.
- [x] (2026-08-31 15:52Z) Replace the poly synth DSP/configuration with the approved dual-oscillator stereo architecture and modulation matrix; eight focused poly tests pass.
- [x] (2026-08-31 15:52Z) Replace the poly parameter/preset FFI and route the synth's native stereo frame into the mixer graph; `cargo check --lib` passes.
- [x] (2026-08-31 16:08Z) Expand the `chords` example into a six-page oscillator/envelope/filter/expression/matrix editor with velocity, octave, and audible note-gate controls; the native+crossterm example compiles.
- [x] (2026-08-31 16:17Z) Add focused DSP and FFI regression coverage, including all 30 parameters, all eight routes, waveform continuity, exact pitch/detune ratios, clamp-once route summation, stereo graph output, rendered velocity brightness and saturation, register-scaled velocity response, preset reset/isolation, chord retriggering, and recorded performance replay; all focused tests pass.
- [x] (2026-08-31 16:17Z) Run the full validation matrix and record the outcome. Builds, formatting, default/native tests, and the requested example pass. All-feature clippy completes the library with warnings but the all-target command is blocked by the unchanged legacy `examples/hihat.rs`, whose API no longer matches the existing `HiHat2` alias.

## Surprises & Discoveries

- Observation: the current FFI setter exposes 14 raw indices, but every chord trigger reloads a factory `PolySynthConfig`, so live edits can be erased immediately.
  Evidence: `gooey_engine_poly_trigger_chord` calls `set_config(preset_config(preset))` before every trigger.
- Observation: the engine and FFI already have a native-stereo instrument seam, but the poly synth is explicitly converted from mono at center pan.
  Evidence: `Instrument::tick_stereo` supports native frames, while `GooeyEngine::render` currently calls `StereoFrame::panned(self.poly_synth.tick(time), 0.5)`.
- Observation: the host filesystem reached its storage limit during validation, and a normal debug link failed with `errno=28` even after the workspace began clean.
  Evidence: `df -h .` reported only 116 MiB free and `cargo test` failed while linking. Removing this workspace's generated `target/` artifacts and using non-incremental, debug-info-free validation profiles reduced `target/` to about 262 MiB without changing source, features, or test behavior.
- Observation: `cargo clippy --all-targets --all-features` exposed two unrelated stale examples. Correcting the two `libgooey` imports in `examples/lfo_test.rs` let validation continue, where it stopped on 14 missing legacy `HiHat` fields/methods in the unchanged `examples/hihat.rs`.
  Evidence: the clippy library/all-feature pass completes with advisory warnings; `git diff origin/main -- examples/hihat.rs` is empty, and the compiler errors reference obsolete names such as `amp_decay`, `set_open`, and `closed_default` that are absent from the existing `HiHat2` API.

## Decision Log

- Decision: expose independent A/B waveform and level controls, shared symmetric detune, and shared width.
  Rationale: this provides a useful stereo oscillator architecture without expanding into independent coarse tuning, phase, noise, or arbitrary unison voices.
  Date/Author: 2026-08-31 / user and Codex.
- Decision: use ADSR envelopes with separate attack and shared decay/release (`fall`) curves for amp, pitch, and filter.
  Rationale: this is expressive enough for graphical editors while keeping the parameter count bounded.
  Date/Author: 2026-08-31 / user and Codex.
- Decision: provide eight note-on modulation routes with velocity/key sources, arbitrary continuous poly destinations, bipolar depth, nonlinear curve, and key-scaled depth.
  Rationale: this directly models velocity behavior that changes across keyboard registers and leaves five routes free after factory expression defaults.
  Date/Author: 2026-08-31 / user and Codex.
- Decision: replace the numeric poly parameter ABI, but retain preset IDs 0 through 4.
  Rationale: the user chose a clean parameter surface; preset IDs remain stable because performance clips persist those values.
  Date/Author: 2026-08-31 / user and Codex.
- Decision: saturation is per voice and pre-filter, with a restrained maximum of 2.5x drive and 20% wet mix.
  Rationale: harder notes create modest extra harmonics which are then shaped by that note's velocity-responsive filter, without chord-bus intermodulation.
  Date/Author: 2026-08-31 / user and Codex.
- Decision: fix the obsolete crate name in `examples/lfo_test.rs`, but do not fold the larger legacy `examples/hihat.rs` migration into this poly-synth change.
  Rationale: the crate-name correction is a safe two-line build repair; redesigning a separate drum editor around `HiHat2` is unrelated feature work and would materially broaden this change.
  Date/Author: 2026-08-31 / Codex.

## Outcomes & Retrospective

The expressive stereo poly synth is implemented. It retains six voices and stable preset IDs while replacing the parameter ABI with the 30 normalized controls and eight-route expression matrix. Each FFI engine owns five editable presets; tests prove edits survive selection, retriggering, and performance replay. Native stereo rendering, the mono path, per-voice saturation, curved releases, and the six-page editor are operational.

The final default/native library suite reports 382 passing tests, with every integration suite and doc-test passing. The focused expressive-poly module has 12 tests, the shared envelope has 3 focused tests, and the new FFI suite has 6 tests. Desktop, iOS, and `chords` example builds pass; formatting and diff checks pass. The sole validation exception is the pre-existing `examples/hihat.rs` all-target clippy compile failure described above.

## Context and Orientation

`src/instruments/poly_synth.rs` owns the six voices, factory presets, oscillator rendering, per-voice envelopes, filters, and normalized runtime parameters. `src/envelope.rs` implements the shared ADSR envelope. `src/ffi.rs` owns a separate `PolySynth` used by chord pads and performance playback, exposes the C API, and scatters the poly source into `MixerGraph`. `examples/chords.rs` wraps a `PolySynth` in `Arc<Mutex<_>>` and auditions theory-generated chords through the native `Engine`.

The new parameter IDs are, in order: oscillator A waveform/level, oscillator B waveform/level, detune, width; amp ADSR/attack curve/fall curve; pitch amount/ADSR/attack curve/fall curve; filter cutoff/resonance/envelope amount/ADSR/attack curve/fall curve; saturation and volume. There are 30 parameters. Every ordinary parameter is normalized 0 to 1. Pitch and filter envelope amounts use 0.5 as neutral. Modulation depth and key scale are the documented bipolar exceptions.

## Plan of Work

First extend `Envelope` with a configurable release curve and a level captured at release time. Preserve the existing constructors by defaulting this curve to linear, add a direct setter, and calculate release from the captured level so the transition remains continuous.

Then replace `PolySynthConfig` and `PolySynthParams` with grouped oscillator, envelope, filter, and expression fields plus eight `PolyModRoute` values. Add the 30 public parameter constants and a single validated set/get dispatcher used by both Rust and FFI. Render sine, analytic triangle, PolyBLEP saw, and PolyBLEP square at exact morph anchors. At note-on, clamp velocity, configure the three curved envelopes from parameter targets, and resolve each route into a per-voice normalized offset. During rendering, combine those offsets with smoothed base controls, apply the pitch envelope in semitones, saturate each oscillator, pan it according to width, process separate left/right filters, and apply the amp envelope, square-root velocity gain, and fixed polyphonic headroom. The mono path will ignore width while sharing the same voice state; the stereo path will return the instrument's native image.

In `src/ffi.rs`, store editable copies of all five presets and the active preset ID. Chord triggers and performance playback load those copies rather than reconstructing factories. Replace the old raw parameter match with public constants and validated active/preset set/get/reset functions. Export a C-compatible modulation-route value and functions to set, get, and clear any route on any preset. Change the FFI render graph to consume the native poly frame.

Finally, update `examples/chords.rs` with paged controls for oscillators, envelopes, filter/expression, and matrix routes. Forward `tick_stereo` through its shared wrapper, expose velocity and octave changes, and ensure note-off occurs after a sounding gate rather than at the exact trigger timestamp.

## Concrete Steps

Work from `/Users/pretzel/conductor/workspaces/libgooey/bozeman-v1`. Edit with small compiling milestones, update this plan after each milestone, and validate with:

    cargo fmt --all
    cargo build
    cargo build --features ios
    cargo build --example chords --features native,crossterm
    cargo test --verbose
    cargo test --features native --verbose
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features

## Validation and Acceptance

Unit tests must pin exact waveform anchors, symmetric detune, pitch depth, envelope curves/release continuity, modulation math, note-register scaling, invalid inputs, voice allocation, and finite output. End-to-end FFI tests must round-trip all 30 parameters and eight routes, prove preset edits survive switching and chord retriggering, restore factory values, and prove the mixer receives a genuinely stereo poly frame. Deterministic render tests must show centered output at width zero, side energy at full width, brighter and more saturated hard notes, and different velocity depth in low and high registers. All repository validation commands above must succeed, with any pre-existing clippy warnings recorded separately from new warnings.

## Idempotence and Recovery

All source edits and tests are repeatable. Preset reset restores only the selected in-memory factory copy and never touches files. Invalid C inputs leave state unchanged. If a DSP milestone regresses audio or tests, revert only that local milestone rather than generated files; `include/gooey.h` is generated and gitignored.

## Artifacts and Notes

Baseline evidence:

    cargo test poly_synth --lib
    running 7 tests
    test result: ok. 7 passed; 0 failed

Focused implementation evidence:

    CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test poly_synth --lib
    running 12 tests
    test result: ok. 12 passed; 0 failed

    CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test --test poly_synth_params
    running 6 tests
    test result: ok. 6 passed; 0 failed

Final validation evidence:

    CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 cargo build
    CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 cargo build --features ios
    CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 cargo build --example chords --features native,crossterm
    CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test --verbose
    CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test --features native --verbose
    cargo fmt --all -- --check
    git diff --check

All commands above pass. `cargo clippy --all-targets --all-features` was also run with the lean validation profile; its library pass completes, then the unchanged legacy `examples/hihat.rs` fails to compile against `HiHat2`.

## Interfaces and Dependencies

No dependency is added. The central Rust interfaces are `PolySynthConfig`, `PolySynthParams`, `PolyModSource`, `PolyModRoute`, `PolySynth::set_param`, `PolySynth::param`, `PolySynth::set_mod_route`, and `PolySynth::tick_frame`. The C interface exports `POLY_PARAM_*`, `POLY_PARAM_COUNT`, `POLY_MOD_SOURCE_*`, `POLY_MOD_ROUTE_COUNT`, `GooeyPolyModRoute`, active/preset parameter accessors, current-preset/reset accessors, and route set/get/clear functions. Existing chord trigger/release and preset IDs remain available.
