# Sampler Rack Amplitude Envelope

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This plan is maintained in accordance with `.agent/PLANS.md` (repository root). Read that file first if you are authoring or revising this plan.

## Purpose / Big Picture

A "sampler rack" in this project is a fixed pool of sample pads and playback voices used by the C FFI engine. It lives in `src/instruments/sampler.rs` as the `SamplerRack` struct and is driven from `src/ffi.rs`. Before this change a triggered pad played its raw PCM data to the very end, with only a tiny fixed 32-frame fade guarding against clicks. A pad loaded with a long recording (say several seconds) would therefore ring on far longer than a drum hit should, and there was no way to shape or shorten it.

After this change every sampler rack owns one shared amplitude envelope with three times in seconds: attack, hold, and release ("A/H/R"). "Amplitude envelope" means a gain curve applied over time: the voice's gain ramps up from silence over the attack, stays at full for the hold, then ramps back down to silence over the release. Every hit — whether triggered by hand, by the rack's step sequencer, or replayed from a recorded performance — is shaped by this one envelope. New racks default to a 1 ms attack, a 1 s hold, and a 50 ms release, roughly 1.051 s of maximum sound, so an over-long sample is bounded automatically while a naturally short sample still ends on its own. Playback stays polyphonic: each voice runs its own copy of the envelope state, so overlapping hits do not interfere.

You can see it working two ways. Run the unit and integration tests (below) and watch the new envelope tests pass. Or run the interactive CLI `cargo run --example sampler_rack --features native,crossterm`, hold down a long pad, and press the hold/release keys: the sounding pad audibly reshapes in real time and the displayed A/H/R numbers track what you hear.

## Progress

- [x] (2026-07-11) Added `SamplerEnvelopeConfig` (validated, finite, non-negative A/H/R) plus `EnvPhase` and per-voice envelope state to `src/instruments/sampler.rs`.
- [x] (2026-07-11) Replaced the fixed voice fade with a real A/H/R envelope that reads the shared config every frame, keeping a buffer-end taper for PCM that runs out first.
- [x] (2026-07-11) Added `SamplerRack::set_amp_envelope` / `amp_envelope` and defaulted new racks to 1 ms / 1 s / 50 ms.
- [x] (2026-07-11) Added FFI `gooey_engine_sampler_set_amp_envelope` and `gooey_engine_sampler_get_amp_envelope` with rack, null-pointer, and NaN/inf/negative rejection; cbindgen regenerated `include/gooey.h`.
- [x] (2026-07-11) Updated `examples/sampler_rack.rs` to use ~2.5 s pads, display live A/H/R, and adjust each with keys while hits sound.
- [x] (2026-07-11) Added six unit tests in `src/instruments/sampler.rs` and four integration tests in `tests/sampler_rack.rs`; full suite green.

## Surprises & Discoveries

- Observation: The release "50% point" lands a couple of frames later than the naive `hold_end + release/2` estimate because the release only begins decrementing the level on the frame after the hold-to-release transition, and the output uses the pre-advance level.
  Evidence: With sr=1000, attack 100 / hold 200 / release 50 frames, `out[325]` measured 0.56 rather than 0.50; the unit test asserts "near 50%" within 0.1 instead of a tight tolerance.

- Observation: `cargo fmt --all` reformats the entire workspace, including files this feature does not touch (`examples/performance_record.rs`, `tests/performance_recording.rs`), which were already non-compliant at the base commit under the local rustfmt.
  Evidence: `git stash && cargo fmt --all -- --check` reports diffs in those two files with no working changes present. They were intentionally left untouched to keep this diff focused; only the four feature files are formatted.

## Decision Log

- Decision: Track each voice's envelope as a running `level` accumulator and read the rack config every frame, changing only the ramp *rate*, rather than recomputing gain from absolute elapsed time.
  Rationale: This makes live edits continuous with no gain jumps — a changed attack/release alters the slope from the current level onward, a shortened hold begins release immediately, and a voice already in release or finished is never resurrected.
  Date/Author: 2026-07-11, Brian Hurlow (via agent)

- Decision: Floor attack and release ramps at a 32-frame minimum (`MIN_RAMP_FRAMES`) but give hold no floor.
  Rationale: A literal 0 ms attack or release would step the gain in a single sample and click; 32 frames is inaudible yet click-free. Hold legitimately may be 0 s (attack straight into release), so it is not floored.
  Date/Author: 2026-07-11, Brian Hurlow (via agent)

- Decision: Keep only a buffer-*end* taper (not the former start taper) as the raw-PCM click guard.
  Rationale: The attack ramp now always starts a voice from silence, so the start is already click-free; the end taper is still needed for samples whose PCM runs out before the envelope finishes.
  Date/Author: 2026-07-11, Brian Hurlow (via agent)

