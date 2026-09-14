# Build a macOS Rust VST3 Kick Proof of Concept

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. This document is maintained according to `.agent/PLANS.md` in the repository root.

## Purpose / Big Picture

This work proves that libgooey can be developed and distributed as a native Rust audio plug-in without placing Swift or an Audio Unit build between the DSP and the host. After the change, an Apple Silicon Mac user can run a small standalone application to audition libgooey's existing kick synthesizer and can build, validate, and optionally install a VST3 instrument named “Gooey Kick POC.” The plug-in exposes seven automatable controls, accepts MIDI note-on events, restores saved state, and displays the same fixed-size egui control surface used by the standalone application.

The proof is observable in three ways. `cargo run -p gooey-kick-xtask -- bundle-vst3` produces `target/bundled/Gooey Kick POC.vst3`; `cargo run -p gooey-kick-xtask -- validate-vst3` opens and tests that bundle with pluginval strictness 5 including GUI tests; and `cargo run -p gooey-kick-vst3 --features standalone --bin gooey-kick-standalone` opens an editor whose Audition button and Space key synthesize audio.

## Progress

- [x] (2026-09-14 00:51Z) Inspected the repository architecture, libgooey kick API, VST3 SDK bindings, egui-baseview API, host tooling, and baseline tests.
- [x] (2026-09-14 00:51Z) Created this self-contained execution plan before implementation.
- [x] (2026-09-14 01:05Z) Converted the root manifest into a workspace and added the plug-in and xtask packages.
- [x] (2026-09-14 01:13Z) Implemented and tested normalized parameters, atomic state decoding, exact-offset scheduling, and the allocation-free kick adapter.
- [x] (2026-09-14 01:27Z) Implemented and tested the VST3 processor, edit controller, `IPluginFactory2`, state, automation, buses, fixed egui editor, and lowercase macOS entry points.
- [x] (2026-09-14 01:32Z) Implemented the standalone CPAL audio path and shared egui knob layout with Audition and Space triggers.
- [x] (2026-09-14 01:37Z) Implemented bundle assembly, architecture/symbol inspection, signing, safe user installation, checksum-pinned pluginval validation, documentation, and Conductor actions.
- [x] (2026-09-14 01:45Z) Ran formatting, all default and no-default libgooey tests, new strict Clippy checks, standalone compilation/launch, bundle inspection, signature verification, and pluginval strictness 5 with GUI tests.
- [x] (2026-09-14 01:46Z) Recorded final evidence, environmental limitations, decisions, and outcomes in this plan.
- [x] (2026-09-14 19:36Z) Investigated PR review and hands-on standalone feedback about stale first-hit parameters, jittering knobs, and the macOS no-input beep.
- [x] (2026-09-14 19:45Z) Snapped bulk DSP parameter loads, changed knobs to cumulative drag distance, and scoped keyboard capture to standalone Space input with regression tests.
- [x] (2026-09-14 19:59Z) Re-ran formatting, focused and repository-wide tests, no-default-feature tests, strict plug-in Clippy, standalone build/window launch, and pluginval strictness 5.
- [ ] Complete a post-fix human interaction pass: drag every knob, listen to subsequent hits, and confirm Space no longer produces the macOS no-input beep.

## Surprises & Discoveries

- Observation: The development environment does not currently contain `rg`, so repository searches use `grep` and `find` as the documented fallback.
  Evidence: the initial source-inspection command returned `zsh: command not found: rg`.

- Observation: `KickConfig` provides a named inherent `default()` function rather than an implementation of Rust's `Default` trait.
  Evidence: `src/instruments/kick.rs` defines `pub fn default() -> Self { Self::tight() }` inside `impl KickConfig`.

- Observation: pluginval 1.0.4 accepts a separately implemented processor/controller pair and the baseview NSView editor without compatibility workarounds.
  Evidence: strictness 5 completed `Open editor whilst processing`, `Plugin state`, `Automation`, `Editor Automation`, and all bus tests before reporting `SUCCESS`.

- Observation: repository-wide strict Clippy is not currently a clean baseline, independently of this work.
  Evidence: `cargo clippy --workspace --all-targets --all-features -- -D warnings` reached existing `gooey` sources and reported about 60 lints such as `should_implement_trait` in `src/instruments/kick.rs` and missing safety documentation in `src/ffi.rs`. Both new packages pass strict Clippy with `--no-deps`.

