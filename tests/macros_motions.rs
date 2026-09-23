//! Macros (one control → many parameters) and motions (one-shot macro
//! automation) through the C ABI.

use gooey::ffi::*;

const SR: f32 = 48_000.0;

struct Engine(*mut GooeyEngine);

impl Engine {
    fn new() -> Self {
        let engine = gooey_engine_new(SR);
        unsafe { gooey_engine_set_bpm(engine, 120.0) };
        Self(engine)
    }

    fn render_seconds(&self, seconds: f32) {
        let frames = (seconds * SR).round() as usize;
        let mut buffer = vec![0.0_f32; 512 * 2];
        let mut remaining = frames;
        while remaining > 0 {
            let chunk = remaining.min(512);
            unsafe { gooey_engine_render(self.0, buffer.as_mut_ptr(), chunk as u32) };
            remaining -= chunk;
        }
    }

    fn cutoff(&self) -> f32 {
        unsafe {
            gooey_engine_get_global_effect_param(self.0, EFFECT_LOWPASS_FILTER, FILTER_PARAM_CUTOFF)
        }
    }

    fn hihat_tone(&self) -> f32 {
        unsafe { gooey_engine_get_hihat_param(self.0, HIHAT_PARAM_TONE) }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { gooey_engine_free(self.0) }
    }
}

fn close(actual: f32, expected: f32, tolerance: f32) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected} ± {tolerance}, got {actual}"
    );
}

/// Macro 0 sweeps the global lowpass from 1 kHz to 5 kHz.
fn cutoff_macro(engine: &Engine) {
    unsafe {
        assert!(gooey_engine_macro_add_mapping(
            engine.0,
            0,
            PARAM_TARGET_GLOBAL_EFFECT,
            EFFECT_LOWPASS_FILTER,
            FILTER_PARAM_CUTOFF,
            1000.0,
            5000.0,
        ));
    }
}

#[test]
fn capture_registers_changed_params_and_reverts_them() {
    let engine = Engine::new();
    unsafe {
        gooey_engine_set_hihat_param(engine.0, HIHAT_PARAM_TONE, 0.2);
        assert!(gooey_engine_poly_set_param(
            engine.0,
            POLY_PARAM_FILTER_CUTOFF,
            0.8
        ));
        engine.render_seconds(0.01);

        assert!(gooey_engine_macro_capture_begin(engine.0));
        assert!(gooey_engine_macro_is_capturing(engine.0));
        assert_eq!(gooey_engine_macro_capture_get_change_count(engine.0), 0);

        gooey_engine_set_hihat_param(engine.0, HIHAT_PARAM_TONE, 0.9);
        assert!(gooey_engine_poly_set_param(
            engine.0,
            POLY_PARAM_FILTER_CUTOFF,
            0.1
        ));
        assert_eq!(gooey_engine_macro_capture_get_change_count(engine.0), 2);

        assert_eq!(
            gooey_engine_macro_capture_commit(engine.0, 0, MACRO_CAPTURE_REPLACE),
            2
        );
        assert!(!gooey_engine_macro_is_capturing(engine.0));

        // Reverted to the pre-capture values.
        close(engine.hihat_tone(), 0.2, 1e-6);
        close(
            gooey_engine_poly_get_param(engine.0, POLY_PARAM_FILTER_CUTOFF),
            0.8,
            1e-6,
        );
        assert_eq!(gooey_engine_macro_get_value(engine.0, 0), 0.0);

        assert_eq!(gooey_engine_macro_get_mapping_count(engine.0, 0), 2);
        let (mut kind, mut index, mut param, mut from, mut to) = (0, 0, 0, 0.0, 0.0);
        assert!(gooey_engine_macro_get_mapping(
            engine.0, 0, 0, &mut kind, &mut index, &mut param, &mut from, &mut to,
        ));
        assert_eq!(
            (kind, index, param),
            (PARAM_TARGET_POLY, 0, POLY_PARAM_FILTER_CUTOFF)
        );
        close(from, 0.8, 1e-6);
        close(to, 0.1, 1e-6);
        assert!(gooey_engine_macro_get_mapping(
            engine.0,
            0,
            1,
            &mut kind,
            &mut index,
            &mut param,
            std::ptr::null_mut(),
            &mut to,
        ));
        assert_eq!(
            (kind, index, param),
            (PARAM_TARGET_DRUM, INSTRUMENT_HIHAT, HIHAT_PARAM_TONE)
        );
        close(to, 0.9, 1e-6);

        // Playing the macro by hand moves both parameters.
        assert!(gooey_engine_macro_set_value(engine.0, 0, 0.5));
        engine.render_seconds(0.01);
        close(engine.hihat_tone(), 0.55, 1e-5);
        close(
            gooey_engine_poly_get_param(engine.0, POLY_PARAM_FILTER_CUTOFF),
            0.45,
            1e-5,
        );
        close(gooey_engine_macro_get_value(engine.0, 0), 0.5, 1e-6);
    }
}

