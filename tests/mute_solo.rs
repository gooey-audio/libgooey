//! Integration tests for mute/solo functionality

use gooey::ffi::*;

#[test]
fn test_default_state() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // All instruments should be unmuted and not soloed by default
        assert!(!gooey_engine_get_instrument_mute(engine, INSTRUMENT_KICK));
        assert!(!gooey_engine_get_instrument_mute(engine, INSTRUMENT_SNARE));
        assert!(!gooey_engine_get_instrument_mute(engine, INSTRUMENT_HIHAT));
        assert!(!gooey_engine_get_instrument_mute(engine, INSTRUMENT_TOM));

        assert!(!gooey_engine_get_instrument_solo(engine, INSTRUMENT_KICK));
        assert!(!gooey_engine_get_instrument_solo(engine, INSTRUMENT_SNARE));
        assert!(!gooey_engine_get_instrument_solo(engine, INSTRUMENT_HIHAT));
        assert!(!gooey_engine_get_instrument_solo(engine, INSTRUMENT_TOM));

        gooey_engine_free(engine);
    }
}

#[test]
fn test_mute_silences_instrument() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Trigger kick and render - should have audio
        gooey_engine_trigger_instrument(engine, INSTRUMENT_KICK);
        let mut buffer = vec![0.0f32; 1024];
        gooey_engine_render(engine, buffer.as_mut_ptr(), 1024);
        let has_audio_before_mute = buffer.iter().any(|&s| s.abs() > 0.001);

        // Mute kick
        gooey_engine_set_instrument_mute(engine, INSTRUMENT_KICK, true);
        assert!(gooey_engine_get_instrument_mute(engine, INSTRUMENT_KICK));

        // Trigger again and let gain smoothing settle
        gooey_engine_trigger_instrument(engine, INSTRUMENT_KICK);
        for _ in 0..10 {
            gooey_engine_render(engine, buffer.as_mut_ptr(), 1024);
        }
        let has_audio_after_mute = buffer.iter().any(|&s| s.abs() > 0.001);

        assert!(has_audio_before_mute, "Should have audio before mute");
        assert!(!has_audio_after_mute, "Should be silent after mute");

        gooey_engine_free(engine);
    }
}

#[test]
fn test_solo_isolates_instrument() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Solo kick only
        gooey_engine_set_instrument_solo(engine, INSTRUMENT_KICK, true);
        assert!(gooey_engine_get_instrument_solo(engine, INSTRUMENT_KICK));

        // Trigger snare (not soloed) and let gain smoothing settle
        gooey_engine_trigger_instrument(engine, INSTRUMENT_SNARE);
        let mut buffer = vec![0.0f32; 1024];
        for _ in 0..10 {
            gooey_engine_render(engine, buffer.as_mut_ptr(), 1024);
        }

        // With only snare triggered and kick soloed, output should be silent
        let has_audio = buffer.iter().any(|&s| s.abs() > 0.001);
        assert!(
            !has_audio,
            "Non-soloed instrument should be silent when another is soloed"
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn test_solo_allows_soloed_instrument() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Solo kick
        gooey_engine_set_instrument_solo(engine, INSTRUMENT_KICK, true);

        // Trigger kick (soloed)
        gooey_engine_trigger_instrument(engine, INSTRUMENT_KICK);
        let mut buffer = vec![0.0f32; 1024];
        gooey_engine_render(engine, buffer.as_mut_ptr(), 1024);

        // Soloed instrument should produce audio
        let has_audio = buffer.iter().any(|&s| s.abs() > 0.001);
        assert!(has_audio, "Soloed instrument should produce audio");

        gooey_engine_free(engine);
    }
}

#[test]
fn test_solo_overrides_mute() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Mute AND solo kick
        gooey_engine_set_instrument_mute(engine, INSTRUMENT_KICK, true);
        gooey_engine_set_instrument_solo(engine, INSTRUMENT_KICK, true);

        // Trigger kick
        gooey_engine_trigger_instrument(engine, INSTRUMENT_KICK);

        // Render - should have audio (solo overrides mute)
        let mut buffer = vec![0.0f32; 1024];
        gooey_engine_render(engine, buffer.as_mut_ptr(), 1024);
        let has_audio = buffer.iter().any(|&s| s.abs() > 0.001);

        assert!(has_audio, "Solo should override mute");

        gooey_engine_free(engine);
    }
}

#[test]
fn test_multiple_solos() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Solo both kick and snare
        gooey_engine_set_instrument_solo(engine, INSTRUMENT_KICK, true);
        gooey_engine_set_instrument_solo(engine, INSTRUMENT_SNARE, true);

        // Both should report as soloed
        assert!(gooey_engine_get_instrument_solo(engine, INSTRUMENT_KICK));
        assert!(gooey_engine_get_instrument_solo(engine, INSTRUMENT_SNARE));
        assert!(!gooey_engine_get_instrument_solo(engine, INSTRUMENT_HIHAT));
        assert!(!gooey_engine_get_instrument_solo(engine, INSTRUMENT_TOM));

        // Trigger both soloed instruments
        gooey_engine_trigger_instrument(engine, INSTRUMENT_KICK);
        gooey_engine_trigger_instrument(engine, INSTRUMENT_SNARE);

        let mut buffer = vec![0.0f32; 1024];
        gooey_engine_render(engine, buffer.as_mut_ptr(), 1024);

        // Should have audio from both
        let has_audio = buffer.iter().any(|&s| s.abs() > 0.001);
        assert!(has_audio, "Multiple soloed instruments should produce audio");

        gooey_engine_free(engine);
    }
}

#[test]
fn test_unmute_restores_audio() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Mute then unmute
        gooey_engine_set_instrument_mute(engine, INSTRUMENT_KICK, true);
        gooey_engine_set_instrument_mute(engine, INSTRUMENT_KICK, false);
        assert!(!gooey_engine_get_instrument_mute(engine, INSTRUMENT_KICK));

        // Trigger kick
        gooey_engine_trigger_instrument(engine, INSTRUMENT_KICK);
        let mut buffer = vec![0.0f32; 1024];
        gooey_engine_render(engine, buffer.as_mut_ptr(), 1024);

        let has_audio = buffer.iter().any(|&s| s.abs() > 0.001);
        assert!(has_audio, "Unmuted instrument should produce audio");

        gooey_engine_free(engine);
    }
}

#[test]
fn test_unsolo_restores_other_instruments() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Solo kick, then unsolo
        gooey_engine_set_instrument_solo(engine, INSTRUMENT_KICK, true);
        gooey_engine_set_instrument_solo(engine, INSTRUMENT_KICK, false);

        // Now trigger snare (which was silenced when kick was soloed)
        gooey_engine_trigger_instrument(engine, INSTRUMENT_SNARE);
        let mut buffer = vec![0.0f32; 1024];
        gooey_engine_render(engine, buffer.as_mut_ptr(), 1024);

        let has_audio = buffer.iter().any(|&s| s.abs() > 0.001);
        assert!(
            has_audio,
            "After unsolo, other instruments should produce audio"
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn test_invalid_instrument_id() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Invalid instrument ID should not crash, should return false
        assert!(!gooey_engine_get_instrument_mute(engine, 99));
        assert!(!gooey_engine_get_instrument_solo(engine, 99));

        // Setting invalid instrument should not crash
        gooey_engine_set_instrument_mute(engine, 99, true);
        gooey_engine_set_instrument_solo(engine, 99, true);

        gooey_engine_free(engine);
    }
}
