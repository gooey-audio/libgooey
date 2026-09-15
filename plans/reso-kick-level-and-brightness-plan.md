# Restore the resonator kick's level, brightness, and pitch

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. This document is maintained in accordance with `.agent/PLANS.md` at the repository root.

## Purpose / Big Picture

The Entity-style resonator kick currently loses most of its energy inside the resonator, routes the entire voice through a very low character filter, hides its noise click, and changes pitch as Resonate rises. After this work, the kick should retain the decay selected by its controls, expose a brighter parallel character ping and click, and settle at the displayed musical pitch even with high feedback. The behavior is observable through focused DSP tests, the interactive `reso_kick` example, and the full Rust test suite.

## Progress

- [x] (2026-09-15 14:30Z) Read the supplied design, repository planning rules, current resonator kick implementation, focused tests, and Nexus integration instructions.
- [x] (2026-09-15 14:30Z) Confirmed that the branch starts clean at `origin/main` and no matching Nexus task exists to claim.
- [x] (2026-09-15 14:42Z) Implemented the bounded linear resonator states and excitation, band-pass tap, feedback cap, pitch compensation, and unit regressions; all five focused tests pass.
- [ ] Rewire and retune the kick voice and presets around the corrected resonator level.
- [ ] Retune Entity Dynamics and replace its ineffective limiter shaping and detector behavior.
- [ ] Add end-to-end kick regressions and update the interactive signal-flow description.
- [ ] Run formatting, build, test, Clippy, and the interactive example where the environment permits it.

## Surprises & Discoveries

- Observation: The branch is exactly at the commit that introduced the resonator kick, with no local modifications.
  Evidence: `git status --porcelain=v1` was empty and `HEAD` equaled `origin/main` at `307246a`.
- Observation: Nexus has no task whose title contains “reso” and no task attached to this branch.
  Evidence: The authenticated task query returned an empty filtered task list, so no task state was changed.
- Observation: Keeping the old `tanh` only in `excite` limited a requested 0.6 strike to 0.537 and left the 100 ms envelope at 0.277, below the designed 0.3 floor.
  Evidence: The first focused test run failed with `envelope=0.27702448`; applying the same identity-through-one state bound to excitation made all five resonator tests pass.

## Decision Log

- Decision: Use the supplied constants and topology as the initial implementation, then use quantitative regressions to make only the tuning adjustments needed for stable, testable behavior.
  Rationale: The attached design includes analytical measurements and explicit target behaviors; focused measurements are the closest available substitute for listening in an automated coding session.
  Date/Author: 2026-09-15 / Codex
- Decision: Preserve `Resonator::process` as the low-pass output API and add `bandpass()` as a non-breaking read-only tap.
  Rationale: Existing callers keep their established behavior while the kick can use the character core in parallel.
  Date/Author: 2026-09-15 / Codex
- Decision: Apply the linear-until-bounded rule to direct excitation as well as per-sample state integration.
  Rationale: Excitation writes the first integrator state directly; retaining `tanh` there still removed meaningful strike energy and missed the explicit no-collapse envelope target.
  Date/Author: 2026-09-15 / Codex

## Outcomes & Retrospective

Implementation is in progress. This section will record the final measured behavior, verification results, and any residual listening work.

## Context and Orientation

`src/filters/resonator.rs` implements a two-integrator state-variable resonator. Its `s1` and `s2` values are the stored integrator states; repeatedly applying `tanh` directly to them removes energy even when the configured decay should retain it. The low-pass state is also fed back to extend ringing, which currently lowers the oscillation frequency and can create a direct-current, or DC, offset when feedback exceeds one.

`src/instruments/reso_kick.rs` uses two of those resonators. The first makes the low body, then an oversampled soft clipper supplies Punch. The second currently receives that complete result in series and returns its low-pass tap, so it removes high-frequency Punch and click content. This plan changes the second resonator into a parallel character voice by exciting it from the punched body plus noise and mixing its band-pass tap beside the direct body and click.

`src/effects/entity_dynamics.rs` is the example's compressor, gain, and limiter section. Its limiter currently detects the already-shaped output against the same ceiling the shaper can never exceed. The feedback gain therefore never reduces, while the hyperbolic-tangent shaper attenuates even ordinary signals.

`tests/reso_kick.rs` renders complete voices without an audio device and measures stability, pitch, level, brightness, and tail behavior. `examples/reso_kick.rs` is the interactive native listening tool.

## Plan of Work

