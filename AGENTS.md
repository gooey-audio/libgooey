# libgooey — Architecture Guide

Real-time audio synthesis engine in Rust. Drum synthesizers, sequencing, LFO modulation, and effects processing. Targets native desktop (CPAL) and iOS (C FFI).

## Module Map

```
src/
├── engine/              # Central coordinator: tick loop, instrument/effect ownership
│   ├── mod.rs           # Engine struct, Instrument + Effect + Modulatable traits
│   ├── engine_output.rs # CPAL audio thread integration (native only)
│   └── lfo.rs           # LFO: BPM-synced or Hz-based sine modulator
│
├── sequencer/           # 16-step sequencer with sample-accurate timing
│   └── sequencer.rs     # Step triggers, parameter blending via PresetBlender
│
├── instruments/         # Drum synthesizers (all implement Instrument + Modulatable)
│   ├── kick.rs          # Pitch-swept FM + noise + resonators
│   ├── snare.rs         # Noise + pitched oscillators + resonators
│   ├── hihat2.rs        # Metallic oscillators + click (closed/open modes)
│   ├── tom2.rs          # Pitched percussion with frequency/decay control
│   └── fm_snap.rs       # FM phase modulator utility
│
├── gen/                 # Signal generators
│   ├── oscillator.rs    # Wavetable oscillator (sine, tri, square, saw)
│   ├── morph_osc.rs     # Blends between waveforms
│   ├── click_osc.rs     # Transient click/pop generator
│   ├── pink_noise.rs    # 1/f noise
│   └── waveform.rs      # Waveform lookup tables
│
├── filters/             # DSP filters
│   ├── state_variable.rs / state_variable_tpt.rs
│   ├── resonant_lowpass.rs / resonant_highpass.rs
│   ├── biquad_bandpass.rs / biquad_highpass.rs
│   └── membrane_resonator.rs
│
├── effects/             # Audio effects (all implement Effect)
│   ├── compressor.rs    # Tube compressor
│   ├── delay.rs         # Delay with feedback
│   ├── saturation.rs    # Tube saturation / waveshaping
│   ├── lowpass_filter.rs
│   ├── waveshaper.rs
│   └── limiter.rs       # Brick-wall limiter
│
├── utils/
│   ├── smoother.rs      # SmoothedParam: bounded param with ~15ms exponential smoothing
│   └── blendable.rs     # PresetBlender: cross-fade between parameter sets
│
├── envelope.rs          # ADSR envelope with curve shaping
├── dsl.rs               # Line-based DSL for declarative *engine* setup (instruments + seq + lfo + fx)
├── graph/               # Node-graph synth engine: describe an instrument's internals as a DSL graph
│   ├── node.rs          # Primitive nodes (osc, env, noise, lp/hp/bp, shape, gain, const, mul/add)
│   ├── parser.rs        # Low-noise DSL → GraphSpec (constant-folding expression parser)
│   └── mod.rs           # CompiledGraph evaluator + GraphInstrument (impl Instrument) + Topology
├── ffi.rs               # C FFI bindings for iOS/Swift integration
└── visualization.rs     # Waveform display (feature-gated)
```

## Two DSLs (don't confuse them)

- **`src/dsl.rs`** wires a whole *engine program*: which instruments exist, their sequencer
  patterns, LFO routes, and global effects. It treats each instrument (kick, snare…) as a black box.
- **`src/graph/`** describes a *single instrument's internals* as a graph of primitive nodes
  (oscillators, envelopes, noise, filters, an opaque `shape`/saturation effect) so you can see and
  re-wire which component shapes which part of the sound. A `GraphInstrument` implements the standard
  `Instrument` trait, so a graph drops straight into the `Engine`, sequencer, and bounce paths. The
  engine is pure-Rust and always compiled (no feature flag), and exposes a `Topology` (nodes + edges)
  for the planned glow/shader node-graph GUI. See `examples/instruments/*.graph` and
  `cargo run --example graph --features bounce -- examples/instruments/kick.graph`.

## Core Traits

- **`Instrument`** (`Send`): `trigger()`, `tick(time) -> f32`, `is_active()`, optional `as_modulatable()`
- **`Effect`** (`Send`): `process(input: f32) -> f32`
- **`Modulatable`**: `modulatable_parameters() -> Vec<&str>`, `apply_modulation(param, value)`

## Signal Flow (per sample)

```
Sequencer ──trigger──▶ Instruments ──tick──▶ Sum ──▶ Master Gain ──▶ Effects Chain ──▶ Output
     ▲                      ▲
     │                      │
  BPM clock            LFO modulation
```

## Key Patterns

- **Config / Params split**: `Config` structs hold static presets (with named constructors like `punchy()`). `Params` structs hold runtime `SmoothedParam` instances for real-time control.
- **0–1 normalization**: All external parameters use normalized 0–1 range. Instruments denormalize internally.
- **Precision**: Audio samples are `f32`. Time accumulation uses `f64` to prevent drift.
- **Thread safety**: `Engine` wrapped in `Arc<Mutex<>>` for audio thread. Trigger queue decouples main/audio threads.
- **Click prevention**: `SmoothedParam` (~15ms smoothing) used on all real-time parameter changes.

## Planning Conventions

- Store execution plans in `.context/plans/`.
- Use dash-separated lowercase filenames for plans, for example `granulator-original-design-gap-plan.md`.
- When writing an ExecPlan, follow `.agent/PLANS.md` and keep the plan self-contained.

## Feature Flags

| Feature | Description |
|---------|-------------|
| `native` | Desktop audio via CPAL (default) |
| `ios` | iOS target — engine only, no audio output |
| `crossterm` | Terminal UI for examples |
| `visualization` | Waveform display (glfw, gl, rustfft) |
| `midi` | MIDI input support (midir) |
