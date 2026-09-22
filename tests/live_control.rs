//! End-to-end coverage for the opt-in concurrent mixer/drum C ABI.

use gooey::ffi::*;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const SR: f32 = 44_100.0;

unsafe fn render(engine: *mut GooeyEngine, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0; frames * GOOEY_OUTPUT_CHANNELS as usize];
    gooey_engine_render(engine, output.as_mut_ptr(), frames as u32);
    output
}

#[test]
fn constants_nulls_and_generation_acknowledgement_are_stable() {
    assert_eq!(GOOEY_LIVE_CONTROL_API_VERSION, 1);
    assert_eq!(GOOEY_LIVE_CONTROL_QUEUE_CAPACITY, 64);
    assert_eq!(GOOEY_DRUM_LANE_COUNT, 4);
    assert_eq!(GOOEY_DRUM_STEP_COUNT, 16);
    assert_eq!(GOOEY_TRACK_RACK_MAX_EFFECTS, 8);
    unsafe {
        assert!(gooey_engine_live_control_new(std::ptr::null_mut()).is_null());
        assert_eq!(
            gooey_live_control_set_track_gain(std::ptr::null_mut(), 0, 1.0),
            0
        );
        assert_eq!(
            gooey_live_control_get_last_applied_generation(std::ptr::null()),
            0
        );

        let engine = gooey_engine_new(SR);
        let control = gooey_engine_live_control_new(engine);
        assert!(!control.is_null());
        assert!(gooey_engine_live_control_new(engine).is_null());
        let generation = gooey_live_control_set_track_gain(control, 0, 0.5);
        assert_ne!(generation, 0);
        assert_eq!(gooey_live_control_get_last_applied_generation(control), 0);
        let _ = render(engine, 1);
        assert_eq!(
            gooey_live_control_get_last_applied_generation(control),
            generation
        );
        gooey_live_control_free(control);
        gooey_engine_free(engine);
    }
}

#[test]
fn queue_full_rejects_without_displacing_the_last_acceptance() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let control = gooey_engine_live_control_new(engine);
        let mut last = 0;
        for index in 0..GOOEY_LIVE_CONTROL_QUEUE_CAPACITY {
            last = gooey_live_control_set_track_gain(control, 0, index as f32 / 64.0);
            assert_ne!(last, 0);
        }
        assert_eq!(gooey_live_control_set_track_gain(control, 0, 2.0), 0);
        let _ = render(engine, 1);
        assert_eq!(
            gooey_live_control_get_last_applied_generation(control),
            last
        );
        assert!((gooey_engine_mixer_get_track_gain(engine, 0) - 63.0 / 64.0).abs() < 1e-6);
        gooey_live_control_free(control);
        gooey_engine_free(engine);
    }
}

#[test]
fn complete_drum_snapshot_lands_atomically_and_empty_lanes_stay_empty() {
    unsafe {
        let engine = gooey_engine_new(SR);
        for instrument in 0..4 {
            gooey_engine_sequencer_set_instrument_step_with_velocity(
                engine, instrument, 0, true, 0.25,
            );
            gooey_engine_sequencer_set_instrument_step_blend(engine, instrument, 0, 0.2, 0.8);
            gooey_engine_sequencer_set_instrument_step_note(engine, instrument, 0, 60);
        }
        let control = gooey_engine_live_control_new(engine);
        let mut pattern = GooeyDrumPattern::default();
        pattern.lanes[GOOEY_DRUM_LANE_KICK as usize][3] = GooeyDrumStep {
            enabled: 1,
            velocity: 0.73,
        };
        let generation = gooey_live_control_submit_drum_pattern(control, &pattern);
        assert_ne!(generation, 0);
        assert!(gooey_engine_sequencer_get_instrument_step_enabled(
            engine, 0, 0
        ));
        let _ = render(engine, 0);
        assert_eq!(
            gooey_live_control_get_last_applied_generation(control),
            generation
        );
        for lane in 0..4 {
            for step in 0..16 {
                let expected = lane == GOOEY_DRUM_LANE_KICK && step == 3;
                assert_eq!(
                    gooey_engine_sequencer_get_instrument_step_enabled(engine, lane, step),
                    expected
                );
                let velocity =
                    gooey_engine_sequencer_get_instrument_step_velocity(engine, lane, step);
                let expected_velocity = if expected { 0.73 } else { 0.0 };
                assert!((velocity - expected_velocity).abs() < 1e-6);
            }
            assert_eq!(
                gooey_engine_sequencer_get_instrument_step_note(engine, lane, 0),
                STEP_NOTE_NONE
            );
        }

        let empty = GooeyDrumPattern::default();
        assert_ne!(gooey_live_control_submit_drum_pattern(control, &empty), 0);
        let audio = render(engine, 1);
        assert!(audio.iter().all(|sample| *sample == 0.0));
        gooey_live_control_free(control);
        gooey_engine_free(engine);
    }
}