- Observation: checking every pre-existing root feature needs an external CMake installation for `glfw-sys`; CMake is absent on this machine.
  Evidence: `cargo check --workspace --all-targets --all-features` stopped in the `glfw-sys` build script with `is cmake not installed?`. This does not affect the VST3 or standalone targets, which compile and validate.

- Observation: `KickAdapter::apply_all_parameters` originally reused real-time setters that changed only `SmoothedParam` targets, but `KickDrum::trigger_with_velocity` snapshots current pitch-envelope values before any sample can advance the smoothers.
  Evidence: a regression that inspects the current config immediately after construction, state replacement, and sample-rate reconstruction fails without a final `KickDrum::snap_params()` call and passes with it.

- Observation: egui 0.36 defines `Response::drag_delta()` as movement since the last frame and `Response::total_drag_delta()` as movement since the drag started.
  Evidence: combining a fixed gesture-start value with per-frame delta repeatedly pulled knobs back toward their starting point; cumulative-drag tests now stay monotonic in both directions.

- Observation: egui-baseview returns ignored for keyboard events unless egui has a focused widget, and baseview's macOS view forwards ignored key events to AppKit's superclass implementation.
  Evidence: Space still triggered the egui action but then reached AppKit and produced the system no-input beep. The standalone now keeps a focus target and captures only the keyboard-types Space key.

- Observation: the first repository-wide validation attempt exhausted the nearly full host volume while Cargo was linking test binaries.
  Evidence: the linker returned `errno=28`; cleaning only this workspace's generated `target/` and rerunning with `CARGO_INCREMENTAL=0` completed the same test matrix successfully.

## Decision Log

- Decision: Keep the existing root `gooey` package as a workspace member rather than moving its source into a new subdirectory.
  Rationale: This preserves all current paths and public Rust/C interfaces while allowing the new packages to share one lockfile and target directory.
  Date/Author: 2026-09-14 / Codex.

- Decision: The hosted plug-in depends on `gooey` with `default-features = false`, while CPAL is an optional dependency enabled only by the standalone binary.
  Rationale: A VST3 host owns its audio device. Preventing CPAL from entering the hosted library avoids device initialization conflicts and unnecessary platform linkage.
  Date/Author: 2026-09-14 / Codex.

- Decision: Processor and controller exchange automation only through VST3 parameter queues and state streams; they never share a Rust object or DSP pointer.
  Rationale: This follows the VST3 threading and lifecycle model and keeps editor work off the real-time audio callback.
  Date/Author: 2026-09-14 / Codex.

- Decision: Persist a fixed `GKST` magic, little-endian version number 1, and seven little-endian `f32` normalized values.
  Rationale: A fixed-size payload is simple to validate atomically, stable across compiler versions, and sufficient for normalized host parameters.
  Date/Author: 2026-09-14 / Codex.

- Decision: Walk host automation and event queues directly for each output frame instead of copying them into a bounded scratch collection.
  Rationale: This applies an arbitrary number of host points at exact offsets without allocation, locks, truncation, or a fixed-capacity overflow policy in the audio callback. The extra comparisons are acceptable for a seven-parameter proof of concept.
  Date/Author: 2026-09-14 / Codex.

- Decision: Report a four-second VST3 tail computed at the active sample rate.
  Rationale: Four seconds is the maximum exposed kick decay. A finite tail lets a host stop processing the one-shot instead of treating the instrument as an infinite generator.
  Date/Author: 2026-09-14 / Codex.

- Decision: Use recoverable staging and backup renames for user installation, and do not install as part of validation.
  Rationale: Installation stays explicit and a failed replacement restores the previous bundle; ordinary build and validation only mutate ignored paths under `target/`.
  Date/Author: 2026-09-14 / Codex.

- Decision: Snap the kick's smoothers only after the adapter's bulk parameter loop, not after individual parameter updates.
  Rationale: Construction, state restoration, and sample-rate reconstruction must be immediately coherent before the first trigger, while automation and live knob changes still need click-free smoothing.
  Date/Author: 2026-09-14 / Codex.

- Decision: Keep the fixed value captured at drag start and combine it with egui's cumulative vertical drag distance.
  Rationale: This preserves the existing sensitivity and host gesture lifecycle while preventing per-frame pointer deltas from resetting the displayed value toward its start.
  Date/Author: 2026-09-14 / Codex.

- Decision: Capture only Space in the standalone editor and ignore all keys in the embedded editor, with a standalone-only focusable keyboard hint.
  Rationale: Space must remain inside the standalone to suppress AppKit's beep, while a hosted editor has no keyboard commands and should leave DAW shortcuts available.
  Date/Author: 2026-09-14 / Codex.

