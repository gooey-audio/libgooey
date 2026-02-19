//! Tests that setting volume to 0 fully silences each instrument

use gooey::ffi::*;

/// Helper: trigger an instrument, set volume to 0, let smoothing settle, then
/// trigger again and verify the output buffer is silent.
unsafe fn assert_volume_zero_silences(
    instrument: u32,
    param_setter: unsafe fn(*mut GooeyEngine, u32, f32),
) {
    let engine = gooey_engine_new(44100.0);
    let mut buffer = vec![0.0f32; 1024];

    // First, verify the instrument produces audio at default volume
    gooey_engine_trigger_instrument(engine, instrument);
    gooey_engine_render(engine, buffer.as_mut_ptr(), 1024);
    let has_audio = buffer.iter().any(|&s| s.abs() > 0.001);
    assert!(
        has_audio,
        "Instrument {} should produce audio at default volume",
        instrument
    );

    // Set volume to 0
    param_setter(engine, instrument, 0.0);

    // Render enough buffers for smoothing to settle (15ms at 44100 = ~661 samples,
    // but we render extra to be safe)
    for _ in 0..10 {
        gooey_engine_render(engine, buffer.as_mut_ptr(), 1024);
    }

    // Trigger again with volume at 0 and render
    gooey_engine_trigger_instrument(engine, instrument);
    // Render a few buffers to capture the full transient region
    for _ in 0..5 {
        gooey_engine_render(engine, buffer.as_mut_ptr(), 1024);
        let max_sample = buffer
            .iter()
            .map(|s| s.abs())
            .fold(0.0f32, f32::max);
        assert!(
            max_sample < 1e-6,
            "Instrument {} should be silent at volume 0, got peak {:.9}",
            instrument,
            max_sample
        );
    }

    gooey_engine_free(engine);
}

/// Helper: trigger an instrument at full volume, then set volume to 0
/// mid-playback and verify the output becomes silent after smoothing.
unsafe fn assert_volume_zero_silences_mid_playback(
    instrument: u32,
    param_setter: unsafe fn(*mut GooeyEngine, u32, f32),
) {
    let engine = gooey_engine_new(44100.0);
    let mut buffer = vec![0.0f32; 1024];

    // Trigger at full volume and verify audio is produced
    gooey_engine_trigger_instrument(engine, instrument);
    gooey_engine_render(engine, buffer.as_mut_ptr(), 1024);
    let has_audio = buffer.iter().any(|&s| s.abs() > 0.001);
    assert!(
        has_audio,
        "Instrument {} should produce audio at default volume",
        instrument
    );

    // Set volume to 0 while sound is still playing
    param_setter(engine, instrument, 0.0);

    // Render enough buffers for smoothing to settle
    for _ in 0..10 {
        gooey_engine_render(engine, buffer.as_mut_ptr(), 1024);
    }

    // After smoothing settles, output must be fully silent
    gooey_engine_render(engine, buffer.as_mut_ptr(), 1024);
    let max_sample = buffer
        .iter()
        .map(|s| s.abs())
        .fold(0.0f32, f32::max);
    assert!(
        max_sample < 1e-6,
        "Instrument {} should be silent after volume set to 0 mid-playback, got peak {:.9}",
        instrument,
        max_sample
    );

    gooey_engine_free(engine);
}

unsafe fn set_kick_volume(engine: *mut GooeyEngine, _instrument: u32, value: f32) {
    gooey_engine_set_kick_param(engine, KICK_PARAM_VOLUME, value);
}

unsafe fn set_snare_volume(engine: *mut GooeyEngine, _instrument: u32, value: f32) {
    gooey_engine_set_snare_param(engine, SNARE_PARAM_VOLUME, value);
}

unsafe fn set_hihat_volume(engine: *mut GooeyEngine, _instrument: u32, value: f32) {
    gooey_engine_set_hihat_param(engine, HIHAT_PARAM_VOLUME, value);
}

unsafe fn set_tom_volume(engine: *mut GooeyEngine, _instrument: u32, value: f32) {
    gooey_engine_set_tom_param(engine, TOM_PARAM_VOLUME, value);
}

#[test]
fn test_kick_volume_zero_silences() {
    unsafe {
        assert_volume_zero_silences(INSTRUMENT_KICK, set_kick_volume);
    }
}

#[test]
fn test_snare_volume_zero_silences() {
    unsafe {
        assert_volume_zero_silences(INSTRUMENT_SNARE, set_snare_volume);
    }
}

#[test]
fn test_hihat_volume_zero_silences() {
    unsafe {
        assert_volume_zero_silences(INSTRUMENT_HIHAT, set_hihat_volume);
    }
}

#[test]
fn test_tom_volume_zero_silences() {
    unsafe {
        assert_volume_zero_silences(INSTRUMENT_TOM, set_tom_volume);
    }
}

#[test]
fn test_kick_volume_zero_mid_playback() {
    unsafe {
        assert_volume_zero_silences_mid_playback(INSTRUMENT_KICK, set_kick_volume);
    }
}

#[test]
fn test_snare_volume_zero_mid_playback() {
    unsafe {
        assert_volume_zero_silences_mid_playback(INSTRUMENT_SNARE, set_snare_volume);
    }
}

#[test]
fn test_hihat_volume_zero_mid_playback() {
    unsafe {
        assert_volume_zero_silences_mid_playback(INSTRUMENT_HIHAT, set_hihat_volume);
    }
}

#[test]
fn test_tom_volume_zero_mid_playback() {
    unsafe {
        assert_volume_zero_silences_mid_playback(INSTRUMENT_TOM, set_tom_volume);
    }
}