#[test]
fn capture_merge_extends_an_existing_macro() {
    let engine = Engine::new();
    unsafe {
        gooey_engine_set_hihat_param(engine.0, HIHAT_PARAM_TONE, 0.2);
        assert!(gooey_engine_macro_capture_begin(engine.0));
        gooey_engine_set_hihat_param(engine.0, HIHAT_PARAM_TONE, 0.9);
        assert_eq!(
            gooey_engine_macro_capture_commit(engine.0, 3, MACRO_CAPTURE_REPLACE),
            1
        );

        assert!(gooey_engine_macro_capture_begin(engine.0));
        gooey_engine_set_hihat_param(engine.0, HIHAT_PARAM_TONE, 0.6);
        gooey_engine_set_global_effect_param(engine.0, EFFECT_DELAY, DELAY_PARAM_MIX, 0.8);
        assert_eq!(
            gooey_engine_macro_capture_commit(engine.0, 3, MACRO_CAPTURE_MERGE),
            2
        );

        let (mut from, mut to) = (0.0, 0.0);
        let null = std::ptr::null_mut();
        assert!(gooey_engine_macro_get_mapping(
            engine.0, 3, 0, null, null, null, &mut from, &mut to
        ));
        // Existing mapping keeps its start and takes the new end point.
        close(from, 0.2, 1e-6);
        close(to, 0.6, 1e-6);
    }
}

#[test]
fn capture_rejects_too_many_changes_and_stays_open() {
    let engine = Engine::new();
    unsafe {
        assert!(gooey_engine_macro_capture_begin(engine.0));
        let mut changed = 0;
        for param in 0..POLY_PARAM_COUNT {
            if param == POLY_PARAM_OSC_A_WAVEFORM || param == POLY_PARAM_OSC_B_WAVEFORM {
                continue;
            }
            let current = gooey_engine_poly_get_param(engine.0, param);
            let next = if current > 0.5 { 0.05 } else { 0.95 };
            assert!(gooey_engine_poly_set_param(engine.0, param, next));
            changed += 1;
        }
        assert!(changed > MACRO_MAX_MAPPINGS as i32);
        assert_eq!(
            gooey_engine_macro_capture_commit(engine.0, 0, MACRO_CAPTURE_REPLACE),
            MACRO_CAPTURE_ERROR_TOO_MANY
        );
        assert!(gooey_engine_macro_is_capturing(engine.0));
        assert_eq!(gooey_engine_macro_get_mapping_count(engine.0, 0), 0);
    }
}

