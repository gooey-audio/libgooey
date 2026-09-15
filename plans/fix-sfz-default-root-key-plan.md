# Fix SFZ default root keys and publish Salamander mobile v2

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. This document is maintained in accordance with `.agent/PLANS.md` at the repository root.

## Purpose / Big Picture

An SFZ region may omit `pitch_keycenter`; the SFZ default is MIDI note 60 (middle C). Libgooey currently substitutes the region's lowest playable key instead. Salamander Grand Piano's C4 regions cover MIDI 59 through 61 without stating a center, so the existing prepared mobile pack labels and tunes those recordings as MIDI 59. After this change, loading or preparing such a region keeps its playable range but roots it at MIDI 60. A replacement eight-layer, six-second stereo pack will be generated from the original WAV archive on GitHub Actions and published as a new immutable private Vercel Blob, leaving v1 intact.

## Progress

- [x] (2026-09-14 20:49Z) Inspected the SFZ parser, preparation tool, voicing tests, existing v1 Blob path and archive layout, GitHub workflows, and baseline focused tests.
- [x] (2026-09-14 20:52Z) Changed SFZ default-center handling and added parser, preparation round-trip, release-contract, and Cmaj7 regressions.
- [x] (2026-09-14 20:52Z) Added a main-only manual workflow that regenerates, validates, archives, and publishes the v2 pack and checksum.
- [x] (2026-09-14 20:53Z) Ran formatting, actionlint, focused Rust tests, the no-native FFI integration test, and the full no-default/bounce suite successfully.
- [x] (2026-09-14 20:52Z) Configured `BLOB_READ_WRITE_TOKEN` as a repository Actions secret without exposing its value.
- [x] (2026-09-14 20:57Z) Pushed commit `ffc142b`, opened PR #236, and observed both Ubuntu and macOS repository test jobs pass.
- [x] (2026-09-14 21:05Z) Merged PR #236 and ran the pack workflow through source verification, generation, archive validation, and candidate retention; its Vercel CLI upload step failed because the clean runner had no account credentials.
- [x] (2026-09-14 21:10Z) Found both v2 objects published after the failed run and independently streamed the private archive to verify its 160,972,915-byte listing, SHA-256, 244 regions, eight corrected C4 centers, and zero old C4 centers.
- [ ] Merge the SDK/idempotent-rerun correction, rerun the workflow from `main`, and capture a green remote verification result.

## Surprises & Discoveries

- Observation: The existing v1 Blob is private at `libgooey/instruments/piano/salamander-mobile-8x6s-v1.tar.gz`, is 160,908,259 bytes, and contains the top-level `piano-mobile/` directory.
  Evidence: An authenticated Blob listing and streamed tar listing returned that pathname, size, and layout.
- Observation: The v1 prepared SFZ contains exactly eight C4-source regions written as `lokey=59 hikey=61 pitch_keycenter=59`, with `B3_059` output names.
  Evidence: Streaming `piano-mobile/instrument.sfz` from v1 showed those entries on lines 111 through 118.
- Observation: The local volume initially had only 117 MiB free, and a native integration build failed with `No space left on device`; no local original pack is present.
  Evidence: `df -h` and the failed `cargo test --test multisample_piano` native build. The same integration test passes with `--no-default-features --features bounce`.
- Observation: No `BLOB_READ_WRITE_TOKEN` Actions secret currently exists, although the credential is available in the global fish environment.
  Evidence: `gh secret list --app actions` listed other secrets but not the Blob credential.
- Observation: The full `cargo test --no-default-features --features bounce` run passes with 411 unit tests passed and one release-contract test intentionally ignored, followed by all integration and doc-test targets passing; five pre-existing `unused_unsafe` warnings remain in `tests/performance_recording.rs`.
  Evidence: Local test output on 2026-09-14.
- Observation: Vercel CLI 58.9.0 does not authenticate its Blob subcommand from `BLOB_READ_WRITE_TOKEN` on a clean GitHub runner; it asked for `vercel login` or the CLI's separate account `--token` even though the Blob credential was present.
  Evidence: GitHub Actions run 34896523766 failed only in `Publish immutable private blobs` with `Error: No existing credentials found`; all generation and validation steps passed.
