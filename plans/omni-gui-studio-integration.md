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
- [ ] Review prerequisite extension/audio ownership contract when implemented.
- [ ] Stack Loop Studio commits onto the prerequisite and resolve additive
  Cargo/README/module conflicts without discarding any existing work.
- [ ] Move Loop Studio into the central panel registry and remove its independent
  eframe application, run loop, CPAL callback and silent worker.
- [ ] Reuse shared widgets, telemetry and diagnostics; preserve recording,
  automation, persistence and offline mixdown behavior.
- [ ] Independently test prerequisite and combined application, repeated panel
  switches, keyboard focus, rendering, recording, save/load and final export.
- [ ] Capture central-app interactions, run extended verification, publish
  prerequisite and update #267 base/dependency and descriptions. Do not merge.

## Surprises & Discoveries

The prior UI work already has a real PR (#267), not merely a local branch or
unsubmitted creation form. It must be updated rather than replaced with an
unrelated new Loop Studio PR. Existing main contains two distinct full GUI
labs plus optional waveform windows in many CLI examples. Consolidation must
preserve their meaningful testing capabilities, not only put launcher buttons
in one window.

The current `examples/loop_studio.rs` owns its eframe shell, native CPAL stream,
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

Implementation of the independent prerequisite is underway. No new source has
yet been rebased or merged into this branch. Earlier Loop Studio functionality
and its verified artifacts remain available until the new host is ready.

## Context and Orientation

The prerequisite lives at `/home/pretzel/code/libgooey-omni-gui` on branch
`omni-gui-test-app`. This branch lives at
`/home/pretzel/code/libgooey-loop-studio` on `loop-studio-engine`. They are native
Git worktrees of the same libgooey repository. `src/studio/mod.rs` supplies the
safe musical session and renderer. `examples/loop_studio.rs` currently contains
all GUI logic; `examples/studio_render.rs` is the independent export/stress CLI.
Main's GUI sources are `examples/polysynth_gui.rs`,
`examples/resonator_voice_gui.rs`, and `src/visualization/`.

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
behavior must be explicit: finalize recording and release held gestures before
stopping/suspending output, and synchronize panel selection with renderer
ownership without dropping DSP resources on the callback.

Finally validate the base application without studio, then the stacked app with
the Loop Studio panel. Update #267 to base on the prerequisite branch, explain
review/merge order and link both PRs. Actual screenshots/recordings must show
panel navigation and the shared chrome, not the old standalone studio window.

## Concrete Steps

Exact app/feature names will be filled in from the implemented prerequisite.
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