#[test]
fn capture_cancel_can_revert_or_keep_changes() {
    let engine = Engine::new();
    unsafe {
        gooey_engine_set_hihat_param(engine.0, HIHAT_PARAM_TONE, 0.2);
        assert!(gooey_engine_macro_capture_begin(engine.0));
        gooey_engine_set_hihat_param(engine.0, HIHAT_PARAM_TONE, 0.7);
        assert!(gooey_engine_macro_capture_cancel(engine.0, true));
        close(engine.hihat_tone(), 0.2, 1e-6);

        assert!(gooey_engine_macro_capture_begin(engine.0));
        gooey_engine_set_hihat_param(engine.0, HIHAT_PARAM_TONE, 0.7);
        assert!(gooey_engine_macro_capture_cancel(engine.0, false));
        close(engine.hihat_tone(), 0.7, 1e-6);
        assert!(!gooey_engine_macro_capture_cancel(engine.0, false));
        assert_eq!(
            gooey_engine_macro_capture_commit(engine.0, 0, MACRO_CAPTURE_REPLACE),
            MACRO_CAPTURE_ERROR_INVALID
        );
    }
}

#[test]
fn discrete_and_invalid_targets_are_rejected() {
    let engine = Engine::new();
    unsafe {
        let add = |kind, index, param| {
            gooey_engine_macro_add_mapping(engine.0, 0, kind, index, param, 0.0, 1.0)
        };
        assert!(!add(PARAM_TARGET_POLY, 0, POLY_PARAM_OSC_A_WAVEFORM));
        assert!(!add(PARAM_TARGET_POLY, 1, POLY_PARAM_FILTER_CUTOFF));
        assert!(!add(
            PARAM_TARGET_DRUM,
            INSTRUMENT_SNARE,
            SNARE_PARAM_FILTER_TYPE
        ));
        assert!(!add(
            PARAM_TARGET_DRUM,
            INSTRUMENT_BASS,
            BASS_PARAM_FILTER_CUTOFF
        ));
        assert!(!add(
            PARAM_TARGET_GLOBAL_EFFECT,
            EFFECT_DELAY,
            DELAY_PARAM_TIMING
        ));
        assert!(!add(
            PARAM_TARGET_GLOBAL_EFFECT,
            EFFECT_DELAY,
            DELAY_PARAM_PINGPONG
        ));
        assert!(!add(99, 0, 0));
        assert!(!gooey_engine_macro_add_mapping(
            engine.0,
            MACRO_COUNT,
            PARAM_TARGET_POLY,
            0,
            POLY_PARAM_FILTER_CUTOFF,
            0.0,
            1.0
        ));
        assert!(!gooey_engine_macro_add_mapping(
            engine.0,
            0,
            PARAM_TARGET_POLY,
            0,
            POLY_PARAM_FILTER_CUTOFF,
            f32::NAN,
            1.0
        ));
        assert!(add(PARAM_TARGET_DRUM, INSTRUMENT_KICK, KICK_PARAM_DECAY));
        assert!(gooey_engine_macro_remove_mapping(
            engine.0,
            0,
            PARAM_TARGET_DRUM,
            INSTRUMENT_KICK,
            KICK_PARAM_DECAY
        ));
        assert_eq!(gooey_engine_macro_get_mapping_count(engine.0, 0), 0);

        assert!(!gooey_engine_motion_trigger(engine.0, 0), "unconfigured");
        assert!(!gooey_engine_motion_configure(
            engine.0,
            MOTION_SLOT_COUNT,
            0,
            1.0
        ));
        assert!(!gooey_engine_motion_configure(
            engine.0,
            0,
            MACRO_COUNT,
            1.0
        ));
        assert_eq!(
            gooey_engine_motion_get_state(engine.0, MOTION_SLOT_COUNT),
            AUTOMATION_INVALID
        );
        assert!(gooey_engine_macro_get_value(engine.0, MACRO_COUNT).is_nan());
    }
}