#[test]
fn rack_descriptors_validate_copy_and_reject_stale_generations() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let control = gooey_engine_live_control_new(engine);
        let filter_params = [
            GooeyEffectParamDescriptor {
                param: FILTER_PARAM_CUTOFF,
                value: 1_200.0,
            },
            GooeyEffectParamDescriptor {
                param: FILTER_PARAM_RESONANCE,
                value: 0.4,
            },
        ];
        let delay_params = [
            GooeyEffectParamDescriptor {
                param: DELAY_PARAM_TIMING,
                value: DELAY_TIMING_EIGHTH as f32,
            },
            GooeyEffectParamDescriptor {
                param: DELAY_PARAM_MIX,
                value: 0.5,
            },
        ];
        let effects = [
            GooeyEffectDescriptor {
                effect: EFFECT_LOWPASS_FILTER,
                params: filter_params.as_ptr(),
                param_count: filter_params.len() as u32,
            },
            GooeyEffectDescriptor {
                effect: EFFECT_DELAY,
                params: delay_params.as_ptr(),
                param_count: delay_params.len() as u32,
            },
        ];
        let rack_generation = gooey_live_control_replace_track_rack(
            control,
            0,
            effects.as_ptr(),
            effects.len() as u32,
        );
        assert_ne!(rack_generation, 0);
        assert_eq!(
            gooey_live_control_set_track_effect_param(
                control,
                0,
                1,
                rack_generation - 1,
                DELAY_PARAM_FEEDBACK,
                0.7,
            ),
            0
        );
        let parameter_generation = gooey_live_control_set_track_effect_param(
            control,
            0,
            1,
            rack_generation,
            DELAY_PARAM_FEEDBACK,
            0.7,
        );
        assert_ne!(parameter_generation, 0);
        assert_eq!(
            gooey_live_control_replace_track_rack(control, 0, std::ptr::null(), 0),
            0,
            "a second replacement is rejected while the transition is active"
        );
        let _ = render(engine, 512);
        assert_eq!(
            gooey_live_control_get_last_applied_generation(control),
            parameter_generation
        );
        assert_eq!(gooey_engine_track_effect_count(engine, 0), 2);
        assert_eq!(
            gooey_engine_track_effect_type_at(engine, 0, 0),
            EFFECT_LOWPASS_FILTER as i32
        );
        assert_eq!(
            gooey_engine_track_effect_type_at(engine, 0, 1),
            EFFECT_DELAY as i32
        );
        let clear_generation =
            gooey_live_control_replace_track_rack(control, 0, std::ptr::null(), 0);
        assert_ne!(clear_generation, 0);

        let duplicate = [
            GooeyEffectParamDescriptor {
                param: FILTER_PARAM_CUTOFF,
                value: 1_000.0,
            },
            GooeyEffectParamDescriptor {
                param: FILTER_PARAM_CUTOFF,
                value: 2_000.0,
            },
        ];
        let bad = GooeyEffectDescriptor {
            effect: EFFECT_LOWPASS_FILTER,
            params: duplicate.as_ptr(),
            param_count: 2,
        };
        assert_eq!(
            gooey_live_control_replace_track_rack(control, 1, &bad, 1),
            0
        );
        assert_eq!(
            gooey_live_control_set_source_trim(control, SOURCE_DRUMKIT, f32::NAN),
            0
        );
        assert_eq!(gooey_live_control_set_track_gain(control, 99, 1.0), 0);

        let _ = render(engine, 512);
        gooey_live_control_free(control);
        gooey_engine_free(engine);
    }
}

