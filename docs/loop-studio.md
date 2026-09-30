# Loop Studio

[Current central-app capture and independent integration verification](omni-studio-verification.md).

[Historical standalone screenshots, screen/audio recording, and independent verification](loop-studio-verification.md).
[Engine API findings and proposed production extensions](loop-studio-api-findings.md).

Loop Studio is a working, deliberately bounded DAW-style proof of concept. It
uses the existing libgooey drum/bass sequencers, Nebula/poly synth, performance
recorder, stereo loop mixer, mixer graph, effect DSP and limiter. There is no
second synthesis engine hidden in the GUI. The demo's fourth track is generated
PCM shaker audio, not a downloaded sample.

## Run

From the repository root:

```bash
cargo run --release --features studio-gui --example omni_gui -- --panel "Loop Studio" --demo
# Compatibility alias, same application and audio host:
cargo run --release --features studio-gui --example loop_studio -- --demo
```

Use **release** mode for live audio. The default native feature uses CPAL's
default audio device. If device discovery/configuration fails, the shared shell
reports the error and falls back to its silent renderer. All CPAL f32/f64 and
signed/unsigned 8/16/32/64-bit formats are supported. Runtime device errors are
counted in the shared health view; restart with `--silent` if a device disappears.
`--load session.json` initializes a stopped saved song (takes precedence over
`--demo`). `--panel`, `--list`, `--stress SECONDS`, and `--soak SECONDS` use the
same central entrypoint as the other labs. `studio-gui` enables `studio` + `gui`;
the `studio` feature alone remains device/window independent.

For Linux without an audio device, including virtual X:

```bash
cargo build --release --no-default-features --features studio-gui --example loop_studio
xvfb-run -a -s '-screen 0 1440x1000x24' env LIBGL_ALWAYS_SOFTWARE=1 \
  target/release/examples/loop_studio --silent --demo
```

An existing X server can instead use `DISPLAY=:99`. Native builds require ALSA
development headers; the GUI needs X11/Wayland development libraries and an
OpenGL implementation. The silent build avoids ALSA/CPAL completely. Both modes
run real engine rendering: silent mode discards samples, but transport, meters,
recording, automation and export still work. It does not produce device sound.
Neither studio feature enables the repository's GLFW visualization feature.

The shared initial window is 1440×1000 (minimum 900×700). Studio content scrolls
vertically; transport and file controls remain pinned alongside central panel
navigation and stereo scope/spectrum/health. Switching labs, Stop audio, and
window close stop the host first, then release held chords, finalize recording,
and stop studio transport. Returning retains the song and DSP, but starts with
transport stopped and no held input. While host audio is stopped the studio
editor is suspended; Resume audio restores it without rebuilding the song.

## Make, play and record a song

1. Start with **Demo song** or `--demo`. Press **Play** or Space. The yellow step
   indicator advances and all four track meters respond.
2. Click any drum/bass step to toggle it. Edit the MIDI number beneath a bass
   step by dragging or typing. Every enabled step has a stored velocity; the
   GUI's default toggle velocities are 0.8 (0.4 for hi-hat). Existing engine bass
   frequency limits apply; MIDI notes 24–55 cover its useful bass range.
3. Each strip has Gain, Pan (0 left, 0.5 center, 1 right), Mute, Solo, Filter Hz,
   Delay mix and Reverb mix. Solo overrides that strip's mute and silences
   non-solo tracks, matching the existing graph. Bass and chords also expose
   synth tone. The master has gain, delay and reverb. The limiter is explicitly
   enabled in the studio, without changing existing FFI defaults.
