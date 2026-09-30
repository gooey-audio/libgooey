# Stack Loop Studio on the centralized graphical test application

This living plan follows `.agent/PLANS.md`. It is for the existing Loop Studio
branch, not the standalone prerequisite branch.

## Purpose / Big Picture

The user wants to debug libgooey through one graphical application, not maintain
separate GUI implementations for each experiment. A prerequisite branch,
`omni-gui-test-app`, is based on `main` and consolidates the existing synth,
resonator and waveform-equipped example interfaces. The existing Loop Studio
PR #267 will then become an extension panel of that application and depend on
the prerequisite. Users should open one app, switch experiments, and use the
same audio backend, scope, diagnostics and controls everywhere.

## Progress

- [x] (2026-09-29) Confirm #267 is open against main and source worktree is clean.
- [x] (2026-09-29) Create isolated native prerequisite worktree from origin/main,
  without using workmux or importing Loop Studio into the prerequisite.
- [x] (2026-09-29) Inventory standalone PolySynth/Resonator GUIs, common GLFW
  waveform renderer, and fourteen terminal examples with graphical paths.
- [x] (2026-09-30) Review public GuiPanel/PanelFactory, BlockRenderer,
  InterleavedAdapter, shared parameter/health and after-stop lifecycle contract.
- [x] (2026-09-30) Stack Loop Studio commits onto prerequisite a006112 and resolve additive
  Cargo/README/module conflicts without discarding any existing work.
- [x] (2026-09-30) Move Loop Studio into the central panel registry and remove its independent
  eframe application, run loop, CPAL callback and silent worker.
- [x] (2026-09-30) Reuse shared widgets, telemetry and diagnostics; preserve recording,
  automation, persistence and offline mixdown behavior.
- [x] (2026-09-30) Stacked automated verification: 563 library + 384 integration
  tests pass with native and no-native studio-gui, including all 24 studio tests,
  shared allocation probe and five new panel/lifecycle/recording/export tests.
  Studio-free GUI headless suites pass in both configurations (16 tests each).
  Dynamic 10s/panel/rate stress (80s audio) and 10s shared-worker soak pass.
- [ ] Parent independently verify prerequisite and capture combined graphical
  interactions/device output; publication and extended captures remain parent-owned.
- [ ] Capture central-app interactions, run extended verification, publish
  prerequisite and update #267 base/dependency and descriptions. Do not merge.

## Surprises & Discoveries