#[test]
fn motion_defaults_and_settings_round_trip() {
    let engine = Engine::new();
    unsafe {
        assert!(gooey_engine_motion_configure(engine.0, 5, 2, 0.75));
        assert_eq!(gooey_engine_motion_get_macro(engine.0, 5), 2);
        assert_eq!(gooey_engine_motion_get_target(engine.0, 5), 0.75);
        assert_eq!(
            gooey_engine_motion_get_duration_unit(engine.0, 5),
            MOTION_DURATION_BEATS
        );
        assert_eq!(gooey_engine_motion_get_duration_value(engine.0, 5), 4.0);
        assert_eq!(
            gooey_engine_motion_get_curve(engine.0, 5),
            MOTION_CURVE_LINEAR
        );
        assert_eq!(
            gooey_engine_motion_get_end_mode(engine.0, 5),
            MOTION_END_HOLD
        );
        assert_eq!(
            gooey_engine_motion_get_quantize(engine.0, 5),
            MOTION_QUANTIZE_NONE
        );
        assert!(gooey_engine_motion_get_start(engine.0, 5).is_nan());

        assert!(gooey_engine_motion_set_duration(
            engine.0,
            5,
            MOTION_DURATION_MS,
            250.0
        ));
        assert!(gooey_engine_motion_set_curve(
            engine.0,
            5,
            MOTION_CURVE_S_CURVE
        ));
        assert!(gooey_engine_motion_set_end_mode(
            engine.0,
            5,
            MOTION_END_RETURN
        ));
        assert!(gooey_engine_motion_set_quantize(
            engine.0,
            5,
            MOTION_QUANTIZE_BAR
        ));
        assert!(gooey_engine_motion_set_start(engine.0, 5, 0.1));
        assert!(!gooey_engine_motion_set_curve(engine.0, 5, 99));
        assert!(!gooey_engine_motion_set_duration(
            engine.0,
            5,
            MOTION_DURATION_MS,
            -1.0
        ));

        // Reconfiguring keeps the other settings.
        assert!(gooey_engine_motion_configure(engine.0, 5, 3, 0.25));
        assert_eq!(gooey_engine_motion_get_macro(engine.0, 5), 3);
        assert_eq!(
            gooey_engine_motion_get_duration_unit(engine.0, 5),
            MOTION_DURATION_MS
        );
        assert_eq!(gooey_engine_motion_get_duration_value(engine.0, 5), 250.0);
        assert_eq!(
            gooey_engine_motion_get_curve(engine.0, 5),
            MOTION_CURVE_S_CURVE
        );
        assert_eq!(
            gooey_engine_motion_get_end_mode(engine.0, 5),
            MOTION_END_RETURN
        );
        assert_eq!(
            gooey_engine_motion_get_quantize(engine.0, 5),
            MOTION_QUANTIZE_BAR
        );
        close(gooey_engine_motion_get_start(engine.0, 5), 0.1, 1e-6);

        assert!(gooey_engine_motion_clear(engine.0, 5));
        assert!(!gooey_engine_motion_is_configured(engine.0, 5));
        assert_eq!(
            gooey_engine_motion_get_macro(engine.0, 5),
            AUTOMATION_INVALID
        );
    }
}

#[test]
fn hold_motion_ramps_over_beats_then_holds() {
    let engine = Engine::new();
    cutoff_macro(&engine);
    unsafe {
        assert!(gooey_engine_motion_configure(engine.0, 0, 0, 1.0));
        assert!(gooey_engine_motion_set_duration(
            engine.0,
            0,
            MOTION_DURATION_BEATS,
            1.0
        ));
        assert!(gooey_engine_motion_trigger(engine.0, 0));

        // One beat at 120 BPM is 0.5 s; halfway is 3 kHz.
        engine.render_seconds(0.25);
        assert_eq!(
            gooey_engine_motion_get_state(engine.0, 0),
            MOTION_STATE_RUNNING
        );
        close(engine.cutoff(), 3000.0, 60.0);
        close(gooey_engine_macro_get_value(engine.0, 0), 0.5, 0.02);
        close(gooey_engine_motion_get_progress(engine.0, 0), 0.5, 0.02);

        engine.render_seconds(0.3);
        assert_eq!(
            gooey_engine_motion_get_state(engine.0, 0),
            MOTION_STATE_IDLE
        );
        close(engine.cutoff(), 5000.0, 1e-3);

        // Stays put afterwards.
        engine.render_seconds(0.2);
        close(engine.cutoff(), 5000.0, 1e-3);
    }
}

