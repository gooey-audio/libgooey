//! Integration tests for stereo-aware global effects (Pass 3).
//!
//! Two properties are asserted:
//!
//! 1. Foundation / behavior-preserving: every global effect now processes a
//!    true left/right pair, but for mono content (the instrument mix is mono)
//!    with no genuinely-stereo behavior engaged, the two channels stay
//!    identical. This guards the per-channel-state refactor — a bug that lets
//!    the two channel states drift would break `left == right`.
//!
//! 2. Showcase: the delay's ping-pong mode (DELAY_PARAM_PINGPONG) crosses the
//!    feedback between channels, so a centered (mono) impulse produces audibly
//!    different left and right output.

use gooey::ffi::*;

const SAMPLE_RATE: f32 = 44_100.0;

/// The six reorderable global effects (the limiter is pinned last and tested
/// implicitly). Each entry is `(effect_id, &[(param, value)])` — the params
/// engage the effect strongly so it actually alters the signal.
fn reorderable_effects() -> Vec<(u32, Vec<(u32, f32)>)> {
    vec![
        (
            EFFECT_LOWPASS_FILTER,
            vec![(FILTER_PARAM_CUTOFF, 2000.0), (FILTER_PARAM_RESONANCE, 0.5)],
        ),
        (
            EFFECT_DELAY,
            vec![
                (DELAY_PARAM_FEEDBACK, 0.6),
                (DELAY_PARAM_MIX, 0.7),
                // ping-pong OFF (default) — the foundation must stay dual-mono
                (DELAY_PARAM_PINGPONG, 0.0),
            ],
        ),
        (
            EFFECT_SATURATION,
            vec![(SATURATION_PARAM_DRIVE, 0.8), (SATURATION_PARAM_MIX, 1.0)],
        ),
        (
            EFFECT_COMPRESSOR,
            vec![
                (COMPRESSOR_PARAM_THRESHOLD, -20.0),
                (COMPRESSOR_PARAM_RATIO, 8.0),
                (COMPRESSOR_PARAM_MIX, 1.0),
            ],
        ),
        (
            EFFECT_TILT_FILTER,
            vec![(TILT_PARAM_CUTOFF, 0.2), (TILT_PARAM_RESONANCE, 0.5)],
        ),
        // NOTE: the spring reverb and plate reverb are intentionally excluded
        // here. The spring's two tanks use different per-channel allpass
        // tables and the plate derives L/R from cross-branch output taps, so
        // both decorrelate mono content into genuine L != R — verified
        // separately by `reverb_decorrelates_left_and_right` and
        // `plate_reverb_decorrelates_left_and_right`.
    ]
}

#[test]
fn each_effect_keeps_left_equal_right_for_mono_input() {
    for (effect_id, params) in reorderable_effects() {
        unsafe {
            let engine = gooey_engine_new(SAMPLE_RATE);

            gooey_engine_set_global_effect_enabled(engine, effect_id, true);
            for (param, value) in &params {
                gooey_engine_set_global_effect_param(engine, effect_id, *param, *value);
            }

            let frames = 4096usize;
            let mut buffer = vec![0.0_f32; frames * 2];

            gooey_engine_trigger_instrument(engine, INSTRUMENT_KICK);
            gooey_engine_render(engine, buffer.as_mut_ptr(), frames as u32);

            assert!(
                !gooey_engine_has_error(engine),
                "effect {effect_id}: render must not set the error flag"
            );

            let peak = buffer.iter().fold(0.0_f32, |acc, s| acc.max(s.abs()));
            assert!(
                peak > 0.001,
                "effect {effect_id}: expected audible output, peak was {peak}"
            );

            for (i, frame) in buffer.chunks_exact(2).enumerate() {
                assert_eq!(
                    frame[0], frame[1],
                    "effect {effect_id}: left != right at frame {i} for mono input ({} vs {})",
                    frame[0], frame[1]
                );
            }

            gooey_engine_free(engine);
        }
    }
}

