# Implement a Noir-Inspired FM Percussion Voice

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept current as work proceeds. This document is maintained in accordance with `.agent/PLANS.md`.

## Purpose / Big Picture

After this change, libgooey users can assign a monophonic FM percussion synthesizer to any existing sequenced channel and create morphing kicks, metallic hits, zaps, hats, and bass-percussion sounds from two oscillators and noise. Per-step MIDI pitch and velocity alter the timbre as well as loudness, while the existing transport, mixer strip, preset blender, LFO, offline bounce, and C/Swift integration continue to work.

The design is behaviorally inspired by Ruismaker Noir's public manual: VCO1 modulates VCO2, both oscillators have signed pitch drops, noise is a parallel source, and pitch and velocity target a small matrix of high-impact parameters. The implementation uses original signal-processing code and original presets rather than copying proprietary code, data, branding, or interface assets.

## Progress

- [x] (2026-09-13 00:00Z) Researched the Ruismaker Noir and Ruismaker FM manuals, digital FM theory, and the existing libgooey DSP, sequencer, FFI, mixer, and testing architecture.
- [x] (2026-09-13 21:10Z) Implemented the phase-accumulating two-oscillator FM voice, curved envelopes, 2x half-band render path, filter/grit/boost/bit-drive stages, 40 normalized controls, macro routes, four presets, and DSP tests.
- [x] (2026-09-13 21:10Z) Integrated the voice with generic instruments, five-strip channel assignment, preset blending, per-step notes, atomic manual note triggers, tuning, LFO dispatch, and normalized C FFI controls.
- [x] (2026-09-13 21:10Z) Added FFI integration tests, the offline morphing `fm_percussion` example, README guidance, and cbindgen-visible API documentation/constants.
- [x] (2026-09-13 21:20Z) Ran formatting, no-default and bounce test suites, host iOS feature build, both iOS target builds, the offline example, generated-header inspection, WAV measurements, and final diff checks.

## Surprises & Discoveries

- Observation: `src/gen/oscillator.rs` derives phase from elapsed time multiplied by the current frequency. Changing its frequency at audio rate therefore changes the historical phase calculation and cannot implement continuous FM safely.
  Evidence: `Oscillator::tick` assigns `current_sample_index = elapsed_since_trigger * sample_rate`, and waveform functions multiply that index by the current frequency.
- Observation: the FFI constant `INSTRUMENT_COUNT` is both the number of addressable voice strips and, informally, the number of available instrument types. Adding an assignable type must not increase render-array sizes or the legacy host-visible voice count.
  Evidence: `NUM_INSTRUMENTS` sizes render arrays and iterates the four drum strips plus bass, while `ChannelInstrument::instrument_type` uses values in the same numeric range.
- Observation: the generic Rust `Instrument` trait already supports staging MIDI notes, while the FFI channel wrapper currently implements pitched sequencer notes by overwriting parameter zero for kick, tom, and bass.
  Evidence: `Engine::advance_control` calls `set_midi_note`; `GooeyEngine::render` instead maps MIDI notes through `freq_range_for_instrument` and calls `ChannelInstrument::set_param(0, ...)`.
- Observation: the baseline `cargo test --no-default-features` suite is green before implementation, with 385 library unit tests and all integration suites passing.
- Observation: measuring the top 4.3 kHz of the host band on an aggressive 9 kHz/7.3 kHz, index-12 phase-modulated signal showed that the production 2x half-band path meets the required 6 dB reduction relative to direct base-rate synthesis.
  Evidence: `two_x_path_reduces_aggressive_high_band_alias_energy_by_six_db` passes using the same direct FM equation on both test paths.
- Observation: cbindgen does not emit constant aliases whose values are Rust module paths.
  Evidence: the first generated header contained the FM functions and category constants but omitted `FM_PERCUSSION_PARAM_*`; using literal stable ABI values makes the entire table appear in `include/gooey.h`.
- Observation: both installed Rust iOS targets build successfully, but this machine has Command Line Tools selected instead of a full Xcode iPhone SDK.
  Evidence: `./scripts/build-ios.sh` produced both 19 MB static libraries and exited zero, while `xcrun` warned that `iphoneos` and `iphonesimulator` SDKs could not be located and the linker fell back to the macOS sysroot.

## Decision Log

- Decision: Add one full-path `FmPercussion` instrument rather than a general operator graph or a six-channel Ruismaker FM clone.
  Rationale: It directly serves the requested unconventional techno workflow and fits libgooey's existing monophonic `Instrument` and per-channel sequencer architecture.
  Date/Author: 2026-09-13 / Codex and user.
- Decision: Keep the existing fixed 16-step sequencer and use its existing note and velocity fields.
  Rationale: This supplies the musically important Noir interaction without expanding transport behavior, serialization, and host synchronization in the same change.
  Date/Author: 2026-09-13 / Codex and user.
