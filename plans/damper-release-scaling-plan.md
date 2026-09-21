# Make piano damper release respond at key release

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. Maintain this document in accordance with `.agent/PLANS.md`.

## Purpose / Big Picture

The multi-sample piano currently chooses a note's damper duration when the note starts. A player therefore cannot hold a chord, adjust the damper control, and hear the new setting when releasing that chord. After this change, the current smoothed damper setting is read when a key is released, when the sustain pedal lifts, or when all notes are released. The lowest setting is also shortened from one quarter of the sample pack's authored release to one sixteenth, while the neutral 1.0x center and 4.0x maximum stay fixed.

A requested release that is longer than the playable sample remaining will be shortened to the available PCM duration, avoiding a tail that is still audible when the recording ends. C and Swift hosts will also be able to read every normalized piano parameter target through a new function, so a successful setter call can be verified directly.

## Progress

- [x] (2026-09-21 17:41Z) Read the handoff, current multi-sample voice code, envelope behavior, FFI conventions, tests, generated-header setup, and release workflow.
- [x] (2026-09-21 17:41Z) Verified the published v1.1.7 iOS archive already contains all six `PIANO_PARAM_*` macros and the existing setter.
- [x] (2026-09-21 17:50Z) Changed voice release timing, the lower-half control curve, and PCM-bound clamping.
- [x] (2026-09-21 17:50Z) Added the normalized piano parameter getter and generated C declaration.
- [x] (2026-09-21 17:50Z) Added unit and integration regressions for every release path, curve endpoints, smoothed-current semantics, clamping, frozen tails, loops, and getter validation.
- [x] (2026-09-21 17:50Z) Ran formatting, focused tests, the complete library/integration suite, iOS builds, header inspection, Clippy, and archive symbol checks.

## Surprises & Discoveries

- Observation: `include/gooey.h` is generated and gitignored, but the v1.1.7 release archive already contains the six piano parameter constants requested by the handoff.
  Evidence: Inspecting `libgooey-ios-v1.1.7.tar.gz` found `PIANO_PARAM_VOLUME` through `PIANO_PARAM_VELOCITY_SPAN` and `gooey_engine_piano_set_param` in its packaged header.

- Observation: Repository-wide strict Clippy is already red independently of this feature.
  Evidence: The baseline reports existing findings in `src/gen/polyblep.rs`, `src/dsl.rs`, `src/envelope.rs`, `src/ffi.rs`, and other untouched files. Validation will ensure the touched code adds no new diagnostics rather than expanding scope.

- Observation: Apple's `nm` reports LLVM attribute-version errors for some Rust standard-library objects produced by Rust 1.96, but it still reads libgooey's object and finds the new symbol in both archives.
  Evidence: Device and simulator output respectively contain `0000000000010ca4 T _gooey_engine_piano_get_param` and `0000000000010bb4 T _gooey_engine_piano_get_param` after stderr is suppressed.

- Observation: The first sustain-loop test fixture did not actually reach the clamp because its three seconds of PCM exceeded the 1.6-second requested release.
  Evidence: Replacing it with a half-second buffer made the assertion exercise the intended finite-PCM branch; the exact remaining-time assertion then passed.

## Decision Log

- Decision: Store the zone-authored release duration on each `MsVoice` and apply the current smoothed multiplier only on the voice's first release request.
  Rationale: Held voices respond to the latest audible control state, while changing the envelope duration after a tail has begun would create an amplitude discontinuity.
  Date/Author: 2026-09-21 / Codex

- Decision: Clamp normal and sustain-loop releases to the seconds of PCM remaining after pitch transposition, but do not clamp continuous loops.
  Rationale: Normal playback and a sustain loop after key release have a finite path to the zone end. A continuous loop can keep supplying PCM until its envelope finishes.
  Date/Author: 2026-09-21 / Codex

- Decision: The new C getter returns `SmoothedParam::target()`, while release events use `SmoothedParam::get()`.
  Rationale: Host state recovery must round-trip immediately after a setter, whereas audio behavior should preserve the engine's existing 15 ms parameter smoothing.
  Date/Author: 2026-09-21 / Codex

## Outcomes & Retrospective

The implementation is complete. Held voices now choose their release duration from the current smoothed damper control at the release event, tails already falling remain frozen, and finite PCM paths cannot end before their envelope reaches zero. The lower half reaches 0.0625x without moving the neutral or maximum points. The new C getter round-trips all six normalized targets and rejects invalid inputs with `NaN`.

All 43 focused multisample tests, six parameter-getter tests, and 19 piano FFI tests pass. The all-feature suite reports 489 library tests passed with two private-artifact tests ignored, followed by every integration target passing. Both 18 MB iOS archives build successfully; the generated header contains all six constants plus the setter/getter declarations, and both archives export `_gooey_engine_piano_get_param`. Formatting and `git diff --check` are clean. Clippy completes with only the repository's documented pre-existing warnings.

## Context and Orientation

`src/instruments/multisample.rs` owns `MultiSampleInstrument`, its fixed pool of `MsVoice` sample voices, and the normalized release curve. A `SampleZone` carries an authored ADSR envelope and PCM buffer. Before this change, `start_zone_voice` converted the instrument's release parameter to a multiplier, and `MsVoice::start` wrote the multiplied value into the envelope before the note sounded. `note_off`, `set_sustain_pedal`, and `release_all` later called a parameterless `MsVoice::release`, so held voices could not observe control changes.

`MsVoice::tick` advances `position` through source PCM by `increment` frames per output sample and advances wall time by `dt` seconds. Normal zones stop at `end`. `LoopSustain` zones loop only while held, then continue toward `end` during release. `LoopContinuous` zones keep looping during release. These values make the finite PCM time remaining equal to `(end - position) / increment * dt`.

