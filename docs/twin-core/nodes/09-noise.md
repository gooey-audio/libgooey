# 9 · Noise

**Filtered noise with its own envelope: mixed in for snares and claps, or used to excite the cores.**

## What it does

The noise path is a second sound source, independent of the body. It supplies snare wires, clap texture, hi-hat sizzle, or a breathy attack on a kick. It has its own filter, decay and level. It can also be triggered on its own (N-TRIG) without striking the body.

## How it works

White noise from a seeded random generator passes through a state-variable filter at `noise_filter` (Q 0.707). Its level is:

- an exponential decay over `noise_decay`,
- × (`noise_bias` + 1) / 2,
- × the square root of the noise velocity.

The noise mode decides where it goes:

| Mode | Route | Use |
|---|---|---|
| **LP** | Low-passed noise mixed straight into the output | Breathy attacks, soft snares |
| **HP** | High-passed noise mixed straight into the output | Snare wires, hats, claps |
| **BODY** | Low-passed noise (× 0.7) fed into core 1 instead of the output | Resonant noise, claps, chaotic hits; the cores colour the noise |

The mode is latched on each trigger. The noise starts on the trigger even when the body is delayed by the [trigger delay](01-trigger-delay.md). `trigger_noise_with_velocity` (N-TRIG) fires only the noise envelope.

## Parameters

| Parameter | Range | Effect |
|---|---|---|
| `noise_filter` | 20 Hz to 0.45 × sample rate (exp) | Filter cutoff |
| `noise_decay` | 0.002–20 s (exp) | Envelope length |
| `noise_bias` | −1 to 1 | Level; −1 is silent |
| `noise_mode` | LP, HP, BODY | Routing; channel index 13, set as 0–2 |

## Connections

- **In:** the trigger (TRIG) and the independent noise trigger (N-TRIG).
- **Out:** the output summer (LP/HP), or the input of [core 1](03-core-1.md) (BODY).

## Sound design

- **Snare:** HP mode, filter around 4 kHz, decay 0.2–0.35 s, bias around 0.2.
- **Clap:** BODY mode, filter around 2.5 kHz, with 15–25 ms of [trigger delay](01-trigger-delay.md).
- **Kick click:** LP mode, very short decay (around 20 ms), low bias.
