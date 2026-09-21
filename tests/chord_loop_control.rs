//! End-to-end coverage for the real-time chord-loop and projected poly controls.

use std::ptr;
use std::sync::Arc;

use gooey::ffi::*;

const SR: f32 = 48_000.0;

fn chord(target: u32, target_id: u32, degree: u32) -> GooeyChordEvent {
    GooeyChordEvent {
        target,
        target_id,
        chord_set: CHORD_SET_TRIADS,
        root: 0,
        scale_type: SCALE_MAJOR,
        degree,
        voicing: VOICING_ROOT_POSITION,
        preset: POLY_PRESET_DEFAULT,
        octave: 4,
        velocity: 0.8,
    }
}

fn loop_event(start_tick: u32, duration_ticks: u32, degree: u32) -> GooeyChordLoopEvent {
    GooeyChordLoopEvent {
        start_tick,
        duration_ticks,
        chord: chord(GOOEY_CHORD_TARGET_POLY, 0, degree),
    }
}

fn render(engine: *mut GooeyEngine, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0; frames * GOOEY_OUTPUT_CHANNELS as usize];
    unsafe { gooey_engine_render(engine, output.as_mut_ptr(), frames as u32) };
    output
}

unsafe fn commit_full_piano(engine: *mut GooeyEngine, piano: u32) {
    assert!(gooey_engine_piano_zone_begin(engine, piano));
    let pcm = vec![0.4_f32; 4096];
    assert!(gooey_engine_piano_zone_add(
        engine,
        piano,
        pcm.as_ptr(),
        pcm.len() as u32,
        1,
        SR,
        0,
        127,
        60,
        1,
        127,
        0.0,
        0.0,
        0.5,
        0.0,
        PIANO_LOOP_CONTINUOUS,
        0,
        2_048,
    ));
    assert!(gooey_engine_piano_zone_commit(engine, piano));
    let _ = render(engine, 1);
}

