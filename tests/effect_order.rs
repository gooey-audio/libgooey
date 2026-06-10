//! Integration tests for the reorderable global-effects chain.

use gooey::ffi::*;

const N: usize = REORDERABLE_EFFECT_COUNT as usize;

fn read_order(engine: *mut GooeyEngine) -> [u32; N] {
    let mut out = [0u32; N];
    let written = unsafe { gooey_engine_get_effect_order(engine, out.as_mut_ptr(), N as u32) };
    assert_eq!(written, REORDERABLE_EFFECT_COUNT);
    out
}

#[test]
fn default_order_matches_legacy_chain() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        let order = read_order(engine);
        assert_eq!(
            order,
            [
                EFFECT_SATURATION,
                EFFECT_LOWPASS_FILTER,
                EFFECT_TILT_FILTER,
                EFFECT_DELAY,
                EFFECT_COMPRESSOR,
                EFFECT_REVERB,
            ]
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn reorderable_count_is_six() {
    assert_eq!(gooey_engine_reorderable_effect_count(), 6);
}

#[test]
fn bulk_set_then_get_round_trips() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        let new_order = [
            EFFECT_REVERB,
            EFFECT_DELAY,
            EFFECT_COMPRESSOR,
            EFFECT_SATURATION,
            EFFECT_TILT_FILTER,
            EFFECT_LOWPASS_FILTER,
        ];
        let ok = gooey_engine_set_effect_order(engine, new_order.as_ptr(), N as u32);
        assert!(ok);
        assert_eq!(read_order(engine), new_order);

        gooey_engine_free(engine);
    }
}

