# 3 · Core 1

**The first resonator. It sets the drum's pitch and carries the pitch sweep that gives a kick its punch.**

## What it does

Core 1 is a resonant two-pole filter that rings when struck, like a tuned drum head. Its pitch starts high and falls to `tune` over an envelope. That fast downward sweep is what makes an 808-style kick thump.

## How it works

Core 1 is a topology-preserving state-variable resonator. Its state is kept in 64-bit floats so very low, very long notes don't fade into rounding noise.

**Pitch.** Each sample, the pitch is computed as follows:

- Start from `tune`, or from the MIDI note when one is set.
- Shift by the channel tuning control (±1 octave).
- Multiply by 2^(`fm_depth` × FM envelope). The FM envelope is an exponential decay from 1 to 0 over `fm_decay`.
- Positive depth sweeps down into the note; negative depth sweeps up into it.

**Decay.** The ring time (T60) is `length` × (0.8 + 2.2 × positive `body_bias`). Raising body bias lets the cores sustain much longer.

**Input.** Core 1's input is the sum of three sources:

- the [exciter](02-exciter.md) pulse,
- BODY-mode noise, when that mode is on,
- the chaos feedback from [core 2](04-core-2.md).

### Chaos, part 1: Duffing stiffness (new)

From 0 to 0.5, `chaos` adds a hardening spring to both cores. The extra restoring force is −k·y·tanh²(level), where:

- *y* is the core's displacement (its low-pass output),
- *k* rises to 12 by chaos 0.5,
- the level is normalized to a 200 Hz reference so kicks and bells respond alike.

The force is solved inside the resonator's zero-delay Newton loop, so it doesn't pump energy in the way per-sample retuning would. A hard hit therefore rings sharp and glides down as it decays, while a soft hit barely bends. At chaos 0.5, a velocity-1 hit starts roughly 30% sharp. Real drum heads do the same.

## Parameters

| Parameter | Range | Effect here |
|---|---|---|
| `tune` | 20–2000 Hz (exp) | Resting pitch |
| `fm_depth` | −5 to 5 octaves | Size and direction of the pitch sweep |
| `fm_decay` | 0.002–8 s (exp) | Speed of the pitch sweep |
| `length` | 0.002–20 s (exp) | Ring time, with `body_bias` |
| `body_bias` | −1 to 1 | Positive values lengthen the ring |
| `chaos` | 0–0.5 | Duffing stiffness (above 0.5 see [core 2](04-core-2.md)) |

## Connections

- **In:** exciter pulse, BODY-mode noise, chaos feedback from core 2.
- **Out:** its low-pass output feeds [core 2](04-core-2.md) in series. It also drives core 2's pitch through **cross mod**, and its band-pass output feeds the MID [body mode](05-body-mode-tap.md).

## Sound design

- **Kick:** low `tune` (40–60 Hz), `fm_depth` around +3 octaves, `fm_decay` 30–80 ms.
- **Tom:** higher `tune`, less sweep, longer `fm_decay`.
- **Rubbery, velocity-sensitive hits:** `chaos` around 0.3; play with velocity.