- Decision: Validate A/H/R in the rack setter and reject invalid FFI input there, leaving the previous config unchanged.
  Rationale: The plan requires invalid updates to be no-ops; centralizing validation in `SamplerEnvelopeConfig::new` keeps the FFI thin and the rule in one place.
  Date/Author: 2026-07-11, Brian Hurlow (via agent)

## Outcomes & Retrospective

The feature is complete and demonstrably working. Every sampler voice is now shaped by the rack's shared A/H/R envelope; a long pad is bounded to ~1.051 s by default, a longer hold extends it, a short PCM buffer still ends naturally, and live edits reshape sounding voices without discontinuities. FFI set/get round-trip and reject bad input without mutating state. The full test suite (283 library tests plus all integration suites, including 10 in `tests/sampler_rack.rs` and 17 sampler unit tests) passes; clippy is clean for the feature files; the generated header exposes both new functions. The only gap left untouched is pre-existing fmt drift in two unrelated files, deliberately not reformatted to keep the change focused.

## Context and Orientation

The reader needs no prior context. The relevant files, by full repository-relative path:

- `src/instruments/sampler.rs` — the rack. `SamplerBuffer` holds copied PCM; `SampleVoice` is one playback voice; `SamplerRack` owns 16 slots, 32 voices, a step `Sequencer`, and now a `SamplerEnvelopeConfig`. `SAMPLER_SLOT_COUNT = 16`, `SAMPLER_VOICE_COUNT = 32`.
- `src/ffi.rs` — the C FFI. `GooeyEngine` holds an array of optional `SamplerRack`s. Sampler functions are grouped near `gooey_engine_sampler_register`; the two new envelope functions sit just after `gooey_engine_sampler_get_step`.
- `examples/sampler_rack.rs` — an interactive terminal CLI over the FFI (native + crossterm features).
- `tests/sampler_rack.rs` — integration tests exercising the FFI end-to-end.
- `include/gooey.h` — generated by cbindgen from `build.rs` on every build; do not edit by hand.

Terms used here: a "voice" is one currently-sounding copy of a pad; "polyphonic" means several voices sound at once; a "frame" is one sample-time step (the engine renders one stereo frame per tick); "PCM" is the raw decoded audio data in a slot.

## Plan of Work

In `src/instruments/sampler.rs`, add `MIN_RAMP_FRAMES = 32.0` and `BUFFER_TAPER_FRAMES = 32.0` constants, a public `SamplerEnvelopeConfig` struct with private `attack_seconds`/`hold_seconds`/`release_seconds`, a validating `new` returning `Option<Self>` (finite and non-negative), getters, a `Default` of 0.001 / 1.0 / 0.05, and private helpers `attack_frames`, `release_frames` (both floored at `MIN_RAMP_FRAMES`), and `hold_frames` (no floor). Add a private `EnvPhase` enum (`Attack`, `Hold`, `Release`, `Finished`). Give `SampleVoice` new fields `phase`, `level`, `hold_elapsed`, `release_from`, initialize them in `start` (phase `Attack`, level 0), and add `advance_envelope(&config, sample_rate)` implementing the state machine. Rewrite `SampleVoice::tick` to take `(&config, sample_rate)`, multiply `level * velocity * buffer_end_taper`, advance the envelope, and free the buffer when PCM ends or the envelope finishes. Add an `envelope` field to `SamplerRack`, initialize it in `new`, copy it in `tick` before the mutable voice loop, and add `set_amp_envelope` (returns bool) and `amp_envelope`.

In `src/ffi.rs`, after `gooey_engine_sampler_get_step`, add `gooey_engine_sampler_set_amp_envelope(engine, rack, attack, hold, release) -> bool` and `gooey_engine_sampler_get_amp_envelope(engine, rack, out_attack, out_hold, out_release) -> bool`, following the existing null-check and `and_then(Option::as_ref/as_mut)` patterns, with a `/// # Safety` section on each.

In `examples/sampler_rack.rs`, lengthen the generated pad to ~2.5 s, add `envelope`/`adjust_envelope` helpers, show the current A/H/R in `draw`, and bind keys `a/A`, `h/H`, `e/E` to nudge attack/hold/release while playing.

Add tests as described in Validation.

## Concrete Steps

Run from the repository root `/Users/pretzel/conductor/workspaces/libgooey/malabo`:

    cargo build
    cargo build --example sampler_rack --features native,crossterm
    cargo test --test sampler_rack
    cargo test
    cargo check --no-default-features --features ios
    cargo clippy --all-targets --all-features

