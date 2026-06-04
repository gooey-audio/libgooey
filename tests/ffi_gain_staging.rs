//! Integration tests for FFI master gain and opt-in nonlinear global effects.

use gooey::ffi::*;

const SAMPLE_RATE: f32 = 44_100.0;
const RENDER_FRAMES: usize = 4_096;
const SETTLE_FRAMES: usize = 16_384;

unsafe fn render(engine: *mut GooeyEngine, frames: usize) -> Vec<f32> {
    let mut buffer = vec![0.0_f32; frames];
    gooey_engine_render(engine, buffer.as_mut_ptr(), frames as u32);
    buffer
}

unsafe fn render_triggered(engine: *mut GooeyEngine, instruments: &[u32]) -> Vec<f32> {
    for &instrument in instruments {
        gooey_engine_trigger_instrument(engine, instrument);
    }
    render(engine, RENDER_FRAMES)
}

fn max_abs(buffer: &[f32]) -> f32 {
    buffer.iter().map(|sample| sample.abs()).fold(0.0, f32::max)
}

#[test]
fn nonlinear_global_effects_default_off_and_remain_opt_in() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);

        for effect in [EFFECT_SATURATION, EFFECT_COMPRESSOR, EFFECT_LIMITER] {
            assert!(
                !gooey_engine_get_global_effect_enabled(engine, effect),
                "effect {effect} should default to bypassed"
            );

            gooey_engine_set_global_effect_enabled(engine, effect, true);
            assert!(
                gooey_engine_get_global_effect_enabled(engine, effect),
                "effect {effect} should be enableable through FFI"
            );

            gooey_engine_set_global_effect_enabled(engine, effect, false);
            assert!(
                !gooey_engine_get_global_effect_enabled(engine, effect),
                "effect {effect} should be disableable through FFI"
            );
        }

        for (effect, param, value) in [
            (EFFECT_SATURATION, SATURATION_PARAM_DRIVE, 0.8),
            (EFFECT_COMPRESSOR, COMPRESSOR_PARAM_THRESHOLD, -18.0),
            (EFFECT_LIMITER, LIMITER_PARAM_THRESHOLD, 0.7),
        ] {
            gooey_engine_set_global_effect_param(engine, effect, param, value);
            let actual = gooey_engine_get_global_effect_param(engine, effect, param);
            assert!(
                (actual - value).abs() < 1e-6,
                "effect {effect} parameter {param} should remain configurable while bypassed"
            );
        }

        gooey_engine_free(engine);
    }
}

#[test]
fn master_gain_defaults_clamps_and_ignores_non_finite_values() {
    unsafe {
        assert_eq!(gooey_engine_get_master_gain(std::ptr::null()), 0.25);

        let engine = gooey_engine_new(SAMPLE_RATE);
        assert_eq!(gooey_engine_get_master_gain(engine), 0.25);

        gooey_engine_set_master_gain(engine, 3.0);
        assert_eq!(gooey_engine_get_master_gain(engine), 2.0);

        gooey_engine_set_master_gain(engine, -1.0);
        assert_eq!(gooey_engine_get_master_gain(engine), 0.0);

        gooey_engine_set_master_gain(engine, 0.75);
        gooey_engine_set_master_gain(engine, f32::NAN);
        gooey_engine_set_master_gain(engine, f32::INFINITY);
        assert_eq!(gooey_engine_get_master_gain(engine), 0.75);

        gooey_engine_free(engine);
    }
}

#[test]
fn default_output_is_a_linear_sum_without_hidden_nonlinear_processing() {
    unsafe {
        let kick_engine = gooey_engine_new(SAMPLE_RATE);
        let tom_engine = gooey_engine_new(SAMPLE_RATE);
        let combined_engine = gooey_engine_new(SAMPLE_RATE);

        let kick = render_triggered(kick_engine, &[INSTRUMENT_KICK]);
        let tom = render_triggered(tom_engine, &[INSTRUMENT_TOM]);
        let combined = render_triggered(combined_engine, &[INSTRUMENT_KICK, INSTRUMENT_TOM]);

        let max_error = combined
            .iter()
            .zip(kick.iter().zip(&tom))
            .map(|(combined, (kick, tom))| (combined - (kick + tom)).abs())
            .fold(0.0_f32, f32::max);

        assert!(
            max_abs(&combined) > 0.01,
            "combined output should be audible"
        );
        assert!(
            max_error < 1e-5,
            "default FFI mix should be linear, max error was {max_error}"
        );

        gooey_engine_free(kick_engine);
        gooey_engine_free(tom_engine);
        gooey_engine_free(combined_engine);
    }
}

