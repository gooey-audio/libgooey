# 5 · Body mode tap

**Picks which filter outputs of the two cores form the body sound.**

## What it does

Each core has a low-pass output (round, fundamental-heavy) and a band-pass output (thinner, more resonant). The body mode chooses a blend of those outputs. It acts like a LO/MID/HI emphasis switch, and it changes how the [wavefolder](06-wavefolder.md) responds downstream.

## How it works

| Mode | Output | Character |
|---|---|---|
| **LOW** | Core 2 low-pass | Full, round; kicks and bass drums |
| **MID** | 0.55 × core 1 band-pass + 0.75 × core 2 band-pass | Both pitches audible; toms, snares |
| **HIGH** | Core 2 band-pass − 0.45 × core 2 low-pass | Thin, ringing, low end removed; metal and claps |

The mode is latched when a hit starts. Changing it mid-note takes effect on the next trigger, so the sound never clicks.

## Parameters

| Parameter | Values | Notes |
|---|---|---|
| `body_mode` | LOW, MID, HIGH | Channel index 12, set as 0–2; takes effect on the next hit |

## Connections

- **In:** the low-pass and band-pass outputs of [core 1](03-core-1.md) and [core 2](04-core-2.md).
- **Out:** the [wavefolder](06-wavefolder.md).