Expected: the library and example build; `cargo test --test sampler_rack` reports `10 passed`; the full `cargo test` reports all suites passing (283 library tests plus integration suites); the iOS check builds; clippy is clean for `src/instruments/sampler.rs`, `src/ffi.rs`, `examples/sampler_rack.rs`, and `tests/sampler_rack.rs`.

To exercise it by ear:

    cargo run --example sampler_rack --features native,crossterm

Press `1`–`4` to trigger long pads. Press `h` to shorten the hold while a pad sounds — the tone cuts off sooner. Press `H` to lengthen it — the pad rings longer. Press `e`/`E` to shorten/lengthen the release fade, and `a`/`A` for the attack. The "Amp envelope" line updates to match what you hear.

## Validation and Acceptance

Unit tests in `src/instruments/sampler.rs` (run `cargo test --lib sampler`, expect `17 passed`): default configuration equals 0.001 / 1.0 / 0.05; the A/H/R progression rises, holds at full, and falls to silence; a short PCM buffer ends naturally before the envelope; a voice deactivates after release even with PCM remaining; zero attack and release still start at silence and ramp across ~32 frames with no click-sized jump; and editing the envelope on a sounding voice keeps consecutive-sample gain changes below a small threshold.

Integration tests in `tests/sampler_rack.rs` (run `cargo test --test sampler_rack`, expect `10 passed`): FFI set/get round-trips and the default hold reads as 1.0 s; invalid input (negative, NaN, infinity, bad rack, null pointer) is rejected without mutating the stored config; the default hold caps a 3 s pad to silence past the envelope while a 2.5 s hold keeps the same pad sounding; and shortening the hold on an active hit releases to silence without a click.

Acceptance is behavior, not structure: a long sample is audibly and measurably bounded by the default envelope, a longer hold audibly extends it, and reshaping a sounding pad is click-free. These new tests fail against the pre-change code (which had no envelope API and no cap) and pass after.

## Idempotence and Recovery

All steps are safe to repeat. The change is additive: existing sampler APIs (trigger, sequencing, recording, routing, slot loading) keep working unchanged, and the existing test `rack_layers_and_steals_without_non_finite_audio` still passes. `include/gooey.h` regenerates deterministically on each build. If a step fails midway, re-run the relevant `cargo` command; nothing is destructive and there is no migration or persisted state.

## Artifacts and Notes

Full validation transcript (abridged):

    $ cargo test
    test result: ok. 283 passed; 0 failed  (library)
    test result: ok. 10 passed; 0 failed   (tests/sampler_rack.rs)
    ... all other integration suites: ok

    $ grep amp_envelope include/gooey.h
    bool gooey_engine_sampler_set_amp_envelope(struct GooeyEngine *engine, ...);
    bool gooey_engine_sampler_get_amp_envelope(const struct GooeyEngine *engine, ...);

Note on the release-timing tolerance (see Surprises): measured `out[325] = 0.56` at sr=1000 with 100/200/50-frame A/H/R, so the progression test asserts the release midpoint is "near 50%" within 0.1 rather than exactly 0.5.

## Interfaces and Dependencies

No new libraries or dependencies. The types and signatures that exist at completion:

In `src/instruments/sampler.rs`:

    pub struct SamplerEnvelopeConfig { /* private A/H/R seconds */ }
    impl SamplerEnvelopeConfig {
        pub fn new(attack_seconds: f32, hold_seconds: f32, release_seconds: f32) -> Option<Self>;
        pub fn attack_seconds(&self) -> f32;
        pub fn hold_seconds(&self) -> f32;
        pub fn release_seconds(&self) -> f32;
    }
    impl Default for SamplerEnvelopeConfig { /* 0.001 / 1.0 / 0.05 */ }

    impl SamplerRack {
        pub fn set_amp_envelope(&mut self, attack_seconds: f32, hold_seconds: f32, release_seconds: f32) -> bool;
        pub fn amp_envelope(&self) -> SamplerEnvelopeConfig;
    }

In `src/ffi.rs`:

    pub unsafe extern "C" fn gooey_engine_sampler_set_amp_envelope(
        engine: *mut GooeyEngine, rack: u32,
        attack_seconds: f32, hold_seconds: f32, release_seconds: f32) -> bool;
    pub unsafe extern "C" fn gooey_engine_sampler_get_amp_envelope(
        engine: *const GooeyEngine, rack: u32,
        out_attack_seconds: *mut f32, out_hold_seconds: *mut f32, out_release_seconds: *mut f32) -> bool;

## Revision Notes

- 2026-07-11: Initial authored-and-implemented version. The plan and the code landed together; every section reflects the completed, test-green state. Written per `.agent/PLANS.md`; the "why" behind the running-accumulator envelope, the 32-frame ramp floor, the end-only buffer taper, and centralized validation is recorded in the Decision Log.
