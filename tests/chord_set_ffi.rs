//! Integration coverage for the chord-set C surface.
//!
//! A chord set is a named seven-pad harmonic palette. Sets 0-4 are the plain
//! diatonic levels the library always had; set 5 ("Neo Soul") is the first
//! stylistic palette, and its pads may sit outside the key. These tests pin the
//! host-visible contract: the ids are stable, every pad of every set actually
//! sounds, recorded clips remember which palette they were played from, and an
//! unknown set id is inert rather than quietly substituting a wrong palette.

use std::ffi::CStr;

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

fn samples_per_step(bpm: f32) -> usize {
    ((60.0 / bpm) / 4.0 * SR) as usize
}

/// Read a static `*const c_char` the library promises never to free.
///
/// # Safety
/// `ptr` must be null or a valid pointer to a nul-terminated string.
unsafe fn static_str(ptr: *const std::ffi::c_char) -> Option<&'static str> {
    if ptr.is_null() {
        None
    } else {
        Some(CStr::from_ptr(ptr).to_str().expect("label must be UTF-8"))
    }
}

#[test]
fn chord_set_ids_are_stable() {
    assert_eq!(CHORD_SET_TRIADS, 0);
    assert_eq!(CHORD_SET_SEVENTHS, 1);
    assert_eq!(CHORD_SET_NINTHS, 2);
    assert_eq!(CHORD_SET_ELEVENTHS, 3);
    assert_eq!(CHORD_SET_THIRTEENTHS, 4);
    assert_eq!(CHORD_SET_NEO_SOUL, 5);
    assert_eq!(CHORD_SET_COUNT, 6);
    assert_eq!(gooey_chord_set_count(), CHORD_SET_COUNT);
    assert_eq!(gooey_chord_set_pad_count(), 7);
}

#[test]
fn metadata_is_populated_for_every_set_and_pad() {
    for set in 0..gooey_chord_set_count() {
        let name = unsafe { static_str(gooey_chord_set_name(set)) };
        assert!(
            name.is_some_and(|name| !name.is_empty()),
            "set {set} has no name"
        );

        for scale in [SCALE_MAJOR, SCALE_MINOR] {
            for degree in 0..gooey_chord_set_pad_count() {
                let label = unsafe { static_str(gooey_chord_set_entry_label(set, scale, degree)) };
                assert!(
                    label.is_some_and(|label| !label.is_empty()),
                    "set {set} scale {scale} pad {degree} has no label"
                );
                // A plain major triad's suffix is legitimately empty, so only
                // require that the pointer is present.
                assert!(
                    unsafe { static_str(gooey_chord_set_entry_quality_suffix(set, scale, degree)) }
                        .is_some(),
                    "set {set} scale {scale} pad {degree} has no quality suffix"
                );

                let notes = gooey_chord_set_entry_note_count(set, scale, degree);
                assert!(
                    (3..=6).contains(&notes),
                    "set {set} scale {scale} pad {degree} has {notes} notes"
                );
                assert!(gooey_chord_set_entry_root(set, 0, scale, degree) < 12);
                assert!(gooey_chord_set_available_voicing_count(set, 0, scale, degree) >= 2);
            }
        }
    }
}

#[test]
fn neo_soul_borrows_the_flat_seventh_and_names_its_pads() {
    // Pad 6 in C major is a bVII9 on A# — ten semitones above the key root.
    assert_eq!(
        gooey_chord_set_entry_root(CHORD_SET_NEO_SOUL, 0, SCALE_MAJOR, 6),
        10
    );
    unsafe {
        assert_eq!(
            static_str(gooey_chord_set_name(CHORD_SET_NEO_SOUL)),
            Some("Neo Soul")
        );
        assert_eq!(
            static_str(gooey_chord_set_entry_label(
                CHORD_SET_NEO_SOUL,
                SCALE_MAJOR,
                6
            )),
            Some("bVII9")
        );
        assert_eq!(
            static_str(gooey_chord_set_entry_quality_suffix(
                CHORD_SET_NEO_SOUL,
                SCALE_MAJOR,
                2
            )),
            Some("7#9")
        );
        assert_eq!(
            static_str(gooey_chord_set_entry_label(
                CHORD_SET_NEO_SOUL,
                SCALE_MINOR,
                1
            )),
            Some("iim9b5")
        );
    }

    // Every Neo Soul pad is five notes, leaving one of the poly synth's six
    // voices free to carry the previous chord's release tail.
    for scale in [SCALE_MAJOR, SCALE_MINOR] {
        for degree in 0..7 {
            assert_eq!(
                gooey_chord_set_entry_note_count(CHORD_SET_NEO_SOUL, scale, degree),
                5,
                "scale {scale} pad {degree}"
            );
        }
    }
}