#[test]
fn ffi_validation_sorting_lengths_and_generation_are_atomic() {
    assert_eq!(GOOEY_CHORD_TARGET_POLY, 0);
    assert_eq!(GOOEY_CHORD_TARGET_PIANO, 1);
    assert_eq!(GOOEY_CHORD_LOOP_TICKS_PER_QUARTER, 96);
    assert_eq!(GOOEY_CHORD_LOOP_MAX_EVENTS, 512);

    unsafe {
        assert!(!gooey_engine_chord_enqueue_trigger(
            ptr::null(),
            ptr::null()
        ));
        assert!(!gooey_engine_chord_enqueue_release_all(ptr::null()));
        assert_eq!(
            gooey_engine_chord_loop_replace(ptr::null(), ptr::null(), 0, 384),
            0
        );

        let engine = gooey_engine_new(SR);
        assert_eq!(
            gooey_engine_chord_loop_replace(engine, ptr::null(), 1, 384),
            0
        );
        assert_eq!(
            gooey_engine_chord_loop_replace(
                engine,
                ptr::null(),
                GOOEY_CHORD_LOOP_MAX_EVENTS + 1,
                384,
            ),
            0
        );
        assert_eq!(
            gooey_engine_chord_loop_replace(engine, ptr::null(), 0, 0),
            0
        );

        let unsorted = [loop_event(200, 20, 4), loop_event(0, 96, 0)];
        let accepted = gooey_engine_chord_loop_replace(
            engine,
            unsorted.as_ptr(),
            unsorted.len() as u32,
            3_072,
        );
        assert_ne!(accepted, 0);

        let invalid_overlap = [loop_event(3_060, 20, 1), loop_event(0, 12, 2)];
        assert_eq!(
            gooey_engine_chord_loop_replace(
                engine,
                invalid_overlap.as_ptr(),
                invalid_overlap.len() as u32,
                3_072,
            ),
            0
        );
        let invalid_arrays = [
            GooeyChordLoopEvent {
                start_tick: 3_072,
                ..loop_event(0, 1, 0)
            },
            GooeyChordLoopEvent {
                duration_ticks: 0,
                ..loop_event(0, 1, 0)
            },
            GooeyChordLoopEvent {
                duration_ticks: 3_073,
                ..loop_event(0, 1, 0)
            },
            GooeyChordLoopEvent {
                chord: GooeyChordEvent {
                    velocity: f32::NAN,
                    ..chord(GOOEY_CHORD_TARGET_POLY, 0, 0)
                },
                ..loop_event(0, 1, 0)
            },
            GooeyChordLoopEvent {
                chord: GooeyChordEvent {
                    chord_set: CHORD_SET_COUNT,
                    ..chord(GOOEY_CHORD_TARGET_POLY, 0, 0)
                },
                ..loop_event(0, 1, 0)
            },
            GooeyChordLoopEvent {
                chord: GooeyChordEvent {
                    target: 99,
                    ..chord(GOOEY_CHORD_TARGET_POLY, 0, 0)
                },
                ..loop_event(0, 1, 0)
            },
            GooeyChordLoopEvent {
                chord: GooeyChordEvent {
                    preset: POLY_PRESET_COUNT,
                    ..chord(GOOEY_CHORD_TARGET_POLY, 0, 0)
                },
                ..loop_event(0, 1, 0)
            },
            GooeyChordLoopEvent {
                chord: chord(GOOEY_CHORD_TARGET_PIANO, 0, 0),
                ..loop_event(0, 1, 0)
            },
        ];
        for invalid in invalid_arrays {
            assert_eq!(
                gooey_engine_chord_loop_replace(engine, &invalid, 1, 3_072),
                0
            );
        }

        assert_eq!(gooey_engine_chord_loop_get_applied_generation(engine), 0);
        let _ = render(engine, 1);
        assert_eq!(
            gooey_engine_chord_loop_get_applied_generation(engine),
            accepted
        );
        assert_eq!(gooey_engine_perf_get_length_ticks(engine), 3_072);
        assert_eq!(gooey_engine_perf_get_event_count(engine), 2);
        let mut first_start = u32::MAX;
        assert!(gooey_engine_perf_get_event(
            engine,
            0,
            &mut first_start,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        ));
        assert_eq!(first_start, 0);

        let empty = gooey_engine_chord_loop_replace(engine, ptr::null(), 0, 3_072);
        assert!(empty > accepted);
        let _ = render(engine, 1);
        assert_eq!(gooey_engine_perf_get_event_count(engine), 0);
        assert_eq!(
            gooey_engine_chord_loop_get_applied_generation(engine),
            empty
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn poly_batches_are_projected_atomic_and_last_value_wins() {
    unsafe {
        let engine = gooey_engine_new(SR);
        assert!(gooey_engine_poly_set_preset_params(
            engine,
            POLY_PRESET_DEFAULT,
            ptr::null(),
            0,
        ));
        assert!(!gooey_engine_poly_set_preset_params(
            engine,
            POLY_PRESET_COUNT,
            ptr::null(),
            0,
        ));
        assert!(!gooey_engine_poly_set_preset_params(
            engine,
            POLY_PRESET_DEFAULT,
            ptr::null(),
            1,
        ));

        let values = [
            GooeyPolyParamValue {
                param: POLY_PARAM_VOLUME,
                value: 0.2,
            },
            GooeyPolyParamValue {
                param: POLY_PARAM_FILTER_CUTOFF,
                value: 0.35,
            },
            GooeyPolyParamValue {
                param: POLY_PARAM_VOLUME,
                value: 0.75,
            },
        ];
        assert!(gooey_engine_poly_set_preset_params(
            engine,
            POLY_PRESET_DEFAULT,
            values.as_ptr(),
            values.len() as u32,
        ));
        assert_eq!(
            gooey_engine_poly_get_preset_param(engine, POLY_PRESET_DEFAULT, POLY_PARAM_VOLUME,),
            0.75
        );

        let before = gooey_engine_poly_get_preset_param(
            engine,
            POLY_PRESET_DEFAULT,
            POLY_PARAM_FILTER_CUTOFF,
        );
        let invalid = [
            GooeyPolyParamValue {
                param: POLY_PARAM_FILTER_CUTOFF,
                value: 0.9,
            },
            GooeyPolyParamValue {
                param: u32::MAX,
                value: 0.4,
            },
        ];
        assert!(!gooey_engine_poly_set_preset_params(
            engine,
            POLY_PRESET_DEFAULT,
            invalid.as_ptr(),
            invalid.len() as u32,
        ));
        assert_eq!(
            gooey_engine_poly_get_preset_param(
                engine,
                POLY_PRESET_DEFAULT,
                POLY_PARAM_FILTER_CUTOFF,
            ),
            before
        );

        let route = GooeyPolyModRoute {
            enabled: true,
            source: POLY_MOD_SOURCE_VELOCITY,
            destination: POLY_PARAM_FILTER_CUTOFF,
            depth: 0.4,
            curve: 0.6,
            key_scale: -0.2,
        };
        assert!(gooey_engine_poly_set_mod_route(
            engine,
            POLY_PRESET_DEFAULT,
            0,
            route,
        ));
        let mut projected = GooeyPolyModRoute {
            enabled: false,
            source: 0,
            destination: 0,
            depth: 0.0,
            curve: 0.0,
            key_scale: 0.0,
        };
        assert!(gooey_engine_poly_get_mod_route(
            engine,
            POLY_PRESET_DEFAULT,
            0,
            &mut projected,
        ));
        assert!(projected.enabled);
        assert_eq!(projected.depth, 0.4);
        assert!(gooey_engine_poly_clear_mod_route(
            engine,
            POLY_PRESET_DEFAULT,
            0,
        ));
        let _ = render(engine, 1);
        gooey_engine_free(engine);
    }
}

#[test]
fn manual_poly_and_registered_piano_commands_latch_harmony() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let piano_event = chord(GOOEY_CHORD_TARGET_PIANO, 0, 0);
        assert!(!gooey_engine_chord_enqueue_trigger(engine, &piano_event));

        let poly_event = chord(GOOEY_CHORD_TARGET_POLY, 0, 0);
        assert!(gooey_engine_chord_enqueue_trigger(engine, &poly_event));
        assert!(!gooey_engine_melody_has_harmony(engine));
        let _ = render(engine, 1);
        assert!(gooey_engine_melody_has_harmony(engine));
        assert!(gooey_engine_chord_enqueue_release_all(engine));
        let _ = render(engine, 1);

        let piano = gooey_engine_piano_register(engine) as u32;
        assert_eq!(piano, 0);
        let piano_event = chord(GOOEY_CHORD_TARGET_PIANO, piano, 3);
        let partial_event = chord(GOOEY_CHORD_TARGET_PIANO, piano, 0);
        assert!(gooey_engine_chord_enqueue_trigger(engine, &piano_event));
        gooey_engine_melody_clear_harmony(engine);
        let _ = render(engine, 1);
        assert!(gooey_engine_melody_has_harmony(engine));

        // Acceptance is independent of sample coverage: a partial map plays
        // its covered chord tone and silently skips the others.
        assert!(gooey_engine_piano_zone_begin(engine, piano));
        let pcm = vec![0.4_f32; 2_048];
        assert!(gooey_engine_piano_zone_add(
            engine,
            piano,
            pcm.as_ptr(),
            pcm.len() as u32,
            1,
            SR,
            60,
            60,
            60,
            1,
            127,
            0.0,
            0.0,
            0.5,
            0.0,
            PIANO_LOOP_CONTINUOUS,
            0,
            1_024,
        ));
        assert!(gooey_engine_piano_zone_commit(engine, piano));
        let _ = render(engine, 1);
        assert!(gooey_engine_chord_enqueue_trigger(engine, &partial_event));
        let _ = render(engine, 1);
        assert_eq!(gooey_engine_piano_active_voices(engine, piano), 1);
        gooey_engine_free(engine);
    }
}

#[test]
fn queued_piano_release_preserves_unrelated_notes_and_damper_behavior() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let piano = gooey_engine_piano_register(engine) as u32;
        commit_full_piano(engine, piano);
        let event = chord(GOOEY_CHORD_TARGET_PIANO, piano, 0);

        assert!(gooey_engine_chord_enqueue_trigger(engine, &event));
        let _ = render(engine, 1);
        assert_eq!(gooey_engine_piano_active_voices(engine, piano), 3);
        assert!(gooey_engine_piano_note_on(engine, piano, 61, 0.8));
        assert!(gooey_engine_chord_enqueue_release_all(engine));
        let _ = render(engine, 4_096);
        assert_eq!(
            gooey_engine_piano_active_voices(engine, piano),
            1,
            "queued release must not release a separately held piano key"
        );
        assert!(gooey_engine_piano_note_off(engine, piano, 61));
        let _ = render(engine, 4_096);
        assert_eq!(gooey_engine_piano_active_voices(engine, piano), 0);

        assert!(gooey_engine_chord_enqueue_trigger(engine, &event));
        let _ = render(engine, 1);
        assert!(gooey_engine_piano_set_sustain(engine, piano, true));
        assert!(gooey_engine_chord_enqueue_release_all(engine));
        let _ = render(engine, 4_096);
        assert_eq!(
            gooey_engine_piano_active_voices(engine, piano),
            3,
            "note_off must respect the piano damper"
        );
        assert!(gooey_engine_piano_set_sustain(engine, piano, false));
        let _ = render(engine, 4_096);
        assert_eq!(gooey_engine_piano_active_voices(engine, piano), 0);
        gooey_engine_free(engine);
    }
}

