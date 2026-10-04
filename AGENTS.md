# libgooey — Architecture Guide

Real-time audio synthesis engine in Rust. Drum synthesizers, sequencing, LFO modulation, and effects processing. Targets native desktop (CPAL) and iOS (C FFI).

## Module Map

```
src/
├── lib.rs               # Crate root: module declarations + public re-exports
├── ffi.rs               # The C ABI (`gooey_engine_*`) every iOS/macOS app links (~14.5k lines)
│                        #   build.rs runs cbindgen → include/gooey.h (generated, not committed);
│                        #   include/module.modulemap exposes it to Swift as `import Gooey`
│
├── engine/              # Central coordinator: tick loop, instrument/effect ownership
│   ├── mod.rs           # Engine struct, Instrument + Effect + Modulatable traits
│   ├── engine_output.rs # CPAL audio thread integration (native only)
│   ├── sequencer.rs     # Per-step blend settings used by the engine's sequencer
│   └── lfo.rs           # LFO: BPM-synced or Hz-based sine modulator
│
├── sequencer/           # 16-step sequencer with sample-accurate timing
│   └── sequencer.rs     # Step triggers, parameter blending via PresetBlender
│
├── instruments/         # Sound sources (drums implement Instrument + Modulatable)
│   ├── kick.rs / reso_kick.rs        # FM kick; dual-resonator kick
│   ├── snare.rs                      # Noise + pitched oscillators + resonators
│   ├── hihat.rs / hihat2.rs          # Hi-hats (hihat2: metallic osc + click, closed/open)
│   ├── tom.rs / tom2.rs              # Pitched percussion
│   ├── percussion_engine.rs          # Selector over resonant percussion architectures
│   ├── resonator_voice.rs / twin_core_perc.rs  # Two-mode resonant voice; Entity-style twin core
│   ├── fm_snap.rs                    # FM phase modulator utility
│   ├── bass.rs                       # Mono bass synth (polyBLEP osc + SVF + waveshaper)
│   ├── poly_synth.rs (+ _control)    # Six-voice expressive stereo synth (Nebula/Tide chords)
│   ├── mono_synth.rs / melody.rs     # Monophonic control over PolySynth; chord-aware melody
│   ├── sampler.rs (+ _control)       # Fixed-size sample-pad rack
│   ├── multisample*.rs               # Velocity-layered keyboard (piano pack load/prep/control)
│   └── granulator.rs                 # Frozen-scan granular instrument
│
├── mixer/               # "Library owns the audio channels": multi-channel stereo loop mixer
│   ├── mod.rs           # LoopMixer
│   ├── graph.rs         # Host-defined mixer graph: named submix tracks fed by SourceIds
│   ├── loop_channel.rs  # One loop player: start/end, speed, fader, mute/solo, effect chain
│   ├── clip_grid.rs     # Transport-synced session clip grid over the loop mixer
│   ├── effect_chain.rs / stereo_buffer.rs / control.rs  # Per-channel FX; shared buffers; cross-thread control
│   └── wsola.rs         # Pitch-preserving time-stretch
│
├── music/               # Music theory: notes, intervals, scales, keys, chords
│   ├── chord.rs / chord_set.rs       # Chords; named seven-pad palettes
│   ├── voicing.rs / voice_leading.rs # Named voicings; loop-aware voicing selection
│   ├── quantizer.rs / dynamics.rs    # Chord-tone pitch quantizer; per-voice strike strength
│   └── key.rs / scale.rs / note.rs / interval.rs
│
├── performance/         # Clip recording + sample-accurate replay of live gestures (chords, loops)
│   └── control.rs       # Nonblocking host→render control plane
│
├── automation/          # Macros (one 0-1 control → many params) + motions (one-shot macro automation) + macro LFOs
│   ├── macros.rs        # ParamTarget, MacroMapping/Definition, render-owned MacroBank
│   ├── motion.rs        # MotionDefinition, curves/end modes/quantize, MotionRunner pool
│   ├── lfo.rs           # Per-macro tempo-synced LFO (sine/tri/saw/square), MacroLfoRunner
│   └── control.rs       # Host→render command queue + published values (C ABI in ffi.rs)
│
├── gen/                 # Signal generators
│   ├── oscillator.rs    # Wavetable oscillator (sine, tri, square, saw)
│   ├── polyblep.rs      # Band-limited saw/square
│   ├── morph_osc.rs     # Blends between waveforms
│   ├── click_osc.rs / exciter.rs  # Transient click; strike signals for resonant voices
│   ├── pink_noise.rs    # 1/f noise
│   └── waveform.rs      # Waveform lookup tables
│
├── filters/             # DSP filters
│   ├── state_variable.rs / state_variable_tpt.rs
│   ├── resonant_lowpass.rs / resonant_highpass.rs
│   ├── biquad_bandpass.rs / biquad_highpass.rs
│   └── membrane_resonator.rs / resonator.rs
│
├── effects/             # Audio effects (all implement Effect)
│   ├── compressor.rs / entity_dynamics.rs  # Tube compressor; sidechain dynamics + feedback limiter
│   ├── delay.rs         # Delay with feedback
│   ├── reverb.rs / plate_reverb.rs         # Spring (allpass chain); Dattorro plate
│   ├── saturation.rs / waveshaper.rs / feedback_waveshaper.rs
│   ├── lowpass_filter.rs / tilt_filter.rs
│   └── limiter.rs       # Brick-wall limiter
│
├── utils/
│   ├── smoother.rs      # SmoothedParam: bounded param with ~15ms exponential smoothing
│   ├── blendable.rs     # PresetBlender: cross-fade between parameter sets
│   ├── oversampler.rs   # Anti-alias oversampling around nonlinear stages
│   └── db.rs / macro_map.rs / rng.rs
│
├── frame.rs             # StereoFrame: the engine's output currency
├── envelope.rs          # ADSR envelope with curve shaping
├── max_curve.rs         # Max/MSP curve~ algorithm
├── metronome.rs         # Optional transport-locked monitor click (post-limiter by default)
├── live_control.rs      # Lock-free control→render handoff for the opt-in live-control ABI
├── output_scope.rs      # Lock-free min/max scope of the post-limiter output
├── bounce.rs            # Offline render/export to WAV (`bounce` feature)
├── dsl.rs               # Line-based DSL for declarative instrument setup
└── visualization.rs (+ visualization/)  # Waveform/spectrogram display (feature-gated)
```

