# Plate engine: implementation plan

Companion to `features/plate-engine-libgooey.md` (the API contract) and `docs/synthesis-research.md` (why). Most of the work is in libgooey (Rust, `$GOOEY_DIR/libgooey`); Ripple's part is the last phase. libgooey keeps execution plans in its own `plans/` directory, so copy this file there as `plans/plate-engine-plan.md` when the libgooey work starts.

## Goal

A seventh channel instrument, `INSTRUMENT_PLATE = 7`: a bank of 32 pitch-tracked resonant modes with strike and pickup position, energy-driven tension pitch glide and a cubic mode cascade. Covers ride, crash, bell, timbale and gong from one engine. Reaches Ripple through the existing generic channel calls, so Ripple adds an `Instrument` case, a `PlateParam` enum, formatters, LFO targets and a picker button.

## Signal flow (per sample)

```
trigger(vel) ─► Exciter (kind + width from Hardness, amp √vel) ─► e
for k in 0..N:   y_k = mode_k.process(e · w_strike[k])        (Resonator core, feedback 0)
                 sum += y_k · w_pickup[k] · g_k
E  = one-pole follower of sum²  (10 ms)                       → tension glide multiplier m(E)
cascade: 4 bands of sum ─► cubic (2x OS) ─► fed one band up    → shimmer
out = brightness LP( (1 − cascade·deplete)·sum + cascade·shimmer ) ─► soft clip
```

Mode frequencies `f_k = f0 · tuning · ratio_k(material)^(1+geometry) · m(E)` and the per-mode T60 are recomputed at control rate (every 32 samples, same cadence as automation in `ffi.rs`), not per sample, because `Resonator::set_frequency` recomputes a `tan()`.

## Phase 1: DSP voice (`src/instruments/plate.rs`)

Model the file on `src/instruments/twin_core_perc.rs` (config struct, `SmoothedParam` array, `Instrument` + `Modulatable` impls, `set_parameter_normalized` with exponential indices, `set_channel_tuning`, `set_ring_limit_secs`, `set_seed`).

1. **Mode tables.** `PlateMaterial { Membrane, Plate, Bar, Bell, Sheet }` with `const fn ratios() -> [f32; MAX_MODES]`:
   - Membrane: circular membrane Bessel-zero ratios (1, 1.594, 2.136, 2.296, 2.653, 2.918, 3.156, 3.501, 3.600, 3.652, ...), extended past the table by `≈ 0.25 + 0.56·k`.
   - Plate: free circular plate (1, 1.73, 2.33, 3.91, 4.11, 6.30, ...), dense inharmonic tail `k^1.12` with seeded jitter.
   - Bar: `1, 2.76, 5.40, 8.93`, then `((2k+1)/3)^2`.
   - Bell: hum 0.5, prime 1, tierce 1.2, quint 1.5, nominal 2, then 2.5, 2.67, 3, 4, 4.5, ... from a bell partial table.
   - Sheet: `k^0.9 · (1 + 0.04·jitter(seed))`, dense and almost harmonic, for metal-sheet thunder.
   - Each table also carries a per-mode gain shape `g_k` (roll-off `1 / ratio_k^0.5`) and a damping shape; modes whose frequency exceeds `0.45 · sr` get `g_k = 0`.
