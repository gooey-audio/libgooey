# 8 · Vactrol LPG

**A low-pass gate after the VCA. Brightness and level fall together, the way a struck object darkens as it dies away.**

## What it does

A low-pass gate is a filter and a VCA in one, controlled by a vactrol (an LED shining on a light-dependent resistor). As the control falls, the gate closes: highs disappear first, then the level. The vactrol responds quickly to rising light and slowly to falling light, which gives the soft, rounded "plonk" of Buchla-style percussion.

With `lpg` at 0 the gate is bypassed. Turning it up blends in the gated signal, so the tail of each hit gets progressively darker.

## How it works

The model follows Parker and D'Angelo, *A Digital Model of the Buchla Lowpass-Gate* (DAFx-13), and lives in `src/filters/vactrol_lpg.rs`.

- **Control.** The body envelope, cubed, drives the vactrol LED. Cubing puts the filter sweep in the audible part of the decay instead of its last 40 dB.
- **Vactrol memory.** A one-pole lag on the LED current, with time constants from the VTL5C3/2 datasheet:
  - About 12 ms to rise.
  - A slower fall, set to 3% of `length` and capped at the datasheet's 250 ms.
  - Both respond faster when already bright.
- **Resistance.** The paper's datasheet fit: Rf = 3.464 / I^1.4 + 1136 Ω, for LED currents from 10 µA to 40 mA.
- **Audio path.** The 292's "Both" mode: a passive two-pole network (C1 = 1 nF, C2 = 220 pF, Rα = 5 MΩ). It is discretized with trapezoidal integrators in topology-preserving form, so the resistance can change every sample without zipper noise or instability. Cutoff and gain both fall as Rf rises.

The gate sits after the VCA, so `length` keeps its meaning: the LPG darkens the decay rather than lengthening it.

## Parameters

| Parameter | Range | Effect |
|---|---|---|
| `lpg` (new) | 0–1 | Wet amount of the gated signal |

The gate's timing follows `length` from the [body VCA](07-body-vca.md).

## Connections

- **In:** the [body VCA](07-body-vca.md) output; the body envelope as control.
- **Out:** the output summer, alongside direct noise.

## Sound design

- **Plucked or struck wood:** `lpg` 1 with `harmonics` 0.3–0.6 and a short `length`.
- **Natural decay on metals:** `lpg` 0.5 tames the long bright tails of the metallic preset.
