# 7 · Body VCA

**The amplitude envelope for the body.**

## What it does

The VCA shapes how the body fades. `length` sets how long it lasts. `body_bias` makes it punchier and more sustained, or quieter and more staccato.

## How it works

The envelope is an exponential decay that falls by 60 dB over `length`, starting when the body is struck.

**Body bias** works in both directions:

- **Positive bias:**
  - lifts the envelope's peak by up to 2.5× (+8 dB),
  - adds a small sustain floor,
  - lengthens the cores' ring (see [core 1](03-core-1.md)).

  At maximum bias and length, a hit can ring for a very long time.
- **Negative bias** scales the body down towards silence: about −6 dB at −0.5 and full mute at −1.

## Parameters

| Parameter | Range | Effect |
|---|---|---|
| `length` | 0.002–20 s (exp) | Envelope duration; also the core ring time |
| `body_bias` | −1 to 1 | Gain and sustain (+), or attenuation (−) |

## Connections

- **In:** the [wavefolder](06-wavefolder.md).
- **Out:** the [vactrol LPG](08-vactrol-lpg.md).
- **Also drives:** the LPG's control signal and the chaos feedback gate.
