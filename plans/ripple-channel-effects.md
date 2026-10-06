# Add independent voice insert effects for Ripple

This ExecPlan is a living document maintained according to `.agent/PLANS.md`.

## Purpose / Big Picture

Ripple users can distort one kick or delay only a hi-hat without processing the other drum voices. Each of the four UI slots retains its own effects when its synth type changes. libgooey must perform this processing before voices are summed into the drum kit, with channel gain and mute applied afterward so muted echoes and reverb are silent. Ripple provides controls; it performs no synthesis, sample timing, or DSP.

## Progress

- [x] (2026-10-06) Inspect voice rendering, reusable stereo effects, mixer graph, and live-control ownership.
- [x] (2026-10-06) Add render-owned voice racks, producer-side descriptor validation, replacement commands, parameter commands, and off-thread retirement.
- [x] (2026-10-06) Verify existing live-control, allocation, instrument-swap, and mixer tests; add voice isolation, stereo delay, mute/solo, generation, and queue-capacity tests.
- [x] (2026-10-06) Add tempo, offline bounce, history-preserving parameter, concurrent rendering, and complete-transition allocation coverage; 978 Rust tests pass, two existing tests are ignored.
- [x] (2026-10-06) Complete library/build validation and open linked libgooey #275 and Ripple #130 PRs. Ripple remains draft pending a library release and manual AUv3 host checks.

## Surprises & Discoveries

The engine already provides a lock-free live-control endpoint, but its rack descriptor validator originally accepts only low-pass, delay, and spring reverb. The reusable `ChannelEffect` DSP already handles saturation, feedback waveshaping, and plate reverb. The extension can reuse that DSP and broaden the producer's validator without implementing new synthesis.

The existing graph routes the complete drum kit as one source. Giving each drum a graph track would change the source-routing contract. A rack on `VoiceStrip` isolates the voices while preserving the existing drum-kit submix and legacy APIs.

The generated `include/gooey.h` is intentionally gitignored by this repository. It is regenerated during Cargo builds and included in the XCFramework/release artifact; the C consumer test verifies its new declarations against the compiled library.

## Decision Log

Decision: Attach insert racks to stable voice indices, not synth type IDs. Rationale: two kicks in different Ripple slots need separate effect state, and instrument replacement should keep the insert rack. Date/Author: 2026-10-06, Codex.

Decision: Add commands to the existing live-control endpoint instead of exposing new direct concurrent engine mutations. Rationale: producers prepare DSP storage; the audio thread installs it at a buffer boundary, crossfades for 10 ms, and returns old storage to the producer for destruction. Date/Author: 2026-10-06, Codex.

Decision: Keep empty-rack rendering's original arithmetic. Rationale: multiplying a panned stereo frame can slightly change floating-point rounding; an empty rack should preserve the previous output exactly. Date/Author: 2026-10-06, Codex.

Decision: Put insert effects before channel gain and mute/solo. Rationale: gain should control the wet signal and mute must silence tails. Muted voices still tick their racks, so DSP histories decay naturally. Date/Author: 2026-10-06, Codex.

## Outcomes & Retrospective

The implementation and regression validation are complete: 978 Rust tests pass, with two existing ignored tests. The allocation probe covers the complete transition and retirement with zero render-thread allocations/deallocations. A compiled C consumer successfully calls the new APIs through the generated header and macOS static library. Device, simulator, and macOS static libraries have been built for Ripple's XCFramework. [libgooey PR #275](https://github.com/gooey-audio/libgooey/pull/275) and dependent [Ripple PR #130](https://github.com/gooey-audio/ripple/pull/130) are open and linked. Ripple's draft is gated on a concrete dependency release; its PR visual workflow builds code commit `9b5d20d1195079c8da8e9a9e54dd68bc9ce1128a` meanwhile. Seven Swift tests, iPhone/iPad UI tests, and iOS/Mac standalone plus extension builds pass. No AUv3 host is installed locally, so actual compact host interactions and reallocation remain release checks.

## Context and Orientation

`src/ffi.rs` owns `GooeyEngine`, its four drum voices and bass voice, all C ABI entrypoints, and the interleaved stereo rendering loop. A voice is represented by `VoiceStrip`; replacing its `ChannelInstrument` changes synthesis while leaving channel state intact. Each drum's stereo frame is accumulated into the existing DrumKit source before the mixer graph and global effects.

`src/mixer/effect_chain.rs` contains `EffectChain`, an ordered collection of existing effects using published `EFFECT_*` and parameter constants. The new `LiveEffectRack` owns a current chain and, during a short transition, an old chain. It runs both chains and blends their outputs for 10 ms. A completed old chain remains owned until it can be returned to the control thread.

`src/live_control.rs` implements fixed-capacity single-producer/single-consumer queues. One serialized host thread submits commands; the render callback consumes them. A second queue carries retired chains back to the host. `GooeyLiveControl` stores projected rack layouts and generations: these describe the commands accepted by the queue, rather than reading live DSP concurrently. A generation is a nonzero integer identifying one accepted command; a replacement's generation also identifies the rack for subsequent parameter commands.

