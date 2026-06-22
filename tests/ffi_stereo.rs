//! Integration tests for the interleaved stereo output contract of
//! `gooey_engine_render`.
//!
//! The engine writes two-channel interleaved output: each frame occupies two
//! consecutive buffer slots laid out as `[left, right]`, so a host must
//! allocate `frames * GOOEY_OUTPUT_CHANNELS` floats. Instruments are mono and
//! are spread across the stereo field at the "stereo seam" by their
//! per-instrument pan (equal-power, default center); with everything centered
//! and no stereo effect engaged left and right stay identical.

use gooey::ffi::*;

const SAMPLE_RATE: f32 = 44_100.0;

#[test]
fn output_channel_count_is_two() {
    assert_eq!(GOOEY_OUTPUT_CHANNELS, 2);
}

#[test]
fn render_fills_an_interleaved_stereo_buffer_without_error() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);

        let frames = 1024usize;
        let mut buffer = vec![0.0_f32; frames * GOOEY_OUTPUT_CHANNELS as usize];

        // A correctly sized buffer must render without tripping the error flag
        // (which a panic across the FFI boundary would set).
        gooey_engine_render(engine, buffer.as_mut_ptr(), frames as u32);

        assert!(
            !gooey_engine_has_error(engine),
            "render must not set the engine error flag for a correctly sized buffer"
        );
        assert_eq!(buffer.len(), frames * 2, "buffer holds frames * 2 samples");
        assert!(
            buffer.iter().all(|s| s.is_finite()),
            "all rendered samples must be finite"
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn left_and_right_match_and_both_carry_audio_for_the_mono_path() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);

        let frames = 2048usize;
        let mut buffer = vec![0.0_f32; frames * 2];

        gooey_engine_trigger_instrument(engine, INSTRUMENT_KICK);
        gooey_engine_render(engine, buffer.as_mut_ptr(), frames as u32);

        // The kick must produce audible output.
        let peak = buffer.iter().fold(0.0_f32, |acc, s| acc.max(s.abs()));
        assert!(
            peak > 0.001,
            "expected audible kick output, peak was {peak}"
        );

        // Every frame's left sample equals its right sample (mono signal path),
        // and audio must be present on both channels.
        let mut both_channels_audible = false;
        for frame in buffer.chunks_exact(2) {
            assert_eq!(
                frame[0], frame[1],
                "left and right must be identical for the mono signal path"
            );
            if frame[0].abs() > 0.001 {
                both_channels_audible = true;
            }
        }
        assert!(
            both_channels_audible,
            "expected non-zero audio on both the left and right channels"
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn instrument_pan_defaults_to_center_and_round_trips() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);

        assert_eq!(
            gooey_engine_get_instrument_pan(engine, INSTRUMENT_KICK),
            0.5,
            "pan defaults to center"
        );

        gooey_engine_set_instrument_pan(engine, INSTRUMENT_KICK, 0.0);
        assert_eq!(
            gooey_engine_get_instrument_pan(engine, INSTRUMENT_KICK),
            0.0
        );

        // Out-of-range values clamp; invalid instrument returns the center default.
        gooey_engine_set_instrument_pan(engine, INSTRUMENT_KICK, 5.0);
        assert_eq!(
            gooey_engine_get_instrument_pan(engine, INSTRUMENT_KICK),
            1.0
        );
        assert_eq!(gooey_engine_get_instrument_pan(engine, 9999), 0.5);

        gooey_engine_free(engine);
    }
}

#[test]
fn hard_left_pan_steers_audio_to_the_left_channel() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);

        let frames = 2048usize;
        let mut buffer = vec![0.0_f32; frames * 2];

        // Settle the smoothed pan to hard-left before triggering, so the kick's
        // loud attack is measured at the final pan position, not mid-ramp.
        gooey_engine_set_instrument_pan(engine, INSTRUMENT_KICK, 0.0);
        let mut warmup = vec![0.0_f32; 1024 * 2];
        gooey_engine_render(engine, warmup.as_mut_ptr(), 1024);

        gooey_engine_trigger_instrument(engine, INSTRUMENT_KICK);
        gooey_engine_render(engine, buffer.as_mut_ptr(), frames as u32);

        let mut left_energy = 0.0_f64;
        let mut right_energy = 0.0_f64;
        for frame in buffer.chunks_exact(2) {
            left_energy += (frame[0] * frame[0]) as f64;
            right_energy += (frame[1] * frame[1]) as f64;
        }

        assert!(left_energy > 0.0, "expected audible left output");
        assert!(
            right_energy < left_energy * 1e-3,
            "hard-left pan should steer audio to the left (left={left_energy}, right={right_energy})"
        );

        gooey_engine_free(engine);
    }
}
