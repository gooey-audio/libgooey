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
    commit_two_layer_map_in_range(engine, piano, frames, 48, 72);
}

/// As `commit_two_layer_map`, with an explicit playable key range.
unsafe fn commit_two_layer_map_in_range(
    engine: *mut GooeyEngine,
    piano: u32,
    frames: usize,
    lokey: u32,
    hikey: u32,
) {
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
                lokey,
                hikey,
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

unsafe fn commit_exact_velocity_map(
    engine: *mut GooeyEngine,
    piano: u32,
    notes: &[u32],
    velocities: &[f32],
) {
    assert_eq!(notes.len(), velocities.len());
    assert!(gooey_engine_piano_zone_begin(engine, piano));
    for (&note, &velocity) in notes.iter().zip(velocities) {
        let pcm = flat_pcm(8192, 0.5);
        let midi_velocity = ((velocity.clamp(0.0, 1.0) * 127.0).round() as u32).max(1);
        assert!(gooey_engine_piano_zone_add(
            engine,
            piano,
            pcm.as_ptr(),
            8192,
            2,
            SR,
            note,
            note,
            note,
            midi_velocity,
            midi_velocity,
            0.0,
            0.0,
            0.5,
            0.3,
            PIANO_LOOP_NONE,
            0,
            0,
        ));
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
        assert!(
            peak(&render(engine, 512)) > 0.01,
            "a struck note must sound"
        );
        // Metering is published once per render buffer so it can be polled
        // from a UI thread, so it reports after the buffer that played it.
        assert_eq!(gooey_engine_piano_active_voices(engine, piano), 1);

        // A note outside the mapped range allocates nothing.
        assert!(!gooey_engine_piano_note_on(engine, piano, 20, 1.0));
        gooey_engine_free(engine);
    }
}

#[test]
fn metering_is_published_and_safe_to_poll_while_rendering() {
    // The pattern an iOS host uses: audio runs on one thread, a UI polls
    // counters from another. Those reads must come from published state, not
    // from the live instrument the render thread owns.
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    struct Ptr(*const GooeyEngine);
    unsafe impl Send for Ptr {}
    unsafe impl Sync for Ptr {}

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
        gooey_engine_piano_note_on(engine, piano, 60, 1.0);

        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Ptr(engine as *const GooeyEngine));

        // A "UI thread" hammering the getters while we render.
        let poller = {
            let stop = Arc::clone(&stop);
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                let mut seen_voices = 0;
                while !stop.load(Ordering::Relaxed) {
                    seen_voices = seen_voices.max(gooey_engine_piano_active_voices(shared.0, 0));
                    let _ = gooey_engine_piano_zone_count(shared.0, 0);
                    let _ = gooey_engine_piano_map_generation(shared.0, 0);
                }
                seen_voices
            })
        };

        for _ in 0..200 {
            render(engine, 128);
        }
        stop.store(true, Ordering::Relaxed);
        let seen = poller.join().unwrap();

        assert_eq!(seen, 1, "the poller should have observed the sounding note");
        assert_eq!(gooey_engine_piano_zone_count(engine, piano), 2);
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
fn piano_velocity_mode_is_an_instrument_property_and_clamps() {
    unsafe {
        let engine = gooey_engine_new(SR);

        assert!(!gooey_engine_piano_set_velocity_mode(engine, 0, 0.5));
        assert!(gooey_engine_piano_get_velocity_mode(engine, 0).is_nan());
        let piano = gooey_engine_piano_register(engine) as u32;

        // Low-weighted is the instrument default.
        assert_eq!(gooey_engine_piano_get_velocity_mode(engine, piano), 0.0);
        assert!(gooey_engine_piano_set_velocity_mode(engine, piano, 0.5));
        assert_eq!(gooey_engine_piano_get_velocity_mode(engine, piano), 0.5);
        assert!(gooey_engine_piano_set_velocity_mode(engine, piano, -1.0));
        assert_eq!(gooey_engine_piano_get_velocity_mode(engine, piano), 0.0);
        assert!(gooey_engine_piano_set_velocity_mode(engine, piano, 2.0));
        assert_eq!(gooey_engine_piano_get_velocity_mode(engine, piano), 1.0);
        assert!(!gooey_engine_piano_set_velocity_mode(
            engine,
            piano,
            f32::NAN
        ));
        assert_eq!(gooey_engine_piano_get_velocity_mode(engine, piano), 1.0);
        assert!(!gooey_engine_piano_set_velocity_mode(
            engine,
            PIANO_INSTRUMENT_MAX,
            0.5
        ));
        assert!(gooey_engine_piano_get_velocity_mode(engine, PIANO_INSTRUMENT_MAX).is_nan());
        gooey_engine_free(engine);
    }
}

