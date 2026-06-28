# Bring The Granulator Closer To The Original Arbhär Design

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `.agent/PLANS.md`, and this document must be maintained in accordance with that file. If this plan is revised, keep it self-contained: a future contributor should be able to read only this file and the current working tree, then continue safely.

## Purpose / Big Picture

The current granulator works and sounds good as a first mono frozen-scan instrument. The purpose of this plan is to extend it toward the original Arbhär-style granular design in small, demonstrable stages while preserving the working command-line listening experience.

After the next milestone, a user should be able to run `cargo run --example granulator --features native,crossterm,bounce`, hear a richer cloud with less mechanical repetition, and adjust new random timing, random amplitude, grain stealing, and saturation controls from the terminal. Later milestones add stereo-aware buffers, multiple layers, follow mode, quantized pitch behavior, live capture, and iOS FFI exposure.

## Progress

- [x] (2026-05-23 16:16 EDT) Created an initial gap plan describing what the V1 granulator does and what remains from the original Arbhär design.
- [x] (2026-05-23 16:37 EDT) Read `.agent/PLANS.md` and converted the gap plan into a self-contained ExecPlan format.
- [x] (2026-05-23 16:37 EDT) Recorded current V1 state, deferred original-design features, recommended milestones, validation commands, and implementation interfaces in this plan.
- [ ] Implement Milestone 1: add `random_timing`, `random_amp`, soft grain stealing, and optional per-instrument saturation to the current mono granulator.
- [ ] Implement Milestone 2: add stereo-aware buffer internals while still returning mono from `Instrument::tick`.
- [ ] Implement Milestone 3: add static multi-layer playback and layer interpolation.
- [ ] Implement Milestone 4: add follow mode, where the read position moves through the buffer automatically.
- [ ] Implement Milestone 5: add pitch deviation, quantized pitch tables, and optional chord distribution.
- [ ] Implement Milestone 6: add live recording/capture into an internal buffer.
- [ ] Implement Milestone 7: expose the stable granulator through the iOS-facing FFI engine.

## Surprises & Discoveries

- Observation: The first V1 granulator produced non-intentional popping even though its user parameters used `SmoothedParam`.
  Evidence: Inspection showed two audio-rate discontinuities: grains were killed immediately when playback crossed buffer edges, and active-grain gain compensation changed instantly when the number of active grains changed. The fix was to constrain grain start positions so grains finish under their envelope and smooth gain compensation with another `SmoothedParam`.

- Observation: The generated demo buffer itself contained hard pulse edges that could be mistaken for granulator artifacts.
  Evidence: The fallback sample generator in `examples/granulator.rs` originally used an on/off pulse. It was changed to a sine-shaped pulse.

- Observation: The current public `Engine` trait returns one `f32` sample per tick, so true stereo output is not available through the main Rust `Engine` without a broader API change.
  Evidence: `src/engine/mod.rs` defines `Instrument::tick(&mut self, current_time: f64) -> f32` and `Engine::tick(&mut self, current_time: f64) -> f32`.

## Decision Log

- Decision: Keep the first completed granulator implementation mono and core-Rust-only.
  Rationale: The existing engine, bounce path, and examples are mono at the `Instrument` trait boundary. A mono implementation allowed a working listening tool before committing to stereo engine API changes or iOS FFI buffer ownership.
  Date/Author: 2026-05-23 / Codex

- Decision: Store this plan in `.context/granulator_original_design_gap_plan.md`.
  Rationale: `.context` is gitignored workspace-local context for future agents in Conductor. The plan can be referenced later without changing the library source.
  Date/Author: 2026-05-23 / Codex

- Decision: The next implementation milestone should be random timing, random amplitude, soft grain stealing, and optional saturation.
  Rationale: These are audible improvements that fit the current mono/static-buffer architecture and avoid large unresolved decisions around stereo, layers, recording, and FFI.
  Date/Author: 2026-05-23 / Codex

- Decision: Stereo, layers, follow mode, pitch tables, recording, and FFI are separate milestones.
  Rationale: Each changes a different boundary: audio channel shape, buffer ownership, playhead state, pitch scheduling, realtime input capture, or external API. Keeping them separate makes each milestone independently testable.
  Date/Author: 2026-05-23 / Codex

