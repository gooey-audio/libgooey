//! End-to-end tests for the granulator FFI surface introduced in
//! `gooey_engine_granulator_*`. Mirrors the host-side calling sequence that
//! a Swift / AUv3 wrapper would use: create engine → load samples → set
//! parameters → trigger → render → inspect output.

use gooey::ffi::*;

const SAMPLE_RATE: f32 = 44100.0;

fn approx_eq(a: f32, b: f32) {
    assert!(
        (a - b).abs() < 1e-6,
        "expected {b}, got {a} (delta {})",
        (a - b).abs()
    );
}

fn sine_samples(seconds: f32, hz: f32) -> Vec<f32> {
    let n = (SAMPLE_RATE * seconds) as usize;
    (0..n)
        .map(|i| (i as f32 / SAMPLE_RATE * hz * std::f32::consts::TAU).sin() * 0.5)
        .collect()
}

#[test]
fn placeholder_buffer_present_until_set_buffer() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        // Length is the contract: 1 means "no host buffer loaded yet".
        // The placeholder's internal sample rate is an implementation
        // detail (intentionally hardcoded to avoid panicking on a bad
        // engine `sample_rate`) and is not asserted here.
        assert_eq!(gooey_engine_granulator_buffer_len(engine), 1);
        assert!(gooey_engine_granulator_buffer_sample_rate(engine) > 0.0);
        gooey_engine_free(engine);
    }
}

#[test]
fn set_buffer_replaces_placeholder() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        let samples = sine_samples(1.0, 220.0);
        let ok = gooey_engine_granulator_set_buffer(
            engine,
            samples.as_ptr(),
            samples.len() as u32,
            SAMPLE_RATE,
        );
        assert!(ok);
        assert_eq!(
            gooey_engine_granulator_buffer_len(engine),
            samples.len() as u32
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn set_buffer_rejects_invalid_inputs() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        let samples = sine_samples(0.1, 440.0);

        assert!(!gooey_engine_granulator_set_buffer(
            engine,
            std::ptr::null(),
            samples.len() as u32,
            SAMPLE_RATE,
        ));
        assert!(!gooey_engine_granulator_set_buffer(
            engine,
            samples.as_ptr(),
            0,
            SAMPLE_RATE,
        ));
        assert!(!gooey_engine_granulator_set_buffer(
            engine,
            samples.as_ptr(),
            samples.len() as u32,
            0.0,
        ));

        // Buffer should still be the 1-sample placeholder
        assert_eq!(gooey_engine_granulator_buffer_len(engine), 1);
        gooey_engine_free(engine);
    }
}

#[test]
fn param_round_trip() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);

        gooey_engine_granulator_set_param(engine, GRANULATOR_PARAM_SCAN_POSITION, 0.25);
        gooey_engine_granulator_set_param(engine, GRANULATOR_PARAM_GRAIN_LENGTH, 0.6);
        gooey_engine_granulator_set_param(engine, GRANULATOR_PARAM_SPRAY, 0.1);
        gooey_engine_granulator_set_param(engine, GRANULATOR_PARAM_PITCH, 0.75);
        gooey_engine_granulator_set_param(engine, GRANULATOR_PARAM_DENSITY, 0.42);
        gooey_engine_granulator_set_param(engine, GRANULATOR_PARAM_TEXTURE, 0.33);
        gooey_engine_granulator_set_param(engine, GRANULATOR_PARAM_DIRECTION, 0.0);
        gooey_engine_granulator_set_param(engine, GRANULATOR_PARAM_CLOUD_DURATION, 0.5);
        gooey_engine_granulator_set_param(engine, GRANULATOR_PARAM_VOLUME, 0.9);
        gooey_engine_granulator_set_param(engine, GRANULATOR_PARAM_RANDOM_TIMING, 0.45);
        gooey_engine_granulator_set_param(engine, GRANULATOR_PARAM_RANDOM_AMP, 0.65);
        gooey_engine_granulator_set_param(engine, GRANULATOR_PARAM_DRIVE, 0.8);

        approx_eq(
            gooey_engine_granulator_get_param(engine, GRANULATOR_PARAM_SCAN_POSITION),
            0.25,
        );
        approx_eq(
            gooey_engine_granulator_get_param(engine, GRANULATOR_PARAM_GRAIN_LENGTH),
            0.6,
        );
        approx_eq(
            gooey_engine_granulator_get_param(engine, GRANULATOR_PARAM_SPRAY),
            0.1,
        );
        approx_eq(
            gooey_engine_granulator_get_param(engine, GRANULATOR_PARAM_PITCH),
            0.75,
        );
        approx_eq(
            gooey_engine_granulator_get_param(engine, GRANULATOR_PARAM_DENSITY),
            0.42,
        );
        approx_eq(
            gooey_engine_granulator_get_param(engine, GRANULATOR_PARAM_TEXTURE),
            0.33,
        );
        approx_eq(
            gooey_engine_granulator_get_param(engine, GRANULATOR_PARAM_DIRECTION),
            0.0,
        );
        approx_eq(
            gooey_engine_granulator_get_param(engine, GRANULATOR_PARAM_CLOUD_DURATION),
            0.5,
        );
        approx_eq(
            gooey_engine_granulator_get_param(engine, GRANULATOR_PARAM_VOLUME),
            0.9,
        );
        approx_eq(
            gooey_engine_granulator_get_param(engine, GRANULATOR_PARAM_RANDOM_TIMING),
            0.45,
        );
        approx_eq(
            gooey_engine_granulator_get_param(engine, GRANULATOR_PARAM_RANDOM_AMP),
            0.65,
        );
        approx_eq(
            gooey_engine_granulator_get_param(engine, GRANULATOR_PARAM_DRIVE),
            0.8,
        );

        assert!(gooey_engine_granulator_get_param(engine, 999).is_nan());
        gooey_engine_free(engine);
    }
}