#[test]
fn move_effect_to_front_shifts_others_right() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Move REVERB (last) to position 0; others should shift right preserving order.
        let ok = gooey_engine_move_effect(engine, EFFECT_REVERB, 0);
        assert!(ok);
        assert_eq!(
            read_order(engine),
            [
                EFFECT_REVERB,
                EFFECT_SATURATION,
                EFFECT_LOWPASS_FILTER,
                EFFECT_TILT_FILTER,
                EFFECT_DELAY,
                EFFECT_COMPRESSOR,
            ]
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn move_effect_to_back_shifts_others_left() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Move SATURATION (first) to last reorderable position.
        let ok = gooey_engine_move_effect(engine, EFFECT_SATURATION, N as u32 - 1);
        assert!(ok);
        assert_eq!(
            read_order(engine),
            [
                EFFECT_LOWPASS_FILTER,
                EFFECT_TILT_FILTER,
                EFFECT_DELAY,
                EFFECT_COMPRESSOR,
                EFFECT_REVERB,
                EFFECT_SATURATION,
            ]
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn move_effect_to_same_position_is_noop() {
    unsafe {
        let engine = gooey_engine_new(44100.0);
        let before = read_order(engine);

        let ok = gooey_engine_move_effect(engine, EFFECT_DELAY, 3);
        assert!(ok);
        assert_eq!(read_order(engine), before);

        gooey_engine_free(engine);
    }
}

#[test]
fn limiter_cannot_be_set_in_order() {
    unsafe {
        let engine = gooey_engine_new(44100.0);
        let before = read_order(engine);

        // Replace REVERB with LIMITER — must be rejected.
        let bad = [
            EFFECT_SATURATION,
            EFFECT_LOWPASS_FILTER,
            EFFECT_TILT_FILTER,
            EFFECT_DELAY,
            EFFECT_COMPRESSOR,
            EFFECT_LIMITER,
        ];
        let ok = gooey_engine_set_effect_order(engine, bad.as_ptr(), N as u32);
        assert!(!ok, "LIMITER must not be accepted in chain order");
        assert_eq!(
            read_order(engine),
            before,
            "order must be unchanged on reject"
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn limiter_cannot_be_moved() {
    unsafe {
        let engine = gooey_engine_new(44100.0);
        let before = read_order(engine);

        let ok = gooey_engine_move_effect(engine, EFFECT_LIMITER, 0);
        assert!(!ok);
        assert_eq!(read_order(engine), before);

        gooey_engine_free(engine);
    }
}

#[test]
fn duplicate_ids_rejected() {
    unsafe {
        let engine = gooey_engine_new(44100.0);
        let before = read_order(engine);

        let bad = [
            EFFECT_SATURATION,
            EFFECT_SATURATION,
            EFFECT_TILT_FILTER,
            EFFECT_DELAY,
            EFFECT_COMPRESSOR,
            EFFECT_REVERB,
        ];
        let ok = gooey_engine_set_effect_order(engine, bad.as_ptr(), N as u32);
        assert!(!ok);
        assert_eq!(read_order(engine), before);

        gooey_engine_free(engine);
    }
}

#[test]
fn wrong_length_rejected() {
    unsafe {
        let engine = gooey_engine_new(44100.0);
        let before = read_order(engine);

        let short = [EFFECT_SATURATION, EFFECT_DELAY];
        let ok = gooey_engine_set_effect_order(engine, short.as_ptr(), short.len() as u32);
        assert!(!ok);
        assert_eq!(read_order(engine), before);

        gooey_engine_free(engine);
    }
}

#[test]
fn unknown_id_rejected() {
    unsafe {
        let engine = gooey_engine_new(44100.0);
        let before = read_order(engine);

        let bad = [
            EFFECT_SATURATION,
            EFFECT_LOWPASS_FILTER,
            EFFECT_TILT_FILTER,
            EFFECT_DELAY,
            EFFECT_COMPRESSOR,
            999, // not a real effect
        ];
        let ok = gooey_engine_set_effect_order(engine, bad.as_ptr(), N as u32);
        assert!(!ok);
        assert_eq!(read_order(engine), before);

        gooey_engine_free(engine);
    }
}

#[test]
fn reordering_changes_audio_output() {
    // Saturation→delay vs delay→saturation should produce different output for the
    // same input. Tests that the dispatch loop actually honors `effect_order`.
    unsafe fn render_with_order(order: &[u32; N]) -> Vec<f32> {
        let engine = gooey_engine_new(44100.0);

        // Enable just saturation + delay; both with strong, audible settings.
        gooey_engine_set_global_effect_enabled(engine, EFFECT_SATURATION, true);
        gooey_engine_set_global_effect_enabled(engine, EFFECT_DELAY, true);
        gooey_engine_set_global_effect_enabled(engine, EFFECT_LIMITER, false);

        // Heavy distortion so its position relative to delay matters.
        gooey_engine_set_global_effect_param(
            engine,
            EFFECT_SATURATION,
            SATURATION_PARAM_DRIVE,
            1.0,
        );
        gooey_engine_set_global_effect_param(engine, EFFECT_SATURATION, SATURATION_PARAM_MIX, 1.0);

        // Audible delay with strong feedback.
        gooey_engine_set_global_effect_param(engine, EFFECT_DELAY, DELAY_PARAM_FEEDBACK, 0.7);
        gooey_engine_set_global_effect_param(engine, EFFECT_DELAY, DELAY_PARAM_MIX, 0.8);

        let ok = gooey_engine_set_effect_order(engine, order.as_ptr(), N as u32);
        assert!(ok);

        gooey_engine_trigger_instrument(engine, INSTRUMENT_KICK);
        let frames = 16_384u32;
        // Interleaved stereo: two output samples per frame.
        let mut buffer = vec![0.0f32; frames as usize * 2];
        gooey_engine_render(engine, buffer.as_mut_ptr(), frames);

        gooey_engine_free(engine);
        buffer
    }

    let order_a = [
        EFFECT_SATURATION,
        EFFECT_DELAY,
        EFFECT_LOWPASS_FILTER,
        EFFECT_TILT_FILTER,
        EFFECT_COMPRESSOR,
        EFFECT_REVERB,
    ];
    let order_b = [
        EFFECT_DELAY,
        EFFECT_SATURATION,
        EFFECT_LOWPASS_FILTER,
        EFFECT_TILT_FILTER,
        EFFECT_COMPRESSOR,
        EFFECT_REVERB,
    ];

    let buf_a = unsafe { render_with_order(&order_a) };
    let buf_b = unsafe { render_with_order(&order_b) };

    let max_diff = buf_a
        .iter()
        .zip(buf_b.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f32, f32::max);
    assert!(
        max_diff > 1e-4,
        "Reordering saturation and delay should change the output (max diff = {max_diff})"
    );
}