4. Hold a chord pad with the mouse or keys **1–7**; release ends the chord gate.
   Root is a pitch-class number (0=C, 1=C#, …, 11=B). Choose major/minor, octave
   and Default/Pad/Pluck/Keys/Strings. Mouse dragging between pads changes chord.
   Key **0**, **Release**, or losing focus releases a held chord.
5. Play drum/bass gestures with the buttons or **Z/X/C/V/B**. Bass button B plays
   C2 (MIDI 36). These are synthesis triggers, not decorative controls.
6. Toggle **Record / overdub** or R. Chord recording arms for the next one-bar
   boundary, shown as **CHORDS ARMED** until **CAPTURING CHORDS** appears. Arm
   before starting an empty transport to capture from the first bar. The existing
   recorder stores held chord gates. Finalized overdubs use **last-take-wins**:
   newly held intervals replace covered older notes, retaining uncovered older
   fragments. Duplicate starts keep the newest take, and gates can wrap across
   the loop seam. When recording ends, the normalized clip is installed into
   live replay as well as saved/exported; it is not merely cleaned up in JSON.
   The original recorder may still play older events while an overdub is open;
   the canonical last-take clip becomes authoritative after recording ends.
   Drum/bass hits and control changes capture immediately while transport runs.
7. Move a slider or toggle Mute/Solo while recording. Stop recording to replay
   the lane each loop. Automation playback is suspended while recording so it
   does not fight the controls; previous musical hits/chords still replay.
   The lane selector shows control name and point count. The graph shows beats,
   range, held values and current cursor; hovering gives an exact beat/value.
   Numerical point summaries appear beneath it. **Clear lanes** removes all
   automation; **Clear performance** removes chords and recorded live hits, not
   the drum/bass step patterns.
8. **Stop** freezes the displayed cursor, ends recording, and releases the
   chord/loop; instrument/effect tails continue to render. **Play after Stop
   starts all clips and automation together at bar zero**, not at the paused
   cursor. Repeated Play while already playing (or Stop while already stopped)
   does nothing. The legacy engine's start call retriggers the current step and
   is not a pause-safe resume API; this studio deliberately uses consistent
   restart semantics rather than letting drum/chord/loop and automation phases
   diverge. **Rewind** rebuilds from the saved musical state at beat
    zero on a preparation worker and preserves play/stop, but disarms recording
    and clears DSP tails. If audio is stopped while preparation runs, the result
    remains stopped when installed.

The grid, chord performance, hit performance and automation are repeating
one-bar clips (384 ticks at 96 ticks per quarter note). A tick is a musical
timing unit. Control points use step-held values with the engine's existing
smoothing, not straight-line interpolation. Multiple writes to the same control
in one tick replace that point. A new lane gets an initial point at tick zero
so its first replay has a defined baseline. Existing lanes overwrite touched
ticks, retaining untouched points. Changing BPM during playback ends recording
and re-cues all clips/automation at bar zero at the new tempo, retaining effect
tails. Reapplying the same BPM does nothing. Tempo changes are not recorded as
automation. The cursor derives from an integer rendered-frame counter, not
repeated floating-point time additions. At each bar the existing sequencers are
re-anchored to the musical cursor so their legacy f32 trigger counters cannot
accumulate hours of drift. The legacy PCM channel playhead continues through
this sequencer/grid-clock re-anchor, so phrases longer than a bar do not rewind
to their beginning. Play after Stop explicitly restarts that PCM playhead too.
Legacy sequencer trigger rounding within a bar and pitch-stretch latency still
apply; this is not a promise of sample-identical alignment among all DSP sources,
or artifact-free re-anchoring for every stretched audio loop.

## Audio loops and persistence

Type a mono/stereo PCM or float WAV path into **Audio loop WAV** and click
**Import loop**. Import captures current BPM as its source tempo and uses the
existing pitch-preserving time stretcher when song and source tempos differ.
When those tempos match, it uses direct loop playback without unnecessary WSOLA
stretching or its latency/correlation ambiguity. The file's
authored tempo can be corrected with **Loop source BPM** after import. It is
saved but not recorded as automation. The file's
duration determines the repeat interval: trim externally to an exact bar or
phrase; import does not automatically detect tempo or slice audio. Import/load
prepares an engine on a worker and installs a stopped session. Avoid editing
while a replacement is preparing, since that replacement is based on the
snapshot at import time.

Type a JSON path into **Session**, then **Save** or **Load**. Files persist
patterns/velocities/notes, chords, hits, mix/FX values, automation, and embedded
stereo audio samples. They have no external sample-file dependency. GUI-only
pad selection and the current transport cursor are not song state. Load always
starts stopped. Saves write a sibling temporary file and rename after writing,
so an unsuccessful save does not truncate an existing session. A failed load
leaves the current engine/song intact. Version, numbers, limits, overlapping
chords, duplicate lanes and point order are validated before installation.

Limits: 512 chords, 4096 live hits, 64 lanes with at most 384 points each, and
128 MiB JSON input. Audio imports are at most 120 seconds and 5,760,000 stereo
frames (120 seconds at 48 kHz); higher sample-rate clips hit the frame limit
earlier. Session samples use float JSON, so large loops create large files and
take longer to save. Only finalized chord gates are saved; release a held chord
or stop recording before saving/exporting.

Both imports and directly constructed sessions enforce actual duration using
the loop's own sample rate, including rates below 48 kHz. WAV sample rate,
channels, bit depth, duration and checked sample-count bounds are validated
before collecting the payload. Malformed rates cannot overflow a `rate * 120`
calculation. Oversized/malformed files fail without replacing the current song.

## Final mixdown and headless verification

Choose an output path and bar count, then **Export WAV**. It snapshots the song
and renders a **fresh engine on a worker**, starting at beat zero. Live transport
does not move and live meters are not consumed. Output includes track/master FX,
automation and recorded performance, then two seconds of release/effect tails.
It is 48 kHz, stereo, 32-bit float WAV. This is the final mix, not stems or an
audio-device recording. Mute/solo and automation affect export exactly as they
affect replay. Export errors appear in the status bar; a failed WAV write may
leave a partial output file, which can be overwritten on retry.

No display or device is needed for the CLI:

```bash
cargo run --release --no-default-features --features studio --example studio_render -- \
  --export /tmp/opencode/studio-demo.wav --save /tmp/opencode/studio-demo.json --bars 4
cargo run --release --no-default-features --features studio --example studio_render -- \
  --load /tmp/opencode/studio-demo.json --export /tmp/opencode/studio-replay.wav --bars 4
cargo run --release --no-default-features --features studio --example studio_render -- \
  --export /tmp/opencode/studio-stress.wav --stress 10000 --bars 1
cargo test --release --no-default-features --features studio
cargo fmt --check
cargo clippy --features studio-gui --example loop_studio --example studio_render --test studio
```

The CLI starts with the demo unless `--load` is supplied. Stress mode changes
gains/mutes and renders 10,000 blocks of 512 stereo frames, checking finite
output. On the implementation environment this took 39.03 seconds for 106.67
seconds of audio. The four-bar demo export reported 493241 frames (including
tails), peak 0.4556 and RMS 0.1443. These are observations, not a universal
real-time performance guarantee.

Repeated fresh-engine demo exports are byte/sample-identical in the tested build
on the same platform; a regression test compares two complete WAV files. The
used engine noise generators start from fixed seeds, and the generated PCM demo
clip is fixed-seed too. This does **not** promise bitwise equality across compiler
versions, architectures or math libraries, or between an offline fresh-engine
render and live playback with previously accumulated voice/filter/effect state.

The studio integration tests exercise export format/duration/energy,
persistence including embedded PCM, validation/atomic failed replacement,
actual mute/solo/pan/FX changes, callback-size-independent automation, live
hit/slider recording, existing chord recording and replay, audio import,
stopped snapshots, idempotent/re-cued transport, tempo changes at 8/44.1/96 kHz,
multiple track/master lanes, fractional-step timing over 1001 bars, hostile WAV
headers/actual duration, demo-overdub persistence/replay including seam gates,
same-build repeated mixdown, and independence of offline mixdown. Test temporary
paths use the platform's `std::env::temp_dir()` for CI/macOS portability. The existing
test suite remains applicable. Clippy currently reports pre-existing library
warnings; `-D warnings` is not clean repository-wide. New studio code is checked
without suppressing those baseline diagnostics.

## Reusable Rust API and deliberate limits

Enable `studio` to use `gooey::studio::{Session, Studio, Control}` without a GUI
or CPAL. `Studio` uniquely owns the engine allocation, frees it with the matching
existing lifecycle function, and never exposes the raw pointer. Rendering and
control require exclusive `&mut Studio`; hosts serialize calls. Session state is
the source of truth, not a parallel GUI reconstruction of engine state. See the
module's example and `tests/studio.rs` for integration.

This is not an unlimited-track DAW. It has one drum submix, one bass voice, one
six-voice poly synth and one audio-loop track. It has no arrangement timeline,
plugin hosting, audio-input/microphone capture, MIDI-device input, undo history,
audio waveform editor, arbitrary rack reorder UI or multibar performance
capture. The underlying library supports richer racks/clip grids; this interface
does not pretend to expose all of them. “Recording” here means capturing musical
gestures and parameters, not recording a microphone.

`src/gui/studio.rs` implements the public `GuiPanel` extension contract using
`InterleavedAdapter` and `BlockRenderer`; `examples/loop_studio.rs` is only a
selected-panel launcher. The sole stream/silent-worker owner is `GuiAudio` in
`src/gui/audio.rs`. The factory creates Studio at the host's negotiated rate;
mounting a renderer does not recreate it or discard saved song state.

The desktop audio callback reuses fixed scratch buffers and uses `try_lock`:
GUI contention silences that block and increments shared lock-miss health,
rather than waiting. File preparation, save/export, session copying, engine
construction and replacement reclamation occur off the callback. Drawing,
snapshot copying and engine swaps can still cause silenced blocks. Synth-tone preset
updates also use the existing engine control path. This POC is not a guaranteed
allocation-free, lock-free hard-real-time host. Future production work should
move studio commands/snapshots to dedicated render/control endpoints and retain
prepared immutable session/audio data off the callback. C ABI signatures and
legacy feature behavior remain unchanged.
