#!/usr/bin/env python3
"""
Diatonic Pitch Shifting POC — CQT-based diatonic pitch quantizer.

Two modes:
  cqt    — Constant-Q Transform domain quantization (handles polyphonic audio)
  pvoc   — Phase vocoder + pYIN pitch detection (monophonic baseline)

Usage:
  python3 scripts/diatonic_pitch_shift.py input.wav output.wav --key C --scale major --mode cqt
  python3 scripts/diatonic_pitch_shift.py input.wav output.wav --key D --scale minor --mode pvoc

Dependencies:
  pip install librosa numpy scipy soundfile pyrubberband
"""

import argparse
import sys
import numpy as np
from pathlib import Path

# ---------------------------------------------------------------------------
# Music theory: note names and diatonic scale logic
# ---------------------------------------------------------------------------

NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]

SCALE_INTERVALS = {
    "major": [0, 2, 4, 5, 7, 9, 11],
    "natural_minor": [0, 2, 3, 5, 7, 8, 10],
    "harmonic_minor": [0, 2, 3, 5, 7, 8, 11],
    "melodic_minor": [0, 2, 3, 5, 7, 9, 11],
    "dorian": [0, 2, 3, 5, 7, 9, 10],
    "phrygian": [0, 1, 3, 5, 7, 8, 10],
    "lydian": [0, 2, 4, 6, 7, 9, 11],
    "mixolydian": [0, 2, 4, 5, 7, 9, 10],
    "locrian": [0, 1, 3, 5, 6, 8, 10],
}


def note_name_to_index(name: str) -> int:
    """Convert a note name like 'C', 'F#', 'Bb', 'Eb' to a chromatic index 0-11."""
    name = name.strip().upper()
    # Handle flat notation: 'b' suffix means one semitone down
    if len(name) > 1 and name[1] == "B":
        base_name = name[0]
        base_idx = NOTE_NAMES.index(base_name)
        return (base_idx - 1) % 12
    return NOTE_NAMES.index(name)


def build_diatonic_set(key_name: str, scale_name: str) -> set:
    """Return the set of MIDI note numbers (0-127) that are diatonic to the key/scale."""
    key_idx = note_name_to_index(key_name)
    intervals = SCALE_INTERVALS.get(scale_name.lower())
    if intervals is None:
        raise ValueError(f"Unknown scale: {scale_name}. Choices: {list(SCALE_INTERVALS.keys())}")
    # Build the set of pitch classes (0-11) in the scale
    pitch_classes = {(key_idx + interval) % 12 for interval in intervals}
    # Expand to all MIDI notes 0-127
    return {midi for midi in range(128) if (midi % 12) in pitch_classes}


def nearest_diatonic_note(midi_note: int, diatonic_set: set) -> int:
    """Find the nearest MIDI note (by semitone distance) that is in the diatonic set."""
    if midi_note in diatonic_set:
        return midi_note
    # Search upward and downward
    for delta in range(1, 12):
        up = midi_note + delta
        down = midi_note - delta
        if up <= 127 and up in diatonic_set:
            return up
        if down >= 0 and down in diatonic_set:
            return down
    return midi_note  # fallback (shouldn't happen)


def freq_to_midi(freq: float) -> float:
    """Convert frequency in Hz to fractional MIDI note number (A4=440Hz -> 69)."""
    return 69.0 + 12.0 * np.log2(np.maximum(freq, 1e-10) / 440.0)


# ---------------------------------------------------------------------------
# Mode A: CQT-based diatonic quantization (polyphonic)
# ---------------------------------------------------------------------------