#[test]
fn return_motion_retraces_to_start() {
    let engine = Engine::new();
    cutoff_macro(&engine);
    unsafe {
        assert!(gooey_engine_motion_configure(engine.0, 0, 0, 1.0));
        assert!(gooey_engine_motion_set_duration(
            engine.0,
            0,
            MOTION_DURATION_MS,
            100.0
        ));
        assert!(gooey_engine_motion_set_end_mode(
            engine.0,
            0,
            MOTION_END_RETURN
        ));
        assert!(gooey_engine_motion_trigger(engine.0, 0));

        engine.render_seconds(0.15);
        assert_eq!(
            gooey_engine_motion_get_state(engine.0, 0),
            MOTION_STATE_RETURNING
        );
        close(engine.cutoff(), 3000.0, 150.0);

        engine.render_seconds(0.1);
        assert_eq!(
            gooey_engine_motion_get_state(engine.0, 0),
            MOTION_STATE_IDLE
        );
        close(engine.cutoff(), 1000.0, 1e-3);
    }
}

#[test]
fn snap_back_motion_jumps_to_start_after_target() {
    let engine = Engine::new();
    cutoff_macro(&engine);
    unsafe {
        assert!(gooey_engine_motion_configure(engine.0, 0, 0, 1.0));
        assert!(gooey_engine_motion_set_duration(
            engine.0,
            0,
            MOTION_DURATION_MS,
            100.0
        ));
        assert!(gooey_engine_motion_set_end_mode(
            engine.0,
            0,
            MOTION_END_SNAP_BACK
        ));
        assert!(gooey_engine_motion_trigger(engine.0, 0));

        engine.render_seconds(0.09);
        assert!(engine.cutoff() > 4000.0);
        engine.render_seconds(0.02);
        assert_eq!(
            gooey_engine_motion_get_state(engine.0, 0),
            MOTION_STATE_IDLE
        );
        close(engine.cutoff(), 1000.0, 1e-3);
    }
}

#[test]
fn concurrent_motions_drive_independent_macros() {
    let engine = Engine::new();
    cutoff_macro(&engine);
    unsafe {
        assert!(gooey_engine_macro_add_mapping(
            engine.0,
            1,
            PARAM_TARGET_DRUM,
            INSTRUMENT_HIHAT,
            HIHAT_PARAM_TONE,
            0.1,
            0.9
        ));
        for (slot, macro_index, ms) in [(0, 0, 100.0), (1, 1, 200.0)] {
            assert!(gooey_engine_motion_configure(
                engine.0,
                slot,
                macro_index,
                1.0
            ));
            assert!(gooey_engine_motion_set_duration(
                engine.0,
                slot,
                MOTION_DURATION_MS,
                ms
            ));
            assert!(gooey_engine_motion_trigger(engine.0, slot));
        }
        engine.render_seconds(0.05);
        close(engine.cutoff(), 3000.0, 100.0);
        close(engine.hihat_tone(), 0.3, 0.02);

        engine.render_seconds(0.06);
        assert_eq!(
            gooey_engine_motion_get_state(engine.0, 0),
            MOTION_STATE_IDLE
        );
        assert_eq!(
            gooey_engine_motion_get_state(engine.0, 1),
            MOTION_STATE_RUNNING
        );
        close(engine.hihat_tone(), 0.54, 0.02);
    }
}

