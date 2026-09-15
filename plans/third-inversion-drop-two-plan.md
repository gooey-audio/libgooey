# Pin Third Inversion and Drop 2 as app-ready voicings

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. This document must be maintained in accordance with `.agent/PLANS.md` from the repository root.

## Purpose / Big Picture

The host app currently offers Root, First, Second, Open, and Spread chord choices. Third Inversion and Drop 2 expand that palette without changing libgooey's ABI: Third Inversion puts the seventh chord tone in the bass, while Drop 2 lowers the second-highest voice of a close-position chord by one octave. After this work, tests will guarantee the intended notes, stable numeric IDs, audible poly-synth routing, four-voice piano triggering, and preservation through performance recording so the host can safely expose both choices.

## Progress

- [x] (2026-09-15 14:22Z) Inspected the music-theory implementation, C FFI, chord triggers, piano tests, performance recorder, generated header, and existing app-facing IDs.
- [x] (2026-09-15 14:22Z) Confirmed the baseline voicing unit suite passes 10 tests and no matching Nexus task exists.
- [x] (2026-09-15 14:22Z) Added exact Third Inversion coverage plus poly, piano, and recorded-event FFI regressions for IDs 3 and 5.
- [x] (2026-09-15 14:25Z) Ran the focused tests, full suite, iOS-feature build, formatting check, generated-header inspection, and diff check; all required validation passes.

## Surprises & Discoveries

- Observation: libgooey already implements and exports both requested choices.
  Evidence: `VoicingType::ThirdInversion` and `VoicingType::Drop2` map to `VOICING_THIRD_INVERSION = 3` and `VOICING_DROP2 = 5` in `src/ffi.rs`, and cbindgen emits both constants in the gitignored `include/gooey.h`.
- Observation: the repository does not contain the external app menu described by the user.
  Evidence: the only interactive chord UI is `examples/chords.rs`, which already uses `available_voicings` and therefore already shows these choices.
- Observation: reordering `available_voicings` would be a compatibility risk even though the host should persist stable IDs.
  Evidence: the function returns an ordered vector and the chord example selects it by index, so other Rust callers may do the same.

## Decision Log

- Decision: preserve every existing voicing ID, transformation, and availability ordering.
  Rationale: engine behavior is already complete, and compatibility is more important than rewriting working code to create a cosmetic diff.
  Date/Author: 2026-09-15 / Codex.
- Decision: implement the repository-owned portion as regression coverage across theory, poly FFI, piano FFI, and performance recording.
  Rationale: these tests turn the existing behavior into an explicit contract the external app can depend on.
  Date/Author: 2026-09-15 / Codex.

## Outcomes & Retrospective

The repository-owned implementation is complete. Third Inversion now has an exact four-note theory regression, and the poly synth, piano, and performance recording integration suites pin the two app-facing choices across their public C entry points. No runtime implementation or ABI change was needed because the engine already supplied the requested transformations and stable IDs. The full suite and iOS build pass, and the generated header still exports IDs 3 and 5.

## Context and Orientation

`src/music/voicing.rs` converts a `Chord` into sorted MIDI note numbers. For C major seventh at octave 4, Third Inversion must yield B4, C5, E5, G5 (`71, 72, 76, 79`), and Drop 2 must yield G3, C4, E4, B4 (`55, 60, 64, 71`). `src/ffi.rs` maps public numeric voicing IDs to these Rust variants and uses the same conversion for poly-synth and multi-sampled piano chord triggers. `src/performance/mod.rs` records the raw numeric voicing ID so a later playback uses the same choice.

## Plan of Work

Add the missing exact Third Inversion unit assertion beside the existing Root, First, Second, and Drop 2 expectations. Add a poly FFI integration test that pins IDs 3 and 5 and proves both choices render audio. Add a piano FFI integration test with a sample map spanning G3 through G5, proving each choice maps all four C major seventh notes and activates four voices. Add a performance-recording integration test that records both choices and reads back their unchanged IDs.

Do not change the public Rust or C interfaces, `available_voicings` ordering, existing numeric IDs, fallback behavior, or performance-event representation. The external host should add explicit label-to-ID mappings in this order: Root, First, Second, Third, Open, Drop 2, Spread.

## Concrete Steps

Work from `/Users/pretzel/conductor/workspaces/libgooey/saskatoon-v2` and run:

    cargo fmt --all
    cargo test --lib music::voicing::tests
    cargo test --test poly_synth_params third_inversion_and_drop2
    cargo test --test multisample_piano third_inversion_and_drop2
    cargo test --test performance_recording perf_events_preserve_third_inversion_and_drop2_ids
    cargo test
    cargo build --no-default-features --features ios
    git diff --check

The four focused commands must pass their new tests, followed by a clean full suite, iOS-feature build, and diff check.

## Validation and Acceptance

The theory test must assert exactly `[71, 72, 76, 79]` for Third Inversion Cmaj7 and retain `[55, 60, 64, 71]` for Drop 2. Public constants must remain 3 and 5. Both values must make `gooey_engine_poly_trigger_chord` produce audible output, make `gooey_engine_piano_trigger_chord` activate four voices with a complete sample map, and round-trip unchanged through `gooey_engine_perf_get_event`. Existing tests must continue to pass.

The final host-app acceptance, outside this repository, is a seven-choice menu ordered Root, First, Second, Third, Open, Drop 2, Spread, with each label mapped to its explicit ABI constant instead of its menu position.

## Idempotence and Recovery

The changes are test-only apart from this plan and can be rerun safely. Cargo may update gitignored build artifacts and `include/gooey.h`. A failing focused test should be corrected in its owning test file without altering stable IDs or existing voicing math.

## Artifacts and Notes

Baseline focused result before the new coverage:

    cargo test --lib music::voicing::tests
    test result: ok. 10 passed; 0 failed

Final focused and full results:

    cargo test --lib music::voicing::tests
    test result: ok. 11 passed; 0 failed

    cargo test --test poly_synth_params third_inversion_and_drop2
    test result: ok. 1 passed; 0 failed

    cargo test --test multisample_piano third_inversion_and_drop2
    test result: ok. 1 passed; 0 failed

    cargo test --test performance_recording perf_events_preserve_third_inversion_and_drop2_ids
    test result: ok. 1 passed; 0 failed

    cargo test
    test result: ok. 404 library tests; all integration suites and doc-tests passed

    cargo build --no-default-features --features ios
    Finished `dev` profile successfully

    include/gooey.h
    #define VOICING_THIRD_INVERSION 3
    #define VOICING_DROP2 5

    cargo fmt --all -- --check
    git diff --check
    both exited successfully

## Interfaces and Dependencies

No interface or dependency changes are permitted. The host uses the existing `VOICING_THIRD_INVERSION` and `VOICING_DROP2` C constants with `gooey_engine_poly_trigger_chord` or `gooey_engine_piano_trigger_chord`. Performance clips continue to persist the numeric `voicing: u32` value.