def diatonic_quantize_cqt(
    audio: np.ndarray,
    sr: float,
    key_name: str,
    scale_name: str,
    blend: float = 1.0,
    softness: int = 2,
    fmin: float = 32.7,  # C1
    n_bins: int = 84,     # 7 octaves * 12 = C1..C8
    attenuation: float = 0.15,  # how much to attenuate non-diatonic bins (0=full mute, 1=passthrough)
) -> np.ndarray:
    """
    Quantize audio to a diatonic scale using CQT-domain masking.

    This works by computing a Constant-Q Transform (with 12 bins per octave,
    so each bin corresponds to one semitone), then attenuating bins that
    fall on non-diatonic notes. The result is reconstructed via inverse CQT.

    This naturally handles polyphonic audio (chords) because each note in the
    chord occupies a different set of CQT bins; non-diatonic notes within the
    chord are attenuated while diatonic ones pass through.

    Parameters
    ----------
    audio : 1D numpy array, mono audio samples
    sr : sample rate
    key_name : root note of the key (e.g., 'C', 'F#')
    scale_name : scale type (e.g., 'major', 'natural_minor')
    blend : 0.0 = dry, 1.0 = fully quantized
    softness : Gaussian blur sigma across neighboring bins (reduces artifacts)
    fmin : lowest frequency for CQT (default C1 = 32.7 Hz)
    n_bins : total CQT bins (7 octaves at 12 bins/octave = 84)
    attenuation : how much to attenuate non-diatonic bins (0.0 = full mute)
    """
    import librosa

    # 1. Harmonic-Percussive Source Separation.
    #    Transients and noise shouldn't be pitch-quantized; we only process
    #    the harmonic (tonal) part and mix the percussive part back untouched.
    y_harm, y_perc = librosa.effects.hpss(audio, margin=(1.0, 1.0))

    # 2. Forward CQT with 12 bins per octave.
    #    Each frequency bin is centered on a semitone, so bin k = note at
    #    fmin * 2^(k/12). This is the key property that makes diatonic
    #    quantization natural in the CQT domain.
    hop_length = 256
    C = librosa.cqt(
        y_harm,
        sr=sr,
        hop_length=hop_length,
        fmin=fmin,
        n_bins=n_bins,
        bins_per_octave=12,
        window="hann",
    )
    mag = np.abs(C)
    phase = np.angle(C)
    n_freq_bins, n_frames = C.shape

    # 3. Build a per-bin per-frame diatonic attenuation mask.
    #    For each CQT bin, we determine its MIDI note from its frequency.
    #    Bin k has center frequency: fmin * 2^(k / bins_per_octave).
    diatonic_set = build_diatonic_set(key_name, scale_name)

    # Compute actual center frequencies of each CQT bin
    bin_freqs = fmin * 2.0 ** (np.arange(n_freq_bins) / 12.0)
    bin_midi = freq_to_midi(bin_freqs)  # fractional MIDI note for each bin

    # Build attenuation mask: 1.0 for diatonic bins, < 1.0 for non-diatonic.
    # We round the fractional MIDI note to the nearest integer semitone.
    mask = np.ones(n_freq_bins, dtype=np.float32)
    for b in range(n_freq_bins):
        midi_int = int(round(bin_midi[b]))
        if 0 <= midi_int <= 127:
            if midi_int not in diatonic_set:
                mask[b] = attenuation
        # else: bins near the edges keep mask=1.0

    # 4. Apply spectral softening to the mask itself (smooth transitions
    #    between diatonic and non-diatonic regions to reduce artifacts).
    if softness > 0:
        from scipy.ndimage import gaussian_filter1d
        mask = gaussian_filter1d(mask.astype(np.float64), sigma=softness / 2.0).astype(np.float32)
        mask = np.clip(mask, 0.0, 1.0)

    # 5. Apply mask: attenuate non-diatonic bins in the magnitude spectrum,
    #    preserving phase of all bins.
    #    Broadcast mask (n_bins,) across frames: (n_bins, n_frames).
    mag_quantized = mag * mask[:, np.newaxis]

    # 6. Reconstruct complex CQT from modified magnitude + original phase.
    C_quantized = mag_quantized * np.exp(1j * phase)

    # 7. Inverse CQT to get back to time domain.
    y_quantized = librosa.icqt(
        C_quantized,
        sr=sr,
        hop_length=hop_length,
        fmin=fmin,
        bins_per_octave=12,
        window="hann",
    )

    # 8. Trim/pad to match original length.
    if len(y_quantized) > len(audio):
        y_quantized = y_quantized[: len(audio)]
    elif len(y_quantized) < len(audio):
        y_quantized = np.pad(y_quantized, (0, len(audio) - len(y_quantized)))

    # 9. Cross-fade between dry harmonic and quantized harmonic.
    result = blend * y_quantized + (1.0 - blend) * y_harm

    # 10. Mix percussive component back unmodified + light limiting.
    result = result + y_perc
    peak = np.max(np.abs(result))
    if peak > 0.99:
        result = result * (0.95 / peak)

    return result.astype(np.float32)