2. **`PlateConfig`** with fields matching the 12 params plus `seed`, and factory constructors `ride()`, `crash()`, `bell()`, `timbale()`, `gong()`.
3. **`PlateVoice`** fields: `modes: [Resonator; MAX_MODES]` (`MAX_MODES = 32`; `plate-hq` Cargo feature raises it to 64), `w_strike`, `w_pickup`, `mode_gain`, `exciter: Exciter`, `params: [SmoothedParam; 11]`, `channel_tuning`, `pending_material / active_material`, `energy: f32`, `cascade_bands: [StateVariableFilterTpt; 4]`, `cascade_os: [Oversampler2x; 4]`, `brightness: OnePole`, `control_counter: u32`, `seed`, `velocity`, `active`, `midi_note`, `ring_limit_secs`.
4. **Trigger.** Latch material and strike position. `w_strike[k] = sin(π · (0.05 + 0.95·pos) · (k+1))` (1-D stand-in; a 2-D product for plate/membrane is a follow-up). Exciter width `= hardness_ms · (1 − 0.3·vel)`, kind by thirds of Hardness (Pulse, ClickTable, NoiseBurst), amplitude `√vel · 0.6`. Call `exciter.trigger(amp, seed ^ hit_counter)`.
5. **Tension glide.** `E` is a 10 ms one-pole of `sum²`, normalised so a full-velocity ride peaks near 1. `m(E) = sqrt(1 + tension · 2 · clamp(E, 0, 1))` (at tension 1 that is about one octave at full energy). Applied in the control-rate frequency update, so the glide is smooth and bounded.
6. **Cascade.** Band edges relative to f0: `[1, 3, 8, 20, 60]·f0`, clamped below `0.45·sr`. For band i: `n_i = cubic(band_i)` inside `Oversampler2x::process` (cubic is `x − x³/3` soft-saturating so it is bounded), then `shimmer += highpass_{i+1}(n_i)`. Output `(1 − cascade·deplete)·sum + cascade·shimmer`. Four SVFs and four 2x cubic stages per sample is cheaper than one extra mode.
7. **Output.** One-pole brightness LP (500 Hz to 20 kHz, exp), `tanh` soft clip, fixed unity level (no voice volume). `is_active` goes false when the exciter is inactive and every mode `is_quiet()`; check every 64 samples.
8. **Smoothing and LFO safety.** Pickup position, tension, cascade, deplete, brightness are `SmoothedParam` (15 ms) and are read every sample or control block, so routed LFOs move them without clicks. Strike position and material latch on trigger (`LfoTarget::Mode` for material).
9. **Allocation.** Everything is fixed-size arrays. Constructed off the render path in `gooey_engine_set_channel_instrument_type`, as for Twin Core.

## Phase 2: prototype and tuning (`examples/plate.rs`)

Copy `examples/reso_kick.rs`. Keys cycle the five presets and nudge Pitch, Strike Pos, Tension, Cascade. Use `--features native,crossterm`. Tune the five factory configs here before touching the FFI. Also render each preset to WAV with `bounce` and inspect spectrograms (`examples/aliasing_plots.rs` shows how) to confirm the cascade moves energy above 4 kHz on `crash` and that `tension` glides pitch down on hard hits.

## Phase 3: FFI (`src/ffi.rs`)

