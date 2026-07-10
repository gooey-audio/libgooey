# Diatonic Pitch Shifting POC — ExecPlan

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `.agent/PLANS.md`, and this document must be maintained in accordance with that file.

## Purpose / Big Picture

A musician has a sample of a synth pad chord (or any polyphonic audio slice) and wants to pitch-shift it so the result stays musically correct within a given key. For example, if they sample a C-major chord and then trigger it at MIDI note D in the key of C major, the output should be D-minor (the diatonic ii chord) rather than D-major (which contains the non-diatonic F#). This is the core behavior of "diatonic pitch shifting" or "scale-aware pitch correction."

After this change, a user can run a Python script on a MacBook that takes an input WAV file and a target key/scale, then writes a diatonic-pitch-quantized output WAV file. The script serves as a proof-of-concept for the algorithm that can later be ported to Rust and Metal for iOS.

## Progress

- [x] (2026-07-10) Create the STFT-based diatonic spectral filter POC script (`scripts/diatonic_pitch_shift.py`).
- [x] (2026-07-10) Create the CQT-based diatonic quantizer as alternative mode in same script.
- [x] (2026-07-10) Create the phase vocoder + pYIN baseline mode for monophonic comparison.
- [x] (2026-07-10) Create test audio generator (`scripts/generate_test_chords.py`).
- [x] (2026-07-10) Validate STFT mode: non-diatonic notes attenuated 0.11-0.46×, diatonic notes preserved 0.94-0.998×.
- [x] (2026-07-10) Validate all three modes with multiple keys/scales (Eb natural minor, A dorian, D harmonic minor).
- [ ] (2026-07-10) Commit, push, open PR.
- [ ] Future: Port STFT diatonic filter to Rust using rustfft.
- [ ] Future: Integrate CREPE neural pitch detector as analysis front-end.
- [ ] Future: Add key detection (Krumhansl-Schmuckler or neural).
- [ ] Future: Metal/MPS acceleration for iOS.
- [ ] Future: Real-time streaming version.

## Surprises & Discoveries

- Observation: The CQT approach with 12 bins/octave has insufficient frequency resolution at low frequencies — adjacent semitone bins overlap significantly, making it impossible to cleanly separate diatonic from non-diatonic notes.
  Evidence: At C3 (~131 Hz), the CQT filter bandwidth is roughly one semitone wide, so attenuating bin N also impacts bins N±1. This was visible as uniform ~0.6× attenuation across all bins regardless of diatonic status.
  Mitigation: Added STFT mode with n_fft=8192, which gives ~5.4 Hz/bin resolution — about 2 bins per semitone at low frequencies and 20+ bins at mid frequencies. This provides clean discrimination.

- Observation: Gaussian mask smoothing must stay small (sigma ≤ 5 Hz) or it eliminates diatonic/non-diatonic discrimination entirely.
  Evidence: At sigma=50 Hz (≈9.3 bins), the mask blurred to ~0.67 for all bins, making attenuation uniform. At sigma=5 Hz (≈0.93 bins), non-diatonic bins attenuated to 0.11-0.46× while diatonic bins stayed at 0.94-0.998×.

- Observation: Naive energy redistribution between CQT bins causes phase discontinuities (audible as roughness/phasiness).
  Evidence: Direct magnitude transfer without phase adjustment produced comb-filter artifacts.
  Mitigation: Switched to masking approach (attenuate non-diatonic bins rather than redistribute). This preserves phase coherence.

## Decision Log

- Decision: Use CQT-domain processing as the primary diatonic quantization algorithm.
  Rationale: The Constant-Q Transform naturally aligns frequency bins with musical notes (log-frequency spacing). With 12 bins per octave, each bin corresponds to exactly one semitone. Diatonic quantization in this domain is simply "zero out non-diatonic bins and redistribute their energy to the nearest diatonic bin." This handles polyphonic audio (chords) without requiring source separation or per-note tracking.
  Date/Author: 2026-07-10 / Codex

- Decision: Implement in Python first, using librosa for CQT and pyrubberband for comparison.
  Rationale: The goal is a POC that demonstrates algorithm feasibility. Python allows rapid iteration with mature audio libraries. The algorithm itself (CQT → mask → ICQT) can later be ported to Rust using existing rustfft infrastructure in the codebase.
  Date/Author: 2026-07-10 / Codex

- Decision: Include a phase-vocoder + pYIN baseline for monophonic comparison.
  Rationale: Having a simpler baseline helps validate the CQT approach and provides a quality reference. The phase vocoder approach is the "standard" way to do pitch-corrected shifting and serves as a sanity check.
  Date/Author: 2026-07-10 / Codex

- Decision: Use HPSS (Harmonic-Percussive Source Separation) as preprocessing to protect transients.
  Rationale: Percussive elements (drums, clicks, attacks) don't have "pitch" in a meaningful sense and should not be quantized. HPSS separates tonal from transient content; we apply quantization only to the harmonic component and mix the percussive component back unchanged. This preserves attack transients.
  Date/Author: 2026-07-10 / Codex

- Decision: Defer CREPE/neural pitch detection to a future milestone.
  Rationale: The CQT approach doesn't require explicit pitch detection — it operates on the spectrogram directly. Neural pitch detection (CREPE) would be valuable for key detection and for the alternative phase-vocoder path, but is not needed to demonstrate the core algorithm.
  Date/Author: 2026-07-10 / Codex

## Outcomes & Retrospective

*(to be filled after implementation)*

## Context and Orientation

The libgooey codebase at `src/` is a Rust real-time audio synthesis engine. It already has:
- Music theory primitives in `src/music/`: `NoteName`, `Key`, `ScaleType`, `Chord`, `Interval` (scales, keys, diatonic chords, voicings)
- C FFI at `src/ffi.rs` for iOS integration
- FFT infrastructure via the `rustfft` dependency (feature-gated)
- Granular pitch shifting in `src/instruments/granulator.rs` (time-domain, grain-based)
- WSOLA time-stretching in `src/mixer/wsola.rs`

This POC lives outside the Rust codebase as a standalone Python script at `scripts/diatonic_pitch_shift.py`. It demonstrates the algorithm in a language and environment suited to rapid experimentation. Once validated, the algorithm can be ported to Rust using the same music theory types and FFT infrastructure already present.

## Plan of Work

### Milestone 1: Python POC Script

Create `scripts/diatonic_pitch_shift.py` with two modes:

**Mode A: CQT Diatonic Quantizer** (primary, handles polyphonic)

1. Load audio via `librosa.load()` or `soundfile.read()`
2. Apply HPSS to separate harmonic (tonal) from percussive (transient) components
3. Compute CQT of the harmonic component with `bins_per_octave=12`
4. For each CQT bin, determine its MIDI note: `midi = bin_index // 12 + cqt_notes[0]` (re: fmin mapping)
5. Build a boolean mask: `mask[bin] = True` if the note is in the target key/scale
6. Apply mask to zero out non-diatonic bins
7. Redistribute: for each non-diatonic bin with energy, add its magnitude to the nearest diatonic neighbor (above or below, whichever is closer)
8. Apply light spectral smoothing (Gaussian blur across a few bins) to reduce artifacts
9. Inverse CQT to reconstruct the harmonic component
10. Mix percussive component back in (unmodified)
11. Write output WAV

**Mode B: Phase Vocoder + pYIN** (baseline, monophonic only)

1. Load audio
2. Detect pitch contour using librosa's pYIN
3. For each frame with detected pitch, snap to nearest diatonic note
4. Compute per-frame pitch shift ratio
5. Apply using pyrubberband
6. Write output WAV

### Milestone 2: Test Audio Generation

Create `scripts/generate_test_chords.py` to produce test material:
- A C-major chord (C4-E4-G4)
- An F-major chord (F4-A4-C5)
- A G-major chord (G4-B4-D5)
- Played as a progression with simple sawtooth synthesis

Run the diatonic quantizer on this test input with key=C major and verify:
- C-major chord passes through unchanged (already diatonic)
- G-major chord's B4 becomes Bb4 or is softened (B is diatonic to C major, so this is correct)
- A D-major test chord (D4-F#4-A4) is quantized to D-minor (D4-F4-A4)

### Milestone 3: Porting Path to Rust (plan only, not implemented)

Outline how the CQT approach would be ported:
- Replace librosa CQT with Constant-Q transform built on `rustfft`
- Use existing `src/music/` types for key/scale/note logic
- Wrap in an `Effect` trait implementation for the engine
- For iOS: accelerate CQT using vDSP (Apple's Accelerate framework) via Metal Performance Shaders

## Concrete Steps

All commands run from the repository root.

### Step 1: Create the POC script

    python3 scripts/diatonic_pitch_shift.py --help

### Step 2: Generate test audio

    python3 scripts/generate_test_chords.py --output /tmp/test_chords.wav

### Step 3: Run diatonic quantizer on test audio

    python3 scripts/diatonic_pitch_shift.py /tmp/test_chords.wav /tmp/quantized.wav --key C --scale major --mode cqt

### Step 4: Listen and compare

    (The user plays both files to evaluate quality.)

### Step 5: Create feature branch, commit, push, PR

    git checkout -b feat/diatonic-pitch-shift-poc
    git add scripts/ plans/
    git commit -m "feat: diatonic pitch shifting POC with CQT-based quantizer"
    git push -u origin HEAD
    gh pr create --title "Diatonic pitch shifting POC" --body "..."

## Validation and Acceptance

- The script runs without errors on a MacBook with Python 3.10+ and librosa installed.
- A test chord progression processed through the CQT mode sounds musically "in key" (non-diatonic notes are suppressed or shifted to diatonic neighbors).
- The output WAV file has the same duration and sample rate as the input.
- No loud artifacts, clicks, or silence gaps appear in the output.

## Idempotence and Recovery

The script is a pure function from input WAV → output WAV. Running it multiple times produces identical output. No state is modified outside the output file.

## Artifacts and Notes

Dependencies (pip install):
- `librosa` (CQT, HPSS, pYIN)
- `numpy` (array computation)
- `scipy` (signal processing)
- `soundfile` (WAV I/O)
- `pyrubberband` (phase vocoder pitch shifting, optional)

## Interfaces and Dependencies

The POC script has no internal dependencies on the libgooey Rust codebase. It is a standalone Python script.

The key algorithm maps to Rust types as follows:

| Python concept | Rust equivalent |
|---|---|
| Note name to MIDI | `src/music/note.rs:note_to_midi()` |
| Key/scale definition | `src/music/key.rs:Key`, `src/music/scale.rs:ScaleType` |
| Is note in key? | `Key::scale_degrees().contains(note)` |
| Nearest diatonic note | min-by-distance over `Key::scale_degrees()` |
| CQT transform | Future: custom CQT using `rustfft` |
| HPSS | Future: median-filter based HPSS using `rustfft` |
