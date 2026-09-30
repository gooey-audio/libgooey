# Consolidate graphical experiments in one extensible application

This living execution plan follows `.agent/PLANS.md`. Work is restricted to the native Git worktree `/home/pretzel/code/libgooey-omni-gui`, branch `omni-gui-test-app`. Do not import Loop Studio or modify its separate worktree. The parent requests review before any commit, overriding the normal frequent-commit convention; this plan will be committed with the reviewed implementation.

## Purpose / Big Picture

Users should open `cargo run --example omni_gui --features gui` and switch between an expressive PolySynth, the editable resonator graph and four-track sequencer, and general engine experiments. All panels share one audio owner, stereo scope, spectrum and diagnostics. A future Loop Studio panel must mount in this shell without owning an application or audio thread.

## Progress

- [x] (2026-09-29) Read repository instructions and original lab entrypoints; resumed after cancelled clean run.
- [x] (2026-09-29) Implemented shared shell, block/interleaved/closure adapters, bounded telemetry, exclusive native/silent lifecycle and host-owned monotonic block timestamps.
- [x] (2026-09-29) Moved resonator topology, inspectors, routing, taps, patches and patterns into the central module; removed its independent audio/scope owners.
- [x] (2026-09-29) Ported all PolySynth parameters, editable presets, modulation fields and two-octave/chord keyboard to egui.
- [x] (2026-09-29) Added real instrument/effect/transport/LFO experiments and `docs/omni-gui.md` mapping every previous graphical path. Terminal graphical invocations forward to central capabilities.
- [x] (2026-09-29) Initial `cargo test --features gui --lib --tests` passed: 529 library tests and all integration suites. Twelve new GUI tests cover controls, routes, patches, lifecycle and headless layout.
- [x] (2026-09-29) Release, no-native accelerated stress passed 600 audio seconds for each of three panels at each of two rates: 103359 blocks per 44.1 kHz panel and 112500 per 48 kHz panel, 3600 aggregate audio seconds.
- [x] (2026-09-29) Final native and no-native GUI suites pass: 533 library tests plus 353 integration tests (886 total each), including sixteen GUI unit tests and the 10000-block allocation probe.
- [x] (2026-09-29) Final dynamic stress passes 600 seconds per panel/rate while changing presets, routes, inspectors, patterns, instruments and effects each simulated second; 3600 aggregate audio seconds on the final harness, in addition to the initial static run.
- [x] (2026-09-29) Wall-clock silent-worker soaks pass: initial 120 seconds / 120 auditions / 20243 blocks; final dynamic 180 seconds / 180 auditions / 30137 blocks, without non-finite audio or renderer failures.
- [x] (2026-09-29) Native GUI binaries, all compatible examples with gui/visualization/crossterm/bounce/midi, and no-default-feature ios builds pass. Format/diff checks pass. Normal Clippy passes with 99 pre-existing warnings, none in src/gui; strict -D warnings remains blocked by those unrelated baseline lints.
- [x] (2026-09-29) Attempted optional retired GLFW and unrelated plots compatibility builds: blocked respectively by missing cmake and fontconfig.pc in this environment. Unified GUI requires neither; record these gaps rather than claim success.
- [x] (2026-09-30) Re-ran final native/no-native 886-test suites and rebuilt all three GUI binaries after the final keyboard shortcut/focus guard; handoff is ready, with source and plan still uncommitted as requested.
- [x] (2026-09-30) Parent reviews and commits source; independently passes release native/no-native 886-test suites, all six dynamic 600-second stress cases, and 180-second worker/control soak (28014 blocks).
- [x] (2026-09-30) Capture actual native GUI/audio interaction across all labs, held-key switches, 24 repeated active panel switches and host stop/resume. Assert one actual stream after every switch; publish selected screenshots/video and reproducible script.
- [x] (2026-09-30) Commit/push standalone prerequisite branch `omni-gui-test-app`; open prefilled web PR creation with full description. Submission is not confirmed because browser automation is disconnected. Loop Studio integration is separately underway.

## Surprises & Discoveries

The resonator example already uses eframe, but owns an EngineOutput and a separate atomic scope. PolySynth uses a hand-rendered GLFW application. The visualization feature currently brings both UI frameworks into every graphical build.

