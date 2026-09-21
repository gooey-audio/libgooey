//! End-to-end coverage for chord-aware live melody through the C surface.

use gooey::ffi::*;

const SR: f32 = 44_100.0;

unsafe fn render(engine: *mut GooeyEngine, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0; frames * 2];
    gooey_engine_render(engine, output.as_mut_ptr(), frames as u32);
    output
}

fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0, f32::max)
}

unsafe fn melody_peak_at_volume(volume: f32) -> f32 {
    let engine = gooey_engine_new(SR);
    assert!(gooey_engine_mixer_unroute_source(engine, SOURCE_POLYSYNTH));
    gooey_engine_poly_trigger_chord(
        engine,
        0,
        SCALE_MAJOR,
        0,
        VOICING_ROOT_POSITION,
        POLY_PRESET_DEFAULT,
        4,
        0.8,
    );
    assert!(gooey_engine_melody_set_param(
        engine,
        POLY_PARAM_VOLUME,
        volume,
    ));
    // Twelve smoothing time constants lets the normalized parameter hit the
    // smoother's settled threshold before a note begins.
    let _ = render(engine, 8192);
    assert_eq!(gooey_engine_melody_note_on(engine, 67, 0.9), 67);
    let result = peak(&render(engine, 4096));
    gooey_engine_free(engine);
    result
}

unsafe fn commit_piano_map(engine: *mut GooeyEngine, piano: u32, low: u32, high: u32) {
    assert!(gooey_engine_piano_zone_begin(engine, piano));
    let pcm = vec![0.25_f32; 4096 * 2];
    assert!(gooey_engine_piano_zone_add(
        engine,
        piano,
        pcm.as_ptr(),
        4096,
        2,
        SR,
        low,
        high,
        60,
        1,
        127,
        0.0,
        0.0,
        0.5,
        0.1,
        PIANO_LOOP_NONE,
        0,
        0,
    ));
    assert!(gooey_engine_piano_zone_commit(engine, piano));
    let _ = render(engine, 32);
}

#[test]
fn pre_harmony_input_starts_on_chord_and_releases_are_independent() {
    unsafe {
        let engine = gooey_engine_new(SR);
        assert!(!gooey_engine_melody_has_harmony(engine));
        assert_eq!(gooey_engine_melody_note_on(engine, 66, 0.8), -1);
        assert_eq!(gooey_engine_melody_get_note(engine), -1);

        gooey_engine_poly_trigger_chord(
            engine,
            0,
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            POLY_PRESET_DEFAULT,
            4,
            0.8,
        );
        assert!(gooey_engine_melody_has_harmony(engine));
        assert_eq!(gooey_engine_melody_get_note(engine), 67);
        assert!(peak(&render(engine, 1024)) > 0.0001);

        // Releasing the accompaniment does not end the held melody.
        gooey_engine_poly_release(engine);
        assert_eq!(gooey_engine_melody_get_note(engine), 67);

        // Re-trigger the accompaniment, then release only the melody. The chord
        // must remain audible through its own independent synth instance.
        gooey_engine_poly_trigger_chord(
            engine,
            0,
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            POLY_PRESET_DEFAULT,
            4,
            0.8,
        );
        gooey_engine_melody_note_off(engine);
        assert_eq!(gooey_engine_melody_get_note(engine), -1);
        assert!(peak(&render(engine, 1024)) > 0.0001);

        gooey_engine_free(engine);
    }
}

#[test]
fn chord_changes_retarget_legato_and_extensions_are_available() {
    unsafe {
        let engine = gooey_engine_new(SR);
        gooey_engine_poly_trigger_chord(
            engine,
            0,
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            POLY_PRESET_DEFAULT,
            4,
            0.8,
        );
        // Cmaj7: F#4 resolves upward to G4.
        assert_eq!(gooey_engine_melody_note_on(engine, 66, 0.8), 67);

        gooey_engine_poly_trigger_chord(
            engine,
            0,
            SCALE_MAJOR,
            1,
            VOICING_ROOT_POSITION,
            POLY_PRESET_DEFAULT,
            4,
            0.8,
        );
        // Dm7: the same intended pitch now resolves down to F4.
        assert_eq!(gooey_engine_melody_get_note(engine), 65);

        gooey_engine_poly_trigger_chord_set(
            engine,
            CHORD_SET_NEO_SOUL,
            0,
            SCALE_MAJOR,
            2,
            VOICING_ROOT_POSITION,
            POLY_PRESET_DEFAULT,
            4,
            0.8,
        );
        // E7#9 includes G as an altered extension, so G4 is accepted exactly.
        assert_eq!(gooey_engine_melody_update_note(engine, 67), 67);

        gooey_engine_free(engine);
    }
}

#[test]
fn piano_chords_update_harmony_only_when_at_least_one_note_sounds() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let piano = gooey_engine_piano_register(engine) as u32;
        commit_piano_map(engine, piano, 0, 127);

        assert!(gooey_engine_piano_trigger_chord(
            engine,
            piano,
            0,
            SCALE_MAJOR,
            1,
            VOICING_ROOT_POSITION,
            4,
            0.8,
        ));
        assert_eq!(gooey_engine_melody_note_on(engine, 66, 0.8), 65);

        // A second registered piano whose only zone is outside the chord
        // sounds nothing; its failed trigger must not replace Dm7 harmony.
        let silent_piano = gooey_engine_piano_register(engine) as u32;
        commit_piano_map(engine, silent_piano, 0, 0);
        assert!(!gooey_engine_piano_trigger_chord(
            engine,
            silent_piano,
            0,
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            4,
            0.8,
        ));
        assert_eq!(gooey_engine_melody_get_note(engine), 65);

        gooey_engine_free(engine);
    }
}