#[test]
fn piano_events_trigger_on_exact_tick_samples_for_all_nebula_lengths() {
    unsafe {
        for bpm in [60.0_f32, 120.0, 150.0] {
            let engine = gooey_engine_new(SR);
            gooey_engine_set_bpm(engine, bpm);
            let piano = gooey_engine_piano_register(engine) as u32;
            commit_full_piano(engine, piano);
            let samples_per_tick = (SR * 60.0 / bpm / 96.0) as usize;

            for length in [384_u32, 768, 1_536, 3_072] {
                gooey_engine_sequencer_stop(engine);
                let _ = render(engine, 1);
                assert!(gooey_engine_piano_release_all(engine, piano));
                let _ = render(engine, 1);

                let event = GooeyChordLoopEvent {
                    start_tick: length - 1,
                    duration_ticks: 1,
                    chord: chord(GOOEY_CHORD_TARGET_PIANO, piano, 0),
                };
                let generation = gooey_engine_chord_loop_replace(engine, &event, 1, length);
                assert_ne!(generation, 0);
                let _ = render(engine, 1);
                assert_eq!(
                    gooey_engine_chord_loop_get_applied_generation(engine),
                    generation
                );

                gooey_engine_sequencer_set_beat_position(engine, f64::from(length - 2) / 96.0);
                gooey_engine_sequencer_start(engine);
                let _ = render(engine, samples_per_tick);
                assert_eq!(gooey_engine_piano_active_voices(engine, piano), 0);

                let _ = render(engine, 1);
                assert_eq!(
                    gooey_engine_piano_active_voices(engine, piano),
                    3,
                    "trigger bpm={bpm} length={length} beat={}",
                    gooey_engine_transport_get_beat_position(engine)
                );
                let _ = render(engine, samples_per_tick - 1);
                assert_eq!(gooey_engine_piano_active_voices(engine, piano), 3);
                // The release command lands on this wrap sample. Piano voice
                // metering includes the short envelope tail, so allow it to
                // finish before observing zero active voices.
                let _ = render(engine, 1);
                let _ = render(engine, 4_096);
                assert_eq!(gooey_engine_piano_active_voices(engine, piano), 0);
            }
            gooey_engine_free(engine);
        }
    }
}