The Rust Engine's sample path can allocate during sequence/LFO advancement, so an allocation-free host does not imply a hard-real-time engine. Audio adapters use a block-level try-lock and publish silence/count contention instead of waiting for GUI locks. The old resonator scope published each voice separately rather than measuring the final mixed stereo output; the common scope fixes that ambiguity.

The old hihat terminal example referred to removed fields and closed/open constructors even though HiHat is already a HiHat2 alias on main. All-examples compilation exposed fourteen stale-API errors. Its compatibility name now includes the working hihat2 terminal implementation rather than maintaining a second incompatible control source.

Recreating a render adapter at time zero would make returning panels' envelope timestamps go backward. The host now supplies `BlockRenderer::set_time` from one cumulative audio frame counter before each block; rate-dependent built-in adapters use it. Scope history is cleared on activation without resetting the audio clock or health counters.

## Decision Log

Decision: introduce an additive `gui` feature, with public `GuiPanel` and `BlockRenderer` interfaces. Keep native audio optional so tests need no sound device. Rationale: later panels should depend on shell facilities rather than duplicate them. Date: 2026-09-29.

Decision: keep old exported visualization APIs compatible during migration, explicitly mark their legacy status and remove independent lab entrypoint implementations. Rationale: engine consumers and the C ABI must not break silently. Date: 2026-09-29.

Decision: `visualization` now aliases `gui`; `legacy-visualization` explicitly enables the retired GLFW backend for external window-API consumers only. With visualization alone, `WaveformDisplay::new` gives an actionable migration error, not a hidden window or false success. Every repository example graphical entry forwards to the central shell regardless of the compatibility feature. Rationale: pumping a GLFW window cannot be transparently emulated by eframe without introducing another application owner, while external compatibility needs an explicit escape hatch. Date: 2026-09-29.

Decision: general effects are created/swapped on the GUI thread, with effect tail reset clearly labelled as a POC constraint. Generic instrument descriptors lack getters, so initial general-panel slider positions are labelled pending edits, not falsely reported factory values. Rationale: preserve engine APIs and make actual controls useful without claiming unsupported production automation semantics. Date: 2026-09-29.

## Outcomes & Retrospective

The standalone prerequisite is implemented, reviewed, committed and pushed. Parent independently repeated native/no-native 886-test suites, dynamic stress across 3600 synthesized seconds, and the 180-second worker/control soak, then recorded actual native GUI/audio operation and checked single-stream ownership during repeated panel switches. The public factory/panel/render adapters are documented for mounting Studio later without creating another application or worker. No Studio source is present in this prerequisite. The browser creation form is prepared but no PR number is yet confirmed. Remaining POC constraints are non-hard-real-time legacy Engine internals, effect-tail resets on edits, general descriptor interfaces without getters, and an explicitly isolated legacy window compatibility exception. Optional retired GLFW/plots builds require missing environment dependencies; strict lint cleanliness cannot be claimed for the unchanged baseline.

## Context and Orientation

`examples/polysynth_gui.rs` contains the GLFW synth lab. `examples/resonator_voice_gui.rs` contains eframe graph editing. `src/engine/mod.rs` provides the Engine sample renderer and instrument/effect ownership. `src/engine/engine_output.rs` contains the existing CPAL integration and optional standalone visualization used by terminal examples. `src/gui/` will own the single graphical shell. A block renderer fills an already allocated slice of stereo frames; a panel draws controls and supplies a renderer to the shell. CPAL is the native sound-device library; egui draws the immediate-mode controls inside eframe's desktop window.

## Plan of Work

First add `src/gui/audio.rs` for audio ownership and bounded atomic telemetry, and `src/gui/mod.rs` for the extension interface, shared controls and single entrypoint. Move resonator editing into `src/gui/resonator.rs`, preserving its editing functions while removing its independent application/audio ownership. Implement `src/gui/poly.rs` against existing PolySynth configuration APIs. Add `src/gui/experiments.rs` for real Engine instruments and effects. Replace old GUI examples with small selected-panel launchers and add `examples/omni_gui.rs`. Record migration semantics and custom mounting in `docs/omni-gui.md`.

