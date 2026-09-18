# Add opt-in piano dynamics and final output gain

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. Maintain this document in accordance with `.agent/PLANS.md`.

## Purpose / Big Picture

Nebula needs the Salamander piano to keep choosing its original eight velocity-layer recordings while reducing their perceived loudness spread to about 2 dB. After this work, a host can opt into an exact 0–24 dB designed span and a configurable compensation ceiling without changing the legacy normalized parameter or defaults. Calibration is computed while a sample map is built, independently for every playable key, and rendering only performs indexed lookups and scalar arithmetic. Release samples, mechanical/noise mappings, and unrelated key mappings do not influence calibration.

The host can also opt into a final, smoothed linear gain after tonal effects and the monitoring metronome and before the existing optional limiter. Default processing remains compatible: effects feed the optional limiter and the metronome is added afterward. The existing limiter transfer function stays intact, but bypass and threshold transitions become click-free. Atomic read-and-reset telemetry reports emitted peak and the count of pre-limiter stereo frames above full scale.

## Progress

- [x] (2026-09-18 18:22Z) Inspected v1.1.6 piano calibration, render topology, C ABI, tests, iOS release workflow, and prepared Salamander v2 pack.
- [x] (2026-09-18 18:22Z) Confirmed 240 tonal mappings covering 30 roots and eight layers, plus four unrelated key-zero attack mappings that must not affect C4 calibration.
- [x] (2026-09-18 19:02Z) Implemented and unit-tested per-key attack-only calibration, exact dB span, and compensation ceiling.
- [x] (2026-09-18 19:02Z) Added the backward-compatible piano C ABI with null, range, and legacy-override coverage.
- [x] (2026-09-18 19:02Z) Implemented opt-in final output, smoothing, retained limiter DSP, and atomic telemetry.
- [x] (2026-09-18 19:02Z) Added integration tests for legacy topology, ordering, smoothing, telemetry, and limiter transitions.
- [x] (2026-09-18 19:02Z) Ran Salamander v2 validation over all keys, layers and boundaries, noise gain, sample identity, and overlapping chords.
- [x] (2026-09-18 19:02Z) Generated and inspected the C header; formatting, 481 library tests, every integration target, and both iOS architecture builds pass. Repository-wide strict clippy and example compilation remain blocked by documented pre-existing failures.
- [x] (2026-09-18 18:41Z) Merged PR #246, tagged the merged-main commit `b71a08a` as v1.1.7, and verified the published iOS archive and generated header.

## Surprises & Discoveries

- Observation: Old calibration grouped zones by recorded root key and took a global median per velocity layer, which can flatten the intended bass-to-treble contour and does not model the layers selectable for each played key.
  Evidence: `src/instruments/multisample.rs` previously stored one `layer_boost_db: Vec<f32>` value per zone and grouped measurements by `root_key`.

- Observation: Salamander v2's largest raw key-local correction is about 38.53 dB on keys 104–106. A 40 dB ceiling reaches a 2 dB target everywhere, while the historical 20 dB ceiling does not.
  Evidence: Artifact inspection found 38 limited keys at 20 dB, six at 30 dB, and zero at 40 dB.

- Observation: Four auxiliary attack mappings use key zero but root key 60, so filtering only by trigger would contaminate C4 in root-grouped calibration.
  Evidence: The final four zones have `lokey=0`, `hikey=0`, `pitch_keycenter=60`, `hivel=127`, and trims near -20 dB.

- Observation: End-to-end rendering is substantially tighter than the requested approximate target: all 88 keys measured 1.88–2.14 dB, and the largest adjacent layer-boundary jump was 0.16 dB.
  Evidence: `GOOEY_MOBILE_PACK_SFZ=.context/salamander-v2/piano-mobile/instrument.sfz cargo test --release --lib --no-default-features --features bounce salamander_v2_two_db_validation -- --ignored --nocapture` passed in 3.78 seconds.

- Observation: The worst compensated final 350 ms of any selected v2 sample was -28.64 dBFS at MIDI key 44, velocity 26, after 17.93 dB gain; no key exceeded the 40 dB ceiling.
  Evidence: The same real-pack validation printed `compensation-limited keys at 40 dB: []` and the decay/noise diagnostic.

- Observation: `cargo clippy --all-targets --all-features -- -D warnings` is not a usable repository gate on the base revision. It reports dozens of existing issues in unrelated files before reaching test completion.
  Evidence: Failures include `src/gen/polyblep.rs` empty doc-comment lines, legacy `Default`-like methods, existing missing FFI safety docs, and unrelated range-loop warnings. The only warning in the touched piano file is on the pre-existing median implementation.

- Observation: `cargo test --all-features` attempts to compile stale examples and fails in `examples/hihat.rs` because that example references removed `HiHat2` fields and presets. The complete library and integration suite is green when examples are excluded.
  Evidence: `cargo test --all-features --lib --tests` passed 479 non-ignored library tests and every integration test; two private-artifact tests remained normally ignored, with the new one also run separately against the artifact.

## Decision Log