`src/ffi.rs` defines the exported C ABI and literal `PIANO_PARAM_*` constants. `build.rs` runs cbindgen and writes the ignored `include/gooey.h`. `tests/param_getters.rs` contains setter/getter round-trip tests for other instruments, while the private unit-test module in `src/instruments/multisample.rs` can inspect voice state directly.

## Plan of Work

In `src/instruments/multisample.rs`, replace the symmetric release curve with a piecewise exponential: `256^(x - 0.5)` below center and `16^(x - 0.5)` at or above center. Preserve exact values of 0.0625x, 1.0x, and 4.0x at normalized inputs 0, 0.5, and 1. Update the bright preset comment to reflect its steeper lower-half multiplier.

Add an authored-release field to `MsVoice`. Initialize it from `SampleZone::envelope.release_time`, leave the envelope unscaled at note-on, and remove the release multiplier argument from `MsVoice::start`. Change `MsVoice::release` to accept a multiplier. Before changing the envelope, reject one-shot zones and voices whose envelope has already started releasing. Clamp the authored duration times the multiplier to the existing 1 ms and 8 second limits, then shorten it further to the playable PCM time remaining for every mode except `LoopContinuous`. Set that duration and begin the envelope release as one operation.

In each of `MultiSampleInstrument::note_off`, `set_sustain_pedal`, and `release_all`, compute the current smoothed multiplier once before mutably iterating through voices, then pass it into every release request. Keep release-zone triggering and pedal bookkeeping unchanged.

In `src/ffi.rs`, add `gooey_engine_piano_get_param` next to the setter. Accept a const engine pointer and return the target of volume, velocity tracking, release, stereo width, dynamic range, or velocity span. Return `NaN` for a null engine, missing piano, or unknown index. Document that it has the same engine-thread access contract as the setter.

Add deterministic tests with synthetic flat PCM. Prove every fixed point of the new curve; prove a control change made after note-on governs direct note-off, pedal lift, and release-all; prove a second release request cannot retime an active tail; prove normal and sustain-loop releases account for the current source position and transposed increment; and prove continuous-loop releases retain the requested duration. Extend FFI getter coverage across all six parameters, finite input clamping, immediate targets, null pointers, invalid piano handles, and invalid indices.

## Concrete Steps

Run all commands from `/Users/pretzel/conductor/workspaces/libgooey/puebla-v1`.

After editing, format and run focused tests:

    cargo fmt --all -- --check
    cargo test --lib --no-default-features instruments::multisample::tests
    cargo test --test multisample_piano --no-default-features
    cargo test --test param_getters --no-default-features

Then run the complete library and integration suite without stale examples:

    cargo test --all-features --lib --tests

Build both iOS static libraries and inspect generated interfaces:

    ./scripts/build-ios.sh
    rg -n "PIANO_PARAM_|gooey_engine_piano_(set|get)_param" include/gooey.h
    nm -gU target/aarch64-apple-ios/release/libgooey.a | rg "_gooey_engine_piano_get_param$"
    nm -gU target/aarch64-apple-ios-sim/release/libgooey.a | rg "_gooey_engine_piano_get_param$"

Finally run `git diff --check` and inspect `git status --short`. The generated header and build outputs must remain untracked and uncommitted.

## Validation and Acceptance

A voice struck at neutral and released after the control settles at zero must use 0.0625 times its authored release, even though that value was not active at note-on. The same must be true when release happens by lifting sustain or calling release-all. Once any of those operations starts a tail, later control changes or release-all calls must not alter its chosen duration.

For finite playback, the envelope release duration must never exceed the seconds required to reach the zone end at the current transposed increment. A sustain loop must stop looping and obey that clamp after release. A continuous loop must instead use the requested release duration because it does not run out of PCM.

The normalized release curve must return exactly 0.0625 at zero, 1.0 at the midpoint, and 4.0 at one. Existing default behavior remains authored 1.0x, and the upper half remains unchanged.

The C getter must round-trip all six setter targets without requiring rendering. Values outside 0–1 must read back clamped. Null engines, unregistered pianos, and invalid indices must return `NaN`. The generated header and both iOS archives must contain the new function symbol.

## Idempotence and Recovery

All edits and tests are repeatable. Cargo may update `target/` and regenerate ignored `include/gooey.h`; neither is committed. No sample packs, migrations, release tags, or remote state are modified. If a test exposes an implementation error, correct only the voice, FFI, test, or plan files involved and rerun the focused command before the complete suite.

## Artifacts and Notes

Stable release mapping after this change:

    normalized 0.0 -> 0.0625x
    normalized 0.5 -> 1.0x
    normalized 1.0 -> 4.0x
    requested release floor -> 0.001 seconds before remaining-PCM clamp
    requested release ceiling -> 8.0 seconds

The handoff's Nebula application changes, version bump, tag, and release publication are outside this repository change. Release-zone loading remains disabled by default and is not altered.

## Interfaces and Dependencies

The only new public interface is:

    float gooey_engine_piano_get_param(const GooeyEngine *engine,
                                       uint32_t piano,
                                       uint32_t param);

No crate dependency is added. The implementation continues to use `Envelope`, `SmoothedParam`, and existing fixed-size voice storage, so the audio render path remains allocation-free.

Plan revision note (2026-09-21): Created after validating the handoff against current `origin/main`, baseline focused tests, the cbindgen release path, and the published v1.1.7 header.

Plan revision note (2026-09-21): Marked implementation complete after focused and all-feature tests, Clippy inspection, both iOS builds, generated-header inspection, and device/simulator symbol verification.