## Concrete Steps

Run commands in `/home/pretzel/code/libgooey-omni-gui` using `/home/pretzel/.cargo/bin/cargo`. Successful final commands are `cargo build --features gui --example omni_gui --example polysynth_gui --example resonator_voice_gui`, `cargo check --examples --features gui,visualization,crossterm,bounce,midi`, `cargo check --no-default-features --features ios`, `cargo test --features gui --lib --tests --quiet`, `cargo test --no-default-features --features gui --lib --tests --quiet`, `cargo clippy --features gui --lib --example omni_gui`, `cargo fmt --all -- --check`, and `git diff --check`. Device-free long-run commands are `cargo run --release --no-default-features --features gui --example omni_gui -- --stress 600` and the same command with `--soak 180`. Launch the GUI with `cargo run --example omni_gui --features gui -- --silent`; select PolySynth, Resonator or Experiments. `target/debug/examples/omni_gui --list` printed exactly those three names. Optional `cargo check --features legacy-visualization --lib` failed because cmake is missing; including plots in the all-examples build failed because fontconfig.pc is missing. Do not mistake those environment failures for successful compatibility verification.

## Validation and Acceptance

The application should retain all named PolySynth controls and modulation routes, release keyboard notes when focus is lost, and stop inactive sequencers on panel switches. Resonator topology and inspectors must edit audible synthesis, not placeholders. One shared stereo scope and spectrum must reflect the selected renderer. Device failures must be visible and fall back to explicitly labelled silent rendering. Test non-finite sanitization, channel conversion, bounded scope history, lifecycle switches and egui layout without a display. Parent will independently capture Linux GUI interactions.

## Idempotence and Recovery

Build and test commands can be repeated. No resets, broad deletion, pushes or commits are authorized before review. Existing engine and FFI interfaces remain untouched unless compatibility is verified. Unfinished work must remain clearly marked in this plan.

## Artifacts and Notes

Final evidence logs are `/tmp/opencode/omni-dynamic-stress-600.log`, `/tmp/opencode/omni-dynamic-soak-180.log`, `/tmp/opencode/omni-native-tests.log`, `/tmp/opencode/omni-no-native-tests.log`, `/tmp/opencode/omni-examples-build.log`, `/tmp/opencode/omni-native-gui-build.log` and `/tmp/opencode/omni-clippy.log`. Earlier static/soak evidence remains in `/tmp/opencode/omni-stress-600.log` and `/tmp/opencode/omni-soak-120.log`. The failed retired-backend build is recorded in `/tmp/opencode/omni-legacy-build.log`. Logs are environment-local evidence, not credentials or repository dependencies.

## Interfaces and Dependencies

Exposed `gooey::gui::{GuiPanel, PanelFactory, BlockRenderer, RenderAdapter, InterleavedAdapter, EngineRenderer, GuiAudio, OmniApp, run, run_with, parameter, Keyboard, diagnostics, stress, soak}`. `GuiPanel` supplies a stable name, draws egui controls and returns a render adapter; `BlockRenderer::render(&mut self, &mut [StereoFrame], f32)` fills stereo blocks, and `set_time(f64)` optionally consumes the host-owned monotonic time. The shell owns the only CPAL stream or silent worker, scope and health view. Use eframe 0.29 and rustfft already present in Cargo.toml. Never require Studio types in this prerequisite.

Revision note: created on resume to make the pending implementation restartable; commits intentionally await parent review.

Revision note: recorded substantive implementation, compatibility decisions, clock discovery and actual test/stress evidence; final soak and compatibility checks remain explicit rather than presumed complete.

Revision note: finalized the handoff with actual native/no-native counts, dynamic long-run and real-worker evidence, baseline lint limitations, stale HiHat compatibility fix, and precise missing-dependency gaps. No commit was made because parent review is required first.

Revision note: parent completes independent review/testing and actual native GUI/audio captures, commits/pushes the prerequisite source/evidence, and repeats dynamic long-run/worker soaks. Distinguish prepared web PR form from confirmed submission. Later Studio integration stays on its existing branch and is not imported into the standalone prerequisite.
