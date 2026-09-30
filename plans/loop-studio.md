# Build a reusable loop studio and playable desktop interface

This living execution plan follows `.agent/PLANS.md`.

## Purpose / Big Picture

Users will edit and mix a four-track song, play and record chord pads and drum/bass gestures, capture parameter movements, save the musical session, and export a stereo WAV without requiring an audio device. A real Rust desktop interface will expose the same safe session API as automated rendering.

## Progress

- [x] (2026-09-29) Inspect existing mixer, sequencer, loop player and performance recorder APIs.
- [x] (2026-09-29) Implement validated session persistence, safe engine ownership, automation and offline rendering in `src/studio/mod.rs`.
- [x] (2026-09-29) Implement eframe interface, CPAL playback and device-free clock fallback in `examples/loop_studio.rs`, plus headless export/stress in `examples/studio_render.rs`.
- [x] (2026-09-29) Add 15 studio integration tests, run release builds/full tests, formatting and lint; document behavior and limits in `docs/loop-studio.md` and README.
- [x] (2026-09-29) Independently exercise/record the native GUI, fix a discovered overdub export blocker, repeat Save/Export/Load/replay, and verify GUI/headless WAV byte identity. Artifacts and commands are in `docs/loop-studio-verification.md`.
- [x] (2026-09-29) Complete final 100,000-block renderer stress, 32-bar WAV inspection, 600-second native live soak, silent-mode record/import/Pluck audition, and source/evidence commit on `loop-studio-engine`.
- [x] (2026-09-29) Prepare complete final PR description at `/tmp/opencode/loop-studio-pr-description.md`, publish source/artifacts on `loop-studio-engine`, and prepare the required browser-creation workflow. Browser automation is disconnected in this tmux session, so opening the form is not proof that a PR was submitted. No main-branch merge.
- [x] (2026-09-29) Follow-up: fix capture-reproduced overlapping overdub gates using canonical last-take-wins clips installed into live replay; add demo overlap/same-tick/wrap save/export/reload regressions.
- [x] (2026-09-29) Follow-up: make Play/Stop idempotent with explicit bar-zero restart, re-cue tempo changes, integer-frame cursor and bar-local legacy trigger clocks; bound WAV headers and actual duration before reading samples; use portable test temporary paths.
- [x] (2026-09-29) Expanded suite passes: 24 studio tests and 927 tests across all suites/doc-tests; formatting passes. Parent rerecording/final review remains pending.

## Surprises & Discoveries

The FFI engine allocation includes a lifecycle header; it cannot safely be recovered with Box::from_raw on the returned engine pointer. The safe studio owner must free it with the existing matching FFI function. The engine already provides atomic chord-loop replacement and sample-accurate chord recording, avoiding another synthesizer or chord player.

The queued initial chord snapshot disarms the existing recorder when installed. Evidence: the first recording integration test failed its `chord_recording()` assertion when armed before the first render. Construction now renders one stopped frame to install queued configuration before exposing the studio to the host; the test passes.

Chord replay recalls its preset on every note-on. A synth-tone change must update the stored preset copies, not just the currently sounding voice. Evidence: `studio_chord_tone_survives_preset_recall` now renders distinctly dark and bright versions of the same chord.

The repository has existing Clippy warnings unrelated to this feature. `cargo clippy ... -- -D warnings` failed with 99 diagnostics before local warnings were fixed; ordinary Clippy completes, and JSON diagnostics are checked for new studio paths rather than suppressing baseline library warnings.

Independent capture found that recording held chords over the demo could create cyclic overlapping events, causing both Save and Export to fail validation. The underlying recorder retains insertion order and only cuts gates covering note-on; later-starting events under a new held interval can survive. Studio canonicalization now assigns each of 384 ticks to the latest finalized take, retains uncovered older fragments, joins seam-crossing fragments, and installs the resulting clip into live replay after recording ends. No new take is dropped to hide the invalidity. Regressions reproduce original-start crossings, same-tick duplicates and wrap gates, then save, export, reload and compare the live installed clip.

The legacy sequencer's start resets its next trigger to its current sample count, so it cannot resume a paused cursor without an immediate trigger. Its f32 scheduling also accumulates rounding error over long runs. Studio Play now re-cues at bar zero and is idempotent; tempo changes re-cue, frame counts determine the cursor exactly within floating-point conversion precision, and each bar bounds the legacy counters. A 1001-bar fractional-step regression uses 237 BPM at 8 kHz, not an integral step length that would mask this error.

The new two-bar PCM regression exposed unnecessary WSOLA correlation ambiguity on constant samples at equal source/song tempo. The studio now uses direct playback at equal tempos, pitch-preserving stretch only when tempos differ, and explicitly restarts the legacy PCM channel on Play after Stop. Grid seeks alone do not restart legacy loop channels.

## Decision Log

Decision: add optional `studio` and `studio-gui` features. Rationale: safe session/headless code should not require a display, CPAL, or GLFW. Date/Author: 2026-09-29, implementation agent.

