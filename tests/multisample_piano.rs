//! Integration coverage for the FFI multi-sample (piano) instrument.
//!
//! Mirrors the host-side calling sequence a Swift / AUv3 wrapper would use:
//! create engine → register instrument → route it to a track → build a zone map
//! → note_on → render → inspect output. Also pins the contracts a host depends
//! on: legacy and sampler source IDs are unchanged, map swaps only take effect
//! at a render boundary, and the sustain pedal holds notes.

use gooey::ffi::*;

const SR: f32 = 44_100.0;

fn render(engine: *mut GooeyEngine, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0; frames * 2];
    unsafe { gooey_engine_render(engine, output.as_mut_ptr(), frames as u32) };
    output
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().map(|s| s.abs()).fold(0.0_f32, f32::max)
}

/// Interleaved stereo PCM at a constant amplitude, so level is easy to assert.
fn flat_pcm(frames: usize, value: f32) -> Vec<f32> {
    vec![value; frames * 2]
}

/// Stage and commit a two-velocity-layer map covering C3..C5 rooted at C4.
/// Returns after the commit is queued; it becomes audible on the next render.
unsafe fn commit_two_layer_map(engine: *mut GooeyEngine, piano: u32, frames: usize) {
    assert!(gooey_engine_piano_zone_begin(engine, piano));
    for (lovel, hivel, level) in [(1, 63, 0.25_f32), (64, 127, 0.9_f32)] {
        let pcm = flat_pcm(frames, level);
        assert!(
            gooey_engine_piano_zone_add(
                engine,
                piano,
                pcm.as_ptr(),
                frames as u32,
                2,
                SR,
                48, // lokey  C3
                72, // hikey  C5
                60, // root   C4
                lovel,
                hivel,
                0.0, // tune cents
                0.0, // volume dB
                0.5, // pan (center)
                0.3, // release seconds
                PIANO_LOOP_NONE,
                0,
                0,
            ),
            "zone {lovel}..{hivel} should be accepted"
        );
    }
    assert!(gooey_engine_piano_zone_commit(engine, piano));
}

#[test]
fn registration_keeps_legacy_and_sampler_sources_and_has_a_fixed_limit() {
    unsafe {
        let engine = gooey_engine_new(SR);
        assert_eq!(SOURCE_COUNT, 5);
        assert_eq!(SOURCE_SAMPLER_BASE, 5);
        assert_eq!(SOURCE_PIANO_BASE, 9);

        for piano in 0..PIANO_INSTRUMENT_MAX {
            assert_eq!(gooey_engine_piano_register(engine), piano as i32);
            assert_eq!(
                gooey_engine_piano_get_source_id(engine, piano),
                SOURCE_PIANO_BASE + piano
            );
        }
        assert_eq!(gooey_engine_piano_register(engine), -1);

        // Registering pianos must not disturb anything already routed.
        assert_eq!(
            gooey_engine_mixer_get_source_route(engine, SOURCE_DRUMKIT),
            0
        );
        assert_eq!(
            gooey_engine_piano_get_source_id(engine, PIANO_INSTRUMENT_MAX),
            u32::MAX
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn registered_piano_can_route_after_default_graph_reset() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let piano = gooey_engine_piano_register(engine) as u32;
        let source = gooey_engine_piano_get_source_id(engine, piano);
        gooey_engine_mixer_reset_default_layout(engine);
        assert!(gooey_engine_mixer_route_source(engine, source, 2));
        assert_eq!(gooey_engine_mixer_get_source_route(engine, source), 2);
        gooey_engine_free(engine);
    }
}

#[test]
fn an_unregistered_or_mapless_piano_is_silent() {
    unsafe {
        let engine = gooey_engine_new(SR);
        // Nothing registered: every call fails, and rendering is silent.
        assert!(!gooey_engine_piano_note_on(engine, 0, 60, 1.0));
        assert!(!gooey_engine_piano_zone_begin(engine, 0));
        assert_eq!(peak(&render(engine, 256)), 0.0);

        // Registered but with no map: still silent, still no panic.
        let piano = gooey_engine_piano_register(engine) as u32;
        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_PIANO_BASE + piano,
            2
        ));
        assert_eq!(gooey_engine_piano_zone_count(engine, piano), 0);
        assert!(!gooey_engine_piano_note_on(engine, piano, 60, 1.0));
        assert_eq!(peak(&render(engine, 256)), 0.0);
        gooey_engine_free(engine);
    }
}