#[test]
fn master_gain_scales_the_dry_sum() {
    unsafe {
        let quarter_gain_engine = gooey_engine_new(SAMPLE_RATE);
        let half_gain_engine = gooey_engine_new(SAMPLE_RATE);
        gooey_engine_set_master_gain(half_gain_engine, 0.5);

        let _ = render(quarter_gain_engine, SETTLE_FRAMES);
        let _ = render(half_gain_engine, SETTLE_FRAMES);

        let quarter_gain = render_triggered(quarter_gain_engine, &[INSTRUMENT_KICK]);
        let half_gain = render_triggered(half_gain_engine, &[INSTRUMENT_KICK]);
        let max_error = half_gain
            .iter()
            .zip(&quarter_gain)
            .map(|(half, quarter)| (half - quarter * 2.0).abs())
            .fold(0.0_f32, f32::max);

        assert!(
            max_abs(&quarter_gain) > 0.01,
            "kick output should be audible"
        );
        assert!(
            max_error < 5e-4,
            "0.5 master gain should be twice 0.25 gain, max error was {max_error}"
        );

        gooey_engine_free(quarter_gain_engine);
        gooey_engine_free(half_gain_engine);
    }
}

#[test]
fn master_gain_is_applied_before_the_optional_limiter() {
    unsafe {
        let dry_engine = gooey_engine_new(SAMPLE_RATE);
        let limited_engine = gooey_engine_new(SAMPLE_RATE);
        gooey_engine_set_global_effect_enabled(limited_engine, EFFECT_LIMITER, true);

        let dry = render_triggered(dry_engine, &[INSTRUMENT_KICK, INSTRUMENT_TOM]);
        let limited = render_triggered(limited_engine, &[INSTRUMENT_KICK, INSTRUMENT_TOM]);
        let max_error = limited
            .iter()
            .zip(&dry)
            .map(|(limited, dry)| (limited - dry.tanh()).abs())
            .fold(0.0_f32, f32::max);

        assert!(
            max_error < 1e-6,
            "limiter should receive the master-gained sum, max error was {max_error}"
        );

        gooey_engine_free(dry_engine);
        gooey_engine_free(limited_engine);
    }
}

#[test]
fn offline_bounce_snaps_to_the_configured_master_gain() {
    unsafe {
        let quarter_gain_engine = gooey_engine_new(SAMPLE_RATE);
        let half_gain_engine = gooey_engine_new(SAMPLE_RATE);
        gooey_engine_set_master_gain(half_gain_engine, 0.5);
        gooey_engine_sequencer_set_step(quarter_gain_engine, 0, true);
        gooey_engine_sequencer_set_step(half_gain_engine, 0, true);

        let mut quarter_len = 0;
        let quarter_ptr = gooey_engine_bounce_to_buffer(quarter_gain_engine, 1, &mut quarter_len);
        let quarter = std::slice::from_raw_parts(quarter_ptr, quarter_len as usize).to_vec();
        gooey_engine_free_buffer(quarter_ptr, quarter_len);

        let mut half_len = 0;
        let half_ptr = gooey_engine_bounce_to_buffer(half_gain_engine, 1, &mut half_len);
        let half = std::slice::from_raw_parts(half_ptr, half_len as usize).to_vec();
        gooey_engine_free_buffer(half_ptr, half_len);

        assert_eq!(quarter_len, half_len);
        let max_error = half
            .iter()
            .zip(&quarter)
            .map(|(half, quarter)| (half - quarter * 2.0).abs())
            .fold(0.0_f32, f32::max);
        assert!(max_abs(&quarter) > 0.01, "bounce should contain audio");
        assert!(
            max_error < 1e-6,
            "bounce should immediately honor master gain target, max error was {max_error}"
        );

        gooey_engine_free(quarter_gain_engine);
        gooey_engine_free(half_gain_engine);
    }
}
