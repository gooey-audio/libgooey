//! End-to-end tests for `gooey_engine_loop_render_to_wav` — the offline
//! single-channel stem export used by Whirlpool's Render view. These exercise
//! the FFI exactly as the host does (create engine → load a loop → gain/effects
//! → render to a temp WAV) and then read the WAV back to assert its format and
//! content. Gated behind the `bounce` feature (which pulls in `hound`), the same
//! feature that gates the FFI function itself.
#![cfg(feature = "bounce")]

use gooey::ffi::*;
use std::ffi::CString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

const SAMPLE_RATE: f32 = 44_100.0;

/// A process-unique temp path so parallel tests never collide.
fn temp_wav() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "gooey_loop_render_{}_{}.wav",
        std::process::id(),
        n
    ))
}

/// Interleaved stereo buffer, both channels set to `value`.
fn dc_stereo(value: f32, frames: usize) -> Vec<f32> {
    vec![value; frames * 2]
}

/// Interleaved stereo ramp: left = right = frame index (for periodicity checks).
fn ramp_stereo(frames: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        out.push(i as f32);
        out.push(i as f32);
    }
    out
}

unsafe fn load(engine: *mut GooeyEngine, channel: u32, samples: &[f32]) {
    let frames = samples.len() / 2;
    assert!(gooey_engine_loop_load(
        engine,
        channel,
        samples.as_ptr(),
        frames as u32,
        2,
        SAMPLE_RATE,
    ));
}

unsafe fn render(
    engine: *mut GooeyEngine,
    channel: u32,
    frames: u32,
    preroll: u32,
    path: &PathBuf,
) -> bool {
    let c = CString::new(path.to_str().unwrap()).unwrap();
    gooey_engine_loop_render_to_wav(engine, channel, frames, preroll, c.as_ptr())
}

/// Read a WAV back as (spec, interleaved f32 samples).
fn read_wav(path: &PathBuf) -> (hound::WavSpec, Vec<f32>) {
    let reader = hound::WavReader::open(path).expect("wav should exist");
    let spec = reader.spec();
    let samples: Vec<f32> = hound::WavReader::open(path)
        .unwrap()
        .into_samples::<f32>()
        .map(|s| s.unwrap())
        .collect();
    (spec, samples)
}

#[test]
fn writes_stereo_32bit_float_with_exact_frame_count() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        load(engine, 0, &dc_stereo(0.5, 4096));
        let path = temp_wav();
        assert!(render(engine, 0, 1000, 512, &path));

        let (spec, samples) = read_wav(&path);
        assert_eq!(spec.channels, 2, "stereo");
        assert_eq!(spec.bits_per_sample, 32);
        assert_eq!(spec.sample_format, hound::SampleFormat::Float);
        assert_eq!(spec.sample_rate, 44_100);
        assert_eq!(samples.len(), 1000 * 2, "exact frame count, no tail");

        gooey_engine_free(engine);
        let _ = std::fs::remove_file(&path);
    }
}

#[test]
fn bakes_in_channel_gain() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        load(engine, 0, &dc_stereo(0.5, 4096));
        gooey_engine_loop_set_gain(engine, 0, 0.5);
        let path = temp_wav();
        assert!(render(engine, 0, 256, 256, &path));

        let (_, samples) = read_wav(&path);
        // 0.5 buffer * 0.5 gain = 0.25, present from the very first sample.
        assert!(
            (samples[0] - 0.25).abs() < 1e-3,
            "gain not baked in: {}",
            samples[0]
        );
        gooey_engine_free(engine);
        let _ = std::fs::remove_file(&path);
    }
}

#[test]
fn repeats_selected_loop_region_to_fill_length() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        load(engine, 0, &ramp_stereo(400));
        // Region = first quarter -> 100-frame period.
        gooey_engine_loop_set_start(engine, 0, 0.0);
        gooey_engine_loop_set_end(engine, 0, 0.25);
        let path = temp_wav();
        assert!(render(engine, 0, 350, 0, &path));

        let (_, samples) = read_wav(&path);
        for i in 0..350 {
            let expected = (i % 100) as f32;
            assert!(
                (samples[i * 2] - expected).abs() < 1e-3,
                "frame {i}: got {}, want {expected}",
                samples[i * 2]
            );
        }
        gooey_engine_free(engine);
        let _ = std::fs::remove_file(&path);
    }
}

#[test]
fn ignores_mute_solo_and_other_channels() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        load(engine, 0, &dc_stereo(0.5, 4096));
        load(engine, 1, &dc_stereo(-0.9, 4096));
        // Hostile transport state: target muted, another channel soloed.
        gooey_engine_loop_set_mute(engine, 0, true);
        gooey_engine_loop_set_solo(engine, 1, true);
        let path = temp_wav();
        assert!(render(engine, 0, 256, 256, &path));

        let (_, samples) = read_wav(&path);
        assert!(
            (samples[0] - 0.5).abs() < 1e-3,
            "render honored mute/solo or leaked another channel: {}",
            samples[0]
        );
        gooey_engine_free(engine);
        let _ = std::fs::remove_file(&path);
    }
}

#[test]
fn effects_are_baked_and_warmed_by_preroll() {
    unsafe fn render_to_vec(preroll: u32) -> Vec<f32> {
        let engine = gooey_engine_new(SAMPLE_RATE);
        load(engine, 0, &ramp_stereo(400));
        gooey_engine_loop_set_end(engine, 0, 0.25);
        let slot = gooey_engine_loop_effect_add(engine, 0, EFFECT_DELAY);
        assert!(slot >= 0);
        gooey_engine_loop_effect_set_param(engine, 0, slot as u32, DELAY_PARAM_FEEDBACK, 0.7);
        gooey_engine_loop_effect_set_param(engine, 0, slot as u32, DELAY_PARAM_MIX, 0.5);
        let path = temp_wav();
        assert!(render(engine, 0, 2000, preroll, &path));
        let (_, samples) = read_wav(&path);
        gooey_engine_free(engine);
        let _ = std::fs::remove_file(&path);
        samples
    }
    unsafe {
        let cold = render_to_vec(0);
        let warm = render_to_vec(20_000);
        let diff = cold
            .iter()
            .zip(&warm)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(diff > 1e-3, "preroll did not warm the delay tail");
    }
}

#[test]
fn rejects_invalid_arguments() {
    unsafe {
        let path = temp_wav();
        let c = CString::new(path.to_str().unwrap()).unwrap();
        // Null engine.
        assert!(!gooey_engine_loop_render_to_wav(
            std::ptr::null_mut(),
            0,
            100,
            0,
            c.as_ptr()
        ));

        let engine = gooey_engine_new(SAMPLE_RATE);
        load(engine, 0, &dc_stereo(0.5, 1024));
        // Null path.
        assert!(!gooey_engine_loop_render_to_wav(engine, 0, 100, 0, std::ptr::null()));
        // Empty path.
        let empty = CString::new("").unwrap();
        assert!(!gooey_engine_loop_render_to_wav(engine, 0, 100, 0, empty.as_ptr()));
        // Zero frame count.
        assert!(!render(engine, 0, 0, 0, &path));
        // Out-of-range channel.
        assert!(!render(engine, 99, 100, 0, &path));
        // Channel with no buffer loaded.
        assert!(!render(engine, 1, 100, 0, &path));

        gooey_engine_free(engine);
        let _ = std::fs::remove_file(&path);
    }
}