Decision: use a bounded four-track proof of concept rather than promise an unlimited DAW. Rationale: existing sources represent one drum kit, one bass synth, one poly synth and a loop submix; these can be exposed accurately without altering C ABI source identifiers. Date/Author: 2026-09-29, implementation agent.

Decision: make session validation reject cyclic overlapping chord gates and duplicate/unordered automation lanes before allocating an engine. Rationale: persisted songs must be acceptable to the existing immutable chord-loop replacement API and deterministic parameter player. Date/Author: 2026-09-29, implementation agent.

Decision: control recording is 96-tick step-held overdub with tick-zero baseline; tempo is editable but not recorded. Rationale: exact held automation maps to existing smoothing without introducing another interpolation clock or ambiguous tempo-dependent export duration. Mute/solo are also recorded discrete lanes. Date/Author: 2026-09-29, implementation agent.

Decision: file I/O, replacement engine preparation and offline export run on workers; the GUI still serializes short draws/controls through a shared engine mutex. Rationale: make the POC usable while honestly documenting that it is not a hard-real-time, allocation-free host. Date/Author: 2026-09-29, implementation agent.

Decision: preserve existing FFI and recorder internals, canonicalizing finalized studio recordings with latest-insertion priority and installing the same clip in live replay. Rationale: fixes actual captured Save/Export failures without silently losing the user's newest gesture or claiming an unrelated persisted performance. Date/Author: 2026-09-29, implementation agent.

Decision: Stop/Play and active tempo changes restart every source/automation from zero, rather than claim pause-safe resume unsupported by the current sequencer API. Rationale: synchronized restart is preferable to subtly desynchronized drum/chord/PCM/automation cursors. Repeated identical transport/tempo calls are no-ops. Date/Author: 2026-09-29, implementation agent.

## Outcomes & Retrospective

The feature provides actual editable musical loops, four sounding demo tracks, track/master effects and mixing, held chord/drum/bass performance capture, readable parameter automation, replay, embedded-sample JSON save/load and fresh-engine stereo final mixdown. It adds no C ABI changes. Release GUI and headless builds succeed; 15 new studio integration tests pass, and the existing full suite remains passing (542 unit tests, two ignored, plus integration suites). The 10,000-block release stress run rendered finite audio in 39.03 seconds for 106.67 seconds of material. Independent screen/audio capture is assigned to the parent.

Deliberate limits are one-bar performance/automation, fixed source/rack layout, one audio-loop track, no arrangement/undo/plugins/microphone/MIDI-device recording. Embedded float JSON can be large, and snapshot/drawing work shares the audio mutex. These are documented production follow-ups, not claimed complete DAW capabilities.

Follow-up outcome: actual captured overdub clips now normalize with newest-take priority and can be saved/exported/reloaded; live replay installs the same normalized gates after recording ends. Transport restarts consistently instead of pretending to pause-resume. Hostile WAV rate/count arithmetic and true per-rate duration are validated before payload allocation. The expanded final release run passes 24 studio tests and 927 tests total, including a nonintegral-step 1001-bar test. A repeated 70-second native GUI capture demonstrates the complete workflow. Its saved three-lane song exports byte-identically from the GUI and headless CLI. Selected screenshots/video are committed under `docs/assets/loop-studio/`; larger generated evidence stays under `/tmp/opencode/studio-evidence-verified/`.

Final independent verification: 100,000 render blocks change gains/mutes and check every sample finite, rendering 17.78 minutes of stereo material in 441.70 seconds under concurrent load. Independent inspection also passes for the following 32-bar WAV. The native GUI completes a 600-second recorded live soak, with no NaN/Inf samples or silence gaps longer than 100 ms below −60 dBFS. Silent-mode recording, imported WAV playback and Pluck chord audition also work. Both GUI processes close cleanly. Preliminary oversized/superseded stress jobs were stopped, not counted as passes. Production API findings are recorded in `docs/loop-studio-api-findings.md`.

## Context and Orientation

`src/ffi.rs` owns the full engine. `src/mixer/graph.rs` supplies track gain, balance, mute/solo and racks. `src/mixer/mod.rs` owns stereo audio loops. `src/performance/mod.rs` supplies looping chord events at 96 clock ticks per quarter note. A tick is a musical timing unit, not an audio sample. `src/studio/mod.rs` will own a serializable song and safely encapsulate engine pointers. `examples/loop_studio.rs` will draw controls and run audio; `examples/studio_render.rs` will export and stress-test without a window.

## Plan of Work

Milestone one adds a validated Session model and Studio owner, using existing FFI exclusively with internally owned pointers. It maps patterns to the engine sequencer, chords to the performance recorder, loop samples to the mixer, and automatable controls to track/master/instrument parameters. Offline rendering creates a fresh engine so exporting never changes live transport.

Milestone two adds an eframe window with transport, musical grid, mixer, pad performance and automation visualization. Native CPAL playback is optional. If unavailable, a worker renders audio into a discarded buffer on a wall-clock schedule; this is explicitly labeled silent mode, not claimed as sound output.

Milestone three validates rendered energy, silence/mute, pan, persistence, malformed input, recorded events and automation. Documentation includes exact run commands and proof-of-concept limits.

## Concrete Steps

