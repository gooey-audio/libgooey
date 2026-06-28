//! Integration tests for the interleaved stereo output contract of
//! `gooey_engine_render`.
//!
//! The engine writes two-channel interleaved output: each frame occupies two
//! consecutive buffer slots laid out as `[left, right]`, so a host must
//! allocate `frames * GOOEY_OUTPUT_CHANNELS` floats. The internal signal path
//! is currently mono, so left and right are identical (the "stereo seam"
//! duplicates the mono sample onto both channels), but callers must always
//! treat the buffer as stereo.

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
