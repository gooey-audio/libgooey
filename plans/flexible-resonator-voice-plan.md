# Build a flexible dual-resonator voice

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must remain current as work proceeds. Maintain this document according to `.agent/PLANS.md`.

## Purpose / Big Picture

The existing `ResoKick` proves that a nonlinear resonator can make compelling kick sounds, but its signal routing and control mappings are fixed around that category. This work adds a reusable `ResonatorVoice` whose transient, noise, two resonant modes, nonlinear drive, and feed-forward routing can make pitched drums, noisy drums, and sounds between those categories. A developer can instantiate one of five factory patches or construct a patch in physical units, and a musician can alter nine normalized macro controls in real time. The existing kick remains source- and sound-compatible.

## Progress

- [x] (2026-09-22 03:05Z) Read the existing resonator, kick, exciter, smoothing, examples, tests, repository planning rules, and Nexus instructions.
- [x] (2026-09-22 03:20Z) Added the public dual-resonator configuration types, fixed allocation-free graph, factory patches, macro layer, and live advanced setters.
- [x] (2026-09-22 03:27Z) Added focused integration coverage for stability, routing, noise tails, deterministic seeds, latching, smoothing, and lifecycle behavior.
- [x] (2026-09-22 03:41Z) Extended the terminal audition example with legacy, generic macro, and generic advanced pages plus all five factory patches.
- [x] (2026-09-22 03:48Z) Passed 14 legacy kick tests, 8 generic voice tests, 476 library tests, all no-default-feature targets, the full default-feature suite, and the native example build.
- [x] (2026-09-22 03:52Z) Captured full-buffer deterministic hashes for all six legacy presets and documented the pre-existing strict Clippy blockers.
- [x] (2026-09-22 04:18Z) Added and smoke-tested a native GLFW graph lab with selectable nodes/cables, live routing edits, presets, waveform display, and audio output.
- [x] (2026-09-22 04:42Z) Added in-window bitmap labels and a four-track, 16-step sample-accurate sequencer view with editable patterns, BPM, playback, and playhead highlighting.

## Surprises & Discoveries

- Observation: `ResoKickParams` is public and callers can mutate its `SmoothedParam` fields directly.
  Evidence: `src/instruments/reso_kick.rs` exposes `pub params: ResoKickParams`; replacing it with a differently shaped generic parameter object would break source compatibility.
- Observation: the existing short `Exciter` is limited to eight milliseconds and cannot itself make a snare-length noise tail.
  Evidence: `Exciter::set_width_ms` clamps to 0.25–8 ms, so `ResonatorVoice` needs a separate filtered noise generator and envelope.
- Observation: a noise-only patch initially deactivated on the first attack sample because its envelope begins at exact zero.
  Evidence: the routing-isolation test rendered an RMS of zero until voice completion was changed to require the noise envelope's full T60 window to elapse.
- Observation: strict repository-wide Clippy is not currently clean for unrelated existing code.
  Evidence: `cargo clippy --lib --no-default-features -- -D warnings` reports 56 existing errors beginning in `src/gen/polyblep.rs`, `src/dsl.rs`, and `src/envelope.rs`; it reports none in `resonator_voice.rs` before stopping.

## Decision Log

- Decision: Keep the existing `ResoKick` signal path intact as the compatibility facade while adding the reusable engine beside it.
  Rationale: This preserves public field access and deterministic legacy rendering. Moving its public smoothers into a generic object would be a source-breaking change, while making the new generic engine emulate kick-only arithmetic would compromise the new architecture.
  Date/Author: 2026-09-22 / Codex
- Decision: Use an acyclic routing matrix with mode one feeding mode two, but never mode two feeding mode one.
  Rationale: It covers parallel, serial, and hybrid patches without adding another potentially unstable feedback loop.
  Date/Author: 2026-09-22 / Codex
- Decision: Smooth physical continuous controls and latch exciter shape and output taps on trigger.
  Rationale: Continuous edits should not click; changing the interpretation of an already-ringing voice is intentionally deferred to the next hit.
  Date/Author: 2026-09-22 / Codex
- Decision: Represent the editor as the fixed audio graph itself instead of another parameter-page UI.
  Rationale: Cable thickness and brightness can directly communicate routing gain, clicking a component or cable selects the corresponding live control, and the existing GLFW/OpenGL approach keeps the lab consistent with `polysynth_gui` without adding dependencies.
  Date/Author: 2026-09-22 / Codex
- Decision: Drive four independent `Engine` sequencers and four resonator voices from the beat grid.
  Rationale: A useful beat needs simultaneous kick, snare, tom, and hybrid roles, and the existing engine sequencer keeps triggers sample-accurate while the GUI thread only edits patterns and transport state.
  Date/Author: 2026-09-22 / Codex

## Outcomes & Retrospective

The crate now exports a two-mode resonator instrument with independent transient and noise excitation, physical-unit live controls, normalized musical macros, trigger-latched discrete choices, five starting patches, a terminal workbench, and a labeled native connected-graph editor with a four-track step sequencer. The unchanged legacy implementation passes its original behavioral tests plus deterministic full-buffer hashes, while the generic voice passes stability, routing, timbre, determinism, smoothing, and lifecycle coverage. The full test suite is green. Repository-wide strict Clippy remains blocked by pre-existing warnings outside this change.

## Context and Orientation