- Decision: Keep `PIANO_PARAM_VELOCITY_SPAN` as normalized 6–24 dB and add explicit dB-native C functions for 0–24 dB.
  Rationale: Existing hosts retain exact behavior; new hosts can request 2 dB without an incompatible mapping.
  Date/Author: 2026-09-18 / Codex

- Decision: Make the compensation ceiling configurable from 0–60 dB with the existing 20 dB default, and validate Salamander at 40 dB.
  Rationale: This preserves the noise-safety default while permitting the measured 38.53 dB worst case when explicitly requested.
  Date/Author: 2026-09-18 / Codex

- Decision: A dB-native span remains authoritative until a legacy span or preset is written; the ceiling persists across presets.
  Rationale: Mixed old/new host behavior is deterministic, and the ceiling is a host safety policy rather than a sound preset value.
  Date/Author: 2026-09-18 / Codex

- Decision: The final stage defaults off, supports linear gain 0–4, and when enabled orders effects, metronome sum, gain, then the existing optional limiter.
  Rationale: Nebula can trim or boost the complete audible output while old clients preserve the post-limiter monitoring click.
  Date/Author: 2026-09-18 / Codex

- Decision: Telemetry records peak after the emitted limiter result and counts one stereo frame when either pre-limiter channel has absolute magnitude greater than 1.0.
  Rationale: The values answer what was emitted and how often the limiter input exceeded full scale.
  Date/Author: 2026-09-18 / Codex

## Outcomes & Retrospective

The implementation and release are complete. Piano calibration is key-local and build-time, legacy defaults remain intact, the new C controls are present in the shipped `gooey.h`, final output and telemetry are opt-in, and limiter A/B DSP is unchanged at settled states. Salamander v2 met the 2 dB design on every playable key with no 40 dB ceiling failures. PR #246 passed Linux and macOS CI and merged as `b71a08a`. Tag v1.1.7 then produced a 12 MB public archive containing 19 MB device and simulator libraries plus the verified header. The release is available at `https://github.com/gooey-audio/libgooey/releases/tag/v1.1.7`.

## Context and Orientation

This Rust real-time audio engine exposes a C interface for iOS. `src/instruments/multisample.rs` owns sample maps, velocity-layer selection, one-time PCM loudness analysis, and piano rendering. A `SampleZone` is one SFZ mapping with a key range, velocity range, trigger, root pitch, trim, and decoded audio. A key-local ladder compares only attack zones the played MIDI key can select. A tonal attack is defined here as an attack mapping whose recorded root lies inside its own playable range; this excludes Salamander's auxiliary key-zero noises and release mappings.

`src/ffi.rs` owns `GooeyEngine`, the per-sample render loop, and exported `gooey_engine_*` symbols. Default flow applies master gain and effects, optionally runs `SoftLimiter`, then adds `Metronome`. `src/effects/limiter.rs` contains the tanh limiter, whose transfer function is not replaced. `src/utils/smoother.rs` provides allocation-free exponential ramps. `build.rs` and `cbindgen.toml` generate uncommitted `include/gooey.h`. `.github/workflows/ios-release.yml` packages device and simulator static libraries plus the header for `v*` tags, while `.github/workflows/salamander-mobile-pack.yml` builds and validates the prepared pack.

The real pack is private and supplied to validation as an extracted directory or workflow artifact. Normal tests use synthetic buffers; a deliberately ignored test exercises the private pack when its path is provided.

## Plan of Work

Change `SampleMap` from one correction per zone to a precomputed correction per zone and MIDI key. Measure each eligible buffer once during `SampleMap::build`, pool round-robin zones sharing a velocity ceiling, and compare layer powers for each key against that key's loudest attack. Store zero for release/auxiliary mappings and keys with fewer than two layers. Synthetic tests prove keys can receive different corrections, the loudest layer remains at zero, and unrelated mappings cannot perturb a tonal ladder.

Extend `MultiSampleInstrument` with an optional exact-span smoother and compensation ceiling. Span clamps to 0–24 dB; ceiling clamps to 0–60 dB and defaults to 20. New voices fetch correction by played key, combine it with the velocity curve, scale by the existing dynamic-range blend, and cap net positive gain. Add `gooey_engine_piano_set/get_velocity_span_db` and `gooey_engine_piano_set/get_compensation_ceiling_db` with false or NaN for invalid pianos and no change for non-finite values.

Replace immediate limiter boolean and threshold writes in `GooeyEngine` with 15 ms smoothed mix and threshold parameters. Settled states call the same `SoftLimiter::process_stereo`. Add an opt-in final topology mix and smoothed 0–4 gain. When off, compute the legacy effects-limiter-metronome path. When on, add metronome after effects, apply gain, then limit. Crossfade topologies only during enable transitions. Bounces keep suppressing metronome and snap smoothed output parameters.

Add atomic peak and overload fields. Rendering updates them without allocation or locks. Export final-stage enable/gain and read-and-reset telemetry functions. Null pointers return disabled, unity, or zero. Tests settle smoothers before asserting steady-state transfer and separately show abrupt bypass and threshold requests do not create one-sample jumps.

