//! End-to-end coverage for the expressive poly-synth C surface.

use std::ptr;

use gooey::ffi::*;

const SR: f32 = 44_100.0;

fn approx_eq(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 1e-6,
        "expected {expected}, got {actual}"
    );
}

fn render(engine: *mut GooeyEngine, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0; frames * 2];
    unsafe { gooey_engine_render(engine, output.as_mut_ptr(), frames as u32) };
    output
}

fn samples_per_step(bpm: f32) -> usize {
    ((60.0 / bpm) / 4.0 * SR) as usize
}

#[test]
fn all_thirty_active_parameters_round_trip_and_survive_retrigger() {
    unsafe {
        let engine = gooey_engine_new(SR);
        assert_eq!(POLY_PARAM_COUNT, 30);
        assert_eq!(gooey_engine_poly_get_preset(engine), POLY_PRESET_DEFAULT);

        for param in 0..POLY_PARAM_COUNT {
            let value = (param + 1) as f32 / (POLY_PARAM_COUNT + 1) as f32;
            assert!(gooey_engine_poly_set_param(engine, param, value));
            approx_eq(gooey_engine_poly_get_param(engine, param), value);
            approx_eq(
                gooey_engine_poly_get_preset_param(engine, POLY_PRESET_DEFAULT, param),
                value,
            );
        }

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
        for param in 0..POLY_PARAM_COUNT {
            let expected = (param + 1) as f32 / (POLY_PARAM_COUNT + 1) as f32;
            approx_eq(gooey_engine_poly_get_param(engine, param), expected);
        }
        gooey_engine_free(engine);
    }
}

#[test]
fn editable_presets_are_isolated_selectable_and_resettable() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let factory_pad =
            gooey_engine_poly_get_preset_param(engine, POLY_PRESET_PAD, POLY_PARAM_STEREO_WIDTH);
        assert!(gooey_engine_poly_set_preset_param(
            engine,
            POLY_PRESET_PAD,
            POLY_PARAM_STEREO_WIDTH,
            0.13,
        ));
        assert!(gooey_engine_poly_set_preset_param(
            engine,
            POLY_PRESET_DEFAULT,
            POLY_PARAM_STEREO_WIDTH,
            0.77,
        ));
        approx_eq(
            gooey_engine_poly_get_param(engine, POLY_PARAM_STEREO_WIDTH),
            0.77,
        );

        assert!(gooey_engine_poly_set_preset(engine, POLY_PRESET_PAD));
        approx_eq(
            gooey_engine_poly_get_param(engine, POLY_PARAM_STEREO_WIDTH),
            0.13,
        );
        assert!(gooey_engine_poly_reset_preset(engine, POLY_PRESET_PAD));
        approx_eq(
            gooey_engine_poly_get_param(engine, POLY_PARAM_STEREO_WIDTH),
            factory_pad,
        );
        assert!(!gooey_engine_poly_set_preset(engine, POLY_PRESET_COUNT));
        assert_eq!(gooey_engine_poly_get_preset(engine), POLY_PRESET_PAD);
        gooey_engine_free(engine);
    }
}