#[test]
fn a_committed_map_can_be_routed_and_played() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let piano = gooey_engine_piano_register(engine) as u32;
        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_PIANO_BASE + piano,
            2
        ));

        assert_eq!(gooey_engine_piano_map_generation(engine, piano), 0);
        commit_two_layer_map(engine, piano, 8192);

        // The swap lands at the next render boundary, not before.
        assert_eq!(gooey_engine_piano_zone_count(engine, piano), 0);
        render(engine, 64);
        assert_eq!(gooey_engine_piano_zone_count(engine, piano), 2);
        assert_eq!(gooey_engine_piano_map_generation(engine, piano), 1);

        assert!(gooey_engine_piano_note_on(engine, piano, 60, 1.0));
        assert_eq!(gooey_engine_piano_active_voices(engine, piano), 1);
        assert!(
            peak(&render(engine, 512)) > 0.01,
            "a struck note must sound"
        );

        // A note outside the mapped range allocates nothing.
        assert!(!gooey_engine_piano_note_on(engine, piano, 20, 1.0));
        gooey_engine_free(engine);
    }
}

#[test]
fn velocity_picks_the_matching_layer() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let piano = gooey_engine_piano_register(engine) as u32;
        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_PIANO_BASE + piano,
            2
        ));
        commit_two_layer_map(engine, piano, 8192);
        render(engine, 64);

        assert!(gooey_engine_piano_note_on(engine, piano, 60, 0.1));
        let soft = peak(&render(engine, 512));
        assert!(gooey_engine_piano_release_all(engine, piano));
        render(engine, 44_100);

        assert!(gooey_engine_piano_note_on(engine, piano, 60, 1.0));
        let hard = peak(&render(engine, 512));

        assert!(
            hard > soft * 1.5,
            "the loud layer should be clearly louder: hard {hard} vs soft {soft}"
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn the_sustain_pedal_holds_notes_until_it_lifts() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let piano = gooey_engine_piano_register(engine) as u32;
        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_PIANO_BASE + piano,
            2
        ));
        // Three seconds of PCM, so "still ringing" is distinguishable from
        // "the recording ran out".
        commit_two_layer_map(engine, piano, 3 * SR as usize);
        render(engine, 64);

        assert!(gooey_engine_piano_set_sustain(engine, piano, true));
        assert!(gooey_engine_piano_note_on(engine, piano, 60, 1.0));
        assert!(gooey_engine_piano_note_off(engine, piano, 60));

        render(engine, SR as usize);
        assert_eq!(
            gooey_engine_piano_active_voices(engine, piano),
            1,
            "a pedalled note should still be sounding"
        );

        assert!(gooey_engine_piano_set_sustain(engine, piano, false));
        render(engine, SR as usize);
        assert_eq!(
            gooey_engine_piano_active_voices(engine, piano),
            0,
            "lifting the pedal should damp the note"
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn presets_and_params_are_validated() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let piano = gooey_engine_piano_register(engine) as u32;

        assert!(gooey_engine_piano_set_preset(
            engine,
            piano,
            PIANO_PRESET_SOFT
        ));
        assert!(gooey_engine_piano_set_preset(
            engine,
            piano,
            PIANO_PRESET_BRIGHT
        ));
        // Unknown presets fall back to the default rather than failing.
        assert!(gooey_engine_piano_set_preset(engine, piano, 99));
        assert!(!gooey_engine_piano_set_preset(
            engine,
            PIANO_INSTRUMENT_MAX,
            0
        ));

        for param in 0..4 {
            assert!(gooey_engine_piano_set_param(engine, piano, param, 0.5));
        }
        assert!(!gooey_engine_piano_set_param(engine, piano, 4, 0.5));
        assert!(!gooey_engine_piano_set_param(engine, piano, 0, f32::NAN));
        gooey_engine_free(engine);
    }
}

