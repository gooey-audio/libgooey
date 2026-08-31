//! FFI coverage for editable poly-synth sounds and amplitude envelopes.

use gooey::ffi::*;

fn approx_eq(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 1e-6,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn current_preset_params_round_trip_and_survive_sound_changes() {
    unsafe {
        let engine = gooey_engine_new(44_100.0);

        assert_eq!(gooey_engine_poly_get_preset(engine), POLY_PRESET_DEFAULT);
        gooey_engine_poly_set_param(engine, POLY_PARAM_AMP_ATTACK, 0.21);
        gooey_engine_poly_set_param(engine, POLY_PARAM_AMP_DECAY, 0.32);
        gooey_engine_poly_set_param(engine, POLY_PARAM_AMP_SUSTAIN, 0.43);
        gooey_engine_poly_set_param(engine, POLY_PARAM_AMP_RELEASE, 0.54);
        gooey_engine_poly_set_param(engine, POLY_PARAM_FILTER_CUTOFF, 0.65);

        approx_eq(
            gooey_engine_poly_get_param(engine, POLY_PARAM_AMP_ATTACK),
            0.21,
        );
        approx_eq(
            gooey_engine_poly_get_preset_param(engine, POLY_PRESET_DEFAULT, POLY_PARAM_AMP_RELEASE),
            0.54,
        );

        gooey_engine_poly_set_preset(engine, POLY_PRESET_PAD);
        approx_eq(
            gooey_engine_poly_get_param(engine, POLY_PARAM_AMP_ATTACK),
            0.8,
        );
        gooey_engine_poly_set_preset(engine, POLY_PRESET_DEFAULT);
        approx_eq(
            gooey_engine_poly_get_param(engine, POLY_PARAM_AMP_ATTACK),
            0.21,
        );
        approx_eq(
            gooey_engine_poly_get_param(engine, POLY_PARAM_FILTER_CUTOFF),
            0.65,
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn inactive_sound_can_be_edited_without_changing_the_live_sound() {
    unsafe {
        let engine = gooey_engine_new(44_100.0);
        let default_attack = gooey_engine_poly_get_param(engine, POLY_PARAM_AMP_ATTACK);

        assert!(gooey_engine_poly_set_preset_param(
            engine,
            POLY_PRESET_STRINGS,
            POLY_PARAM_AMP_ATTACK,
            0.12,
        ));
        assert!(gooey_engine_poly_set_preset_param(
            engine,
            POLY_PRESET_STRINGS,
            POLY_PARAM_DETUNE_AMOUNT,
            0.73,
        ));
        approx_eq(
            gooey_engine_poly_get_param(engine, POLY_PARAM_AMP_ATTACK),
            default_attack,
        );

        gooey_engine_poly_trigger_chord(
            engine,
            0,
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            POLY_PRESET_STRINGS,
            4,
            0.8,
        );
        assert_eq!(gooey_engine_poly_get_preset(engine), POLY_PRESET_STRINGS);
        approx_eq(
            gooey_engine_poly_get_param(engine, POLY_PARAM_AMP_ATTACK),
            0.12,
        );
        approx_eq(
            gooey_engine_poly_get_param(engine, POLY_PARAM_DETUNE_AMOUNT),
            0.73,
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn preset_reset_restores_factory_values() {
    unsafe {
        let engine = gooey_engine_new(44_100.0);
        assert!(gooey_engine_poly_set_preset_param(
            engine,
            POLY_PRESET_PLUCK,
            POLY_PARAM_AMP_SUSTAIN,
            0.9,
        ));
        assert!(gooey_engine_poly_reset_preset(engine, POLY_PRESET_PLUCK));
        approx_eq(
            gooey_engine_poly_get_preset_param(engine, POLY_PRESET_PLUCK, POLY_PARAM_AMP_SUSTAIN),
            0.0,
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn poly_param_ffi_rejects_invalid_ids_and_clamps_values() {
    unsafe {
        let engine = gooey_engine_new(44_100.0);

        assert!(!gooey_engine_poly_set_preset_param(
            engine,
            POLY_PRESET_COUNT,
            POLY_PARAM_VOLUME,
            0.5,
        ));
        assert!(!gooey_engine_poly_set_preset_param(
            engine,
            POLY_PRESET_DEFAULT,
            POLY_PARAM_COUNT,
            0.5,
        ));
        assert!(gooey_engine_poly_get_param(engine, POLY_PARAM_COUNT).is_nan());
        assert!(
            gooey_engine_poly_get_preset_param(engine, POLY_PRESET_COUNT, POLY_PARAM_VOLUME,)
                .is_nan()
        );

        assert!(gooey_engine_poly_set_preset_param(
            engine,
            POLY_PRESET_PAD,
            POLY_PARAM_VOLUME,
            2.0,
        ));
        approx_eq(
            gooey_engine_poly_get_preset_param(engine, POLY_PRESET_PAD, POLY_PARAM_VOLUME),
            1.0,
        );

        assert!(gooey_engine_poly_get_param(std::ptr::null(), POLY_PARAM_VOLUME).is_nan());
        assert!(gooey_engine_poly_get_preset_param(
            std::ptr::null(),
            POLY_PRESET_PAD,
            POLY_PARAM_VOLUME,
        )
        .is_nan());
        assert!(!gooey_engine_poly_set_preset_param(
            std::ptr::null_mut(),
            POLY_PRESET_PAD,
            POLY_PARAM_VOLUME,
            0.5,
        ));
        assert!(!gooey_engine_poly_reset_preset(
            std::ptr::null_mut(),
            POLY_PRESET_PAD,
        ));

        gooey_engine_free(engine);
    }
}
