# 2 · Exciter

**A short pulse that strikes the first resonator.**

## What it does

The exciter is the stick hitting the drum. It produces a single 0.75 ms pulse that is summed into core 1's input. Core 1 then rings at its own frequency; the pulse itself is never heard directly.

## How it works

On each body start the exciter fires a pulse with amplitude 0.9 × velocity. A short pulse contains energy across a wide band, so it can excite the cores at any tuning. Harder hits put more energy into the cores, which:

- makes them louder,
- folds harder in the [wavefolder](06-wavefolder.md),
- pushes them further into their nonlinear range when [chaos](03-core-1.md) is up.

That is how velocity changes timbre, not just level.

Before core 1, the pulse is summed with two other sources:

- low-passed noise, when the noise mode is BODY ([node 9](09-noise.md)),
- the chaos feedback from core 2 ([node 4](04-core-2.md)).

## Parameters

None of its own. Its level follows **velocity**.

## Connections

- **In:** fired by the [trigger delay](01-trigger-delay.md).
- **Out:** the input summer of [core 1](03-core-1.md).