#[test]
fn running_replacements_coalesce_at_old_wrap_and_clear_preserves_transport() {
    unsafe {
        let engine = gooey_engine_new(SR);
        gooey_engine_set_bpm(engine, 120.0);
        gooey_engine_set_metronome_enabled(engine, true);
        let old = loop_event(0, 96, 0);
        let old_generation = gooey_engine_chord_loop_replace(engine, &old, 1, 96);
        let _ = render(engine, 1);
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 1);

        let first = loop_event(80, 20, 3);
        let first_generation = gooey_engine_chord_loop_replace(engine, &first, 1, 192);
        let newest = loop_event(90, 20, 5);
        let newest_generation = gooey_engine_chord_loop_replace(engine, &newest, 1, 192);
        assert!(newest_generation > first_generation);
        assert_eq!(
            gooey_engine_chord_loop_get_applied_generation(engine),
            old_generation
        );

        gooey_engine_sequencer_set_beat_position(engine, 0.99);
        let _ = render(engine, 240);
        assert_eq!(
            gooey_engine_chord_loop_get_applied_generation(engine),
            old_generation
        );
        let _ = render(engine, 1);
        assert_eq!(
            gooey_engine_chord_loop_get_applied_generation(engine),
            newest_generation
        );
        assert_eq!(gooey_engine_perf_get_length_ticks(engine), 192);
        let mut degree = u32::MAX;
        assert!(gooey_engine_perf_get_event(
            engine,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut degree,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        ));
        assert_eq!(degree, 5);

        let beat_before_clear = gooey_engine_transport_get_beat_position(engine);
        let clear_generation = gooey_engine_chord_loop_clear(engine);
        let _ = render(engine, 1);
        assert_eq!(
            gooey_engine_chord_loop_get_applied_generation(engine),
            clear_generation
        );
        assert_eq!(gooey_engine_perf_get_event_count(engine), 0);
        assert!(gooey_engine_get_metronome_enabled(engine));
        assert!(gooey_engine_transport_get_beat_position(engine) > beat_before_clear);
        gooey_engine_free(engine);
    }
}