`src/filters/resonator.rs` supplies one bounded two-pole ringing mode. `src/gen/exciter.rs` supplies a short deterministic transient. `src/instruments/reso_kick.rs` combines two resonators with hard-coded kick-oriented routing. The new `src/instruments/resonator_voice.rs` owns two resonators, two oversampled nonlinear stages, the short transient, a separate deterministic filtered-noise tail, parameter smoothers, and a feed-forward routing matrix. `tests/resonator_voice.rs` proves the new graph behavior. `examples/reso_kick.rs` is the interactive native terminal audition tool.

A resonant mode is a filter that stores energy and rings at a chosen frequency. T60 is the time for that ring to fall by 60 decibels. A tap selects either the low-pass or band-pass output of a resonator. Feed-forward routing means signals only travel toward the output; the only recirculation is each resonator's already-bounded internal feedback.

## Plan of Work

Export the new instrument module from `src/instruments/mod.rs`. Define copyable public configuration structures for the two modes, exciter, noise tail, routing matrix, macro defaults, and whole voice. Sanitize all input, constrain feedback to the resonator's stable range, and use `SmoothedParam` for continuous state. At each trigger, latch discrete shape and tap choices, start deterministic excitation, and retain resonator energy for natural retriggers. Per sample, render transient and filtered noise, route them into mode one and mode two, optionally feed mode one's raw response into mode two, select and drive each tap, then mix direct and resonant outputs.

Expose normalized macros named `pitch`, `pitch_sweep`, `sweep_time`, `decay`, `body_character`, `noise`, `coupling`, `drive`, and `volume` through `Modulatable`. Also expose setters in physical units so a patch editor can change base pitch, each mode, excitation, noise, routing, and output level while audio runs. Provide kick, tom, snare, hybrid, and metallic-drone constructors as useful starting materials rather than synthesis modes.

Keep `ResoKick` unchanged so its public parameter fields, preset mappings, modulation strings, MIDI behavior, and floating-point render order remain compatible. Extend the existing terminal example so its original page still edits `ResoKick`, while generic macro and advanced pages audition the new factory patches and raw routing controls.

## Concrete Steps

From `/Users/pretzel/conductor/workspaces/libgooey/tacoma-v3`, format and validate with:

    cargo fmt --check
    cargo test --test reso_kick
    cargo test --test resonator_voice --no-default-features
    cargo test --lib
    cargo test --all-targets --no-default-features
    cargo test
    cargo build --example reso_kick --features native,crossterm

The two focused integration suites report 14 legacy kick tests and 8 new resonator voice tests passing. The native example builds successfully.

## Validation and Acceptance

All five generic patches must produce finite audible output. A deliberately extreme patch must remain finite and below the documented composite-output bound for ten seconds. The snare patch must retain measurable broadband tail energy after its transient ends, while the tom must be spectrally darker. Isolated direct-noise routing must remain audible. Equal seeds must produce identical samples and unequal seeds must diverge. Tap changes must leave the current hit unchanged and take effect on the next trigger. Non-finite advanced edits must be sanitized, and the ring limit must stop the voice.

The unchanged `tests/reso_kick.rs` suite is the compatibility proof: it covers all old presets, parameter endpoints, pitch, brightness, decay, velocity, stability, and ring-limit behavior. Because its implementation is retained, its arithmetic and deterministic render remain unchanged.

For human acceptance, run `cargo run --example reso_kick --features native,crossterm`. Trigger all legacy and generic patches, switch between macro and advanced pages, alter every displayed control, and listen for distinct pitched, noisy, coupled, and long-ringing material without clicks or runaway output. Then run `cargo run --example resonator_voice_gui --features native,visualization`; click labeled source, resonator, output, and cable elements, edit them while triggering hits, and confirm cable strength, selection highlighting, window-title values, and the live scope all respond. Press Tab, edit the labeled kick/snare/tom/hybrid grid by click or keyboard, change BPM, start playback, and confirm the highlighted playhead and resulting beat remain synchronized.

## Idempotence and Recovery

All commands are safe to repeat. Tests create only normal Cargo build artifacts under `target/`. If a new configuration becomes unstable, reduce its routing gains or internal feedback rather than weakening the bounded-output tests. The legacy implementation remains separately available, so generic-engine changes cannot prevent restoring its exact behavior.

## Artifacts and Notes

The initial compatibility baseline was:

    running 13 tests
    test result: ok. 13 passed; 0 failed

Final focused and broad validation was:

    reso_kick:       14 passed; 0 failed
    resonator_voice:  8 passed; 0 failed
    library:         476 passed; 0 failed
    full/default and all-target/no-default suites: passed
    native reso_kick example: built successfully
    native resonator graph GUI: built, launched, opened audio output, and remained responsive

No active Nexus task matched this repository when planning began, so there is no remote task record to update.

## Interfaces and Dependencies

`gooey::instruments` exports `ResonatorVoice`, `ResonatorVoiceParams`, `ResonatorVoiceConfig`, `ResonatorModeConfig`, `ResonatorExciterConfig`, `ResonatorNoiseConfig`, `ResonatorRoutingConfig`, `ResonatorMacroConfig`, `ResonatorExciterShape`, and `ResonatorOutputTap`. `ResonatorVoice` implements the existing `Instrument` and `Modulatable` traits. The implementation uses only existing crate facilities: `Resonator`, `Exciter`, `StateVariableFilterTpt`, `Oversampler`, `SmoothedParam`, and `XorShift32`. It adds no C ABI, DSL syntax, dependency, heap allocation in the audio graph, or variable-size mode bank.

Revision note (2026-09-22): Created the implementation plan after the initial architecture and test surface were established; recorded the compatibility constraint that prevents replacing the public legacy parameter object directly. Updated after implementation with the noise-only lifecycle fix, final validation, deterministic legacy fixtures, the pre-existing Clippy result, and the follow-up native graph editor.