#[test]
fn performance_replay_uses_the_engines_edited_preset_copy() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let bpm = 120.0;
        gooey_engine_set_bpm(engine, bpm);
        gooey_engine_perf_set_record_mode(engine, PERF_RECORD_MODE_PUNCH_OUT);
        gooey_engine_perf_set_record_armed(engine, true);
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 64);

        gooey_engine_poly_trigger_chord(
            engine,
            0,
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            POLY_PRESET_PAD,
            4,
            0.8,
        );
        let _ = render(engine, samples_per_step(bpm));
        gooey_engine_poly_release(engine);
        assert_eq!(gooey_engine_perf_get_event_count(engine), 1);

        // Edit the recorded preset after capture, select another sound, and
        // then cross the loop boundary. Playback must resolve the stable
        // preset id through this engine's editable preset bank.
        assert!(gooey_engine_poly_set_preset_param(
            engine,
            POLY_PRESET_PAD,
            POLY_PARAM_STEREO_WIDTH,
            0.19,
        ));
        assert!(gooey_engine_poly_set_preset(engine, POLY_PRESET_DEFAULT));
        let _ = render(engine, samples_per_step(bpm) * 16 + 512);

        assert_eq!(gooey_engine_poly_get_preset(engine), POLY_PRESET_PAD);
        approx_eq(
            gooey_engine_poly_get_param(engine, POLY_PARAM_STEREO_WIDTH),
            0.19,
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn all_eight_modulation_routes_round_trip_and_clear() {
    unsafe {
        let engine = gooey_engine_new(SR);
        assert_eq!(POLY_MOD_ROUTE_COUNT, 8);
        for slot in 0..POLY_MOD_ROUTE_COUNT {
            let expected = GooeyPolyModRoute {
                enabled: slot % 2 == 0,
                source: if slot % 2 == 0 {
                    POLY_MOD_SOURCE_VELOCITY
                } else {
                    POLY_MOD_SOURCE_KEY_POSITION
                },
                destination: slot % POLY_PARAM_COUNT,
                depth: -0.7 + slot as f32 * 0.2,
                curve: slot as f32 / (POLY_MOD_ROUTE_COUNT - 1) as f32,
                key_scale: 0.4 - slot as f32 * 0.1,
            };
            assert!(gooey_engine_poly_set_mod_route(
                engine,
                POLY_PRESET_KEYS,
                slot,
                expected,
            ));
            let mut actual = GooeyPolyModRoute {
                enabled: false,
                source: 0,
                destination: 0,
                depth: 0.0,
                curve: 0.0,
                key_scale: 0.0,
            };
            assert!(gooey_engine_poly_get_mod_route(
                engine,
                POLY_PRESET_KEYS,
                slot,
                &mut actual,
            ));
            assert_eq!(actual.enabled, expected.enabled);
            assert_eq!(actual.source, expected.source);
            assert_eq!(actual.destination, expected.destination);
            approx_eq(actual.depth, expected.depth);
            approx_eq(actual.curve, expected.curve);
            approx_eq(actual.key_scale, expected.key_scale);
        }

        assert!(gooey_engine_poly_clear_mod_route(
            engine,
            POLY_PRESET_KEYS,
            3,
        ));
        let mut cleared = GooeyPolyModRoute {
            enabled: true,
            source: 99,
            destination: 99,
            depth: 1.0,
            curve: 1.0,
            key_scale: 1.0,
        };
        assert!(gooey_engine_poly_get_mod_route(
            engine,
            POLY_PRESET_KEYS,
            3,
            &mut cleared,
        ));
        assert!(!cleared.enabled);
        approx_eq(cleared.depth, 0.0);
        gooey_engine_free(engine);
    }
}

#[test]
fn invalid_poly_inputs_leave_state_unchanged() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let before = gooey_engine_poly_get_param(engine, POLY_PARAM_VOLUME);
        assert!(!gooey_engine_poly_set_param(
            engine,
            POLY_PARAM_VOLUME,
            f32::NAN,
        ));
        assert!(!gooey_engine_poly_set_param(engine, POLY_PARAM_COUNT, 0.5));
        approx_eq(
            gooey_engine_poly_get_param(engine, POLY_PARAM_VOLUME),
            before,
        );
        assert!(gooey_engine_poly_get_param(engine, POLY_PARAM_COUNT).is_nan());

        let invalid_source = GooeyPolyModRoute {
            enabled: true,
            source: 99,
            destination: POLY_PARAM_VOLUME,
            depth: 0.5,
            curve: 0.5,
            key_scale: 0.0,
        };
        assert!(!gooey_engine_poly_set_mod_route(
            engine,
            POLY_PRESET_DEFAULT,
            0,
            invalid_source,
        ));
        assert!(!gooey_engine_poly_get_mod_route(
            engine,
            POLY_PRESET_DEFAULT,
            0,
            ptr::null_mut(),
        ));
        assert!(!gooey_engine_poly_set_param(
            ptr::null_mut(),
            POLY_PARAM_VOLUME,
            0.5,
        ));
        assert!(gooey_engine_poly_get_param(ptr::null(), POLY_PARAM_VOLUME).is_nan());
        gooey_engine_free(engine);
    }
}

#[test]
fn ffi_graph_preserves_the_poly_synth_native_stereo_image() {
    unsafe {
        let engine = gooey_engine_new(SR);
        for (param, value) in [
            (POLY_PARAM_OSC_A_WAVEFORM, 0.0),
            (POLY_PARAM_OSC_B_WAVEFORM, 1.0),
            (POLY_PARAM_OSC_A_LEVEL, 1.0),
            (POLY_PARAM_OSC_B_LEVEL, 1.0),
            (POLY_PARAM_DETUNE, 1.0),
            (POLY_PARAM_STEREO_WIDTH, 1.0),
            (POLY_PARAM_AMP_ATTACK, 0.0),
            (POLY_PARAM_AMP_SUSTAIN, 1.0),
            (POLY_PARAM_FILTER_CUTOFF, 1.0),
            (POLY_PARAM_FILTER_ENV_AMOUNT, 0.5),
            (POLY_PARAM_SATURATION, 0.0),
        ] {
            assert!(gooey_engine_poly_set_param(engine, param, value));
        }
        gooey_engine_poly_trigger_chord(
            engine,
            0,
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            POLY_PRESET_DEFAULT,
            4,
            1.0,
        );
        let samples = render(engine, 4096);
        let mut energy = 0.0;
        let mut side_energy = 0.0;
        for frame in samples.chunks_exact(2) {
            energy += frame[0] * frame[0] + frame[1] * frame[1];
            let side = frame[0] - frame[1];
            side_energy += side * side;
        }
        assert!(energy > 0.001, "poly source should be audible: {energy}");
        assert!(
            side_energy > 0.001,
            "width and independent waveforms should reach the graph: {side_energy}"
        );
        gooey_engine_free(engine);
    }
}
