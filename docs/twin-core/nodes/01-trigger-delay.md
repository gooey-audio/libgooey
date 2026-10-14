# 1 · Trigger delay

**Holds back the body strike while the noise fires on time.**

## What it does

A trigger starts two things at once: the body (the resonators) and the noise. The trigger delay postpones only the body, by up to 75 ms. The noise envelope still starts at the moment of the trigger.

That gap is what makes a hit sound like more than one event. Claps, flams, "thick" snares and slapback-style percussion all come from noise arriving slightly before the tone.

## How it works

When `trigger_delay` is zero, the exciter fires on the trigger sample. Otherwise the voice arms the body and fires the exciter on the first sample where the time since the trigger is at least the delay. Every body envelope (pitch sweep, VCA, LPG, chaos gate) is measured from that delayed start, so a delayed hit has the same shape as an undelayed one, just later.

## Parameters

| Parameter | Range | Notes |
|---|---|---|
| `trigger_delay` | 0–75 ms (linear) | 0 keeps body and noise in sync |

## Connections

- **In:** the trigger (TRIG).
- **Out:** fires the [exciter](02-exciter.md).
- **Unaffected:** the [noise](09-noise.md) path, which starts immediately.

## Sound design

- **Clap:** 15–25 ms delay, noise in BODY or HP mode, short length.
- **Snare with weight:** a few milliseconds of delay softens the attack and separates the snap from the drum.
- **Flam or echo:** longer delays (40–75 ms) with a short noise decay read as two separate hits.