## Outcomes & Retrospective

The proof of concept meets its automated acceptance target. The root is a workspace, the hosted VST3 links libgooey without its CPAL-backed default feature, and the standalone enables CPAL separately. Seven stable parameters, versioned atomic state, MIDI velocity/retrigger behavior, exact-offset host automation, safe sample-rate rebuilds, finite stereo output, separate VST3 processor/controller objects, host-mediated editor gestures, and fixed 560 by 300 egui knobs are implemented. libgooey's public Rust and C interfaces were not changed.

`target/bundled/Gooey Kick POC.vst3` is an arm64 Mach-O bundle with the intended property-list metadata, all three entry symbols, a valid ad-hoc signature, and module metadata. pluginval 1.0.4 passed strictness 5 including GUI, processing, state, automation, and bus tests. The standalone launched and remained running with its window and CPAL stream until deliberately interrupted.

The PR follow-up makes every bulk-loaded parameter current before the next hit without weakening live smoothing. The shared editor now uses cumulative drag distance, standalone Space input is captured instead of forwarded to AppKit, and embedded editors explicitly pass keyboard input through. Twenty-three plug-in tests, the full workspace and no-default-feature matrices, strict plug-in Clippy, and a fresh pluginval strictness-5 run pass after the changes.

Two limitations remain environmental or manual rather than implementation failures. No human listening confirmation was possible from the coding session, so a user should still press Audition and Space and turn every knob using the Conductor action. The repository's unrelated visualization feature needs CMake, and the existing root crate has strict-Clippy debt; all new targets pass their own strict Clippy checks and the complete normal test matrix passes.

## Context and Orientation

The repository root is both the existing Rust package `gooey` and, after this change, the Cargo workspace root. Its DSP implementation remains in `src/`. `src/instruments/kick.rs` defines `KickConfig` and `KickDrum`; the plug-in consumes these public types without modifying them.

The new package `plugins/gooey-kick-vst3/` contains all product-specific code. `src/params.rs` defines stable parameter IDs, display conversion, defaults, an atomic parameter mirror, and the binary state format. `src/dsp.rs` adapts `KickDrum` to host sample frames and keeps time as a 64-bit sample counter converted to seconds. `src/schedule.rs` merges MIDI and parameter changes at exact sample offsets without heap allocation. `src/editor.rs` contains the reusable egui layout. `src/vst3_plugin.rs` contains VST3 COM objects and entry points. `src/bin/standalone.rs` owns the CPAL stream and opens the same editor with standalone-only audition controls.

A VST3 has two logical halves. The processor runs on the host's real-time audio thread and turns MIDI plus parameters into samples. The edit controller describes parameters and owns the graphical editor. COM is VST3's reference-counted binary interface; the `vst3` crate supplies its raw Rust declarations and wrappers. The host mediates every gesture by receiving `beginEdit`, one or more `performEdit` calls, and `endEdit`, then delivering resulting values to the processor. A VST3 bundle is a macOS directory with an `Info.plist` and executable library arranged under `Contents/`.

The new package `xtask/`, published locally as `gooey-kick-xtask`, automates release compilation and packaging. It also downloads the universal pluginval 1.0.4 archive only on demand into ignored `target/tools/`, verifies SHA-256 `3c4c533bda0c5059eea3ddaea752d757ee2025041f0f47e6bcb0e87f6082b29f`, and runs strictness level 5 without disabling GUI tests. `.conductor/settings.toml` exposes local, nonconcurrent actions for the standalone app and validator; workspace setup performs no network download.

## Plan of Work

First, add a workspace stanza to `Cargo.toml` and create the plug-in and xtask manifests. The hosted library must never enable gooey's default `native` feature. The standalone feature alone enables CPAL. Keep macOS-specific binaries and exported entry points behind target configuration so the workspace remains diagnosable on another platform even though shipping support is arm64 macOS only.

Second, implement the independent core before COM. Seven normalized parameters derive their defaults from `KickConfig::default()`. Validate values as finite and clamp them at every external boundary. Decode the complete state payload into a temporary array before committing any value, so truncated, non-finite, or unknown-version data cannot partially mutate live state. The DSP adapter owns one `KickDrum`, reconstructs it when sample rate changes, reapplies all targets, advances an integer sample counter, and replaces a non-finite generated value with zero. Parameter 1 drives both oscillator and amplitude decay. Rendering writes the mono sample identically to left and right channels.