Working directory for all commands is `/home/pretzel/code/libgooey-loop-studio`.

    cargo test --release --no-default-features --features studio
    cargo run --release --no-default-features --features studio --example studio_render -- --export /tmp/opencode/studio.wav --bars 4 --stress 10000
    cargo run --release --features studio-gui --example loop_studio -- --silent --demo
    cargo fmt --check
    cargo clippy --no-default-features --features studio-gui --example loop_studio

## Validation and Acceptance

The demo must export finite nonzero stereo audio. Muting all tracks must produce silence after tails settle. Saving and loading must retain all patterns, chords, samples and automation. In the GUI, starting transport advances the highlighted step and meters; sliders and mute/solo alter the renderer, and recording pad gestures makes visible replayable events. Export must work even with no display or device.

## Idempotence and Recovery

Builds and tests can be repeated. Export/save only write user-specified files. Loading validates a complete session before replacing the current session; errors must leave the current song intact. Work remains isolated on the native Git worktree branch, with a final commit and PR after independent verification; no main-branch merge or source resets are part of this plan.

## Artifacts and Notes

Observed implementation evidence:

    test result: ok. 15 passed; 0 failed; 0 ignored (tests/studio.rs)
    test result: ok. 542 passed; 0 failed; 2 ignored (existing library tests)
    stress: 10000 blocks, 39.03s elapsed, finite audio
    exported /tmp/opencode/studio-demo.wav: 493241 stereo frames at 48000 Hz, peak 0.4556, RMS 0.1443
    Studio-path Clippy diagnostics: 0

Full final test output is in `/tmp/opencode/studio-final-tests.log`; all suites pass. `cargo fmt --check` passes after final formatting. The GUI compiled in release with native audio, and the feature also compiles without native audio.

`target/release/examples/loop_studio --demo` starts stopped with all four clips populated; Space starts playback. Under virtual X without audio, use `--silent` and software OpenGL. The top transport/file controls remain pinned; the center scrolls through steps, mixer, performance, automation and loop import. Parent receives these launch commands and control mappings for independent capture.

Follow-up tests additionally cover direct/stretched-loop transport, sample rates 8/44.1/96 kHz, changing BPM, combined master/track automation, over 1000 bars with nonintegral step timing, checked malformed WAV headers, actual duration at low rates, and same-build repeated WAV identity. Noise sources in the used engine are fixed-seed; repeated fresh exports match within the same build/platform, not necessarily across architectures or live DSP histories. Do not interpret these tests as lock-free/hard-real-time or sample-identical alignment claims for every source.

Final follow-up evidence is saved in `/tmp/opencode/studio-robustness-final.log`:

    test result: ok. 24 passed; 0 failed; 0 ignored (studio)
    test result: ok. 542 passed; 0 failed; 2 ignored (library)
    Total passed across all suites/doc-tests: 927

The parent should rebuild the release example before rerecording; binaries launched before this source change still contain the old overlapping-gate behavior. First capture failure is addressed by regressions rather than a GUI-specific workaround.

## Interfaces and Dependencies

`studio::Session` is persisted through serde_json. `studio::Studio` owns the engine and exposes safe render, control, transport, recording and snapshot methods. hound writes 32-bit float stereo WAV and reads imported WAV. eframe 0.29 with glow, default fonts, X11 and Wayland provides the window; CPAL remains behind the existing native feature.

Revision 2026-09-29: initial plan after infrastructure inspection; preserve the C ABI and avoid duplicated DSP.

Revision 2026-09-29: record completed API/GUI/headless implementation, initial recorder-install bug, preset-recall correction, 15-test coverage and observed release stress/export evidence. Keep independent capture/review pending and explicitly document POC limits.

Revision 2026-09-29: add atomic session-save replacement and input-size bounds, source-BPM correction for audio loops, always-honored pad key-up, final full-suite output and zero new studio-path Clippy diagnostics. These improve safe persistence and usable loop/keyboard behavior without expanding the C ABI.

Revision 2026-09-29: independent capture exposed overlapping recorded gates; follow-up canonicalizes/installs the newest take and tests Save/Export/reload. Parent also requested transport, long-run, sample-rate, import and determinism robustness. Changes are limited to studio code/tests/docs/plan while the GUI is under capture; final expanded suite and rerecording remain pending.

Revision 2026-09-29: repeat real GUI capture successfully after the fix; add reproducible capture/independent WAV verification scripts and checked evidence. Final release suite independently passes 927 tests and GUI/headless recorded-song WAVs match exactly. Extended final-build rendering, live soak, and PR submission remain explicitly pending.

Revision 2026-09-29: finalize extended stress/live soak and silent import/audition evidence; document measured results and cancelled preliminary runs honestly. Add explicit existing/new/proposed engine API findings. Commit/push remains isolated in the native libgooey worktree; final browser PR preparation remains to finish.

Revision 2026-09-29: complete final PR description with verification/artifacts/limits, commit and publish documentation on the isolated feature branch, and distinguish the web-creation workflow from confirmed PR publication. Authentication/push work, but the harness has no connected desktop browser to automate submission; do not claim a PR number without confirmation.