- Observation: Both v2 objects appeared in the private store shortly after the failed workflow, outside that run's failed CLI step.
  Evidence: Blob metadata reports archive upload at 21:07:37Z and checksum upload at 21:08:05Z. An authenticated stream hashes to `7065831ae3c6a8e104d36d4680b43c122fcdb0b2c0ff7392ee5724086fcc0812`, matching the published sidecar; the embedded SFZ has 244 regions, eight center-60 C4 regions, and no center-59 C4 regions.

## Decision Log

- Decision: Store MIDI 60 as the parsed region's default `pitch_keycenter`, and retain a conversion fallback to the same constant rather than to `lokey`.
  Rationale: This represents the SFZ default explicitly, makes preparation serialize the correct center, and remains safe if an invalid center opcode clears the optional field.
  Date/Author: 2026-09-14 / Codex
- Decision: Publish v2 at the same private Blob prefix as v1 and preserve the `piano-mobile/` archive root.
  Rationale: Inspection established those as the existing consumer-facing contracts; a versioned new pathname avoids cache and overwrite hazards.
  Date/Author: 2026-09-14 / Codex
- Decision: Generate on a manually dispatched Ubuntu GitHub Actions runner and refuse publication unless the checked-out ref is `main`.
  Rationale: The user selected CI generation because local disk cannot safely hold the source archive, extraction, prepared pack, and compressed artifact together.
  Date/Author: 2026-09-14 / Codex
- Decision: Upload with pinned `@vercel/blob` 2.8.0 and pass `BLOB_READ_WRITE_TOKEN` directly to `put`, rather than using Vercel CLI.
  Rationale: The Blob SDK directly supports token-authenticated private multipart uploads; this avoids introducing or storing a broader Vercel account credential solely for CLI login.
  Date/Author: 2026-09-14 / Codex
- Decision: Handle the archive and checksum independently on reruns: verify each existing object against its regenerated local counterpart, upload only a missing object, and never overwrite or accept a mismatch.
  Rationale: Both v2 objects are now present, but sequential immutable uploads can leave a valid partial publication after a transient failure. Per-object recovery preserves immutable pathnames while allowing a later run to complete safely.
  Date/Author: 2026-09-14 / Codex

## Outcomes & Retrospective

The parser, regression coverage, artifact contract test, and CI publisher were merged in PR #236. The first pack run proved the source download, source checksum, preparation, corrected C4 mapping, audio properties, deterministic archive, and libgooey reload, and retained the candidate as a temporary Actions artifact. Its Vercel CLI upload client failed authentication, but both expected private v2 objects were subsequently published and independently verified. A follow-up changes the upload client to the Blob SDK and makes every existing object verification-only while uploading any missing counterpart; one green main-branch rerun remains.

## Context and Orientation

`src/instruments/multisample_pack.rs` parses SFZ text into private `Region` values and converts each region to a `SampleZone`. `SampleZone::root_key` controls transposition during playback. Today `Region::into_zone` chooses `pitch_keycenter` and then incorrectly falls back to `lokey`. `src/instruments/multisample_prep.rs` loads those zones, writes trimmed WAV files, and emits a new SFZ whose `pitch_keycenter` is the zone root; therefore the load-time error is baked permanently into prepared packs. `src/music/voicing.rs` independently turns a chord and voicing into MIDI notes and is the direct place to pin the requested Cmaj7 notes.

The command-line preparation entry point is `examples/prepare_piano_pack.rs`. Its mobile preset keeps eight velocity layers, caps each recording at six seconds, writes stereo 16-bit PCM, excludes release-triggered regions, and can embed Salamander's required CC-BY 3.0 attribution. `scripts/fetch-piano-pack.sh` identifies the source as Archive.org's `SalamanderGrandPianoV3_44.1khz16bit.tar.bz2`. The source archive is 488,713,261 bytes with SHA-1 `ec5a179a1218a590d2f88728f56b583e79876896` according to Archive.org metadata.

## Plan of Work

In `src/instruments/multisample_pack.rs`, introduce a private named constant for MIDI 60. Set `Region::new().pitch_keycenter` to that value and change `into_zone` to use the same value when the optional field is absent, never `lokey`. Add a parser-level test shaped like Salamander's C4 region and assertions that both the default and an explicit center behave correctly.