#[test]
fn malformed_zone_input_is_rejected_without_committing() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let piano = gooey_engine_piano_register(engine) as u32;

        // No builder open yet.
        let pcm = flat_pcm(128, 0.5);
        assert!(!gooey_engine_piano_zone_add(
            engine,
            piano,
            pcm.as_ptr(),
            128,
            2,
            SR,
            60,
            60,
            60,
            1,
            127,
            0.0,
            0.0,
            0.5,
            0.2,
            PIANO_LOOP_NONE,
            0,
            0
        ));
        assert!(!gooey_engine_piano_zone_commit(engine, piano));

        assert!(gooey_engine_piano_zone_begin(engine, piano));
        // Null PCM, zero frames, an unsupported channel count, and a non-finite
        // parameter are each rejected.
        assert!(!gooey_engine_piano_zone_add(
            engine,
            piano,
            std::ptr::null(),
            128,
            2,
            SR,
            60,
            60,
            60,
            1,
            127,
            0.0,
            0.0,
            0.5,
            0.2,
            PIANO_LOOP_NONE,
            0,
            0
        ));
        assert!(!gooey_engine_piano_zone_add(
            engine,
            piano,
            pcm.as_ptr(),
            0,
            2,
            SR,
            60,
            60,
            60,
            1,
            127,
            0.0,
            0.0,
            0.5,
            0.2,
            PIANO_LOOP_NONE,
            0,
            0
        ));
        assert!(!gooey_engine_piano_zone_add(
            engine,
            piano,
            pcm.as_ptr(),
            128,
            3,
            SR,
            60,
            60,
            60,
            1,
            127,
            0.0,
            0.0,
            0.5,
            0.2,
            PIANO_LOOP_NONE,
            0,
            0
        ));
        assert!(!gooey_engine_piano_zone_add(
            engine,
            piano,
            pcm.as_ptr(),
            128,
            2,
            SR,
            60,
            60,
            60,
            1,
            127,
            f32::NAN,
            0.0,
            0.5,
            0.2,
            PIANO_LOOP_NONE,
            0,
            0
        ));
        // An empty builder cannot be committed.
        assert!(!gooey_engine_piano_zone_commit(engine, piano));
        assert_eq!(gooey_engine_piano_map_generation(engine, piano), 0);
        gooey_engine_free(engine);
    }
}

#[test]
fn two_pianos_render_independently() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let a = gooey_engine_piano_register(engine) as u32;
        let b = gooey_engine_piano_register(engine) as u32;
        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_PIANO_BASE + a,
            2
        ));
        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_PIANO_BASE + b,
            3
        ));
        commit_two_layer_map(engine, a, 8192);
        commit_two_layer_map(engine, b, 8192);
        render(engine, 64);

        assert!(gooey_engine_piano_note_on(engine, a, 60, 1.0));
        assert_eq!(gooey_engine_piano_active_voices(engine, a), 1);
        assert_eq!(
            gooey_engine_piano_active_voices(engine, b),
            0,
            "playing one instrument must not trigger the other"
        );
        assert!(peak(&render(engine, 256)) > 0.01);
        gooey_engine_free(engine);
    }
}

#[test]
fn output_stays_finite_under_a_dense_pedalled_chord() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let piano = gooey_engine_piano_register(engine) as u32;
        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_PIANO_BASE + piano,
            2
        ));
        commit_two_layer_map(engine, piano, 3 * SR as usize);
        render(engine, 64);

        assert!(gooey_engine_piano_set_sustain(engine, piano, true));
        // Far more notes than the voice pool, all held under the pedal.
        for round in 0..4 {
            for note in 48..=72 {
                gooey_engine_piano_note_on(engine, piano, note, 0.5 + 0.1 * round as f32);
            }
        }
        let output = render(engine, 4096);
        assert!(output.iter().all(|s| s.is_finite()));
        assert!(peak(&output) > 0.0);
        gooey_engine_free(engine);
    }
}