#[test]
fn gain_and_source_trim_are_smoothed_and_isolated() {
    unsafe {
        let baseline = gooey_engine_new(SR);
        let adjusted = gooey_engine_new(SR);
        let control = gooey_engine_live_control_new(adjusted);
        assert_ne!(
            gooey_live_control_set_source_trim(control, SOURCE_DRUMKIT, 2.0),
            0
        );
        assert_ne!(gooey_live_control_set_track_gain(control, 1, 0.5), 0);
        let _ = render(baseline, 20_000);
        let _ = render(adjusted, 20_000);

        gooey_engine_trigger_instrument(baseline, INSTRUMENT_KICK);
        gooey_engine_trigger_instrument(adjusted, INSTRUMENT_KICK);
        let dry = render(baseline, 2_048);
        let doubled = render(adjusted, 2_048);
        let error = dry
            .iter()
            .zip(&doubled)
            .map(|(dry, doubled)| (doubled - dry * 2.0).abs())
            .fold(0.0_f32, f32::max);
        assert!(error < 1e-4, "source trim error {error}");

        gooey_live_control_free(control);
        gooey_engine_free(adjusted);
        gooey_engine_free(baseline);

        let baseline = gooey_engine_new(SR);
        let adjusted = gooey_engine_new(SR);
        let control = gooey_engine_live_control_new(adjusted);
        assert_ne!(gooey_live_control_set_track_gain(control, 1, 0.5), 0);
        let _ = render(baseline, 20_000);
        let _ = render(adjusted, 20_000);
        gooey_engine_trigger_instrument(baseline, INSTRUMENT_BASS);
        gooey_engine_trigger_instrument(adjusted, INSTRUMENT_BASS);
        let full = render(baseline, 2_048);
        let half = render(adjusted, 2_048);
        let error = full
            .iter()
            .zip(&half)
            .map(|(full, half)| (half - full * 0.5).abs())
            .fold(0.0_f32, f32::max);
        assert!(error < 1e-4, "independent bass track gain error {error}");

        gooey_live_control_free(control);
        gooey_engine_free(adjusted);
        gooey_engine_free(baseline);
    }
}

#[test]
fn control_submissions_and_rendering_can_run_concurrently() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let control = gooey_engine_live_control_new(engine);
        let engine_address = engine as usize;
        let worker = thread::spawn(move || {
            let engine = engine_address as *mut GooeyEngine;
            for _ in 0..2_000 {
                let audio = render(engine, 64);
                assert!(audio.iter().all(|sample| sample.is_finite()));
            }
        });
        let mut accepted = 0;
        for index in 0..5_000 {
            let gain = (index % 201) as f32 / 100.0;
            if gooey_live_control_set_track_gain(control, index % 4, gain) != 0 {
                accepted += 1;
            }
        }
        assert!(accepted > 0);
        worker.join().unwrap();
        let _ = render(engine, 1);
        gooey_live_control_free(control);
        gooey_engine_free(engine);
    }
}

#[test]
fn engine_free_waits_for_the_control_handle_to_detach() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let control = gooey_engine_live_control_new(engine);
        let engine_address = engine as usize;
        let (done_tx, done_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            gooey_engine_free(engine_address as *mut GooeyEngine);
            done_tx.send(()).unwrap();
        });
        thread::sleep(Duration::from_millis(10));
        assert!(done_rx.try_recv().is_err());
        assert_eq!(gooey_live_control_set_track_gain(control, 0, 1.0), 0);
        gooey_live_control_free(control);
        done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        worker.join().unwrap();
    }
}