The prior UI work already has a real PR (#267), not merely a local branch or
unsubmitted creation form. It must be updated rather than replaced with an
unrelated new Loop Studio PR. Existing main contains two distinct full GUI
labs plus optional waveform windows in many CLI examples. Consolidation must
preserve their meaningful testing capabilities, not only put launcher buttons
in one window.

The original `examples/loop_studio.rs` owned its eframe shell, native CPAL stream,
fixed render scratch, silent render worker and parameter widgets. Keeping that
implementation beside a new app would not satisfy the user's request. The
engine-only `src/studio/mod.rs` and `studio_render` should remain device/window
independent; it is the GUI ownership and entrypoint that must move.

## Decision Log

Decision: keep the prerequisite independently reviewable against `main`, then
stack #267 on its branch. Rationale: the centralized lab is useful before the
larger loop-studio exploration lands. Date/author: 2026-09-29, parent agent.

Decision: retain old GUI example names only as thin central-app entrypoints.
Rationale: users can keep recognizable launch commands without perpetuating
separate UI/audio implementations. Date/author: 2026-09-29, parent agent.

Decision: preserve the headless session/renderer boundary and expose Loop Studio
as an optional central panel. Rationale: offline export and core clients must
not acquire GUI/audio-device dependencies. Date/author: 2026-09-29, parent agent.

## Outcomes & Retrospective

Loop Studio is rebased on prerequisite a006112 and now implemented in
`src/gui/studio.rs` behind `studio-gui = [studio, gui]`. Its old example is a
three-line central-shell launcher. No base-worktree source was edited. Existing
musical controls, file workflows and artifacts were retained; historical
standalone captures are labeled as such. Renderer mount preserves the Studio
constructed at the chosen host rate. Deactivation releases/finalizes/stops only
after host shutdown, and inactive UI cannot retrigger it. Prepared replacements
are constructed/reclaimed off callback, including rewind/demo/new. Publication
and captures remain the parent agent's responsibility; no commits/pushes/merges
have been made for the migration edits.

## Context and Orientation

The prerequisite lives at `/home/pretzel/code/libgooey-omni-gui` on branch
`omni-gui-test-app`. This branch lives at
`/home/pretzel/code/libgooey-loop-studio` on `loop-studio-engine`. They are native
Git worktrees of the same libgooey repository. `src/studio/mod.rs` supplies the
safe musical session and renderer. `src/gui/studio.rs` now contains the editor;
`examples/loop_studio.rs` selects it through the central entrypoint, and
`examples/studio_render.rs` remains the independent export/stress CLI.
The prerequisite's shared host is `src/gui/mod.rs` + `src/gui/audio.rs`, with
PolySynth, Resonator and Experiments editors in neighboring modules.

## Plan of Work

First review the prerequisite's actual public panel/factory and block-render
contract. Check that hidden panels cannot continue producing sound or leak
streams/workers, and that the device rate is used to construct each renderer.
The UI shell should publish compact health/scope information without requiring
another audio callback per panel.

After the prerequisite is committed, rebase the two existing Loop Studio
commits onto it. Resolve feature definitions by making `studio-gui` depend on
the centralized GUI feature plus `studio`, not an independent eframe wiring.
Extract musical UI state and drawing into a panel in the central GUI modules.
Keep only compatibility selection logic in `examples/loop_studio.rs`.

Use the central audio adapter for `Studio` and remove its previous AudioClock,
make_stream and native/silent selection code. Use shared parameter widgets and
show the shared diagnostics for studio output. Off-thread import/save/export
behavior and validated musical snapshots must be retained. Inactive-panel
behavior must be explicit: host stops output first, then finalize recording and
release held gestures, and synchronize panel selection with renderer
ownership without dropping DSP resources on the callback.

Finally validate the base application without studio, then the stacked app with
the Loop Studio panel. Update #267 to base on the prerequisite branch, explain
review/merge order and link both PRs. Actual screenshots/recordings must show
panel navigation and the shared chrome, not the old standalone studio window.

## Concrete Steps

Current central commands (in this stacked worktree):

    cargo run --release --features studio-gui --example omni_gui -- --panel "Loop Studio" --demo
    cargo run --no-default-features --features studio-gui --example omni_gui -- --panel "Loop Studio" --silent --load studio-session.json
    cargo run --no-default-features --features studio-gui --example omni_gui -- --stress 10
    cargo run --no-default-features --features studio-gui --example omni_gui -- --soak 10

`loop_studio` is a compatibility alias that defaults to Loop Studio; --panel can
still select any lab. --list/--help are central options. --load takes precedence
over --demo. GUI recording/export regression renders actual held gates, hits and
automation, deactivates, saves/reloads on a worker and verifies replay + stereo
48 kHz WAV energy. Five new tests cover that path, try-lock silence/rate/song
retention, every studio section/headless keyboard focus, real shell switching,
and pending rewind + held-key autorepeat across deactivate/remount. A lifecycle
epoch prevents an asynchronous rewind from restarting transport after switching
away and returning. Stress varies record/chords/hits and real control lanes.

Run commands in the matching worktree. Initial core verification remains:

    cargo test --release --no-default-features --features studio
    cargo run --release --no-default-features --features studio --example studio_render -- --bars 4 --export /tmp/opencode/stacked-studio.wav
    cargo fmt --check
    git diff --check

Headless GUI tests must cover shared ownership and input routing. Actual Linux
interaction uses a virtual X display and optional virtual audio sink, with
recorded screen/audio and independent WAV inspection. Existing server tooling
is installed; graphical services are recreated only as needed.

## Validation and Acceptance

The prerequisite app opens independently and exposes all migrated GUI labs with
real sounding/testable controls and common scope/health display. Old graphical
example entrypoints delegate to it. No subprocess launches of the old apps count
as consolidation. The combined app adds Loop Studio to the same registry,
without another eframe::App/run_native or audio/silent callback implementation.

Switching labs while keys are held or a sequence/recording is active must release
the previous gestures and stop its output. Repeated switching must not increase
stream/worker counts or panic/deadlock. The studio can still record automation
and hits/chords, save, load and export; a fresh headless export of its saved
session must agree with the GUI's mixdown. Existing engine tests remain passing.

## Idempotence and Recovery

Only the isolated feature branches are changed. Before rebasing, verify a clean
worktree or commit task-owned changes; preserve the published branch's prior
commit identity for a force-with-lease push. Never reset or discard unfamiliar
work. PRs remain unmerged so the prerequisite can be reviewed independently.

## Artifacts and Notes

Subagent verification, 2026-09-30 (all commands in this stacked worktree):

    cargo test --features studio-gui --lib --tests
    cargo test --no-default-features --features studio-gui --lib --tests
    cargo test --features gui --lib gui::
    cargo test --no-default-features --features gui --lib gui::
    cargo build --features studio-gui --example omni_gui --example loop_studio --example studio_render
    cargo clippy --features studio-gui --lib --example omni_gui --example loop_studio --example studio_render --test studio
    cargo clippy --no-default-features --features studio-gui --lib --example omni_gui --example loop_studio --example studio_render --test studio
    cargo fmt --check
    git diff --check

All succeeded. Normal Clippy reports 99 native / 98 no-native baseline warnings
outside GUI/studio; no new GUI/studio warnings. Two external-resource library
tests remain ignored. Final full test totals are 947 passing per configuration.
The final 10s worker soak processed 1384 blocks with no non-finite samples or
render failures. CLI --list reports four panels; --load of a generated session
followed by 1s/panel/rate stress passes. Headless one-bar demo export/save smoke
produced `/tmp/opencode/central-studio-smoke.{json,wav}`: 195310 stereo frames at
48 kHz, peak 0.4545, RMS 0.1280. These are automated observations, not new screen
captures or claims of physical-speaker audition. Native example binaries link;
no-native central and compatibility examples also ran the stress/soak commands.

Rebased preserved commits: b63fd13 → 68812f8, 4859150 → 6db044a,
9520695 → a256b5a, all on prerequisite a006112. Migration edits are uncommitted
as requested; `src/gui/studio.rs` is a new file to include in the parent commit.
Cargo regenerated the lockfile after removal of standalone eframe feature
wiring, pruning unused WGPU/legacy dependencies rather than updating DSP APIs.

Original verified artifacts remain in `docs/assets/loop-studio/` and
`/tmp/opencode/studio-evidence-verified/`. New central-app evidence must be
clearly labeled and must not imply that historical standalone recordings show
the consolidated interface. Final descriptions will include verification,
remaining compatibility constraints, shared-host tradeoffs and dependency links.

## Interfaces and Dependencies

The prerequisite must define one eframe shell, shared plotting/parameter/input
utilities and a block-render/audio lifecycle extension usable by Rust Engine
labs and the later `Studio`. The studio panel uses the actual existing safe
session API; a second synth or GUI-only mirrored engine is not acceptable.
Exact public names and migration commands will be recorded after the base
contract is reviewed rather than invented before it exists.

Revision 2026-09-29: initialize the stacked integration plan after the user
redirected work toward a prerequisite centralized GUI. Keep GUI consolidation
separate from the engine/session work and preserve the open #267 for migration.