#[test]
fn metadata_rejects_an_unknown_set() {
    let bad = CHORD_SET_COUNT;
    assert!(gooey_chord_set_name(bad).is_null());
    assert!(gooey_chord_set_entry_label(bad, SCALE_MAJOR, 0).is_null());
    assert!(gooey_chord_set_entry_quality_suffix(bad, SCALE_MAJOR, 0).is_null());
    assert_eq!(gooey_chord_set_entry_note_count(bad, SCALE_MAJOR, 0), 0);
    assert_eq!(
        gooey_chord_set_available_voicing_count(bad, 0, SCALE_MAJOR, 0),
        0
    );
    assert!(gooey_chord_set_name(u32::MAX).is_null());
}

#[test]
fn legacy_voicing_count_matches_the_sevenths_set() {
    for scale in [SCALE_MAJOR, SCALE_MINOR] {
        for degree in 0..7 {
            assert_eq!(
                gooey_engine_poly_available_voicing_count(0, scale, degree),
                gooey_chord_set_available_voicing_count(CHORD_SET_SEVENTHS, 0, scale, degree),
                "scale {scale} degree {degree}"
            );
        }
    }
}

#[test]
fn every_neo_soul_pad_renders_audio_in_both_scales() {
    unsafe {
        for scale in [SCALE_MAJOR, SCALE_MINOR] {
            for degree in 0..7 {
                let engine = gooey_engine_new(SR);
                gooey_engine_poly_trigger_chord_set(
                    engine,
                    CHORD_SET_NEO_SOUL,
                    0, // C major / C minor
                    scale,
                    degree,
                    VOICING_ROOT_POSITION,
                    POLY_PRESET_DEFAULT,
                    4,
                    0.8,
                );
                let level = peak(&render(engine, 1024));
                assert!(
                    level > 0.001,
                    "Neo Soul scale {scale} pad {degree} should sound (peak {level})"
                );
                gooey_engine_free(engine);
            }
        }
    }
}

#[test]
fn an_unknown_set_sounds_nothing_and_records_nothing() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let bpm = 120.0;
        gooey_engine_set_bpm(engine, bpm);
        gooey_engine_perf_set_record_mode(engine, PERF_RECORD_MODE_OVERDUB);
        gooey_engine_perf_set_record_armed(engine, true);
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 64);
        assert!(gooey_engine_perf_is_recording(engine));

        gooey_engine_poly_trigger_chord_set(
            engine,
            CHORD_SET_COUNT, // one past the last valid id
            0,
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            POLY_PRESET_DEFAULT,
            4,
            0.8,
        );
        assert_eq!(peak(&render(engine, 1024)), 0.0);
        gooey_engine_poly_release(engine);
        assert_eq!(gooey_engine_perf_get_event_count(engine), 0);
        gooey_engine_free(engine);
    }
}