- Decision: Expose the voice as an assignable channel type across Rust and C FFI.
  Rationale: Existing channel strips already own sequencing, panning, gain, routing, and preset blending, and multiple strips provide layering without adding a permanent mixer source.
  Date/Author: 2026-09-13 / Codex and user.
- Decision: Use phase modulation, expressed as `carrier_phase + index * modulator / TAU`, for the audio-rate FM stage.
  Rationale: This is the standard digital form of musical FM, has a dimensionless modulation index, and preserves a continuous carrier phase.
  Date/Author: 2026-09-13 / Codex.
- Decision: Keep `INSTRUMENT_COUNT` equal to five and add `INSTRUMENT_TYPE_COUNT` equal to six.
  Rationale: `INSTRUMENT_COUNT` is already part of the C ABI and describes addressable render voices in practice; changing it would make host loops address nonexistent strips.
  Date/Author: 2026-09-13 / Codex.

## Outcomes & Retrospective

The new voice is implemented as an original monophonic two-oscillator phase-modulation percussion synth with independent tracking/fixed pitch modes and signed pitch curves, parallel noise, three ring modes, LP/HP filtering with grit, resonant boost, bit-drive, and pitch/velocity macro routing. It is assignable to any of the five existing strips and preserves sequencer and mixer state when swapped. The Rust and C surfaces expose all 40 normalized parameters, four factory presets, category anchors, exact parameter reads, channel-addressed preset loading, and render-boundary note/velocity triggering.

The full no-default suite passed with 396 library tests plus every integration suite; the bounce-enabled suite passed with 419 library tests plus every integration suite. The host iOS feature build and both target builds passed. The final example produced a 48 kHz, 16-bit mono, 662 KB WAV with 76,063 nonzero samples, a peak of 22,069, and clearly different per-hit RMS values across the programmed steps. The ignored generated header contains the complete parameter table and all new function declarations.

No implementation work is deferred. A packaging consumer should still run the iOS script on a machine with full Xcode selected to remove the SDK fallback warnings described above.

## Context and Orientation

`src/instruments/` contains monophonic drum and bass voices. Each conventional voice separates a copyable normalized `Config` from runtime `SmoothedParam` values and implements `Instrument`, usually `Modulatable`, and `Blendable`. `src/engine/sequencer.rs` contains the sample-accurate step sequencer. Each `SequencerStep` already carries enabled state, velocity, an optional preset-blend position, and an optional MIDI note.

`src/ffi.rs` owns the embeddable engine used by Swift and iOS. A `VoiceStrip` contains a `ChannelInstrument`, sequencer, blender, gain, pan, mute/solo state, and trigger atomics. Four strips are grouped as the drum-kit mixer source and a fifth bass strip is routed separately. Reassigning the synthesizer inside a strip must leave the rest of that strip untouched.

The new voice is monophonic. A trigger resets oscillator phases and one-shot envelopes. VCO1 is the modulator and also remains an audible parallel source; VCO2 is the carrier. White noise is a third parallel source. These sources feed a percussion-oriented filter and compact dirty-output chain before the enclosing voice strip applies gain and pan.

## Plan of Work

Create `src/instruments/fm_percussion.rs`. Define stable public parameter constants numbered 0 through 39, categorical enums for waveform, ring mode, and filter mode, normalized config structures, runtime smoothed parameters, four factory presets, and `Blendable`. Continuous values clamp to `0.0..=1.0`; signed values use 0.5 as neutral. Categorical parameter getters return the exact normalized anchors documented by the FFI.

Build private f64 phase accumulators that can evaluate sine, triangle, PolyBLEP square, and a six-pulse metallic bank at an optional phase offset. Render VCO1 first, then evaluate VCO2 at `carrier_phase + index * vco1 / TAU`. Ring mode multiplies VCO1 and VCO2. Cross-ring uses one-sample-delayed bidirectional phase feedback before multiplication, with fixed bounded feedback and finite-value protection. VCO1's audible level must not alter its modulation strength.

Use one-shot, allocation-free curved envelopes for amplitude, both pitch drops, noise, and filter motion. Tracking oscillators interpret their frequency knob as a plus-or-minus 24-semitone offset from the staged MIDI note; fixed oscillators map it logarithmically from 20 Hz to 12 kHz. Signed pitch drop spans minus to plus 60 semitones and decays to the steady frequency. The full voice runs at twice the host sample rate and is half-band downsampled.

Implement the six pitch and velocity macro destinations as normalized morphing around each base value: pitch drop, FM index, noise level, oscillator balance, filter cutoff, and combined output level. For each route, positive depth moves toward its source, negative depth moves toward the inverse source, and the two route deltas sum around the base before one clamp. Latch note and velocity sources on trigger; do not write macro results back into base parameters. Default velocity-to-level depth is maximum, and no additional hidden velocity multiplier is applied.

