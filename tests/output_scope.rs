//! Integration tests for the lock-free post-limiter output scope.

use gooey::ffi::*;

const SAMPLE_RATE: f32 = 48_000.0;
const POINT_COUNT: usize = OUTPUT_SCOPE_POINT_COUNT as usize;

unsafe fn render(engine: *mut GooeyEngine, frames: usize) -> Vec<f32> {
    let mut buffer = vec![0.0_f32; frames * GOOEY_OUTPUT_CHANNELS as usize];
    gooey_engine_render(engine, buffer.as_mut_ptr(), frames as u32);
    buffer
}

unsafe fn read_scope(engine: *const GooeyEngine) -> (u64, Vec<f32>, Vec<f32>) {
    let mut min = vec![f32::NAN; POINT_COUNT];
    let mut max = vec![f32::NAN; POINT_COUNT];
    let position = gooey_engine_read_output_scope(
        engine,
        min.as_mut_ptr(),
        max.as_mut_ptr(),
        POINT_COUNT as u32,
    );
    (position, min, max)
}

#[test]
fn triggered_kick_reaches_the_scope_through_render() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        let (position, min, max) = read_scope(engine);
        assert_eq!(position, 0);
        assert!(min.iter().chain(&max).all(|value| *value == 0.0));

        gooey_engine_trigger_kick(engine);
        let output = render(engine, 4_800);

        let samples_per_bin = (SAMPLE_RATE / POINT_COUNT as f32).round() as usize;
        let expected_bins = 4_800 / samples_per_bin;
        let (position, min, max) = read_scope(engine);
        assert_eq!(position, expected_bins as u64);
        assert!(min
            .iter()
            .chain(&max)
            .all(|value| value.is_finite() && value.abs() <= 1.0));
        assert!(
            max.iter().any(|value| *value > 0.0),
            "kick should be visible"
        );

        // Each published bin is the min/max of the mono downmix of its frames.
        let padding = POINT_COUNT - expected_bins;
        for bin in 0..expected_bins {
            let frames = &output[bin * samples_per_bin * 2..(bin + 1) * samples_per_bin * 2];
            let mono = frames
                .chunks_exact(2)
                .map(|frame| ((frame[0] + frame[1]) * 0.5).clamp(-1.0, 1.0));
            let expected_min = mono.clone().fold(f32::INFINITY, f32::min);
            let expected_max = mono.fold(f32::NEG_INFINITY, f32::max);
            assert_eq!(min[padding + bin], expected_min, "bin {bin} min");
            assert_eq!(max[padding + bin], expected_max, "bin {bin} max");
        }

        // No render, no new bins: the position is stable for redraw skipping.
        let (again, _, _) = read_scope(engine);
        assert_eq!(again, position);

        gooey_engine_free(engine);
    }
}

#[test]
fn silence_keeps_the_scope_scrolling() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        let _ = render(engine, SAMPLE_RATE as usize * 2);
        let (position, min, max) = read_scope(engine);
        assert!(position > POINT_COUNT as u64);
        assert!(min.iter().chain(&max).all(|value| *value == 0.0));
        gooey_engine_free(engine);
    }
}

#[test]
fn short_reads_return_the_newest_bins() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        let _ = render(engine, 4_800);
        let (position, full_min, full_max) = read_scope(engine);

        let mut min = [f32::NAN; 16];
        let mut max = [f32::NAN; 16];
        let short = gooey_engine_read_output_scope(engine, min.as_mut_ptr(), max.as_mut_ptr(), 16);
        assert_eq!(short, position);
        assert_eq!(min[..], full_min[POINT_COUNT - 16..]);
        assert_eq!(max[..], full_max[POINT_COUNT - 16..]);
        gooey_engine_free(engine);
    }
}

#[test]
fn offline_bounce_does_not_feed_the_scope() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        let mut length = 0_u32;
        let buffer = gooey_engine_bounce_to_buffer(engine, 1, &mut length);
        assert!(!buffer.is_null() && length > 0);
        gooey_engine_free_buffer(buffer, length);

        let (position, _, _) = read_scope(engine);
        assert_eq!(position, 0);
        gooey_engine_free(engine);
    }
}

#[test]
fn null_arguments_are_safe() {
    unsafe {
        let mut min = [0.0_f32; 4];
        let mut max = [0.0_f32; 4];
        assert_eq!(
            gooey_engine_read_output_scope(std::ptr::null(), min.as_mut_ptr(), max.as_mut_ptr(), 4),
            0
        );

        let engine = gooey_engine_new(SAMPLE_RATE);
        let _ = render(engine, 470);
        assert_eq!(
            gooey_engine_read_output_scope(engine, std::ptr::null_mut(), max.as_mut_ptr(), 4),
            10
        );
        assert_eq!(
            gooey_engine_read_output_scope(engine, min.as_mut_ptr(), max.as_mut_ptr(), 0),
            10
        );
        gooey_engine_free(engine);
    }
}