## Outcomes & Retrospective

V1 achieved a useful listening result: a working granular instrument, WAV loading helper, deterministic grain generation, and an interactive terminal example. A follow-up pass reduced processing artifacts by smoothing gain compensation and preventing abrupt edge kills. What remains is not a bug fix but feature growth toward the original Arbhär design.

The most important lesson is that granular synthesis clicks can come from sources other than parameter smoothing. Any future milestone must preserve zero-start/zero-end grain envelopes, avoid abrupt voice termination, and smooth aggregate gain changes when voices are added or removed.

## Context and Orientation

This repository is a Rust audio engine named `gooey`. The primary public instrument abstraction lives in `src/engine/mod.rs`. The trait is called `Instrument`; an instrument can be triggered and then asked for one mono audio sample at a time through `tick`. A second trait, `Modulatable`, lets LFOs change named instrument parameters.

The current granulator implementation lives in `src/instruments/granulator.rs`. It defines `SampleBuffer`, `GranulatorConfig`, and `Granulator`. `SampleBuffer` owns mono sample data. `GranulatorConfig` stores normalized `0.0..1.0` parameter values. `Granulator` owns the sample buffer, a fixed array of active grains, smoothed parameters, and a deterministic pseudo-random generator. A grain is a short playback voice: it reads a small region from the sample buffer, applies a window envelope that starts and ends at zero, and contributes to the output cloud.

The terminal listening example lives in `examples/granulator.rs`. It follows the pattern of other examples such as `examples/tom2.rs` and `examples/bass.rs`: it creates a shared instrument behind `Arc<Mutex<_>>`, adds it to `Engine`, starts `EngineOutput`, then uses `crossterm` to read keyboard input and redraw parameter bars.

The original Arbhär source material is under `/Users/pretzel/Downloads/arbhar_updater_2-13`. The most relevant files are `arbhar_play_with_buffers`, `arbharGrainCore.pd`, `configurationDataInitFile.txt`, and `arbhar_structure.md`. The C/PD implementation includes stereo playback, multiple layers, live recording into buffers, follow mode, random timing and amplitude, quantized pitch deviation, MIDI/chord behavior, per-grain panning, soft grain killing, and hardware/shared-memory integration.

Important plain-language terms used in this plan:

A grain is one short playback voice reading a small slice of audio. A cloud is many grains overlapping. A window is the envelope shape that fades each grain in and out. A playhead is the current read position in a buffer. Scan mode means the user chooses a mostly fixed read position. Follow mode means the playhead moves automatically through the buffer. FFI means foreign function interface: C-callable functions in `src/ffi.rs` used by iOS/Swift code.

## Plan of Work

The plan proceeds through milestones. Each milestone leaves the system working and audible through `examples/granulator.rs`.

Milestone 1 improves the existing mono static-buffer granulator. In `src/instruments/granulator.rs`, extend `GranulatorConfig` and `GranulatorParams` with normalized `random_timing`, `random_amp`, and `drive` parameters. Random timing should jitter each scheduled grain by a bounded fraction of the grain interval while keeping average density stable. Random amplitude should scale each grain volume by a deterministic random factor between `1.0 - amount` and `1.0`. Drive should apply a soft saturation after grain summing and before final volume; use a local `tanh`-style curve so this milestone does not require changing the global effects chain. Implement soft grain stealing by choosing an active grain to fade out when all slots are occupied instead of dropping the new grain. Add a small per-grain kill/fade state so stolen grains leave under a short fade rather than stopping immediately.

Milestone 1 must also update `examples/granulator.rs`. Add visible controls for `random_timing`, `random_amp`, and `drive`. Keep the current keyboard model: arrow keys select and coarse-adjust, brackets fine-adjust, `SPACE` triggers a cloud, `A` toggles auto-trigger, `R` reseeds, and `Q` quits. The CLI should remain useful with no WAV path by using the generated demo buffer.

Milestone 2 adds stereo-aware internals without changing the `Instrument` trait. In `src/instruments/granulator.rs`, introduce a stereo buffer representation that can store left and right samples. Add WAV loading that preserves stereo when the file has two or more channels and duplicates mono files into both channels. Add per-grain panning with an equal-power pan law, which means left gain is `cos(pan * PI / 2)` and right gain is `sin(pan * PI / 2)` for `pan` in `0.0..1.0`. Because `Instrument::tick` still returns mono, fold the stereo grain result back to mono at the final step by averaging left and right. This prepares the DSP for a future stereo engine while preserving compatibility.

