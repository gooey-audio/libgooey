# 4 · Core 2

**The second resonator, in series after core 1 and tuned below it. Cross mod and chaos make the two cores interact.**

## What it does

Core 2 rings at a second pitch, `detune` octaves below core 1. Because it is fed by core 1, the body carries two resonances. That dyad is what lets the voice move from a clean kick (cores close together) to toms, snares and metal (cores far apart).

## How it works

Core 2 is the same kind of resonator as [core 1](03-core-1.md), with the same ring time. Its input is core 1's low-pass output. Its pitch is core 1's pitch (including the sweep) × 2^(−`detune`), never below 20 Hz.

### Cross mod (new)

`cross_mod` lets core 1 bend core 2's pitch at audio rate. Each sample, core 2's frequency is multiplied by 2^(2 × `cross_mod` × tanh(6 × level₁)), where level₁ is core 1's output normalized to a 200 Hz reference. At full cross mod that is up to ±2 octaves.

This is frequency modulation between two decaying oscillators, so it produces inharmonic sidebands: bells, cowbells, gongs and clang. Because it follows core 1's level, it is strongest at the strike and fades as the hit decays.

Core 2's damping is computed from its unmodulated pitch, so cross mod does not change how long the voice rings.

### Chaos, part 2: feedback (new)

Above 0.5, `chaos` closes a loop from core 2 back into core 1. Core 2's band-pass output is normalized, saturated with tanh, scaled by up to 1.5 and added to core 1's input.

- The band-pass tap has no DC, so the loop cannot latch into a stuck offset.
- The feedback is multiplied by the body envelope, so the loop fades with `length` and every hit still ends.

Two stiffened resonators fighting each other give the unstable, shifting bodies of Plumbutter, Pulsar-23 or Kastle Drum-style percussion.

## Parameters

| Parameter | Range | Effect here |
|---|---|---|
| `detune` | 0–2.5 octaves (linear) | Interval below core 1 |
| `cross_mod` | 0–1 | Core 1 to core 2 pitch modulation, up to ±2 octaves |
| `chaos` | 0.5–1 | Core 2 to core 1 saturating feedback |

## Connections

- **In:** core 1's low-pass output (audio) and core 1's level (cross mod).
- **Out:** low-pass and band-pass taps to the [body mode tap](05-body-mode-tap.md); band-pass back to core 1 when chaos > 0.5.

## Sound design

- **Tight kick:** `detune` near 0; the cores reinforce one pitch.
- **Tom or snare body:** `detune` around 0.5–0.7 octaves, MID body mode.
- **Bell or cowbell:** `detune` above 1 octave, `cross_mod` 0.4–0.8, HIGH body mode.
- **Unstable, gnarly:** `chaos` 0.7–1 with a longer `length`.
