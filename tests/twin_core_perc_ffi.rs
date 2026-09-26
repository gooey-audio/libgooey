use gooey::ffi::*;
use std::ffi::CStr;

const SAMPLE_RATE: f32 = 48_000.0;

unsafe fn render(voice: *mut GooeyTwinCorePercVoice, frames: usize) -> Vec<f32> {
    let mut samples = vec![0.0; frames];
    assert!(gooey_twin_core_perc_render(
        voice,
        samples.as_mut_ptr(),
        frames as u32,
    ));
    samples
}

fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|sample| sample * sample).sum()
}

#[test]
fn standalone_ffi_lifecycle_renders_every_preset() {
    unsafe {
        let voice = gooey_twin_core_perc_new(SAMPLE_RATE);
        assert!(!voice.is_null());
        let mut energies = Vec::new();
        for preset in TWIN_CORE_PERC_PRESET_KICK..=TWIN_CORE_PERC_PRESET_METALLIC {
            assert!(gooey_twin_core_perc_set_preset(voice, preset));
            assert!(gooey_twin_core_perc_trigger(voice, 0.8));
            let samples = render(voice, 24_000);
            assert!(samples.iter().all(|sample| sample.is_finite()));
            energies.push(energy(&samples));
            assert!(gooey_twin_core_perc_reset(voice));
        }
        assert!(energies.iter().all(|value| *value > 1.0e-5));
        assert!(!gooey_twin_core_perc_set_preset(voice, 99));
        gooey_twin_core_perc_destroy(voice);
        gooey_twin_core_perc_destroy(std::ptr::null_mut());
    }
}

#[test]
fn ffi_exposes_controls_modes_names_and_noise_trigger() {
    unsafe {
        let voice = gooey_twin_core_perc_new(SAMPLE_RATE);
        assert!(gooey_twin_core_perc_set_parameter(voice, 7, 0.73));
        assert!((gooey_twin_core_perc_get_parameter(voice, 7) - 0.73).abs() < 1.0e-6);
        assert!(!gooey_twin_core_perc_set_parameter(
            voice,
            TWIN_CORE_PERC_PARAM_COUNT,
            0.5,
        ));
        assert!(gooey_twin_core_perc_get_parameter(voice, 99).is_nan());
        assert_eq!(
            CStr::from_ptr(gooey_twin_core_perc_parameter_name(7))
                .to_str()
                .unwrap(),
            "harmonics"
        );
        assert!(gooey_twin_core_perc_parameter_name(99).is_null());
        assert!(gooey_twin_core_perc_set_body_mode(
            voice,
            TWIN_CORE_PERC_BODY_HIGH,
        ));
        assert_eq!(
            gooey_twin_core_perc_get_body_mode(voice),
            TWIN_CORE_PERC_BODY_HIGH
        );
        assert!(gooey_twin_core_perc_set_noise_mode(
            voice,
            TWIN_CORE_PERC_NOISE_HIGHPASS,
        ));
        assert_eq!(
            gooey_twin_core_perc_get_noise_mode(voice),
            TWIN_CORE_PERC_NOISE_HIGHPASS
        );
        assert!(!gooey_twin_core_perc_set_body_mode(voice, 99));
        assert!(!gooey_twin_core_perc_set_noise_mode(voice, 99));
        assert!(gooey_twin_core_perc_set_seed(voice, 1234));
        assert!(gooey_twin_core_perc_set_midi_note(voice, 64));
        assert!(gooey_twin_core_perc_set_ring_limit(voice, 0.25));
        assert!(gooey_twin_core_perc_trigger_noise(voice, 1.0));
        assert!(energy(&render(voice, 4_800)) > 1.0e-5);
        gooey_twin_core_perc_destroy(voice);
    }
}

#[test]
fn ffi_rejects_null_handles_and_buffers() {
    unsafe {
        assert!(!gooey_twin_core_perc_trigger(std::ptr::null_mut(), 1.0));
        assert!(!gooey_twin_core_perc_trigger_noise(
            std::ptr::null_mut(),
            1.0,
        ));
        assert_eq!(
            gooey_twin_core_perc_get_body_mode(std::ptr::null()),
            u32::MAX
        );
        assert_eq!(
            gooey_twin_core_perc_get_noise_mode(std::ptr::null()),
            u32::MAX
        );
        assert!(!gooey_twin_core_perc_render(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            64,
        ));
        let voice = gooey_twin_core_perc_new(SAMPLE_RATE);
        assert!(!gooey_twin_core_perc_render(
            voice,
            std::ptr::null_mut(),
            64,
        ));
        gooey_twin_core_perc_destroy(voice);
    }
}

#[test]
fn selectable_engine_ffi_supports_a_dropdown_and_shared_presets() {
    unsafe {
        let voice = gooey_percussion_engine_new(SAMPLE_RATE, PERCUSSION_ENGINE_ROUTING_MATRIX);
        assert!(!voice.is_null());
        assert_eq!(
            gooey_percussion_engine_get_selected(voice),
            PERCUSSION_ENGINE_ROUTING_MATRIX
        );
        assert_eq!(gooey_percussion_engine_parameter_count(voice), 9);
        assert!(gooey_percussion_engine_set_preset(
            voice,
            PERCUSSION_PRESET_SNARE
        ));
        assert!(gooey_percussion_engine_trigger(voice, 0.8));
        let mut matrix = vec![0.0; 9_600];
        assert!(gooey_percussion_engine_render(
            voice,
            matrix.as_mut_ptr(),
            matrix.len() as u32,
        ));
        assert!(energy(&matrix) > 1.0e-5);

        assert!(gooey_percussion_engine_select(
            voice,
            PERCUSSION_ENGINE_TWIN_CORE
        ));
        assert_eq!(
            gooey_percussion_engine_get_selected(voice),
            PERCUSSION_ENGINE_TWIN_CORE
        );
        assert_eq!(
            gooey_percussion_engine_get_preset(voice),
            PERCUSSION_PRESET_SNARE
        );
        assert_eq!(gooey_percussion_engine_parameter_count(voice), 12);
        assert_eq!(
            CStr::from_ptr(gooey_percussion_engine_parameter_name(voice, 7))
                .to_str()
                .unwrap(),
            "harmonics"
        );
        assert!(gooey_percussion_engine_set_parameter(voice, 7, 0.8));
        assert!((gooey_percussion_engine_get_parameter(voice, 7) - 0.8).abs() < 1.0e-6);
        gooey_percussion_engine_destroy(voice);
    }
}