Third, implement processor and controller COM classes and the factory. The processor declares no input bus, one stereo output, one 16-channel event input, and only 32-bit processing. In each process block, apply every automation point and note-on at its exact sample offset; pitch and note-offs have no effect. Accept a parameter-only flush containing no audio buffers. Reject invalid bus arrangements, sample sizes, and sample rates. Track output silence flags from actual rendered samples. Implement state streams on the processor and component-state loading on the controller. The controller exposes all seven automatable parameters and creates a fixed 560 by 300 NSView-compatible plug view. The view uses baseview and egui-baseview; knob gesture boundaries call the three VST3 edit methods and controller `setParamNormalized` refreshes atomic editor values.

Fourth, implement the standalone binary around the same DSP adapter and editor layout. A lock-free atomic snapshot communicates controls to the CPAL callback. A monotonically increasing atomic audition counter communicates button and Space presses. The callback notices changes, triggers a full-velocity one-shot, and duplicates the mono signal to every configured device channel. Device and stream errors appear as actionable stderr output.

For the review follow-up, finish every bulk adapter parameter application by snapping the kick's current smoother values to those targets. Leave single-parameter updates on the smoothed path. Derive knob values from the value saved at drag start and egui's total vertical drag distance. In standalone mode, configure egui-baseview to capture the keyboard-types Space key and keep a focusable keyboard-hint response active when nothing else has focus; configure embedded mode to ignore keys so the host retains its shortcuts.

Fifth, implement idempotent packaging in xtask. Build the plug-in library in release mode, stage the macOS bundle hierarchy, write the required property list, copy and rename the dynamic library, and ad-hoc sign it with `/usr/bin/codesign`. The install subcommand writes only beneath the current user's VST3 directory and refuses to replace an existing bundle whose identifier is not `audio.gooey.kick-poc` unless `--force` is supplied. Validation assembles a fresh bundle, downloads and verifies pluginval when absent, unpacks it, and invokes `--validate` with `--strictness-level 5` while leaving GUI tests enabled.

Finally, add repository documentation and the two Conductor actions, run the acceptance commands, inspect the produced executable architecture and symbol table, and update this document with concise evidence.

## Concrete Steps

Run every command from `/Users/brianhurlow/conductor/workspaces/libgooey/seattle`.

Format and run the Rust tests:

    cargo fmt --all -- --check
    cargo test --workspace --all-targets
    cargo test -p gooey --no-default-features --lib --tests
    cargo clippy --workspace --all-targets --all-features -- -D warnings

Compile the standalone application without launching it, then launch it for manual audition:

    cargo build -p gooey-kick-vst3 --features standalone --bin gooey-kick-standalone
    cargo run -p gooey-kick-vst3 --features standalone --bin gooey-kick-standalone

Build and inspect the VST3 bundle:

    cargo run -p gooey-kick-xtask -- bundle-vst3
    plutil -lint 'target/bundled/Gooey Kick POC.vst3/Contents/Info.plist'
    file 'target/bundled/Gooey Kick POC.vst3/Contents/MacOS/Gooey Kick POC'
    nm -gU 'target/bundled/Gooey Kick POC.vst3/Contents/MacOS/Gooey Kick POC'
    codesign --verify --deep --strict --verbose=2 'target/bundled/Gooey Kick POC.vst3'

The `file` output must say `arm64`. The symbol output must contain `GetPluginFactory`, `bundleEntry`, and `bundleExit`. Codesign must report that the bundle satisfies its designated requirement.

Validate the complete host-facing result:

    cargo run -p gooey-kick-xtask -- validate-vst3

The first run may download pluginval into `target/tools/`. It must report the expected checksum, open the editor during GUI coverage, and exit with status zero at strictness level 5.

## Validation and Acceptance

Unit tests must demonstrate IDs 0 through 6, defaults derived from the kick config, exact display mappings, finite clamping, byte-for-byte state round trips, and atomic rejection of malformed state. Scheduling tests must put automation and note-on events at offsets 0, interior frames, and the last frame and observe changes only from those frames onward.

DSP tests must observe silence before a trigger and finite nonzero energy afterward at 44,100, 48,000, and 96,000 Hz. They must observe identical stereo channels, lower energy for lower velocity, audible changes from parameters, and a fresh transient after retriggering. COM tests must enumerate exactly the processor and controller classes, create and query their interfaces, inspect buses and parameters, and release every reference without leaks or crashes.