Outside `src/`:

| Path | What's there |
| --- | --- |
| `include/` | `module.modulemap` (committed) + generated `gooey.h` |
| `tests/` | Integration tests, one file per area (`mixer_graph.rs`, `chord_loop_control.rs`, `ffi_*.rs`, ...); `tests/c/` has a C consumer of the live-control ABI |
| `examples/` | Runnable demos and GUIs (`kick`, `polysynth_gui`, `reverb_lab`, `bounce`, `aliasing_plots`, ...); `examples/programs/*.gooey` are DSL programs |
| `docs/` | ABI notes for consuming apps (`live-control-abi.md`, `macros-motions-abi.md`, `mixer-graph-migration.md`) |
| `scripts/build-ios.sh` | Builds the iOS static libs; consuming apps use their own `build_libgooey_xcframework.sh` |
| `plans/`, `.agent/PLANS.md` | Committed execution plans and the ExecPlan format |

## Core Traits

- **`Instrument`** (`Send`): `trigger()`, `tick(time) -> f32`, `is_active()`, optional `as_modulatable()`
- **`Effect`** (`Send`): `process(input: f32) -> f32`
- **`Modulatable`**: `modulatable_parameters() -> Vec<&str>`, `apply_modulation(param, value)`

## Signal Flow (per sample)

```
Sequencer ──trigger──▶ Instruments ──tick──▶ Sum ──▶ Master Gain ──▶ Effects Chain ──▶ Limiter ──▶ (+) ──▶ Output
     ▲                      ▲                                                                      ▲
     │                      │                                                                      │
  BPM clock            LFO modulation                                          Metronome (monitor click, off by default)
```

The metronome taps in *after* the limiter deliberately: it is a monitoring aid,
so enabling it must not alter the material being auditioned, and it is bypassed
entirely during offline bounce.

The C ABI also exposes an opt-in final-output stage. When enabled, the live
metronome is summed after tonal effects, then smoothed final gain is applied,
and the existing optional limiter runs last. This alternate topology is off by
default so the signal flow above remains the backward-compatible behavior.

For UI waveforms, `gooey_engine_read_output_scope` returns a lock-free
min/max envelope of the post-limiter output (`src/output_scope.rs`):
`OUTPUT_SCOPE_POINT_COUNT` (1024) bins covering about one second, captured on
the render thread next to the final-output peak telemetry. Offline bounce does
not feed it.

## Key Patterns

- **Config / Params split**: `Config` structs hold static presets (with named constructors like `punchy()`). `Params` structs hold runtime `SmoothedParam` instances for real-time control.
- **0–1 normalization**: Legacy indexed parameters use normalized 0–1 ranges and instruments denormalize internally. Explicitly named dB-native C APIs are additive exceptions for controls that require exact engineering units.
- **Precision**: Audio samples are `f32`. Time accumulation uses `f64` to prevent drift.
- **Thread safety**: `Engine` wrapped in `Arc<Mutex<>>` for audio thread. Trigger queue decouples main/audio threads.
- **Click prevention**: `SmoothedParam` (~15ms smoothing) used on all real-time parameter changes.

## Planning Conventions

- Store execution plans in `./plans/` (the repo-root `plans/` directory). Plans are committed so they can be referenced, continued, and pulled up across PRs.
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
| `bounce` | Offline render/export to WAV (hound); enabled by `ios` |
| `plots` | Offline aliasing spectrum/spectrogram PNGs (rustfft, plotters) |

## Pull Requests

- Apps consume libgooey through `ffi.rs`. When a PR adds or changes a
  `gooey_engine_*` function, say so in the PR body and note which apps
  (Nebula, Ripple, Kelp, Tide) need to follow up.
- Put audible or visible evidence in the PR body when it helps review: a short
  WAV render (`bounce` feature or an example), a plot (`plots` feature), or a
  GUI example screenshot. Upload with `scripts/pr-screenshot.sh <files>`, which
  stores them in Gooey Audio's shared public dev Blob store
  (`GOOEY_DEV_PUBLIC_READ_WRITE_TOKEN`) under `libgooey/pr-screenshots/<branch>/`
  and prints Markdown to paste. Never use `BLOB_READ_WRITE_TOKEN` for this; in
  libgooey it is the private store that holds the piano pack.