In `src/instruments/multisample_prep.rs`, add a focused temporary-pack test with one stereo WAV and an SFZ region whose range is 59 through 61 but whose center is omitted. Prepare it without thinning, assert that the emitted mapping and output filename explicitly identify root 60, reload the result through `load_sfz`, and assert the reloaded zone still has range 59 through 61 and root 60. In `src/music/voicing.rs`, add a direct root-position C-major-seventh assertion for `[60, 64, 67, 71]` at octave 4.

Add `.github/workflows/salamander-mobile-pack.yml` with `workflow_dispatch` only and read-only repository permissions. The job must exit unless `GITHUB_REF` is `refs/heads/main`, download and verify the original archive, extract it, run the release-mode mobile preparation command with Salamander attribution, and validate the prepared directory. Validation must establish 244 WAV zones, eight corrected C4 regions, no erroneous center-59 C4 regions, six-second-or-shorter stereo 16-bit audio, attribution, and successful reloading through libgooey. Package the existing `piano-mobile/` root deterministically, generate a conventional `.sha256` sidecar, and upload both immutable private objects through pinned `@vercel/blob` with private multipart upload, the Blob read/write token, no random suffix, and overwrite disabled. Query the uploaded object, download it with authentication, and require its digest to equal the sidecar before reporting success.

## Concrete Steps

Work from `/Users/pretzel/conductor/workspaces/libgooey/el-paso-v2`. Apply the source and workflow edits, then run:

    cargo fmt --all -- --check
    cargo test --lib --no-default-features --features bounce instruments::multisample_pack::tests
    cargo test --lib --no-default-features --features bounce instruments::multisample_prep::tests
    cargo test --lib --no-default-features --features bounce music::voicing::tests
    cargo test --test multisample_piano --no-default-features --features bounce

Before dispatching the merged workflow, copy the existing local Blob credential into the repository Actions secret without printing it. Dispatch `Salamander Mobile Pack` against `main`, monitor it to completion, and capture its summary. The upload must fail rather than overwrite if v2 unexpectedly already exists.

## Validation and Acceptance

The parser regression must fail on the original code because the parsed center is absent, and pass with center 60 after the fix. The preparation regression must fail on the original code because it emits `pitch_keycenter=59` and a `B3_059` filename; after the fix it must emit `pitch_keycenter=60`, a `C4_060` filename, and reload with root 60. The voicing regression must return exactly MIDI 60, 64, 67, and 71.

The CI artifact is accepted only if it contains `piano-mobile/instrument.sfz`, `piano-mobile/NOTICE.txt`, and 244 files under `piano-mobile/samples/`; the C4 range has eight velocity layers centered at MIDI 60; WAVs are stereo 16-bit and no longer than six seconds; and an authenticated download from the v2 Blob hashes to the exact digest published in `salamander-mobile-8x6s-v2.tar.gz.sha256`. On a retry, each existing Blob must match its regenerated local counterpart before the workflow may upload the other object.

## Idempotence and Recovery

Source changes and tests are repeatable. The preparation output directory is recreated on an ephemeral runner. The v2 Blob pathnames are intentionally immutable and uploads disable overwrites. A retry authenticates and verifies any existing archive or checksum against newly generated local bytes, then uploads only the missing counterpart. A mismatch or duplicate exact pathname stops the workflow without modifying Blob state. V1 is never modified.

## Artifacts and Notes

Expected corrected SFZ entries have this shape:

    <region> lokey=59 hikey=61 pitch_keycenter=60 ... sample=C4_060_...

The published objects are:

    libgooey/instruments/piano/salamander-mobile-8x6s-v2.tar.gz
    libgooey/instruments/piano/salamander-mobile-8x6s-v2.tar.gz.sha256

## Interfaces and Dependencies

No Rust or C public API changes are made. The only new operational interface is the manually dispatched GitHub Actions workflow. It uses the existing Rust stable toolchain, `curl`, GNU tar, gzip, standard checksum tools, Node.js, and pinned `@vercel/blob` 2.8.0. `BLOB_READ_WRITE_TOKEN` is supplied only as an Actions secret. The Blob store remains private, so verification downloads send the credential in an authorization header without logging it.