#[test]
fn recorded_events_remember_their_chord_set() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let bpm = 120.0;
        gooey_engine_set_bpm(engine, bpm);
        gooey_engine_perf_set_record_mode(engine, PERF_RECORD_MODE_OVERDUB);
        gooey_engine_perf_set_record_armed(engine, true);
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 64);

        // Neo Soul pad 6 (the borrowed bVII9) through the new entry point.
        gooey_engine_poly_trigger_chord_set(
            engine,
            CHORD_SET_NEO_SOUL,
            0,
            SCALE_MAJOR,
            6,
            VOICING_ROOT_POSITION,
            POLY_PRESET_PAD,
            4,
            0.9,
        );
        let _ = render(engine, samples_per_step(bpm));
        gooey_engine_poly_release(engine);

        // The pre-chord-set entry point must keep recording diatonic sevenths.
        let _ = render(engine, samples_per_step(bpm));
        gooey_engine_poly_trigger_chord(
            engine,
            0,
            SCALE_MAJOR,
            4,
            VOICING_ROOT_POSITION,
            1,
            4,
            0.9,
        );
        let _ = render(engine, samples_per_step(bpm));
        gooey_engine_poly_release(engine);

        assert_eq!(gooey_engine_perf_get_event_count(engine), 2);
        assert_eq!(
            gooey_engine_perf_get_event_chord_set(engine, 0),
            CHORD_SET_NEO_SOUL
        );
        assert_eq!(
            gooey_engine_perf_get_event_chord_set(engine, 1),
            CHORD_SET_SEVENTHS
        );

        // The rest of the event still reads back through the untouched getter.
        let mut degree = 0u32;
        let mut preset = 0u32;
        assert!(gooey_engine_perf_get_event(
            engine,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut degree,
            std::ptr::null_mut(),
            &mut preset,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        ));
        assert_eq!(degree, 6);
        assert_eq!(preset, POLY_PRESET_PAD);

        // Out of range and null both report the legacy default.
        assert_eq!(
            gooey_engine_perf_get_event_chord_set(engine, 99),
            CHORD_SET_SEVENTHS
        );
        assert_eq!(
            gooey_engine_perf_get_event_chord_set(std::ptr::null(), 0),
            CHORD_SET_SEVENTHS
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn a_recorded_neo_soul_pad_replays_from_the_clip() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let bpm = 120.0;
        gooey_engine_set_bpm(engine, bpm);
        gooey_engine_perf_set_record_mode(engine, PERF_RECORD_MODE_PUNCH_OUT);
        gooey_engine_perf_set_record_armed(engine, true);
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 64);

        gooey_engine_poly_trigger_chord_set(
            engine,
            CHORD_SET_NEO_SOUL,
            0,
            SCALE_MAJOR,
            3, // IVmaj7#11
            VOICING_ROOT_POSITION,
            POLY_PRESET_DEFAULT,
            4,
            0.9,
        );
        let _ = render(engine, samples_per_step(bpm) * 2);
        gooey_engine_poly_release(engine);
        assert_eq!(gooey_engine_perf_get_event_count(engine), 1);

        // Finish the bar so punch-out completes and the pass becomes playable,
        // then let the loop come back around to the recorded pad.
        let _ = render(engine, samples_per_step(bpm) * 14 + 512);
        assert!(!gooey_engine_perf_is_record_armed(engine));
        let replayed = peak(&render(engine, samples_per_step(bpm)));
        assert!(
            replayed > 0.001,
            "clip replay should sound (peak {replayed})"
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn piano_chord_set_trigger_sounds_every_note_and_rejects_a_bad_set() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let piano = gooey_engine_piano_register(engine) as u32;
        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_PIANO_BASE + piano,
            2
        ));

        // One zone spanning the whole keyboard, so every chord note is mapped.
        assert!(gooey_engine_piano_zone_begin(engine, piano));
        let pcm = vec![0.5_f32; 8192 * 2];
        assert!(gooey_engine_piano_zone_add(
            engine,
            piano,
            pcm.as_ptr(),
            8192,
            2,
            SR,
            0,
            127,
            60,
            1,
            127,
            0.0,
            0.0,
            0.5,
            0.3,
            PIANO_LOOP_NONE,
            0,
            0,
        ));
        assert!(gooey_engine_piano_zone_commit(engine, piano));
        render(engine, 64);

        // Neo Soul pads are five notes, so five piano voices should engage.
        assert!(gooey_engine_piano_trigger_chord_set(
            engine,
            piano,
            CHORD_SET_NEO_SOUL,
            0,
            SCALE_MAJOR,
            2, // III7#9
            VOICING_ROOT_POSITION,
            4,
            0.8,
        ));
        assert!(peak(&render(engine, 512)) > 0.001);
        assert_eq!(gooey_engine_piano_active_voices(engine, piano), 5);

        assert!(gooey_engine_piano_release_all(engine, piano));
        render(engine, SR as usize);

        // An unknown set strikes nothing at all.
        assert!(!gooey_engine_piano_trigger_chord_set(
            engine,
            piano,
            CHORD_SET_COUNT,
            0,
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            4,
            0.8,
        ));
        render(engine, 512);
        assert_eq!(gooey_engine_piano_active_voices(engine, piano), 0);

        // The legacy entry point still plays diatonic sevenths: four notes.
        assert!(gooey_engine_piano_trigger_chord(
            engine,
            piano,
            0,
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            4,
            0.8,
        ));
        render(engine, 512);
        assert_eq!(gooey_engine_piano_active_voices(engine, piano), 4);
        gooey_engine_free(engine);
    }
}