Milestone 3 adds static multi-layer playback. Replace the single `SampleBuffer` owned by `Granulator` with a small layer collection. A layer is one loaded buffer. Add normalized `layer` and `layer_interpolate` controls. When interpolation is zero, grains read one selected layer. When interpolation is nonzero, grains crossfade between the selected layer and the next layer. Update the CLI to accept multiple WAV paths and show the selected layer. If no path is passed, generate at least two distinct demo layers.

Milestone 4 adds follow mode. Add an enum-like state for `FrozenScan` and `Follow`. In follow mode, `scan_position` becomes a starting offset and a new `follow_speed` parameter moves the internal playhead each sample or each grain spawn. The pitch parameter must still control grain playback speed, not playhead movement. Add a CLI toggle for follow mode and a `follow_speed` control. Validate that the playhead wraps cleanly and does not create clicks at buffer boundaries.

Milestone 5 adds musical pitch behavior. Add a pitch deviation table based on semitone offsets, quantized random pitch selection per grain, and optional MIDI-note/chord distribution. Keep the existing continuous `pitch` control as the base multiplier. This milestone should not require the iOS FFI path. It should be demonstrable through tests and, if useful, a small CLI control for enabling quantized pitch.

Milestone 6 adds live recording/capture. This is the largest DSP milestone. Design a realtime-safe recording buffer that is preallocated and never grows in the audio callback. Add capture start/stop and clear APIs. Add record-head protection fades so grains do not click when reading near the point currently being written. Decide and document whether recording replaces or overdubs existing samples. Add tests that simulate writing samples while grains play.

Milestone 7 exposes the stable granulator through `src/ffi.rs`. Add an instrument type constant, extend `ChannelInstrument`, add parameter constants, and add safe buffer ownership APIs for iOS callers. This milestone must decide how Swift passes sample data into Rust and how Rust releases it.

## Concrete Steps

All commands in this plan assume the working directory is:

    /Users/pretzel/conductor/workspaces/libgooey/kathmandu-v2

Before starting any milestone, inspect the current worktree:

    git status --short

For Milestone 1, edit `src/instruments/granulator.rs`. Add `random_timing`, `random_amp`, and `drive` fields to `GranulatorConfig`, `GranulatorParams`, getters, setters, and `Modulatable::modulatable_parameters`. Update `apply_modulation` and `parameter_range` so LFOs can target these new controls. Add per-grain fields for amplitude and soft-kill state. Change the full-pool behavior in `spawn_grain` so the new grain still starts by fading out an existing grain and reusing it once safe. Add tests in the same file or in `tests/granulator.rs` that prove dense clouds remain finite and deterministic with a fixed seed.

Then edit `examples/granulator.rs`. Add the new parameters to `PARAM_INFO`, `get_param_value`, `set_param_value`, and the display details. Keep the command line unchanged:

    cargo run --example granulator --features native,crossterm,bounce

For every milestone, format and test before stopping:

    cargo fmt
    cargo test granulator --features bounce
    cargo check --example granulator --features native,crossterm,bounce

Expected successful output includes lines like:

    test result: ok
    Finished `dev` profile

If a milestone touches shared engine behavior or FFI, run the broader tests:

    cargo test --features bounce

## Validation and Acceptance

Milestone 1 is accepted when `cargo run --example granulator --features native,crossterm,bounce` starts the interactive CLI, `SPACE` produces a cloud, `A` auto-triggers clouds, and the visible `random_timing`, `random_amp`, and `drive` controls audibly change the sound without producing non-intentional clicks. With `random_timing` raised, grains should feel less grid-like. With `random_amp` raised, the cloud should have more dynamic variation. With `drive` raised, the output should become denser or more saturated while remaining finite and controlled.

Milestone 2 is accepted when stereo WAV files load without being reduced immediately at load time, per-grain pan changes the internal left/right balance, and the final mono output remains compatible with the existing engine. Tests should prove stereo files produce finite folded-down output.

