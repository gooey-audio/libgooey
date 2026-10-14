# 6 · Wavefolder

**Adds harmonics by folding the waveform back on itself. Symmetry tilts the fold to add even harmonics.**

## What it does

A wavefolder drives the signal past a limit, then reflects it back instead of clipping it flat. Gentle folding adds brightness and bite; heavy folding turns a sine-like ring into a complex, buzzy tone.

Because the cores decay, the fold also changes over time. The loud start folds most, and the tail gets cleaner. Velocity therefore controls timbre as well as level.

## How it works

The folder runs at 2× oversampling to keep aliasing down.

- **Harmonics.** `harmonics` (h) drives the signal by 1 + 7h into a triangle fold, then crossfades between dry and folded by h. At 0 the folder is bypassed.
- **Symmetry (new).** `symmetry` shifts the fold point by up to a quarter of the fold period, after the drive.
  - A centred fold adds only odd harmonics. Shifting it makes the fold lopsided, which adds even harmonics (octaves and fifths above the fundamental): a hollower, more "blown" sound.
  - The static part of the shift is subtracted.
  - An 8 Hz DC trap after the folder removes the slow offset left by folding a decaying tone.
- Symmetry has no effect while `harmonics` is 0, as on the hardware.

## Parameters

| Parameter | Range | Effect |
|---|---|---|
| `harmonics` | 0–1 | Fold depth and wet amount |
| `symmetry` (new) | −1 to 1 | Fold offset; 0 is symmetric, ±1 gives the most even harmonics |

## Connections

- **In:** the [body mode tap](05-body-mode-tap.md).
- **Out:** the [body VCA](07-body-vca.md).

## Sound design

- **A touch of punch:** `harmonics` 0.1–0.2 on a kick in LOW mode.
- **Snare crack:** `harmonics` 0.4–0.6 in MID mode.
- **West Coast "bongo":** `harmonics` 0.5+, `symmetry` 0.5, short `length`, with the [LPG](08-vactrol-lpg.md) up.
