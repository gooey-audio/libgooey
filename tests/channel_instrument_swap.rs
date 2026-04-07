//! Integration tests for channel instrument swap API

use gooey::ffi::*;

#[test]
fn test_default_channel_types() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        assert_eq!(
            gooey_engine_get_channel_instrument_type(engine, 0),
            INSTRUMENT_KICK
        );
        assert_eq!(
            gooey_engine_get_channel_instrument_type(engine, 1),
            INSTRUMENT_SNARE
        );
        assert_eq!(
            gooey_engine_get_channel_instrument_type(engine, 2),
            INSTRUMENT_HIHAT
        );
        assert_eq!(
            gooey_engine_get_channel_instrument_type(engine, 3),
            INSTRUMENT_TOM
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn test_swap_and_query() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        gooey_engine_set_channel_instrument_type(engine, 0, INSTRUMENT_SNARE);
        assert_eq!(
            gooey_engine_get_channel_instrument_type(engine, 0),
            INSTRUMENT_SNARE
        );

        gooey_engine_set_channel_instrument_type(engine, 0, INSTRUMENT_TOM);
        assert_eq!(
            gooey_engine_get_channel_instrument_type(engine, 0),
            INSTRUMENT_TOM
        );

        // Other channels unchanged
        assert_eq!(
            gooey_engine_get_channel_instrument_type(engine, 1),
            INSTRUMENT_SNARE
        );
        assert_eq!(
            gooey_engine_get_channel_instrument_type(engine, 2),
            INSTRUMENT_HIHAT
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn test_swap_same_type_noop() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Swapping to the same type should be a no-op
        gooey_engine_set_channel_instrument_type(engine, 0, INSTRUMENT_KICK);
        assert_eq!(
            gooey_engine_get_channel_instrument_type(engine, 0),
            INSTRUMENT_KICK
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn test_swap_produces_audio() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Swap channel 0 to tom
        gooey_engine_set_channel_instrument_type(engine, 0, INSTRUMENT_TOM);

        // Trigger channel 0 (now a tom)
        gooey_engine_trigger_channel(engine, 0);
        let mut buffer = vec![0.0f32; 1024];
        gooey_engine_render(engine, buffer.as_mut_ptr(), 1024);

        let has_audio = buffer.iter().any(|&s| s.abs() > 0.001);
        assert!(has_audio, "Swapped tom on channel 0 should produce audio");

        gooey_engine_free(engine);
    }
}

#[test]
fn test_channel_state_preserved_on_swap() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Set channel 0 gain to 0.5
        gooey_engine_set_instrument_gain(engine, 0, 0.5);
        assert_eq!(gooey_engine_get_instrument_gain(engine, 0), 0.5);

        // Mute channel 0
        gooey_engine_set_instrument_mute(engine, 0, true);

        // Swap channel 0 to hihat
        gooey_engine_set_channel_instrument_type(engine, 0, INSTRUMENT_HIHAT);

        // Verify channel state preserved
        assert_eq!(gooey_engine_get_instrument_gain(engine, 0), 0.5);
        assert!(gooey_engine_get_instrument_mute(engine, 0));

        gooey_engine_free(engine);
    }
}

#[test]
fn test_blend_position_preserved_on_swap() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Enable blend and set position on channel 0
        gooey_engine_blend_enable(engine, 0);
        gooey_engine_blend_set_position(engine, 0, 0.7, 0.3);

        assert!((gooey_engine_blend_get_position_x(engine, 0) - 0.7).abs() < 0.01);
        assert!((gooey_engine_blend_get_position_y(engine, 0) - 0.3).abs() < 0.01);

        // Swap channel 0 to snare
        gooey_engine_set_channel_instrument_type(engine, 0, INSTRUMENT_SNARE);

        // Blend position preserved
        assert!((gooey_engine_blend_get_position_x(engine, 0) - 0.7).abs() < 0.01);
        assert!((gooey_engine_blend_get_position_y(engine, 0) - 0.3).abs() < 0.01);
        assert!(gooey_engine_blend_is_enabled(engine, 0));

        gooey_engine_free(engine);
    }
}