#[test]
fn chord_trigger_uses_theory_voicing_and_base_velocity() {
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
        assert!(gooey_engine_piano_set_velocity_mode(engine, piano, 0.5));

        assert!(gooey_engine_piano_trigger_chord(
            engine,
            piano,
            0, // C
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            4,
            0.1,
        ));
        let soft = peak(&render(engine, 512));
        assert_eq!(gooey_engine_piano_active_voices(engine, piano), 4);

        assert!(gooey_engine_piano_release_all(engine, piano));
        render(engine, SR as usize);
        assert!(gooey_engine_piano_trigger_chord(
            engine,
            piano,
            0, // C
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            4,
            1.0,
        ));
        let hard = peak(&render(engine, 512));
        assert_eq!(gooey_engine_piano_active_voices(engine, piano), 4);
        assert!(
            hard > soft * 1.5,
            "base chord velocity should reach a louder layer: hard {hard} vs soft {soft}"
        );

        assert!(!gooey_engine_piano_trigger_chord(
            engine,
            piano,
            0, // C
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            4,
            f32::NAN,
        ));
        assert!(!gooey_engine_piano_trigger_chord(
            engine,
            PIANO_INSTRUMENT_MAX,
            0, // C
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            4,
            1.0,
        ));
        gooey_engine_free(engine);
    }
}

#[test]
fn default_velocity_mode_is_low_weighted_and_deterministic() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let piano = gooey_engine_piano_register(engine) as u32;
        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_PIANO_BASE + piano,
            2
        ));

        let notes = [60, 64, 67, 71]; // Cmaj7, root position, octave 4.
        let expected = [1.0, 1.0 - 0.34 / 3.0, 1.0 - 0.68 / 3.0, 0.66];
        commit_exact_velocity_map(engine, piano, &notes, &expected);
        render(engine, 64);

        assert!(gooey_engine_piano_trigger_chord(
            engine,
            piano,
            0, // C
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            4,
            1.0,
        ));
        render(engine, 512);
        assert_eq!(gooey_engine_piano_active_voices(engine, piano), 4);

        // There is no hidden random control: identical chord hits use the same
        // per-note velocities until the instrument's slider property changes.
        assert!(gooey_engine_piano_trigger_chord(
            engine,
            piano,
            0, // C
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            4,
            1.0,
        ));
        gooey_engine_free(engine);
    }
}

#[test]
fn chord_trigger_reports_partial_maps_but_sounds_covered_notes() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let piano = gooey_engine_piano_register(engine) as u32;
        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_PIANO_BASE + piano,
            2
        ));
        // Cmaj7 is C4/E4/G4/B4. Stop the map at G4 so only B4 is missing.
        commit_two_layer_map_in_range(engine, piano, 8192, 60, 67);
        render(engine, 64);
        assert!(gooey_engine_piano_set_velocity_mode(engine, piano, 0.5));

        assert!(!gooey_engine_piano_trigger_chord(
            engine,
            piano,
            0, // C
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            4,
            1.0,
        ));
        assert!(peak(&render(engine, 512)) > 0.0);
        assert_eq!(gooey_engine_piano_active_voices(engine, piano), 3);
        gooey_engine_free(engine);
    }
}

#[test]
fn chord_trigger_overlaps_non_shared_notes_and_self_masks_restrikes() {
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
        assert!(gooey_engine_piano_set_velocity_mode(engine, piano, 0.5));

        assert!(gooey_engine_piano_trigger_chord(
            engine,
            piano,
            0, // C
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            4,
            0.8,
        ));
        render(engine, 512);
        assert_eq!(gooey_engine_piano_active_voices(engine, piano), 4);

        // Repeating the same chord fades and replaces its four shared keys.
        assert!(gooey_engine_piano_trigger_chord(
            engine,
            piano,
            0, // C
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            4,
            0.8,
        ));
        render(engine, 512);
        assert_eq!(gooey_engine_piano_active_voices(engine, piano), 4);

        // Dm7 shares no notes with Cmaj7, so both four-note chords remain.
        assert!(gooey_engine_piano_trigger_chord(
            engine,
            piano,
            0, // C
            SCALE_MAJOR,
            1,
            VOICING_ROOT_POSITION,
            4,
            0.8,
        ));
        render(engine, 512);
        assert_eq!(gooey_engine_piano_active_voices(engine, piano), 8);
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
        assert!(peak(&render(engine, 256)) > 0.01);
        assert_eq!(gooey_engine_piano_active_voices(engine, a), 1);
        assert_eq!(
            gooey_engine_piano_active_voices(engine, b),
            0,
            "playing one instrument must not trigger the other"
        );
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