#[test]
fn manual_macro_gesture_stops_its_motion() {
    let engine = Engine::new();
    cutoff_macro(&engine);
    unsafe {
        assert!(gooey_engine_motion_configure(engine.0, 0, 0, 1.0));
        assert!(gooey_engine_motion_set_duration(
            engine.0,
            0,
            MOTION_DURATION_MS,
            200.0
        ));
        assert!(gooey_engine_motion_trigger(engine.0, 0));
        engine.render_seconds(0.05);
        assert!(gooey_engine_macro_set_value(engine.0, 0, 0.0));
        engine.render_seconds(0.05);
        assert_eq!(
            gooey_engine_motion_get_state(engine.0, 0),
            MOTION_STATE_IDLE
        );
        close(engine.cutoff(), 1000.0, 1e-3);
    }
}

#[test]
fn stop_freezes_motion_in_place() {
    let engine = Engine::new();
    cutoff_macro(&engine);
    unsafe {
        assert!(gooey_engine_motion_configure(engine.0, 0, 0, 1.0));
        assert!(gooey_engine_motion_set_duration(
            engine.0,
            0,
            MOTION_DURATION_MS,
            200.0
        ));
        assert!(gooey_engine_motion_trigger(engine.0, 0));
        engine.render_seconds(0.1);
        assert!(gooey_engine_motion_stop(engine.0, 0));
        engine.render_seconds(0.01);
        let frozen = engine.cutoff();
        engine.render_seconds(0.2);
        assert_eq!(engine.cutoff(), frozen);
        assert!(frozen > 2500.0 && frozen < 3500.0, "{frozen}");
    }
}

#[test]
fn explicit_start_makes_a_hold_motion_repeatable() {
    let engine = Engine::new();
    cutoff_macro(&engine);
    unsafe {
        assert!(gooey_engine_motion_configure(engine.0, 0, 0, 1.0));
        assert!(gooey_engine_motion_set_duration(
            engine.0,
            0,
            MOTION_DURATION_MS,
            50.0
        ));
        assert!(gooey_engine_motion_set_start(engine.0, 0, 0.0));
        for _ in 0..2 {
            assert!(gooey_engine_motion_trigger(engine.0, 0));
            engine.render_seconds(0.025);
            close(engine.cutoff(), 3000.0, 150.0);
            engine.render_seconds(0.05);
            close(engine.cutoff(), 5000.0, 1e-3);
        }
    }
}

#[test]
fn bar_quantized_motion_waits_for_the_next_bar() {
    let engine = Engine::new();
    cutoff_macro(&engine);
    unsafe {
        gooey_engine_sequencer_start(engine.0);
        engine.render_seconds(0.3); // beat ~0.6
        let untouched = engine.cutoff();
        assert!(gooey_engine_motion_configure(engine.0, 0, 0, 1.0));
        assert!(gooey_engine_motion_set_duration(
            engine.0,
            0,
            MOTION_DURATION_MS,
            50.0
        ));
        assert!(gooey_engine_motion_set_quantize(
            engine.0,
            0,
            MOTION_QUANTIZE_BAR
        ));
        assert!(gooey_engine_motion_trigger(engine.0, 0));

        engine.render_seconds(0.01);
        assert_eq!(
            gooey_engine_motion_get_state(engine.0, 0),
            MOTION_STATE_PENDING
        );
        // Beat 4 arrives at 2.0 s.
        engine.render_seconds(1.6);
        assert_eq!(
            gooey_engine_motion_get_state(engine.0, 0),
            MOTION_STATE_PENDING
        );
        assert_eq!(engine.cutoff(), untouched, "nothing moves before the bar");
        engine.render_seconds(0.2);
        assert_eq!(
            gooey_engine_motion_get_state(engine.0, 0),
            MOTION_STATE_IDLE
        );
        close(engine.cutoff(), 5000.0, 1e-3);
    }
}

