//! Round-trip tests for the FFI parameter getters used by Swift state recovery.

use gooey::ffi::*;

fn approx_eq(a: f32, b: f32) {
    assert!(
        (a - b).abs() < 1e-6,
        "expected {b}, got {a} (delta {})",
        (a - b).abs()
    );
}

#[test]
fn kick_param_round_trip() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        gooey_engine_set_kick_param(engine, KICK_PARAM_FREQUENCY, 0.42);
        gooey_engine_set_kick_param(engine, KICK_PARAM_PUNCH, 0.13);
        gooey_engine_set_kick_param(engine, KICK_PARAM_PITCH_ENVELOPE, 0.77);
        gooey_engine_set_kick_param(engine, KICK_PARAM_TUNING, 0.6);

        approx_eq(
            gooey_engine_get_kick_param(engine, KICK_PARAM_FREQUENCY),
            0.42,
        );
        approx_eq(gooey_engine_get_kick_param(engine, KICK_PARAM_PUNCH), 0.13);
        approx_eq(
            gooey_engine_get_kick_param(engine, KICK_PARAM_PITCH_ENVELOPE),
            0.77,
        );
        approx_eq(gooey_engine_get_kick_param(engine, KICK_PARAM_TUNING), 0.6);

        assert!(gooey_engine_get_kick_param(engine, 999).is_nan());

        gooey_engine_free(engine);
    }
}

#[test]
fn snare_param_round_trip() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        gooey_engine_set_snare_param(engine, SNARE_PARAM_DECAY, 0.55);
        gooey_engine_set_snare_param(engine, SNARE_PARAM_NOISE, 0.31);
        gooey_engine_set_snare_param(engine, SNARE_PARAM_FILTER_TYPE, 2.0);

        approx_eq(
            gooey_engine_get_snare_param(engine, SNARE_PARAM_DECAY),
            0.55,
        );
        approx_eq(
            gooey_engine_get_snare_param(engine, SNARE_PARAM_NOISE),
            0.31,
        );
        assert_eq!(
            gooey_engine_get_snare_param(engine, SNARE_PARAM_FILTER_TYPE),
            2.0
        );

        assert!(gooey_engine_get_snare_param(engine, 999).is_nan());

        gooey_engine_free(engine);
    }
}

#[test]
fn hihat_param_round_trip() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        gooey_engine_set_hihat_param(engine, HIHAT_PARAM_TONE, 0.81);
        gooey_engine_set_hihat_param(engine, HIHAT_PARAM_ATTACK, 0.22);

        approx_eq(gooey_engine_get_hihat_param(engine, HIHAT_PARAM_TONE), 0.81);
        approx_eq(
            gooey_engine_get_hihat_param(engine, HIHAT_PARAM_ATTACK),
            0.22,
        );

        assert!(gooey_engine_get_hihat_param(engine, 999).is_nan());

        gooey_engine_free(engine);
    }
}

#[test]
fn tom_param_round_trip() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Params 0-7 are stored as 0-100 internally; the FFI must renormalize.
        gooey_engine_set_tom_param(engine, TOM_PARAM_TUNE, 0.4);
        gooey_engine_set_tom_param(engine, TOM_PARAM_DECAY, 0.9);
        // Tuning is the exception that stays on a 0-1 scale internally.
        gooey_engine_set_tom_param(engine, TOM_PARAM_TUNING, 0.7);

        approx_eq(gooey_engine_get_tom_param(engine, TOM_PARAM_TUNE), 0.4);
        approx_eq(gooey_engine_get_tom_param(engine, TOM_PARAM_DECAY), 0.9);
        approx_eq(gooey_engine_get_tom_param(engine, TOM_PARAM_TUNING), 0.7);

        assert!(gooey_engine_get_tom_param(engine, 999).is_nan());

        gooey_engine_free(engine);
    }
}

#[test]
fn piano_param_round_trip() {
    unsafe {
        let engine = gooey_engine_new(44100.0);
        assert!(gooey_engine_piano_get_param(engine, 0, PIANO_PARAM_RELEASE).is_nan());

        let piano = gooey_engine_piano_register(engine) as u32;
        let cases = [
            (PIANO_PARAM_VOLUME, -0.5, 0.0),
            (PIANO_PARAM_VELOCITY_TRACK, 0.22, 0.22),
            (PIANO_PARAM_RELEASE, 0.33, 0.33),
            (PIANO_PARAM_STEREO_WIDTH, 0.44, 0.44),
            (PIANO_PARAM_DYNAMIC_RANGE, 0.55, 0.55),
            (PIANO_PARAM_VELOCITY_SPAN, 1.5, 1.0),
        ];
        for (param, requested, expected) in cases {
            assert!(gooey_engine_piano_set_param(
                engine, piano, param, requested
            ));
            approx_eq(gooey_engine_piano_get_param(engine, piano, param), expected);
        }

        assert!(gooey_engine_piano_get_param(engine, piano, 999).is_nan());
        assert!(
            gooey_engine_piano_get_param(engine, PIANO_INSTRUMENT_MAX, PIANO_PARAM_RELEASE)
                .is_nan()
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn null_engine_returns_nan() {
    unsafe {
        assert!(gooey_engine_get_kick_param(std::ptr::null(), KICK_PARAM_FREQUENCY).is_nan());
        assert!(gooey_engine_get_snare_param(std::ptr::null(), SNARE_PARAM_DECAY).is_nan());
        assert!(gooey_engine_get_hihat_param(std::ptr::null(), HIHAT_PARAM_TONE).is_nan());
        assert!(gooey_engine_get_tom_param(std::ptr::null(), TOM_PARAM_TUNE).is_nan());
        assert!(gooey_engine_piano_get_param(std::ptr::null(), 0, PIANO_PARAM_RELEASE).is_nan());
    }
}