# ---------------------------------------------------------------------------
# Mode B: STFT-based diatonic spectral filter (higher resolution, polyphonic)
# ---------------------------------------------------------------------------

def diatonic_quantize_stft(
    audio: np.ndarray,
    sr: float,
    key_name: str,
    scale_name: str,
    blend: float = 1.0,
    attenuation: float = 0.05,
    smooth_hz: float = 5.0,  # Gaussian smoothing sigma in Hz (keep small!)
) -> np.ndarray:
    """
    Quantize audio using a high-resolution STFT mask.

    Unlike the CQT approach (which only has 1 bin per semitone), the STFT
    approach uses a large FFT to get fine frequency resolution, then
    attenuates frequency bins that fall on non-diatonic notes. This gives
    much better separation between adjacent semitones.

    The mask is smoothed in the frequency direction (Gaussian kernel with
    `smooth_hz` sigma) to prevent brick-wall filter artifacts (ringing).

    Parameters
    ----------
    audio : 1D numpy array, mono audio samples
    sr : sample rate
    key_name : root note of the key
    scale_name : scale type
    blend : 0.0 = dry, 1.0 = fully quantized
    attenuation : how much to attenuate non-diatonic bins (0.0 = full mute)
    smooth_hz : Gaussian smoothing sigma in Hz (higher = softer transitions)
    """
    import librosa
    from scipy.signal import stft as scipy_stft, istft as scipy_istft
    from scipy.ndimage import gaussian_filter1d

    # 1. HPSS to protect transients.
    y_harm, y_perc = librosa.effects.hpss(audio, margin=(1.0, 1.0))

    # 2. Forward STFT with high frequency resolution.
    #    n_fft=8192 gives ~5.4 Hz per bin at 44.1 kHz — enough to separate
    #    semitones even at low frequencies (C2=65.4 Hz has ~12 bins/semitone).
    n_fft = 8192
    hop_length = n_fft // 4  # 75% overlap
    f, t, Zxx = scipy_stft(y_harm, sr, nperseg=n_fft, noverlap=n_fft - hop_length, window='hann')
    mag = np.abs(Zxx)
    phase = np.angle(Zxx)

    # 3. Build diatonic mask in the STFT domain.
    diatonic_set = build_diatonic_set(key_name, scale_name)

    # For each STFT frequency bin, determine its MIDI note
    bin_midi = freq_to_midi(f)  # fractional MIDI note per bin
    n_freq_bins = len(f)

    mask = np.ones(n_freq_bins, dtype=np.float32)
    for b in range(n_freq_bins):
        midi_int = int(round(bin_midi[b]))
        if 0 <= midi_int <= 127:
            if midi_int not in diatonic_set:
                mask[b] = attenuation

    # 4. Smooth the mask in Hz to prevent ringing artifacts.
    #    Convert smooth_hz to bin indices.
    freq_resolution = f[1] - f[0]  # Hz per bin
    sigma_bins = smooth_hz / freq_resolution
    if sigma_bins > 0.5:
        mask = gaussian_filter1d(mask.astype(np.float64), sigma=sigma_bins).astype(np.float32)
        mask = np.clip(mask, 0.0, 1.0)

    # 5. Apply mask to magnitude spectrum.
    mag_quantized = mag * mask[:, np.newaxis]

    # 6. Reconstruct complex STFT and invert.
    Zxx_quantized = mag_quantized * np.exp(1j * phase)
    _, y_quantized = scipy_istft(
        Zxx_quantized, sr, nperseg=n_fft, noverlap=n_fft - hop_length, window='hann'
    )

    # 7. Trim/pad to match original length.
    if len(y_quantized) > len(audio):
        y_quantized = y_quantized[: len(audio)]
    elif len(y_quantized) < len(audio):
        y_quantized = np.pad(y_quantized, (0, len(audio) - len(y_quantized)))

    # 8. Cross-fade between dry harmonic and quantized harmonic.
    result = blend * y_quantized + (1.0 - blend) * y_harm

    # 9. Mix percussive component back unmodified + limiting.
    result = result + y_perc
    peak = np.max(np.abs(result))
    if peak > 0.99:
        result = result * (0.95 / peak)

    return result.astype(np.float32)