#[test]
fn hold_motion_projects_poly_getter_to_its_end_value() {
    let engine = Engine::new();
    unsafe {
        assert!(gooey_engine_macro_add_mapping(
            engine.0,
            0,
            PARAM_TARGET_POLY,
            0,
            POLY_PARAM_FILTER_CUTOFF,
            0.9,
            0.2
        ));
        assert!(gooey_engine_motion_configure(engine.0, 0, 0, 1.0));
        assert!(gooey_engine_motion_trigger(engine.0, 0));
        close(
            gooey_engine_poly_get_param(engine.0, POLY_PARAM_FILTER_CUTOFF),
            0.2,
            1e-6,
        );
    }
}

#[test]
fn higher_macro_keeps_a_shared_parameter_when_a_lower_one_moves() {
    let engine = Engine::new();
    cutoff_macro(&engine);
    unsafe {
        assert!(gooey_engine_macro_add_mapping(
            engine.0,
            1,
            PARAM_TARGET_GLOBAL_EFFECT,
            EFFECT_LOWPASS_FILTER,
            FILTER_PARAM_CUTOFF,
            200.0,
            600.0,
        ));
        assert!(gooey_engine_macro_set_value(engine.0, 1, 0.5));
        engine.render_seconds(0.01);
        close(engine.cutoff(), 400.0, 1e-3);

        // Macro 0 moving later (by hand or by motion) does not take it over.
        assert!(gooey_engine_macro_set_value(engine.0, 0, 1.0));
        engine.render_seconds(0.01);
        close(engine.cutoff(), 400.0, 1e-3);
        assert!(gooey_engine_motion_configure(engine.0, 0, 0, 0.0));
        assert!(gooey_engine_motion_set_duration(
            engine.0,
            0,
            MOTION_DURATION_MS,
            20.0
        ));
        assert!(gooey_engine_motion_trigger(engine.0, 0));
        engine.render_seconds(0.05);
        close(engine.cutoff(), 400.0, 1e-3);
    }
}

#[test]
fn clear_keeps_the_slot_configured_when_the_queue_is_full() {
    let engine = Engine::new();
    unsafe {
        assert!(gooey_engine_motion_configure(engine.0, 0, 0, 1.0));
        // Alternate macros so manual values cannot coalesce.
        let mut pushed = 0;
        while gooey_engine_macro_set_value(engine.0, pushed % 2, 0.5) {
            pushed += 1;
        }
        assert!(!gooey_engine_motion_clear(engine.0, 0));
        assert!(gooey_engine_motion_is_configured(engine.0, 0));

        engine.render_seconds(0.01);
        assert!(gooey_engine_motion_clear(engine.0, 0));
        assert!(!gooey_engine_motion_is_configured(engine.0, 0));
    }
}

#[test]
fn pending_motion_re_aims_after_a_seek() {
    let engine = Engine::new();
    cutoff_macro(&engine);
    unsafe {
        gooey_engine_sequencer_start(engine.0);
        gooey_engine_sequencer_set_beat_position(engine.0, 5.0);
        engine.render_seconds(0.01);
        assert!(gooey_engine_motion_configure(engine.0, 0, 0, 1.0));
        assert!(gooey_engine_motion_set_duration(
            engine.0,
            0,
            MOTION_DURATION_MS,
            20.0
        ));
        assert!(gooey_engine_motion_set_quantize(
            engine.0,
            0,
            MOTION_QUANTIZE_BAR
        ));
        assert!(gooey_engine_motion_trigger(engine.0, 0));
        engine.render_seconds(0.01);
        assert_eq!(
            gooey_engine_motion_get_state(engine.0, 0),
            MOTION_STATE_PENDING
        );

        // Back to beat 3.5: the next bar is beat 4 (0.25 s away), not beat 8.
        gooey_engine_sequencer_set_beat_position(engine.0, 3.5);
        engine.render_seconds(0.2);
        assert_eq!(
            gooey_engine_motion_get_state(engine.0, 0),
            MOTION_STATE_PENDING
        );
        engine.render_seconds(0.1);
        assert_eq!(
            gooey_engine_motion_get_state(engine.0, 0),
            MOTION_STATE_IDLE
        );
        close(engine.cutoff(), 5000.0, 1e-3);
    }
}
