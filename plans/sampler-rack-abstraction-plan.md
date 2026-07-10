# Sampler Rack Abstraction

This ExecPlan is a living document. Maintain it according to `.agent/PLANS.md` whenever implementation changes.

## Purpose / Big Picture

libgooey hosts can now register up to four sample-pad racks alongside the existing drum kit, bass, poly synth, granulator, and loop mixer. Each rack has sixteen copied PCM slots and a sixteen-step pattern. A host can route a rack into any mixer-graph track, play pads live, and capture those live hits in the same transport-locked performance clip used by chord pads.

The observable validation is `cargo run --example sampler_rack --features native,crossterm`. It opens an interactive terminal UI backed by the default system audio device, plays a routed in-memory pad sequence, and accepts live pad/record controls without requiring an audio file.

## Progress

- [x] (2026-07-10) Add fixed-size sampler DSP storage and playback voices.
- [x] (2026-07-10) Add registered sampler graph sources, FFI, and shared performance capture.
- [x] (2026-07-10) Add integration tests, an interactive CPAL-backed CLI example, and mixer documentation.
- [x] (2026-07-10) Run `cargo test`: 269 library tests, 4 sampler integration tests, all other integration tests, and doc tests passed. Existing `tests/performance_recording.rs` emits five `unused_unsafe` warnings.

## Surprises & Discoveries

- Observation: the FFI engine has five legacy graph sources while its default graph has four tracks. Adding a sampler track by default would break host layout assumptions.
  Evidence: `tests/mixer_graph.rs` asserts source IDs 0 through 4 and four default tracks.

## Decision Log

- Decision: preserve `SOURCE_COUNT == 5` and reserve sampler source IDs beginning at `SOURCE_SAMPLER_BASE == 5`.
  Rationale: Existing host source IDs and default mixer layouts stay compatible while registered racks remain independently routable.
  Date/Author: 2026-07-10 / Codex

- Decision: use four persistent racks with sixteen slots and thirty-two shared voices per rack.
  Rationale: This bounds real-time memory and CPU while allowing layered pad hits and multiple independent kits.
  Date/Author: 2026-07-10 / Codex

- Decision: sampler performance events use a typed lane inside `PerformanceRecorder`; existing chord query APIs remain chord-only.
  Rationale: The shared transport and record mode remain coherent without breaking existing C callers.
  Date/Author: 2026-07-10 / Codex

## Outcomes & Retrospective

The implementation and full-suite verification are complete. Racks are configuration-time registrations and the render path walks only fixed arrays; PCM is copied from the host and voices retain an `Arc` reference while playing. The internal gain seam currently provides only a fixed de-click taper, leaving future configurable amplitude envelopes additive.

## Context and Orientation

`src/ffi.rs` owns the C-facing `GooeyEngine`, its transport, the mixer graph, and the performance recorder. `src/mixer/graph.rs` maps stable numeric source IDs to host-created tracks. `src/performance/mod.rs` stores timed looping events. `src/instruments/sampler.rs` is the new fixed sampler-rack DSP module.

A slot is one finite PCM buffer supplied as interleaved `f32` frames with one or two channels. A rack voice is one simultaneous playback cursor over a slot buffer. The source graph is the table that routes each engine audio source to a named track with its own strip and effect rack.

## Plan of Work

Create `SamplerBuffer`, `SamplerRack`, and a fixed thirty-two-voice pool in `src/instruments/sampler.rs`. Validate finite mono/stereo PCM, interpolate source frames, advance by source sample rate divided by engine sample rate, and select the oldest active voice when all voices are occupied. Replacing or clearing a slot stops its current voices. Keep voice gain isolated from buffer decoding so an envelope can later replace the fixed click guard.

In `src/mixer/graph.rs`, retain legacy sources 0 through 4 and add four inactive sampler source entries. Registration activates a source; routing rejects inactive IDs. Do not alter the default four-track layout.

In `src/ffi.rs`, store four optional racks. Registration returns rack IDs 0 through 3, starts their sequencers with the global transport, and scatters their stereo frames to source IDs `SOURCE_SAMPLER_BASE + rack`. Add C functions to register, route by source ID, load/clear/query slots, trigger a pad, and set/query sequencer cells. The load function copies host PCM and accepts exactly one or two channels.

Extend `PerformanceRecorder` with `SamplerClipEvent` values. The recorder stamps only successful manual sampler triggers while record-arm is active. The audio clock emits recorded hits on later loop passes; internal sampler sequencer hits never call the recorder. Preserve the existing chord event count and query API, and expose separate sampler event count/query functions.

## Concrete Steps

From the repository root run:

    cargo test --test sampler_rack
    cargo test
    cargo run --example sampler_rack --features native,crossterm

The example should show its terminal UI and play the sequence through the default audio device. Press `1` through `4` for live pads, `r` to record them, and `q` to exit.

## Validation and Acceptance

The sampler feature is accepted when a test can register four racks but not a fifth, route a rack without changing legacy graph source IDs, load mono PCM, render a non-zero signal from manual and sequenced hits, and query the configured sequence cell. A record-armed manual hit must create one sampler event; sequencer hits must create none. Unit tests must cover stereo interpolation and bounded layered-voice behavior. `cargo test` must pass without failures.

## Idempotence and Recovery

Registration is intentionally one-way for an engine lifetime. Slot replacement and clearing are safe repeatedly; they stop only voices using the affected slot. A failed buffer validation leaves the existing slot untouched. If a host does not register a rack, no new source is active and legacy engine output is unchanged.

## Interfaces and Dependencies

The C surface is in `src/ffi.rs`: `SAMPLER_RACK_MAX`, `SAMPLER_SLOT_COUNT`, `SOURCE_SAMPLER_BASE`, `gooey_engine_sampler_register`, `gooey_engine_sampler_get_source_id`, slot buffer/query functions, `gooey_engine_sampler_trigger`, sequence set/query functions, and sampler performance event queries. Use `gooey_engine_mixer_route_source` with the returned source ID. No new external dependency is required; host applications decode files before passing PCM.

Plan updated 2026-07-10 to record the implemented bounded-rack design and its validation commands.