# ---------------------------------------------------------------------------
# Mode C: Phase vocoder + pYIN pitch detection (monophonic baseline)
# ---------------------------------------------------------------------------

def diatonic_quantize_pvoc(
    audio: np.ndarray,
    sr: float,
    key_name: str,
    scale_name: str,
) -> np.ndarray:
    """
    Quantize monophonic audio using pYIN pitch detection and phase vocoder shifting.

    For each frame with a detected pitch:
      1. Quantize to nearest diatonic note
      2. Compute shift ratio (target_freq / detected_freq)
      3. Apply pitch shift to that frame

    This path only works well for monophonic (single-note) material.
    """
    import librosa

    diatonic_set = build_diatonic_set(key_name, scale_name)

    # 1. Pitch detection via pYIN
    hop_length = 256
    f0, voiced_flag, _ = librosa.pyin(
        audio.astype(np.float64),
        fmin=librosa.note_to_hz("C2"),
        fmax=librosa.note_to_hz("C7"),
        sr=sr,
        hop_length=hop_length,
    )
    # f0 is NaN where unvoiced
    n_frames = len(f0)
    time_per_frame = hop_length / sr

    # 2. For each voiced frame, compute shift ratio
    #    We'll build a pitch shift curve and use rubberband
    semitone_shifts = np.zeros(n_frames)
    for i in range(n_frames):
        if voiced_flag[i] and not np.isnan(f0[i]) and f0[i] > 0:
            detected_midi = freq_to_midi(f0[i])
            nearest_midi = nearest_diatonic_note(int(round(detected_midi)), diatonic_set)
            semitone_shifts[i] = float(nearest_midi) - detected_midi
        else:
            semitone_shifts[i] = 0.0

    # 3. Smooth the shift curve to avoid discontinuities
    if n_frames > 3:
        from scipy.ndimage import gaussian_filter1d
        semitone_shifts = gaussian_filter1d(semitone_shifts, sigma=1.0)

    # 4. Apply pitch shift using pyrubberband
    import pyrubberband as pyrb

    # pyrubberband works on the whole file with a single shift.
    # For per-frame shifting, we need to split into segments or use
    # rubberband's pitch_map feature.
    #
    # Simplified approach: use the median shift amount (works for steady pitches)
    valid_shifts = semitone_shifts[voiced_flag & ~np.isnan(f0)]
    if len(valid_shifts) == 0:
        return audio.astype(np.float32)

    median_shift = float(np.median(valid_shifts))

    # Rubberband pitch shift (in semitones)
    shifted = pyrb.pitch_shift(audio.astype(np.float32), sr, median_shift)

    # Trim/pad
    if len(shifted) > len(audio):
        shifted = shifted[:len(audio)]
    elif len(shifted) < len(audio):
        shifted = np.pad(shifted, (0, len(audio) - len(shifted)))

    peak = np.max(np.abs(shifted))
    if peak > 0.99:
        shifted = shifted * (0.95 / peak)

    return shifted.astype(np.float32)


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def main():
    parser = argparse.ArgumentParser(
        description="Diatonic pitch shifting POC — quantize audio to stay in a musical key/scale.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  # CQT mode (polyphonic): quantize a synth pad chord to C major
  python diatonic_pitch_shift.py chord.wav out.wav --key C --scale major --mode cqt

  # Phase vocoder mode (monophonic): auto-tune a vocal to D minor
  python diatonic_pitch_shift.py vocal.wav out.wav --key D --scale natural_minor --mode pvoc

  # CQT with softer redistribution (fewer artifacts) and dry/wet blend
  python diatonic_pitch_shift.py chord.wav out.wav --key C --mode cqt --softness 4 --blend 0.8
        """,
    )
    parser.add_argument("input", type=str, help="Input WAV file")
    parser.add_argument("output", type=str, help="Output WAV file")
    parser.add_argument("--key", type=str, default="C", help="Root note (default: C)")
    parser.add_argument(
        "--scale", type=str, default="major",
        help=f"Scale type (default: major). Choices: {list(SCALE_INTERVALS.keys())}"
    )
    parser.add_argument(
        "--mode", type=str, default="stft", choices=["stft", "cqt", "pvoc"],
        help="Quantization mode: stft (polyphonic, high-res), cqt (polyphonic, note-aligned), pvoc (monophonic baseline)"
    )
    parser.add_argument(
        "--blend", type=float, default=1.0,
        help="Dry/wet blend: 0.0 = dry, 1.0 = fully quantized (default: 1.0)"
    )
    parser.add_argument(
        "--softness", type=int, default=2,
        help="Spectral softening radius for CQT mode (default: 2, higher = fewer artifacts)"
    )
    parser.add_argument(
        "--fmin", type=float, default=32.7,
        help="Lowest CQT frequency in Hz (default: 32.7 = C1)"
    )
    parser.add_argument(
        "--no-hpss", action="store_true",
        help="Disable HPSS pre-processing (skip harmonic/percussive separation)"
    )
    parser.add_argument(
        "--attenuation", type=float, default=0.15,
        help="Attenuation factor for non-diatonic bins: 0.0 = full mute, 1.0 = passthrough (default: 0.15)"
    )

    args = parser.parse_args()

    # Validate input
    input_path = Path(args.input)
    if not input_path.exists():
        print(f"Error: Input file not found: {args.input}", file=sys.stderr)
        sys.exit(1)

    # Validate key — accept sharps (#) and flats (b)
    key_clean = args.key.strip()
    try:
        note_name_to_index(key_clean)
    except (ValueError, IndexError):
        print(f"Error: Invalid key '{args.key}'. Use note names like C, C#, D, Eb, F#, etc.",
              file=sys.stderr)
        sys.exit(1)

    # Load audio
    import soundfile as sf
    try:
        audio, sr = sf.read(input_path, dtype="float32", always_2d=False)
    except Exception as e:
        print(f"Error reading {args.input}: {e}", file=sys.stderr)
        sys.exit(1)

    # Convert to mono for processing
    if audio.ndim == 2:
        audio = np.mean(audio, axis=1)
    audio = audio.astype(np.float32)

    print(f"Input:  {args.input}  ({len(audio)/sr:.2f}s, {sr} Hz, mono)")
    print(f"Key:    {args.key} {args.scale}")
    print(f"Mode:   {args.mode}")

    # Process
    if args.mode == "stft":
        print("Running STFT diatonic spectral filter (polyphonic, high-res)...")
        output = diatonic_quantize_stft(
            audio, sr,
            key_name=args.key,
            scale_name=args.scale,
            blend=args.blend,
            attenuation=args.attenuation,
        )
    elif args.mode == "cqt":
        print("Running CQT diatonic quantizer (polyphonic mode)...")
        output = diatonic_quantize_cqt(
            audio, sr,
            key_name=args.key,
            scale_name=args.scale,
            blend=args.blend,
            softness=args.softness,
            fmin=args.fmin,
            attenuation=args.attenuation,
        )
    else:
        print("Running phase vocoder + pYIN (monophonic mode)...")
        output = diatonic_quantize_pvoc(
            audio, sr,
            key_name=args.key,
            scale_name=args.scale,
        )

    # Write output
    sf.write(args.output, output, int(sr), subtype="FLOAT")
    print(f"Output: {args.output}  ({len(output)/sr:.2f}s)")
    print("Done.")


if __name__ == "__main__":
    main()
