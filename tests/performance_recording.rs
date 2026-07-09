//! FFI integration tests for Stage 1 performance (chord clip) recording.

use gooey::ffi::*;
use std::ptr;

fn render_frames(engine: *mut GooeyEngine, frames: usize) {
    let mut buf = vec![0.0_f32; frames * 2];
    unsafe {
        gooey_engine_render(engine, buf.as_mut_ptr(), frames as u32);
    }
}

/// Samples per 16th-note step at the given BPM and sample rate.
fn samples_per_step(bpm: f32, sample_rate: f32) -> f32 {
    (60.0 / bpm) / 4.0 * sample_rate
}

#[test]
fn perf_defaults_disarmed_empty_clip() {
    let engine = unsafe { gooey_engine_new(44_100.0) };
    assert!(!engine.is_null());

    unsafe {
        assert!(!gooey_engine_perf_is_record_armed(engine));
        assert!(!gooey_engine_perf_is_recording(engine));
        assert_eq!(
            gooey_engine_perf_get_record_mode(engine),
            PERF_RECORD_MODE_PUNCH_OUT
        );
        assert_eq!(gooey_engine_perf_get_event_count(engine), 0);
        assert_eq!(gooey_engine_perf_get_length_steps(engine), 16);
        assert_eq!(gooey_engine_perf_get_length_ticks(engine), 384);
        gooey_engine_free(engine);
    }
}

#[test]
fn perf_record_punch_out_one_chord() {
    let sample_rate = 44_100.0;
    let bpm = 120.0;
    let engine = unsafe { gooey_engine_new(sample_rate) };
    assert!(!engine.is_null());

    unsafe {
        gooey_engine_set_bpm(engine, bpm);
        gooey_engine_perf_set_record_mode(engine, PERF_RECORD_MODE_PUNCH_OUT);
        gooey_engine_perf_set_record_armed(engine, true);
        gooey_engine_sequencer_start(engine);

        // One buffer advances the clock so recording becomes active at tick 0.
        render_frames(engine, 64);
        assert!(gooey_engine_perf_is_recording(engine));

        // Chord on: degree I
        gooey_engine_poly_trigger_chord(engine, 0, 0, 0, 0, 1, 4, 0.9);

        // Hold for roughly a quarter note (~ samples for 1 beat).
        let hold = samples_per_step(bpm, sample_rate) as usize * 4;
        render_frames(engine, hold);

        gooey_engine_poly_release(engine);
        assert_eq!(gooey_engine_perf_get_event_count(engine), 1);

        // Run the rest of the bar so punch-out completes (16 steps total).
        let rest = samples_per_step(bpm, sample_rate) as usize * 12;
        render_frames(engine, rest + 512);

        assert!(
            !gooey_engine_perf_is_record_armed(engine),
            "punch-out should disarm after one loop"
        );
        assert_eq!(gooey_engine_perf_get_event_count(engine), 1);

        let mut start = 0u32;
        let mut dur = 0u32;
        let mut degree = 0u32;
        let mut velocity = 0.0f32;
        assert!(gooey_engine_perf_get_event(
            engine,
            0,
            &mut start,
            &mut dur,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut degree,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut velocity,
        ));
        assert_eq!(degree, 0);
        assert!(dur > 0);
        assert!((velocity - 0.9).abs() < 0.001);

        gooey_engine_free(engine);
    }
}

#[test]
fn perf_overdub_keeps_arm_and_appends() {
    let sample_rate = 44_100.0;
    let bpm = 120.0;
    let engine = unsafe { gooey_engine_new(sample_rate) };
    assert!(!engine.is_null());

    unsafe {
        gooey_engine_set_bpm(engine, bpm);
        gooey_engine_perf_set_record_mode(engine, PERF_RECORD_MODE_OVERDUB);
        gooey_engine_perf_set_record_armed(engine, true);
        gooey_engine_sequencer_start(engine);
        render_frames(engine, 64);

        gooey_engine_poly_trigger_chord(engine, 0, 0, 0, 0, 1, 4, 0.9);
        let q = samples_per_step(bpm, sample_rate) as usize * 4;
        render_frames(engine, q);
        gooey_engine_poly_release(engine);
        assert_eq!(gooey_engine_perf_get_event_count(engine), 1);

        // Finish the bar and enter second loop.
        render_frames(engine, samples_per_step(bpm, sample_rate) as usize * 12 + 256);
        assert!(gooey_engine_perf_is_record_armed(engine));

        // Second pass: different degree
        gooey_engine_poly_trigger_chord(engine, 0, 0, 4, 0, 1, 4, 0.8);
        render_frames(engine, q);
        gooey_engine_poly_release(engine);

        assert!(gooey_engine_perf_get_event_count(engine) >= 2);
        assert!(gooey_engine_perf_is_record_armed(engine));

        gooey_engine_free(engine);
    }
}

#[test]
fn perf_clear_clip() {
    let engine = unsafe { gooey_engine_new(44_100.0) };
    unsafe {
        gooey_engine_set_bpm(engine, 120.0);
        gooey_engine_perf_set_record_mode(engine, PERF_RECORD_MODE_OVERDUB);
        gooey_engine_perf_set_record_armed(engine, true);
        gooey_engine_sequencer_start(engine);
        render_frames(engine, 128);
        gooey_engine_poly_trigger_chord(engine, 0, 0, 1, 0, 1, 4, 1.0);
        render_frames(engine, 1024);
        gooey_engine_poly_release(engine);
        assert!(gooey_engine_perf_get_event_count(engine) >= 1);

        gooey_engine_perf_clear_clip(engine);
        assert_eq!(gooey_engine_perf_get_event_count(engine), 0);

        gooey_engine_free(engine);
    }
}

#[test]
fn live_chord_still_works_without_arm() {
    let engine = unsafe { gooey_engine_new(44_100.0) };
    unsafe {
        gooey_engine_poly_trigger_chord(engine, 0, 0, 0, 0, 1, 4, 0.9);
        let mut buf = vec![0.0_f32; 2048];
        gooey_engine_render(engine, buf.as_mut_ptr(), 1024);
        let peak = buf.iter().map(|s| s.abs()).fold(0.0_f32, f32::max);
        assert!(
            peak > 0.001,
            "live chord should produce audio without recording"
        );
        assert_eq!(gooey_engine_perf_get_event_count(engine), 0);
        gooey_engine_free(engine);
    }
}