#[test]
fn test_trigger_channel_api() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Swap channel 0 to hihat
        gooey_engine_set_channel_instrument_type(engine, 0, INSTRUMENT_HIHAT);

        // Trigger via channel API
        gooey_engine_trigger_channel_with_velocity(engine, 0, 0.8);
        let mut buffer = vec![0.0f32; 1024];
        gooey_engine_render(engine, buffer.as_mut_ptr(), 1024);

        let has_audio = buffer.iter().any(|&s| s.abs() > 0.001);
        assert!(
            has_audio,
            "Trigger channel with velocity should produce audio"
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn test_set_channel_param() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Swap channel 0 to tom
        gooey_engine_set_channel_instrument_type(engine, 0, INSTRUMENT_TOM);

        // Set param via channel API (TOM_PARAM_TUNE = 0)
        gooey_engine_set_channel_param(engine, 0, 0, 0.8);

        // Trigger and verify audio
        gooey_engine_trigger_channel(engine, 0);
        let mut buffer = vec![0.0f32; 1024];
        gooey_engine_render(engine, buffer.as_mut_ptr(), 1024);

        let has_audio = buffer.iter().any(|&s| s.abs() > 0.001);
        assert!(
            has_audio,
            "Channel param set + trigger should produce audio"
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn test_backward_compat_param_setter() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Default: channel 0 = kick. set_kick_param should work
        gooey_engine_set_kick_param(engine, KICK_PARAM_VOLUME, 0.8);
        gooey_engine_trigger_instrument(engine, INSTRUMENT_KICK);
        let mut buffer = vec![0.0f32; 1024];
        gooey_engine_render(engine, buffer.as_mut_ptr(), 1024);
        let has_audio = buffer.iter().any(|&s| s.abs() > 0.001);
        assert!(
            has_audio,
            "set_kick_param should still work at default mapping"
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn test_duplicate_instrument_types() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Put kick on both channel 0 and channel 2
        gooey_engine_set_channel_instrument_type(engine, 2, INSTRUMENT_KICK);

        assert_eq!(
            gooey_engine_get_channel_instrument_type(engine, 0),
            INSTRUMENT_KICK
        );
        assert_eq!(
            gooey_engine_get_channel_instrument_type(engine, 2),
            INSTRUMENT_KICK
        );

        // Trigger both channels, both should produce audio
        gooey_engine_trigger_channel(engine, 0);
        gooey_engine_trigger_channel(engine, 2);
        let mut buffer = vec![0.0f32; 1024];
        gooey_engine_render(engine, buffer.as_mut_ptr(), 1024);

        let has_audio = buffer.iter().any(|&s| s.abs() > 0.001);
        assert!(has_audio, "Two kick channels should produce audio");

        gooey_engine_free(engine);
    }
}

#[test]
fn test_invalid_args() {
    unsafe {
        let engine = gooey_engine_new(44100.0);

        // Invalid channel index
        gooey_engine_set_channel_instrument_type(engine, 99, INSTRUMENT_KICK);
        assert_eq!(
            gooey_engine_get_channel_instrument_type(engine, 99),
            0xFFFFFFFF
        );

        // Invalid instrument type
        gooey_engine_set_channel_instrument_type(engine, 0, 99);
        assert_eq!(
            gooey_engine_get_channel_instrument_type(engine, 0),
            INSTRUMENT_KICK
        );

        // Invalid channel for trigger
        gooey_engine_trigger_channel(engine, 99);
        gooey_engine_trigger_channel_with_velocity(engine, 99, 1.0);

        // Invalid channel for param
        gooey_engine_set_channel_param(engine, 99, 0, 0.5);

        // Null engine
        gooey_engine_set_channel_instrument_type(std::ptr::null_mut(), 0, INSTRUMENT_KICK);
        assert_eq!(
            gooey_engine_get_channel_instrument_type(std::ptr::null(), 0),
            0xFFFFFFFF
        );

        gooey_engine_free(engine);
    }
}
