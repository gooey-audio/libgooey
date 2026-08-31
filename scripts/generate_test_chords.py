#!/usr/bin/env python3
"""
Generate test audio for the diatonic pitch shifter POC.

Produces a WAV file with a chord progression plus some deliberately
non-diatonic chords, so the quantizer has something to fix.

Usage:
  python3 scripts/generate_test_chords.py --output /tmp/test_chords.wav
"""

import argparse
import numpy as np
import soundfile as sf

SR = 44100
DURATION_PER_CHORD = 1.0  # seconds
CROSSFADE = 0.02  # seconds crossfade between chords

NOTE_FREQS = {
    "C2": 65.41, "C#2": 69.30, "D2": 73.42, "D#2": 77.78, "E2": 82.41,
    "F2": 87.31, "F#2": 92.50, "G2": 98.00, "G#2": 103.83, "A2": 110.00,
    "A#2": 116.54, "B2": 123.47,
    "C3": 130.81, "C#3": 138.59, "D3": 146.83, "D#3": 155.56, "E3": 164.81,
    "F3": 174.61, "F#3": 185.00, "G3": 196.00, "G#3": 207.65, "A3": 220.00,
    "A#3": 233.08, "B3": 246.94,
    "C4": 261.63, "C#4": 277.18, "D4": 293.66, "D#4": 311.13, "E4": 329.63,
    "F4": 349.23, "F#4": 369.99, "G4": 392.00, "G#4": 415.30, "A4": 440.00,
    "A#4": 466.16, "B4": 493.88,
    "C5": 523.25, "C#5": 554.37, "D5": 587.33, "D#5": 622.25, "E5": 659.25,
    "F5": 698.46, "F#5": 739.99, "G5": 783.99, "G#5": 830.61, "A5": 880.00,
    "A#5": 932.33, "B5": 987.77,
}

# Chords defined as (name, [note_names], is_diatonic_to_C_major)
# We include some that ARE in C major and some that aren't — the quantizer
# should fix the non-diatonic ones.
CHORDS = [
    # Diatonic to C major:
    ("Cmaj  (I)",     ["C3", "E3", "G3", "C4"], True),
    ("Dm   (ii)",     ["D3", "F3", "A3", "D4"], True),
    ("Em   (iii)",    ["E3", "G3", "B3", "E4"], True),
    ("Fmaj (IV)",     ["F3", "A3", "C4", "F4"], True),
    ("Gmaj  (V)",     ["G3", "B3", "D4", "G4"], True),
    ("Am   (vi)",     ["A3", "C4", "E4", "A4"], True),
    ("Bdim (vii)",    ["B3", "D4", "F4", "B4"], True),

    # NON-diatonic to C major — these should be quantized:
    ("Dmaj — not in C",   ["D3", "F#3", "A3", "D4"], False),  # F# is not in C major
    ("Emaj — not in C",   ["E3", "G#3", "B3", "E4"], False),  # G# is not in C major
    ("Fm — not in C",     ["F3", "G#3", "C4", "F4"], False),  # Ab is not in C major
    ("Bbmaj — not in C",  ["A#3", "D4", "F4", "A#4"], False), # Bb is not in C major
    ("C#maj — not in C",  ["C#3", "F3", "G#3", "C#4"], False), # C# and G# not in C major
]


def make_sawtooth_wave(freqs: list, duration: float, sr: int) -> np.ndarray:
    """Generate a sawtooth waveform with given frequencies as a rich pad-like timbre."""
    t = np.arange(int(duration * sr)) / sr
    signal = np.zeros_like(t)
    for freq in freqs:
        # Sawtooth: rich harmonics, sounds like a synth
        phase = np.mod(freq * t, 1.0) * 2.0 - 1.0  # sawtooth
        # Add some filtered harmonics for a warmer pad sound
        signal += phase * 0.3
    # Soft saturation
    signal = np.tanh(signal * 2.0) * 0.5
    return signal


def generate_test_audio(output_path: str):
    """Generate the chord progression test WAV."""
    total_duration = len(CHORDS) * DURATION_PER_CHORD
    total_samples = int(total_duration * SR)
    audio = np.zeros(total_samples, dtype=np.float32)

    for i, (name, notes, diatonic) in enumerate(CHORDS):
        start_sample = int(i * DURATION_PER_CHORD * SR)
        end_sample = int((i + 1) * DURATION_PER_CHORD * SR)
        chord_len = end_sample - start_sample

        freqs = [NOTE_FREQS[n] for n in notes]
        chord_wave = make_sawtooth_wave(freqs, DURATION_PER_CHORD, SR)

        # Apply fade-in/fade-out envelope
        fade_len = int(CROSSFADE * SR)
        envelope = np.ones(chord_len)
        if fade_len > 0:
            envelope[:fade_len] = np.linspace(0, 1, fade_len)
            envelope[-fade_len:] = np.linspace(1, 0, fade_len)
        chord_wave = chord_wave[:chord_len] * envelope

        # Place into output
        if end_sample <= total_samples and chord_len > 0:
            audio[start_sample:end_sample] += chord_wave[:chord_len]

        label = "IN KEY" if diatonic else "OUT OF KEY"
        print(f"  {i+1:2d}. {name:25s} [{label}]  notes: {' '.join(notes)}")

    # Final limiter
    peak = np.max(np.abs(audio))
    if peak > 0.95:
        audio = audio * (0.90 / peak)

    sf.write(output_path, audio, SR, subtype="FLOAT")
    print(f"\nWrote {output_path} ({total_duration:.1f}s, {SR} Hz)")
    print(f"Chords 1-7 are diatonic to C major (should pass through)")
    print(f"Chords 8-12 are NON-diatonic (should be quantized)")


def main():
    parser = argparse.ArgumentParser(
        description="Generate test chord progression for diatonic pitch shifter POC."
    )
    parser.add_argument("--output", type=str, default="/tmp/test_chords.wav",
                        help="Output WAV file path")
    args = parser.parse_args()

    print("Generating test chord progression...\n")
    generate_test_audio(args.output)


if __name__ == "__main__":
    main()