Review regressions must also prove that construction, state replacement, and sample-rate reconstruction expose the requested current parameter values before rendering any sample, while an individual parameter update still takes more than one sample to reach its target. Editor tests must prove cumulative upward and downward drag calculations are monotonic and clamp to 0–1, standalone capture includes only Space, and embedded capture ignores keyboard input.

The produced bundle passes the hierarchy, property-list, arm64, exported-symbol, and ad-hoc signature checks in Concrete Steps. Pluginval exits zero at strictness 5 with GUI tests. Manual acceptance launches the “Kick Standalone” Conductor action, clicks Audition and presses Space, hears a kick for both gestures, and confirms every knob affects subsequent hits. Manual listening is explicitly recorded rather than inferred from compilation.

## Idempotence and Recovery

Cargo builds, bundle assembly, pluginval download validation, signing, and Conductor actions are repeatable. xtask stages only known paths under `target/`; a failed bundle command can be rerun without touching installed plug-ins. If a cached archive has the wrong checksum, validation removes that archive and reports the mismatch rather than executing it. Installation is never implicit. If the user VST3 directory contains a same-name unrelated bundle, the command stops before changing it; after verifying the destination manually, rerun with `install-vst3 --force` only when replacement is intended.

## Artifacts and Notes

The initial no-default-feature baseline completed successfully before modifications:

    cargo test --no-default-features --lib --tests
    test result: ok. 385 passed; 0 failed

Final build and pluginval evidence:

    cargo test --workspace --all-targets
    test result: ok (385 lib tests, every integration suite, 23 plug-in tests, 2 xtask tests)

    cargo test -p gooey --no-default-features --lib --tests
    test result: ok (385 lib tests and every integration suite)

    cargo clippy -p gooey-kick-vst3 --all-targets --all-features --no-deps -- -D warnings
    Finished dev profile; no warnings

    file target/bundled/.../Gooey Kick POC
    Mach-O 64-bit dynamically linked shared library arm64

    nm -gU target/bundled/.../Gooey Kick POC
    T _GetPluginFactory
    T _bundleEntry
    T _bundleExit

    codesign --verify --deep --strict --verbose=2 target/bundled/Gooey\ Kick\ POC.vst3
    valid on disk; satisfies its Designated Requirement

    cargo run -p gooey-kick-xtask -- validate-vst3
    Strictness level: 5
    Reported taillength: 4
    SUCCESS

    cargo build -p gooey-kick-vst3 --features standalone --bin gooey-kick-standalone
    Finished dev profile; launching the binary created a Gooey Kick POC window

## Interfaces and Dependencies

The plug-in package uses `vst3 = "0.3.0"` for Steinberg-compatible COM declarations, `baseview = "0.3.4"` for child and standalone native windows, and `egui-baseview = "0.7.1"` for the OpenGL-backed egui integration. CPAL is optional and present only in the `standalone` feature. `gooey = { path = "../..", default-features = false }` is mandatory.

`params::ParamId` is a stable `u32` mapping with seven entries. `params::Parameters` owns `[f32; 7]` and provides atomic state validation. `dsp::KickAdapter` provides `new(sample_rate, values)`, `set_sample_rate`, `set_parameter`, `trigger`, and `next_sample`. It performs no allocation or locking in `next_sample`.

The processor implements `IPluginBase`, `IComponent`, and `IAudioProcessor`. The controller implements `IPluginBase`, `IEditController`, and creates an `IPlugView`. The exported factory implements `IPluginFactory2`, enumerates the processor first and controller second, and uses the exact class IDs `7D29F218-35C8-4BDF-A7E7-D903B4896721` and `D9F6B809-64F2-4367-98CC-1A271CD103B2`. macOS exports exact C symbols `bundleEntry`, `bundleExit`, and `GetPluginFactory`.

Revision note (2026-09-14 00:51Z): Created the initial implementation-ready plan after local source and dependency research so subsequent work and validation can be resumed from this file alone.

Revision note (2026-09-14 01:46Z): Marked implementation milestones complete, recorded final architecture decisions, added build and pluginval evidence, and distinguished automated success from the remaining human listening check and unrelated root-tooling limitations.

Revision note (2026-09-14 19:59Z): Addressed PR review and hands-on standalone feedback by documenting immediate bulk parameter snapping, cumulative knob dragging, scoped Space capture, new regression coverage, successful revalidation, and the remaining post-fix human interaction check.