1. Constants: `INSTRUMENT_PLATE = 7`, `INSTRUMENT_COUNT = 8`, `PLATE_PARAM_*` 0 to 11 and `PLATE_PARAM_COUNT = 12`, `PLATE_PRESET_RIDE 0, CRASH 1, BELL 2, TIMBALE 3, GONG 4`, `gooey_engine_plate_param_count()`.
2. `ChannelInstrument::Plate(PlateVoice)` as its own variant (not through `PercussionEngine`, whose preset set is Kick/Tom/Snare/Clap/Metallic). Add match arms in `instrument_type`, `trigger_with_velocity`, `tick`, `is_active`, `tuning_target`, `set_tuning`, `set_param`, `get_param`, `lfo_target` (centered: pitch, geometry, pickup pos, hardness, decay, hf damping, tension, cascade, deplete, brightness; mode: material; strike pos is latched so route it as `Mode`), and the instrument-type switch at `gooey_engine_set_channel_instrument_type`. The compiler lists every missed arm since the matches are exhaustive.
3. Presets: `gooey_engine_set_channel_preset` dispatches on channel type. Plate channels accept `PLATE_PRESET_*` (0 to 4), load `PlateConfig::<preset>()`, restore tuning, clear `param_locks` and `saved_global_freq`, return false for Resonator/Twin Core ids. `gooey_engine_get_channel_preset` returns the plate preset id.
4. Param locks and macros need nothing new: `CHANNEL_PARAM_LOCK_CAPACITY` is 32, `PARAM_TARGET_DRUM` already addresses channel params by index.
5. `PlateVoice` must not be `#[repr(C)]` so cbindgen emits an opaque struct (the #262 lesson). Regenerate `include/gooey.h` and keep the CI `clang -fsyntax-only -x c include/gooey.h` step green.

## Phase 4: tests

- `tests/plate_voice.rs` (unit, no FFI): each material table is monotonic and below Nyquist; `excite` at velocity 1.0 glides the fundamental down by more than 2 percent with tension 1 and by less than 0.2 percent with tension 0 (measure zero crossings in the first and last 100 ms); crash preset moves more than 20 percent of energy above 4 kHz with cascade 1 versus under 5 percent at 0; strike position 0 and 1 produce different spectra; output is finite and bounded at every parameter extreme (sweep a grid, including tension 1 + cascade 1 + decay 20 s); voice reports inactive after decay.
- `tests/plate_channels.rs` (FFI, copy `tests/percussion_channels.rs`): type switch renders audio via the sequencer; `gooey_engine_instrument_count() == 8`; `gooey_engine_plate_param_count() == 12`; locks survive triggers; preset clears locks and keeps gain, mute, solo, tuning; blend calls are no-ops; tuning shifts pitch by ±12 st; LFO route on pickup position renders without discontinuities; no allocation on the render path (`tests/percussion_channel_alloc.rs` pattern).
- Update `tests/channel_instrument_swap.rs` and `tests/percussion_channels.rs` counts.
- CPU: a `#[ignore]` bench in `tests/plate_channels.rs` that renders 10 s of a plate channel and prints the ratio to a Twin Core channel. Target: at most 2x Twin Core at 48 kHz.

## Phase 5: release and Ripple adoption

1. libgooey: `cargo build`, `cargo build --example plate --features native,crossterm`, `cargo test --verbose`, tag `v1.2.0`, CI builds the iOS release with the header.
2. Ripple: `./scripts/upgrade-libgooey.sh v1.2.0`, `./scripts/build_libgooey_xcframework.sh --force`, `./scripts/setup-libgooey.sh local`, clear derived data (CLAUDE.md).
3. Swift (file list from the engine audit):
   - `Ripple/RippleCoreAudio/RustBridge.swift`: `Instrument.plate = 7` in `engines`, `supportsXYBlend` false, `displayName` "Plate", `shortName` "PLATE", empty `cornerLabels`, `volumeParam` nil; `PlateParam: UInt32, CaseIterable` with `displayName` and `range` (0...4 for material); `PlatePreset` enum (Ride, Crash, Bell, Timbale, Gong) and `Instrument.presets` returning it for `.plate`.
   - `Ripple/Ripple/Models/InstrumentParams.swift`: descriptors with `exponentialHz(30, 2000)`, `exponentialSeconds(0.05, 20)`, `exponentialHz(500, 20000)`, `ms` for hardness, percent for the rest, `ParamFormat.labels(["Membrane","Plate","Bar","Bell","Sheet"])`.
   - `Ripple/Ripple/Models/ModulationModel.swift`: `modulationParams` and `curatedModTargets` (Pickup Pos, Tension, Cascade, Decay, Brightness).
   - `Ripple/Ripple/Views/InstrumentSelectorPopover.swift`: PLATE button and accessibility id.
   - `Ripple/Ripple/Models/DrumKitModel.swift`: accent colour (teal is free); `SequencerModel.mutateParams` `break` case.
   - Preset modal: `PercPreset` is shared by Resonator and Twin Core; either generalise the modal over `Instrument.presets` or add a `PlatePreset` branch.
   - `FEATURES.md`: add Plate under Drum Synthesis and the LFO targets line.
4. Verify: `xcodebuild build -scheme Ripple -destination 'platform=iOS Simulator,name=iPhone 17 Pro' -quiet` and the `RippleMac` scheme (shared models), then `/pr-screenshots` for the Shape picker and PARAMS tab.

## Risks and decisions

- **Stability.** All nonlinear paths are bounded (soft cubic, `tanh`, clamped E, `Resonator::bound_state`). The parameter-grid test in Phase 4 is the gate.
- **Cost.** 32 `Resonator::process` calls with feedback 0 skip the Newton loop; the `tan()` updates happen at control rate. Expect roughly 1.5x Twin Core. If `plate-hq` 64 modes is too slow on A14, keep it desktop-only.
- **Material switching** latches on trigger to avoid retuning a ringing bank; Ripple shows it as a discrete param like Twin Core's Body Mode.
- **Strike position model** starts 1-D. A 2-D (i, j) mode enumeration for membrane and plate is a follow-up that does not change the API.
- **XY blend** stays a no-op as for the other engines; corner presets are a later `ChannelBlender` request.

## Progress

- [x] Phase 1: `src/instruments/plate.rs` (`PlateMaterial`, `PlateConfig`, `PlateVoice`).
- [x] Runtime mode count (8 to 128) as param 12, replacing the `plate-hq` feature; China, Tam-tam and Gamelan presets.
- [x] Phase 2: `examples/plate.rs` interactive lab (presets, velocity, material, 16-step pattern).
- [x] Unit tests: `tests/plate_voice.rs`.
- [ ] Phase 3: FFI channel instrument.
- [ ] Phase 4: FFI tests, CPU bench.
- [ ] Phase 5: release and Ripple adoption.

## Decision log

- **Position weights.** `sin(π(0.05 + 0.95·pos)(k+1))` is zero for every mode at pos = 1, so edge hits would be silent. The voice maps `p = 0.5 − 0.47·pos` (centre excites odd shapes only, edge is thin and bright) and normalises the weights to unit RMS so level holds across positions.
- **Cascade feeds the modes, not the output.** Bands are formed by grouping the modes by ratio (`<3`, `3–8`, `8–20`, `≥20`·f0) instead of four SVFs over the sum, which is exact and free. Each band's bounded cubic (2x oversampled) drives the modes one band up, so the shimmer rings at the upper modes' own decay and blooms after the strike. High-Q modes ignore cubic products that miss their frequency, so the injection also carries white noise enveloped by the cubic level, normalised per mode by `sqrt(4ζω)` to keep the bloom independent of decay time. The path is strictly upward (no loop), so it cannot self-oscillate.
- **Deplete** is energy-dependent extra damping of the lower three bands, `T60 / (1 + 3·deplete·cascade·E)`, rather than an output crossfade. Extra damping is always stable.
- **Mode input normalisation.** Inputs are scaled by `1/ω_k` and the exciter by its area, so mode amplitude tracks the exciter spectrum rather than the mode frequency; hardness then behaves like mallet width.
- **Test thresholds.** The cascade test asserts the crash's above-4 kHz share at least 1.5x the dry share (measured about 0.24 vs 0.11 at full velocity with a two-pole split), not the 20 % / 5 % FFT figures.

- **Mode count is a runtime parameter, not a Cargo feature.** `PLATE_PARAM_MODES 12` (so `PLATE_PARAM_COUNT` becomes 13) steps through 8, 16, 24, 32, 48, 64, 96, 128 and latches on trigger like the material. The bank is a fixed 128-slot array; only the active count is processed, so cost scales with the setting. For Plate, Bell and Sheet a higher count fills the same frequency span more densely (32 modes reproduces the original tables bit for bit). Membrane continues upward with higher Bessel zeros (McMahon and Airy-zero expansions, within about 1 %). Bar is a sparse physical series, so its extra modes mostly land above Nyquist. Strike and cascade drive scale by `sqrt(32 / enabled modes)` above 32 so a denser bank keeps roughly the same level (measured within about 4 dB).

## Surprises

- **CPU.** Release build at 48 kHz on desktop: the crash preset re-struck every 0.5 s costs about 1.2 % of one core, 4.5x a Twin Core voice. That misses the 2x Twin Core target. Cost is linear in the mode count: 1.3 % at 32 modes, 2.3 % at 64 and 4.6 % at 128. The mode loop dominates; options are a cheaper f32 two-pole per mode instead of `Resonator`, or skipping modes whose state is below the quiet floor.