Milestone 3 is accepted when the CLI can load multiple WAV paths and switching the layer parameter changes which source material is heard. Layer interpolation should produce a smooth crossfade rather than an abrupt switch.

Milestone 4 is accepted when the CLI can toggle follow mode and the cloud moves through the sample over time without the user changing scan position manually. Frozen scan mode must keep its current behavior.

Milestone 5 is accepted when enabling pitch deviation produces discrete, repeatable pitch choices per grain with a fixed random seed. The continuous pitch parameter must still work.

Milestone 6 is accepted when a test can write generated audio into a recording buffer while the granulator plays from it, and the output remains finite and click-controlled around the write head.

Milestone 7 is accepted when the FFI engine can select the granulator as a channel instrument, set its parameters, provide a sample buffer, trigger it, and render nonzero finite output from `gooey_engine_render`.

## Idempotence and Recovery

The plan is additive. Running `cargo fmt`, `cargo test`, and `cargo check` multiple times is safe. If a milestone fails halfway, keep the last passing state by reverting only the files touched for that milestone; do not revert unrelated user changes.

The `.context` directory is workspace-local and gitignored. Updating this plan does not affect the compiled library. Source changes for milestones should stay scoped to `src/instruments/granulator.rs`, `examples/granulator.rs`, and `tests/granulator.rs` until a milestone explicitly requires broader files such as `src/ffi.rs` or `src/engine/mod.rs`.

For audio regressions, first reduce the problem to the generated demo buffer and fixed seed. If clicks appear, inspect grain start/end envelope behavior, buffer-edge reads, voice stealing, and aggregate gain changes before changing user-facing controls.

## Artifacts and Notes

Relevant original-design source files:

    /Users/pretzel/Downloads/arbhar_updater_2-13/arbhar_play_with_buffers
    /Users/pretzel/Downloads/arbhar_updater_2-13/arbharGrainCore.pd
    /Users/pretzel/Downloads/arbhar_updater_2-13/configurationDataInitFile.txt
    /Users/pretzel/Downloads/arbhar_updater_2-13/arbhar_structure.md

Current V1 verification commands used during initial implementation:

    cargo test
    cargo test --features bounce
    cargo check --example granulator --features native,crossterm,bounce

Focused verification after click reduction:

    cargo test granulator --features bounce
    cargo check --example granulator --features native,crossterm,bounce

The most important current behavior is in `src/instruments/granulator.rs`: `spawn_grain` chooses a safe start position so grains finish under their window envelope, and `tick_grains` smooths gain compensation as the active grain count changes.

## Interfaces and Dependencies

Do not add new dependencies for Milestone 1. Use the existing `crate::utils::SmoothedParam` for smoothed normalized parameters and the existing local `XorShift32` pseudo-random generator for deterministic random choices. The word deterministic means that with the same seed and same inputs, the output samples are exactly repeatable.

At the end of Milestone 1, these public methods should exist on `Granulator` in `src/instruments/granulator.rs`:

    pub fn random_timing(&self) -> f32;
    pub fn random_amp(&self) -> f32;
    pub fn drive(&self) -> f32;
    pub fn set_random_timing(&mut self, value: f32);
    pub fn set_random_amp(&mut self, value: f32);
    pub fn set_drive(&mut self, value: f32);

The `Modulatable` parameter list for `Granulator` should include:

    "scan_position"
    "grain_length"
    "spray"
    "pitch"
    "density"
    "texture"
    "direction"
    "random_timing"
    "random_amp"
    "drive"
    "volume"

If Milestone 2 introduces stereo internals, keep `Instrument::tick` unchanged. Add any stereo-specific types inside `src/instruments/granulator.rs` first. Do not change `src/engine/mod.rs` until there is a separate plan for stereo engine output.

If Milestone 7 introduces FFI support, update `src/ffi.rs` only after the core instrument behavior has tests and a stable parameter set. Any new FFI buffer API must state who owns the memory and how it is freed.

## Revision Notes

2026-05-23 / Codex: Rewrote the original gap list into a `.agent/PLANS.md`-compliant ExecPlan after the user imported planning instructions. The rewrite adds required living-document sections, self-contained repository context, concrete implementation milestones, exact validation commands, interface expectations, and rationale for staging the remaining Arbhär design work.