#[test]
fn host_replace_disarms_recording_and_clears_sampler_lane() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let rack = gooey_engine_sampler_register(engine) as u32;
        let pcm = vec![0.3_f32; 1024];
        assert!(gooey_engine_sampler_set_slot_buffer(
            engine,
            rack,
            0,
            pcm.as_ptr(),
            pcm.len() as u32,
            1,
            SR,
        ));
        let _ = render(engine, 1);
        gooey_engine_perf_set_record_mode(engine, PERF_RECORD_MODE_OVERDUB);
        gooey_engine_perf_set_record_armed(engine, true);
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 1);
        assert!(gooey_engine_perf_is_recording(engine));
        assert!(gooey_engine_sampler_trigger(engine, rack, 0, 0.7));
        assert_eq!(gooey_engine_perf_get_sampler_event_count(engine), 1);

        let replacement = loop_event(0, 24, 0);
        assert_ne!(
            gooey_engine_chord_loop_replace(engine, &replacement, 1, 384),
            0
        );
        let _ = render(engine, 1);
        assert!(!gooey_engine_perf_is_record_armed(engine));
        assert!(!gooey_engine_perf_is_recording(engine));
        assert_eq!(gooey_engine_perf_get_sampler_event_count(engine), 0);
        gooey_engine_free(engine);
    }
}

#[test]
fn producer_staging_can_run_concurrently_with_finite_monotonic_rendering() {
    struct SharedEngine(*const GooeyEngine);
    unsafe impl Send for SharedEngine {}
    unsafe impl Sync for SharedEngine {}

    unsafe {
        let engine = gooey_engine_new(SR);
        gooey_engine_set_bpm(engine, 120.0);
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 1);
        let shared = Arc::new(SharedEngine(engine));
        let producer = {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                for index in 0..500_u32 {
                    let values = [GooeyPolyParamValue {
                        param: POLY_PARAM_FILTER_CUTOFF,
                        value: (index % 101) as f32 / 100.0,
                    }];
                    assert!(gooey_engine_poly_set_preset_params(
                        shared.0,
                        POLY_PRESET_DEFAULT,
                        values.as_ptr(),
                        1,
                    ));
                    let event = loop_event(0, 24, index % 7);
                    assert_ne!(gooey_engine_chord_loop_replace(shared.0, &event, 1, 96), 0);
                }
            })
        };

        let mut previous_beat = gooey_engine_transport_get_beat_position(engine);
        for _ in 0..500 {
            let output = render(engine, 64);
            assert!(output.iter().all(|sample| sample.is_finite()));
            let beat = gooey_engine_transport_get_beat_position(engine);
            assert!(beat >= previous_beat);
            previous_beat = beat;
        }
        producer.join().unwrap();
        let _ = render(engine, 1);
        assert!(gooey_engine_chord_loop_get_applied_generation(engine) > 0);
        gooey_engine_free(engine);
    }
}