#[test]
fn invalid_calls_preserve_state_and_parameters_are_independent() {
    unsafe {
        let engine = gooey_engine_new(SR);
        gooey_engine_poly_trigger_chord(
            engine,
            0,
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            POLY_PRESET_DEFAULT,
            4,
            0.8,
        );
        assert_eq!(gooey_engine_melody_note_on(engine, 66, 0.8), 67);

        assert_eq!(gooey_engine_melody_note_on(engine, 128, 0.8), -1);
        assert_eq!(gooey_engine_melody_note_on(engine, 60, f32::NAN), -1);
        assert_eq!(gooey_engine_melody_update_note(engine, 128), -1);
        assert_eq!(gooey_engine_melody_get_note(engine), 67);

        gooey_engine_poly_trigger_chord_set(
            engine,
            u32::MAX,
            0,
            SCALE_MAJOR,
            1,
            VOICING_ROOT_POSITION,
            POLY_PRESET_DEFAULT,
            4,
            0.8,
        );
        assert_eq!(gooey_engine_melody_get_note(engine), 67);

        assert!(gooey_engine_poly_set_param(
            engine,
            POLY_PARAM_FILTER_CUTOFF,
            0.8
        ));
        assert!(gooey_engine_melody_set_param(
            engine,
            POLY_PARAM_FILTER_CUTOFF,
            0.2,
        ));
        assert_eq!(
            gooey_engine_poly_get_param(engine, POLY_PARAM_FILTER_CUTOFF),
            0.8
        );
        assert_eq!(
            gooey_engine_melody_get_param(engine, POLY_PARAM_FILTER_CUTOFF),
            0.2
        );
        assert!(!gooey_engine_melody_set_param(engine, u32::MAX, 0.5));
        assert!(gooey_engine_melody_get_param(engine, u32::MAX).is_nan());

        gooey_engine_melody_clear_harmony(engine);
        assert!(!gooey_engine_melody_has_harmony(engine));
        assert_eq!(gooey_engine_melody_get_note(engine), -1);
        assert_eq!(gooey_engine_melody_update_note(engine, 60), -1);

        assert_eq!(
            gooey_engine_melody_note_on(std::ptr::null_mut(), 60, 1.0),
            -1
        );
        assert_eq!(gooey_engine_melody_get_note(std::ptr::null()), -1);
        assert!(!gooey_engine_melody_has_harmony(std::ptr::null()));
        assert!(gooey_engine_melody_get_param(std::ptr::null(), 0).is_nan());

        gooey_engine_free(engine);
    }
}

#[test]
fn melody_volume_attenuates_only_the_lead() {
    unsafe {
        let loud = melody_peak_at_volume(0.85);
        let quiet = melody_peak_at_volume(0.20);
        let muted = melody_peak_at_volume(0.0);
        assert!(loud > 0.0001);
        assert!(quiet < loud * 0.35, "quiet={quiet}, loud={loud}");
        assert!(muted < 0.000001, "muted peak={muted}");

        let engine = gooey_engine_new(SR);
        assert!(gooey_engine_melody_set_param(
            engine,
            POLY_PARAM_VOLUME,
            0.0,
        ));
        gooey_engine_poly_trigger_chord(
            engine,
            0,
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            POLY_PRESET_DEFAULT,
            4,
            0.8,
        );
        assert!(peak(&render(engine, 4096)) > 0.0001);
        gooey_engine_free(engine);
    }
}

#[test]
fn recorded_chord_playback_retargets_a_held_melody() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let beat_frames = (SR as usize * 60) / 120;

        gooey_engine_set_bpm(engine, 120.0);
        gooey_engine_perf_set_record_mode(engine, PERF_RECORD_MODE_OVERDUB);
        gooey_engine_perf_set_record_armed(engine, true);
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 64);

        gooey_engine_poly_trigger_chord(
            engine,
            0,
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            POLY_PRESET_DEFAULT,
            4,
            0.8,
        );
        let _ = render(engine, beat_frames);
        gooey_engine_poly_trigger_chord(
            engine,
            0,
            SCALE_MAJOR,
            1,
            VOICING_ROOT_POSITION,
            POLY_PRESET_DEFAULT,
            4,
            0.8,
        );
        let _ = render(engine, beat_frames);
        gooey_engine_poly_release(engine);
        gooey_engine_perf_set_record_armed(engine, false);
        assert_eq!(gooey_engine_perf_get_event_count(engine), 2);

        gooey_engine_melody_clear_harmony(engine);
        assert_eq!(gooey_engine_melody_note_on(engine, 66, 0.8), -1);

        // Advance from beat ~2 to the next loop. The event at tick 0 restores
        // Cmaj7 and starts the pre-held melody on G4.
        let _ = render(engine, beat_frames * 2 + 256);
        assert_eq!(gooey_engine_melody_get_note(engine), 67);

        // The second event was recorded one beat later and changes the held
        // melody to F4 from the same intended F#4 input.
        let _ = render(engine, beat_frames + 256);
        assert_eq!(gooey_engine_melody_get_note(engine), 65);

        gooey_engine_free(engine);
    }
}
