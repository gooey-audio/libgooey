# Resolve PR #260 against main

This ExecPlan is a living document. Keep its Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective sections current as work proceeds. Follow `.agent/PLANS.md` in the repository root.

## Purpose / Big Picture

PR #260 lets a host change the strike strength of future piano chord-loop notes without replacing the loop or interrupting the sounding chord. It currently conflicts with `main`, which recently learned to hand a live chord strike to its recorded loop event without striking twice. After this merge, both behaviors must work, and GitHub must report https://github.com/gooey-audio/libgooey/pull/260 as mergeable.

## Progress

- [x] (2026-09-28 23:54Z) Inspect the PR, main, and merge conflict; confirm one manual conflict in `src/ffi.rs`.
- [x] (2026-09-28 23:54Z) Fetch both branches and create `.context/pr-260` worktree from the PR head.
- [x] (2026-09-28 23:56Z) Commit this plan, merge current `origin/main`, and resolve the trigger conflict.
- [x] (2026-09-28 23:56Z) Run formatting, focused iOS tests, the default test suite, and the generated C header check.
- [ ] Push the merge commit to the existing PR branch and confirm GitHub mergeability and CI.

## Surprises & Discoveries

- Git's three-way merge combines `src/performance/control.rs` and `tests/chord_loop_control.rs` automatically. Its only manual conflict is the `PlayerAction::Trigger` arm in `GooeyEngine::apply_performance_action` in `src/ffi.rs`.
- `main` changed the trigger arm after PR #260 branched. It now checks `pending_live_chord_handoff` before deciding whether to call `trigger_controlled_chord`.
- The merged code passes all requested local checks. The default suite reports 515 unit tests passed; the focused chord-loop integration suite reports 9 passed. The existing `tests/performance_recording.rs` code emits five unnecessary-`unsafe` warnings, but no test failures.

## Decision Log

- Decision: Update PR #260 in place with a regular merge commit, without force pushing. Rationale: The user chose the existing PR as the destination, and a merge retains both histories. Date/Author: 2026-09-28, Codex.
- Decision: Apply the piano velocity override only at the actual loop-trigger call after the live handoff checks. Rationale: A matching recorded onset must take ownership of the already sounding live chord without retriggering it. Date/Author: 2026-09-28, Codex.

## Outcomes & Retrospective

The manual conflict has been resolved without removing either behavior. Local formatting, Rust tests, and generated C header validation pass. Remote push and CI confirmation remain.

## Context and Orientation

The PR branch is `bhurlow/loop-piano-strike-control`; its target is `main`. `src/performance/control.rs` stores an optional atomic piano-loop velocity override. `src/ffi.rs` exports `gooey_engine_chord_loop_set_piano_velocity` and routes scheduled performance actions to instruments. `tests/chord_loop_control.rs` checks the public C-facing controls and live loop behavior. A loop snapshot is the stored set of chord events; changing the velocity override must not replace that snapshot or reset transport time.

`main` added a live handoff in `GooeyEngine::apply_performance_action`: when playback encounters the onset just recorded from a live pad strike, it marks that sounding chord as loop-owned and returns without a new note-on. The PR's velocity override belongs on the path that calls `trigger_controlled_chord`, after this handoff decision.

## Plan of Work

From the repository root, create this plan on the PR head in an isolated worktree and commit it. Merge `origin/main` into that branch. In `src/ffi.rs`, preserve the complete trigger arm from main, including its handoff and rescan returns, and change only the final trigger call to pass `self.chord_control.with_loop_piano_velocity(event)`. Keep the PR's public setter and its control-plane and integration tests. Stage the resolution, update this living plan, and commit the merge.

## Concrete Steps

The working directory for these commands is `/Users/pretzel/conductor/workspaces/libgooey/dublin-v2/.context/pr-260`. The worktree was created from `origin/bhurlow/loop-piano-strike-control` after fetching both branches.

    git add plans/pr-260-conflict-resolution-plan.md
    git commit -m "Document PR 260 conflict resolution"
    git merge origin/main

The merge should stop at one conflict in `src/ffi.rs`. Edit that conflict as described above, then run:

    git add src/ffi.rs plans/pr-260-conflict-resolution-plan.md
    git diff --cached --check
    git commit

Before pushing, fetch both remote branches again. If either head has advanced, inspect and incorporate the new commits before pushing. Push the local merge commit to the PR branch with `git push origin HEAD:bhurlow/loop-piano-strike-control`; never force push.

## Validation and Acceptance

Run these commands from the same worktree and expect each to exit successfully:

    cargo fmt --check
    cargo test --no-default-features --features ios loop_velocity_changes_only_future_piano_actions
    cargo test --no-default-features --features ios --test chord_loop_control
    cargo test
    clang -fsyntax-only -x c include/gooey.h

The Rust build script generates `include/gooey.h`; verify it contains `gooey_engine_chord_loop_set_piano_velocity`. The focused tests must show that a velocity change affects future piano loop triggers, leaves synth events alone, and does not replace the active chord or reset transport. Main's chord-loop wrap behavior must continue to pass. After the push, `gh pr view 260 --repo gooey-audio/libgooey --json mergeable,mergeStateStatus` should report the PR mergeable, and `gh pr checks 260 --repo gooey-audio/libgooey` should show passing checks.

## Idempotence and Recovery

The worktree keeps the Conductor workspace branch untouched. If the merge has not been committed and the resolution must restart, `git merge --abort` restores the PR head in this worktree. If the remote PR head moves before push, fetch it and merge its new commits into this local branch, revalidate, and retry the ordinary push. Do not reset or force push the remote branch.

## Artifacts and Notes

Before implementation, `git merge-tree` reports one manual conflict at `PlayerAction::Trigger` in `src/ffi.rs`. The other two modified PR files merge automatically. No Nexus task linked to PR #260 was found in the available task list.

Local verification completed at 2026-09-28 23:56Z:

    cargo fmt --check                                             passed
    cargo test --no-default-features --features ios loop_velocity_changes_only_future_piano_actions  1 passed
    cargo test --no-default-features --features ios --test chord_loop_control  9 passed
    cargo test                                                    515 unit tests passed; integration suites passed
    clang -fsyntax-only -x c include/gooey.h                      passed

The generated header includes `bool gooey_engine_chord_loop_set_piano_velocity(const struct GooeyEngine *engine, float velocity);`.

## Interfaces and Dependencies

Keep the PR's C ABI `gooey_engine_chord_loop_set_piano_velocity(engine: *const GooeyEngine, velocity: f32) -> bool`. It returns false for a null engine or non-finite velocity, clamps finite values to 0–1, and changes only future scheduled piano chord-loop strikes. It must not affect direct chord triggers, synth loop events, the sounding chord, or transport. No new dependency or migration is required.

Revision note (2026-09-28 23:56Z): Updated progress and evidence after resolving the sole merge conflict and completing local validation; remote verification is still pending.