Add ignored Salamander v2 validation and call it in the pack workflow. It enumerates keys 21–108, sees eight layers per key, renders both sides of every layer boundary without replacing selected sources, checks an approximately 2 dB soft-to-hard span, reports keys exceeding 40 dB, assesses amplified quiet-tail noise, and renders overlapping chords without non-finite output. Generate the header, run tests/lints, and prepare v1.1.7 notes. A public tag must point to merged main.

## Concrete Steps

Run from `/Users/pretzel/conductor/workspaces/libgooey/abuja-v2`:

    cargo fmt --all -- --check
    cargo test --all-features --no-run
    cargo test --all-features instruments::multisample
    cargo test --all-features --test ffi_gain_staging
    cargo test --all-features --test metronome

Run real validation after extracting the pack:

    GOOEY_MOBILE_PACK_SFZ=/absolute/path/instrument.sfz cargo test --release --lib --no-default-features --features bounce salamander_v2_two_db_validation -- --ignored --nocapture

It must report eight layers for 88 keys, no compensation-limited keys at 40 dB, finite overlapping chords, and span diagnostics near 2 dB. Before handoff run:

    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-features
    cargo build --release --no-default-features --features ios

Inspect `include/gooey.h`. After landing on `main`, push annotated tag `v1.1.7`; the existing iOS workflow must produce `libgooey-ios-v1.1.7.tar.gz` with device and simulator libraries, header, and README.

## Validation and Acceptance

Legacy acceptance means a new engine keeps a 20 dB ceiling, old normalized 6–24 dB span, final output disabled at unity, limiter disabled, and the topology with metronome after limiter. Existing ABI and tests pass.

Piano acceptance means exact setters accept 0 and 24 dB endpoints, reject non-finite inputs, and 2 dB with a 40 dB ceiling retains original zone selection while meeting tolerance across eight Salamander layers. Calibration completes before rendering, with no PCM analysis or calibration allocation in rendering.

Output acceptance means final gain changes are smoothed, enabled ordering puts effects and metronome before final gain and limiter, telemetry safely reads/resets, and overload counts once per stereo frame. Settled limiter output stays the tanh result while bypass/threshold changes avoid a one-sample jump.

Release acceptance means the generated header exposes all APIs and the iOS release build succeeds. Publish only from a `v1.1.7` tag on merged `main`.

## Idempotence and Recovery

Build/test commands are repeatable. Extract the private archive into a temporary or `.context` path and never commit it. If storage fills, remove only this workspace's `target` via `cargo clean`; never delete shared caches or user files. Record failures in `Surprises & Discoveries` and resume from the first incomplete progress item.

Do not force-move or publish a release tag from the feature branch. If a tagged main build fails, fix main normally and use a new patch version rather than replace a public tag.

## Artifacts and Notes

Prepared pack contract:

    tonal zones: 240 (30 roots x 8 layers)
    playable keys: 21..108 (88 keys)
    auxiliary mappings: 4 key-zero attacks, excluded
    maximum raw key-local correction: about 38.53 dB
    keys limited at 40 dB: []

Stable historical behavior:

    legacy span: 6.0 + normalized * 18.0 dB
    default compensation ceiling: 20.0 dB
    limiter: existing SoftLimiter tanh
    final output: disabled, gain target 1.0

## Interfaces and Dependencies

`MultiSampleInstrument` provides `set_velocity_span_db(f32)`, `velocity_span_db() -> f32`, `clear_velocity_span_db_override()`, `set_compensation_ceiling_db(f32)`, and `compensation_ceiling_db() -> f32`. `SampleMap` provides `layer_boost_db_for_key(u8, usize) -> f32` and `layer_levels_for_key(u8) -> Vec<(u8, f32)>`; diagnostic allocation is control-time only.

The generated C interface contains:

    bool gooey_engine_piano_set_velocity_span_db(GooeyEngine *, uint32_t, float);
    float gooey_engine_piano_get_velocity_span_db(const GooeyEngine *, uint32_t);
    bool gooey_engine_piano_set_compensation_ceiling_db(GooeyEngine *, uint32_t, float);
    float gooey_engine_piano_get_compensation_ceiling_db(const GooeyEngine *, uint32_t);
    void gooey_engine_set_final_output_enabled(GooeyEngine *, bool);
    bool gooey_engine_get_final_output_enabled(const GooeyEngine *);
    void gooey_engine_set_final_output_gain(GooeyEngine *, float);
    float gooey_engine_get_final_output_gain(const GooeyEngine *);
    float gooey_engine_take_final_output_peak(const GooeyEngine *);
    uint64_t gooey_engine_take_final_output_overload_frames(const GooeyEngine *);

No new crate dependency is needed. Use `SmoothedParam` for gain, topology, bypass, and threshold transitions, and `AtomicU32`/`AtomicU64` with relaxed ordering for telemetry.

Plan revision note (2026-09-18): Created after source and artifact inspection so decisions and release boundaries survive context changes. Updated after implementation to record measured Salamander results, generated-header/iOS verification, and pre-existing repository-wide lint/example blockers.

Plan revision note (2026-09-18): Marked the plan complete after PR #246 merged and the v1.1.7 workflow published and artifact-level verification confirmed the new C symbols.