First, update `src/filters/resonator.rs`. Keep nominal frequency as the public musical pitch, derive the internal coefficient from a feedback-compensated effective frequency, and recompute it whenever frequency or feedback changes. Derive decay damping from the same effective frequency. Limit feedback to 0.8. Replace state-wide `tanh` calls with a helper that is exactly linear through absolute state one and asymptotically bounds larger state to absolute value two. Store the current band-pass value and expose it through `bandpass()`. Reset that tap with the other states. Extend unit tests to prove nominal pitch at zero and nonzero feedback, sustained amplitude under a one-second decay, bounded output, and a near-zero long-tail mean.

Second, update `src/instruments/reso_kick.rs`. Make Punch unity at its minimum and cap it at sixteen with the gentler macro curve from the supplied design. Limit Resonate to the core's new 0.8 cap. Feed noise to the parallel character resonator and mix the direct punched body, character band-pass tap, and direct click using initial mix constants of 0.7 and 0.5. Remove obsolete output makeup, reduce ripple's body scaling, remap preset frequency controls to preserve their historical audible pitches, and rewrite the module description to reflect the new signal topology.

Third, update `src/effects/entity_dynamics.rs`. Move the compressor threshold from -12 to -6 dBFS. Detect limiting below the final ceiling, use the detector to engage feedback gain on overdriven signals, and use a transfer function that is identity through the detection knee and smoothly approaches the ceiling above it. Add a unit test that establishes a +12 dB input drive causes limiter gain reduction while retaining the existing finite-output bound.

Finally, update `tests/reso_kick.rs` with complete-voice regressions for retained tail level, pitch stability across Resonate values, increased high-frequency energy from Character, a long ringing tail without DC, the new Punch endpoints, and the hotter expected peak window. Update the `examples/reso_kick.rs` signal line. Tune only where measurements show the supplied starting values miss their explicit acceptance windows.

## Concrete Steps

Work from `/Users/pretzel/conductor/workspaces/libgooey/yerevan`. After each coherent edit, run the focused test target that covers it. At completion, run:

    cargo build
    cargo build --example reso_kick --features native,crossterm
    cargo test --verbose
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features
    cargo run --example reso_kick --features native,crossterm

The last command requires an interactive terminal and audio output. If those are unavailable, report that limitation explicitly while retaining all automated render measurements.

## Validation and Acceptance

The resonator test must count 58 through 62 positive zero crossings in one second at a nominal 60 Hz both without feedback and with feedback 0.6. A resonator excited to 0.6 with a one-second T60 must peak at or above 0.5 and retain an absolute low-pass envelope of at least 0.3 around 100 ms. At maximum feedback it must remain finite and bounded, with a tail mean near zero rather than a latched DC value.

The complete Classic 808 voice at velocity 0.75 and midpoint Punch must peak between -4 and 0 dBFS. Its 100–300 ms RMS must be no more than 15 dB below overall peak. Tail pitch measured between 300 and 600 ms at Resonate 0.3 and 0.8 must differ by less than five percent. Raising Character from 0.2 to 0.8 must increase first-difference RMS. The Sub Drone tail after eight seconds must have RMS above 0.01 and absolute mean below 0.005. Ten seconds at every maximum setting must stay finite and below absolute amplitude 3.0.

All repository tests must pass, formatting must be unchanged after `cargo fmt`, and Clippy must report no errors. In an audio-capable interactive session, presets one through six and the velocity cycle should confirm an audible click, useful Punch harmonics, a sustained approximately 31 Hz Sub Drone without a DC thump, and tail pitch matching the displayed value.

## Idempotence and Recovery

All source edits and validation commands are repeatable. Tests render into memory and do not create user data. Build artifacts remain under Cargo's normal target directory. If tuning fails an acceptance window, inspect the measured assertion values and adjust only the relevant voice mix or preset control; do not weaken stability, pitch, or DC rejection requirements.

## Artifacts and Notes

The intended voice mix is:

    punched body + 0.7 × character band-pass + 0.5 × exciter noise

The intended limiter transfer is exactly linear below `0.8 * LIMITER_CEILING`, then continuously and smoothly approaches `LIMITER_CEILING` above that knee.

## Interfaces and Dependencies

`crate::filters::Resonator` retains `pub fn process(&mut self, input: f32) -> f32` as its low-pass output and gains `pub fn bandpass(&self) -> f32`. No external crates or public C interfaces are added. `ResoKickConfig` retains its existing normalized fields, but the built-in preset values and the physical Punch mapping change. `EntityDynamics` retains its public controls and effect interfaces; only its internal threshold and limiter transfer change.

Revision note (2026-09-15 14:42Z): Recorded completion of the resonator milestone and the measured reason direct excitation uses the new state bound too.