#[test]
fn ping_pong_delay_makes_left_and_right_diverge() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);

        gooey_engine_set_global_effect_enabled(engine, EFFECT_DELAY, true);
        // Short timing so several echoes fit in the render window, high feedback
        // and full wet so the bouncing echoes dominate, ping-pong ON.
        gooey_engine_set_global_effect_param(engine, EFFECT_DELAY, DELAY_PARAM_TIMING, 4.0);
        gooey_engine_set_global_effect_param(engine, EFFECT_DELAY, DELAY_PARAM_FEEDBACK, 0.85);
        gooey_engine_set_global_effect_param(engine, EFFECT_DELAY, DELAY_PARAM_MIX, 1.0);
        gooey_engine_set_global_effect_param(engine, EFFECT_DELAY, DELAY_PARAM_PINGPONG, 1.0);

        // Render long enough to span multiple delay periods.
        let frames = 32_768usize;
        let mut buffer = vec![0.0_f32; frames * 2];

        gooey_engine_trigger_instrument(engine, INSTRUMENT_KICK);
        gooey_engine_render(engine, buffer.as_mut_ptr(), frames as u32);

        assert!(
            !gooey_engine_has_error(engine),
            "render must not set the error flag"
        );

        // The output must stay finite and stable (high feedback must not blow up).
        assert!(
            buffer.iter().all(|s| s.is_finite() && s.abs() < 100.0),
            "ping-pong delay output must stay finite and bounded"
        );

        // With crossed feedback and a centered impulse, left and right diverge:
        // echoes appear on one channel before the other.
        let max_diff = buffer
            .chunks_exact(2)
            .fold(0.0_f32, |acc, frame| acc.max((frame[0] - frame[1]).abs()));
        assert!(
            max_diff > 1e-4,
            "ping-pong delay should make left and right diverge, max |L-R| was {max_diff}"
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn reverb_decorrelates_left_and_right() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);

        gooey_engine_set_global_effect_enabled(engine, EFFECT_REVERB, true);
        // Long decay and plenty of wet so the decorrelated tails build up.
        gooey_engine_set_global_effect_param(engine, EFFECT_REVERB, REVERB_PARAM_DECAY, 0.7);
        gooey_engine_set_global_effect_param(engine, EFFECT_REVERB, REVERB_PARAM_MIX, 0.8);

        // Render a generous window so the spring tails on both channels develop.
        let frames = 32_768usize;
        let mut buffer = vec![0.0_f32; frames * 2];

        gooey_engine_trigger_instrument(engine, INSTRUMENT_KICK);
        gooey_engine_render(engine, buffer.as_mut_ptr(), frames as u32);

        assert!(
            !gooey_engine_has_error(engine),
            "render must not set the error flag"
        );

        let peak = buffer.iter().fold(0.0_f32, |acc, s| acc.max(s.abs()));
        assert!(peak > 0.001, "expected audible output, peak was {peak}");

        // The reverb tail must stay finite and bounded.
        assert!(
            buffer.iter().all(|s| s.is_finite() && s.abs() < 100.0),
            "reverb output must stay finite and bounded"
        );

        // The two tanks use different allpass tables, so a mono input produces
        // genuinely different left and right.
        let max_diff = buffer
            .chunks_exact(2)
            .fold(0.0_f32, |acc, frame| acc.max((frame[0] - frame[1]).abs()));
        assert!(
            max_diff > 1e-4,
            "reverb should decorrelate left and right, max |L-R| was {max_diff}"
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn plate_reverb_decorrelates_left_and_right() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);

        gooey_engine_set_global_effect_enabled(engine, EFFECT_PLATE_REVERB, true);
        // Long decay and plenty of wet so the cross-branch tap tails build up.
        gooey_engine_set_global_effect_param(engine, EFFECT_PLATE_REVERB, PLATE_PARAM_DECAY, 0.7);
        gooey_engine_set_global_effect_param(engine, EFFECT_PLATE_REVERB, PLATE_PARAM_MIX, 0.8);

        // Render a generous window so the plate tail develops.
        let frames = 32_768usize;
        let mut buffer = vec![0.0_f32; frames * 2];

        gooey_engine_trigger_instrument(engine, INSTRUMENT_KICK);
        gooey_engine_render(engine, buffer.as_mut_ptr(), frames as u32);

        assert!(
            !gooey_engine_has_error(engine),
            "render must not set the error flag"
        );

        let peak = buffer.iter().fold(0.0_f32, |acc, s| acc.max(s.abs()));
        assert!(peak > 0.001, "expected audible output, peak was {peak}");

        // The reverb tail must stay finite and bounded.
        assert!(
            buffer.iter().all(|s| s.is_finite() && s.abs() < 100.0),
            "plate reverb output must stay finite and bounded"
        );

        // Left reads mostly from tank branch B and right from branch A, so a
        // mono input produces genuinely different left and right.
        let max_diff = buffer
            .chunks_exact(2)
            .fold(0.0_f32, |acc, frame| acc.max((frame[0] - frame[1]).abs()));
        assert!(
            max_diff > 1e-4,
            "plate reverb should decorrelate left and right, max |L-R| was {max_diff}"
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn ping_pong_off_keeps_delay_dual_mono() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);

        gooey_engine_set_global_effect_enabled(engine, EFFECT_DELAY, true);
        gooey_engine_set_global_effect_param(engine, EFFECT_DELAY, DELAY_PARAM_FEEDBACK, 0.85);
        gooey_engine_set_global_effect_param(engine, EFFECT_DELAY, DELAY_PARAM_MIX, 1.0);
        // Explicitly off.
        gooey_engine_set_global_effect_param(engine, EFFECT_DELAY, DELAY_PARAM_PINGPONG, 0.0);
        assert_eq!(
            gooey_engine_get_global_effect_param(engine, EFFECT_DELAY, DELAY_PARAM_PINGPONG),
            0.0,
            "ping-pong should report off"
        );

        let frames = 8192usize;
        let mut buffer = vec![0.0_f32; frames * 2];

        gooey_engine_trigger_instrument(engine, INSTRUMENT_KICK);
        gooey_engine_render(engine, buffer.as_mut_ptr(), frames as u32);

        for (i, frame) in buffer.chunks_exact(2).enumerate() {
            assert_eq!(
                frame[0], frame[1],
                "non-ping-pong delay must stay dual-mono at frame {i}"
            );
        }

        gooey_engine_free(engine);
    }
}