## Plan of Work

First add `LiveEffectRack` and the two channel command variants. A voice owns one live rack. At each render boundary, install queued replacements and edits; before and after rendering, attempt to return completed old racks. Track and voice transitions use distinct busy indices in the existing shared status array. Installation cannot drop DSP storage on the render thread.

Extend `build_live_rack` and `live_effect_param_is_valid` to accept saturation, feedback waveshaping, plate reverb, and all five delay parameters. Validate descriptor counts, duplicate parameter IDs, finite values, ranges, channel indices, and rack generations before publication. Accepted descriptors are copied synchronously so the caller can release their memory after the API returns.

Render nonempty voice racks from the panned dry sample, then multiply by the voice gain and mute smoothers. Preserve the previous empty-rack path and the existing dry compressor-sidechain sample. Propagate engine BPM changes to current and transitioning chains. Offline bounce installs accepted commands before resetting voice-rack DSP history, then uses the same render path.

Add integration tests in `tests/channel_effects.rs`, extend the allocation probe in `tests/live_control_alloc.rs`, and document the API in `docs/live-control-abi.md`. `cargo` invokes `build.rs`, which regenerates `include/gooey.h` using cbindgen. Validate that generated header against the built static library. Keep the APIs additive: callers that do not submit channel racks get the original empty-rack behavior.

## Concrete Steps

Run commands from this libgooey worktree, not from Ripple's repository root:

    cargo test --no-default-features --features ios --test channel_effects
    cargo test --no-default-features --features ios --test live_control --test live_control_alloc --test mixer_graph --test channel_instrument_swap
    cargo test --no-default-features --features ios

The expected result is passing tests with no render-time allocations or deallocations. Format only touched Rust files with rustfmt, then inspect the diff and generated header. In the Ripple workspace, build the dependency using:

    LIBGOOEY_PATH="$PWD/.context/libgooey" ./scripts/build_libgooey_xcframework.sh --force

This produces a gitignored `Vendor/Gooey.xcframework` with device, simulator, and Mac slices. Ripple's Swift bridge consumes the new channel APIs through the existing Gooey module.

## Validation and Acceptance

Two slots assigned to kick must remain independent: saturation on slot 1 changes its waveform while rendering a kick in slot 0 matches the dry baseline exactly. A delay on a hard-panned hi-hat must generate stereo echoes; after the channel mute ramp settles, its wet output must be silent. Soloing another voice must also silence those tails. Instrument replacement and preset selection must preserve the rack generation and allow subsequent parameter edits.

An incorrect generation, unsupported effect, duplicate parameter, NaN, invalid channel, busy transition, or full command queue must return zero without changing accepted projected state. After rendering and retirement, retrying a valid replacement should succeed. Editing a delay parameter must preserve an already recorded echo; replacement or offline reset must clear old DSP history. A counting allocator must report zero allocations and zero deallocations during installation, scalar updates, the entire crossfade, and retirement publication. Concurrent control submissions and rendering must produce finite audio and complete without a crash.

## Idempotence and Recovery

The work is isolated in branch `feature/ripple-slot-effects`; the user's main libgooey checkout is unchanged. Tests and builds can be repeated. A rejected live command is not an accepted change: the host must retain its latest desired state and retry from its serialized producer after rendering makes space. Never retry by allocating, locking, or invoking host callbacks inside render.

The dependent Ripple PR must not merge against a release lacking these symbols. After a libgooey release with the generated header and binaries exists, update both Ripple release and visual-smoke workflow pins to that real tag. Do not invent a release tag or publish one as part of PR creation.

## Artifacts and Notes

Initial targeted validation:

    channel_instrument_swap: 11 passed
    live_control: 7 passed
    live_control_alloc: 1 passed
    mixer_graph: 10 passed
    channel_effects: 4 passed

## Interfaces and Dependencies

The additive C entrypoints are:

    uint64_t gooey_live_control_replace_channel_rack(GooeyLiveControl *control,
        uint32_t channel, const GooeyEffectDescriptor *effects, uint32_t effect_count);
    uint64_t gooey_live_control_set_channel_effect_param(GooeyLiveControl *control,
        uint32_t channel, uint32_t slot, uint64_t rack_generation,
        uint32_t param, float value);

Channel indices address the five existing voices; Ripple exposes indices 0 through 3. A zero count with a null descriptor pointer clears a rack. Success returns a nonzero submission generation; zero signals rejection. Rack limits and effect descriptors use the existing live-control ABI types. Rendering acknowledges accepted commands through `gooey_live_control_get_last_applied_generation`.

Revision note: Initial implementation and passing targeted tests recorded on 2026-10-06. Later revisions must record final validation and PR links here.

Revision note: Full regression suite (978 passed, two ignored), complete-transition allocation coverage, and the C consumer have been verified. Generated headers follow the repository's existing artifact policy.

Revision note: Linked PRs and downstream validation recorded on 2026-10-06. The dependency source pin identifies the tested code commit before this documentation-only revision.