#[test]
fn param_get_nan_on_null_engine() {
    unsafe {
        assert!(
            gooey_engine_granulator_get_param(std::ptr::null(), GRANULATOR_PARAM_VOLUME,).is_nan()
        );
    }
}

#[test]
fn trigger_and_render_produces_finite_nonzero_audio() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        let samples = sine_samples(1.0, 220.0);
        assert!(gooey_engine_granulator_set_buffer(
            engine,
            samples.as_ptr(),
            samples.len() as u32,
            SAMPLE_RATE,
        ));
        gooey_engine_granulator_set_seed(engine, 7);
        gooey_engine_granulator_set_param(engine, GRANULATOR_PARAM_VOLUME, 1.0);
        gooey_engine_granulator_set_param(engine, GRANULATOR_PARAM_DENSITY, 0.5);
        gooey_engine_granulator_set_param(engine, GRANULATOR_PARAM_CLOUD_DURATION, 0.5);
        gooey_engine_granulator_snap_params(engine);
        gooey_engine_granulator_trigger(engine, 1.0);

        // ~0.5 seconds; interleaved stereo means two output samples per frame.
        let frames = 22_050usize;
        let mut buffer = vec![0.0_f32; frames * 2];
        gooey_engine_render(engine, buffer.as_mut_ptr(), frames as u32);

        let max_abs = buffer.iter().fold(0.0_f32, |acc, s| {
            assert!(s.is_finite(), "non-finite sample in render output");
            acc.max(s.abs())
        });
        assert!(max_abs > 1e-4, "expected audible granulator output");

        gooey_engine_free(engine);
    }
}

#[test]
fn active_grain_count_rises_after_trigger() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        let samples = sine_samples(1.0, 220.0);
        gooey_engine_granulator_set_buffer(
            engine,
            samples.as_ptr(),
            samples.len() as u32,
            SAMPLE_RATE,
        );
        gooey_engine_granulator_set_seed(engine, 11);
        gooey_engine_granulator_set_param(engine, GRANULATOR_PARAM_DENSITY, 0.6);
        gooey_engine_granulator_set_param(engine, GRANULATOR_PARAM_CLOUD_DURATION, 0.5);
        gooey_engine_granulator_snap_params(engine);
        gooey_engine_granulator_trigger(engine, 1.0);

        let frames = 4096u32;
        // Interleaved stereo: two output samples per frame.
        let mut buffer = vec![0.0_f32; frames as usize * 2];
        gooey_engine_render(engine, buffer.as_mut_ptr(), frames);

        assert!(
            gooey_engine_granulator_active_grain_count(engine) > 0,
            "expected at least one active grain after trigger+render"
        );
        gooey_engine_free(engine);
    }
}
