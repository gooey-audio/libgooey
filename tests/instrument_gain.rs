//! Integration tests for per-instrument channel gain

use gooey::ffi::*;

#[test]
fn test_default_gain() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        assert_eq!(gooey_engine_get_instrument_gain(engine, INSTRUMENT_KICK), 1.0);
        assert_eq!(gooey_engine_get_instrument_gain(engine, INSTRUMENT_SNARE), 1.0);
        assert_eq!(gooey_engine_get_instrument_gain(engine, INSTRUMENT_HIHAT), 1.0);
        assert_eq!(gooey_engine_get_instrument_gain(engine, INSTRUMENT_TOM), 1.0);

        gooey_engine_free(engine);
    }
}

#[test]
fn test_gain_zero_silences_instrument() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Trigger kick with default gain - should have audio
        gooey_engine_trigger_instrument(engine, INSTRUMENT_KICK);
        let mut buffer = vec![0.0f32; 1024];
        gooey_engine_render(engine, buffer.as_mut_ptr(), 1024);
        let has_audio_before = buffer.iter().any(|&s| s.abs() > 0.001);

        // Set gain to zero
        gooey_engine_set_instrument_gain(engine, INSTRUMENT_KICK, 0.0);
        assert_eq!(gooey_engine_get_instrument_gain(engine, INSTRUMENT_KICK), 0.0);

        // Trigger again and let gain smoothing settle
        gooey_engine_trigger_instrument(engine, INSTRUMENT_KICK);
        for _ in 0..10 {
            gooey_engine_render(engine, buffer.as_mut_ptr(), 1024);
        }
        let has_audio_after = buffer.iter().any(|&s| s.abs() > 0.001);

        assert!(has_audio_before, "Should have audio at default gain");
        assert!(!has_audio_after, "Should be silent at gain 0.0");

        gooey_engine_free(engine);
    }
}

#[test]
fn test_gain_reduces_level() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Render kick at full gain
        gooey_engine_trigger_instrument(engine, INSTRUMENT_KICK);
        let mut buffer_full = vec![0.0f32; 1024];
        gooey_engine_render(engine, buffer_full.as_mut_ptr(), 1024);
        let peak_full: f32 = buffer_full.iter().map(|s| s.abs()).fold(0.0, f32::max);

        // Reset engine, render kick at half gain
        let engine2 = gooey_engine_new(44100.0);
        gooey_engine_set_instrument_gain(engine2, INSTRUMENT_KICK, 0.5);
        // Let smoothing settle
        let mut buffer_half = vec![0.0f32; 1024];
        for _ in 0..10 {
            gooey_engine_render(engine2, buffer_half.as_mut_ptr(), 1024);
        }
        gooey_engine_trigger_instrument(engine2, INSTRUMENT_KICK);
        gooey_engine_render(engine2, buffer_half.as_mut_ptr(), 1024);
        let peak_half: f32 = buffer_half.iter().map(|s| s.abs()).fold(0.0, f32::max);

        assert!(peak_full > 0.001, "Full gain should produce audio");
        assert!(peak_half > 0.001, "Half gain should produce audio");
        assert!(
            peak_half < peak_full,
            "Half gain ({}) should be quieter than full gain ({})",
            peak_half,
            peak_full
        );

        gooey_engine_free(engine);
        gooey_engine_free(engine2);
    }
}

#[test]
fn test_gain_clamped() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        gooey_engine_set_instrument_gain(engine, INSTRUMENT_KICK, 2.0);
        assert_eq!(gooey_engine_get_instrument_gain(engine, INSTRUMENT_KICK), 1.0);

        gooey_engine_set_instrument_gain(engine, INSTRUMENT_KICK, -0.5);
        assert_eq!(gooey_engine_get_instrument_gain(engine, INSTRUMENT_KICK), 0.0);

        gooey_engine_free(engine);
    }
}

#[test]
fn test_gain_with_mute() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Set gain to 1.0 but mute - should be silent
        gooey_engine_set_instrument_gain(engine, INSTRUMENT_KICK, 1.0);
        gooey_engine_set_instrument_mute(engine, INSTRUMENT_KICK, true);

        gooey_engine_trigger_instrument(engine, INSTRUMENT_KICK);
        let mut buffer = vec![0.0f32; 1024];
        for _ in 0..10 {
            gooey_engine_render(engine, buffer.as_mut_ptr(), 1024);
        }
        let has_audio = buffer.iter().any(|&s| s.abs() > 0.001);

        assert!(!has_audio, "Muted instrument should be silent even with gain 1.0");

        gooey_engine_free(engine);
    }
}

#[test]
fn test_invalid_instrument_id() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Invalid instrument ID should not crash, should return 1.0
        assert_eq!(gooey_engine_get_instrument_gain(engine, 99), 1.0);

        // Setting invalid instrument should not crash
        gooey_engine_set_instrument_gain(engine, 99, 0.5);

        gooey_engine_free(engine);
    }
}