Extend `ChannelInstrument` and `ChannelBlender`, add the FM factory presets, and allow instrument type 5 in channel reassignment. Rename the private render-size constant to `NUM_VOICE_STRIPS` while keeping the public legacy count at five. Add channel-specific parameter reads, channel-specific FM preset loading, note-plus-velocity manual triggering, and the FM parameter/type/category constants. Manual note triggers are staged atomically and consumed at a render boundary. FM sequencer notes are staged directly and do not overwrite the stored base-pitch knob; the old parameter-zero override path remains for kick, tom, and bass.

Add `examples/fm_percussion.rs` as a bounce-enabled example. It must render a monophonic pattern whose notes and velocities audibly morph one patch. Update `Cargo.toml` and `README.md` with the feature and example. Generated `include/gooey.h` remains ignored and is regenerated by the normal build script.

## Concrete Steps

Work from the repository root `/Users/brianhurlow/conductor/workspaces/libgooey/ashgabat`.

Implement and format the feature:

    cargo fmt --all

Run the primary platform-independent suite:

    cargo test --no-default-features

Run the bounce-enabled suite and example:

    cargo test --no-default-features --features bounce
    cargo run --no-default-features --features bounce --example fm_percussion

Build the iOS configuration and, where the local Xcode toolchain has both Rust targets installed, run the packaging script:

    cargo build --no-default-features --features ios
    ./scripts/build-ios.sh

The test commands must finish with zero failures. The example must create `fm_percussion.wav`; inspecting its samples must show finite, nonzero audio and multiple hits with different energy or spectral character.

## Validation and Acceptance

DSP tests must prove that an index of zero produces the unmodulated carrier and that nonzero sine-on-sine modulation produces energy at carrier-plus/minus-modulator bins. They must cover signed pitch direction and convergence, different envelope slopes, deterministic retriggering, volume-zero silence, bit-driver bypass, all presets and categorical modes, and finite bounded output at 44.1, 48, and 96 kHz. A test-only base-rate render path should establish that the production 2x path reduces aggressive high-band alias energy by at least 6 dB; if that test reveals that a fixed 2x path cannot meet the threshold, record the evidence here and promote the internal rate to 4x rather than weakening the assertion.

Macro tests must prove neutral routes preserve base values, positive and negative depths respond in opposite directions, pitch and velocity combination is independent of evaluation order, and repeated triggers never accumulate modulation into the stored patch.

FFI tests must cover all 40 parameter round trips, clamping and non-finite rejection, category anchors, instrument and type counts, duplicated FM assignments, channel-state preservation on swap, channel-addressed preset loading, manual pitched triggers, per-step pitched triggers, base-pitch restoration, LFO dispatch, and existing transport/mute/pan behavior. Existing ABI functions and constants must retain their values and behavior.

## Idempotence and Recovery

All implementation steps are additive or narrowly refactor private names. Re-running formatting, tests, builds, header generation, and the example is safe. The generated header and Cargo build outputs are ignored. The example WAV is a disposable artifact and must not be committed. If a platform target is unavailable, preserve the exact failure transcript in `Surprises & Discoveries`, run the host-side iOS feature build, and leave source state formatted and tested.

## Artifacts and Notes

The public reference behavior comes from `https://ruismaker.com/manuals/noir_guide.pdf`. The FM equation and the importance of a changing modulation index for evolving percussive spectra come from John Chowning's “The Synthesis of Complex Audio Spectra by Means of Frequency Modulation.” The exact cross-ring, grit, frequency-boost, parameter-range, and preset implementations in libgooey are original approximations because the public documentation describes their musical role rather than their proprietary equations.

## Interfaces and Dependencies

No new crate dependency is required. Reuse `halfband`, the existing PolyBLEP helpers, `StateVariableFilterTpt`, `SmoothedParam`, and `Blendable`.

The Rust surface must export `FmPercussion`, `FmPercussionConfig`, `FmPercussionParams`, `FmWaveform`, `FmRingMode`, `FmFilterMode`, the four preset constructors, and `FM_PARAM_*` constants ending in `FM_PARAM_COUNT = 40`.

The C surface must export `INSTRUMENT_FM_PERCUSSION = 5`, `INSTRUMENT_TYPE_COUNT = 6`, normalized category anchors, preset IDs and count, `gooey_engine_fm_percussion_param_count`, `gooey_engine_get_channel_param`, a channel-addressed FM preset loader returning success, and `gooey_engine_trigger_channel_note`. Existing `INSTRUMENT_COUNT` remains 5.

Revision note (2026-09-13): Initial self-contained implementation plan created from the approved research and repository inspection.

Revision note (2026-09-13): Marked the implementation, integration, documentation, and validation milestones complete; recorded cbindgen behavior, iOS SDK warnings, test counts, and WAV measurements.
