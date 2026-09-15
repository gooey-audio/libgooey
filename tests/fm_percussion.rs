use gooey::ffi::*;

unsafe fn render_trigger(note: u8, velocity: f32) -> Vec<f32> {
    let engine = gooey_engine_new(48_000.0);
    gooey_engine_set_channel_instrument_type(engine, 0, INSTRUMENT_FM_PERCUSSION);
    assert!(gooey_engine_trigger_channel_note(engine, 0, note, velocity));
    let mut output = vec![0.0; 4_096 * 2];
    gooey_engine_render(engine, output.as_mut_ptr(), 4_096);
    gooey_engine_free(engine);
    output
}

fn spectral_centroid(interleaved: &[f32], sample_rate: f64) -> f64 {
    let mono: Vec<f64> = interleaved
        .chunks_exact(2)
        .skip(64)
        .take(1_024)
        .map(|frame| frame[0] as f64)
        .collect();
    let mut weighted = 0.0;
    let mut total = 0.0;
    for bin in 1..mono.len() / 2 {
        let mut real = 0.0;
        let mut imaginary = 0.0;
        for (sample, value) in mono.iter().enumerate() {
            let window =
                0.5 - 0.5 * (std::f64::consts::TAU * sample as f64 / (mono.len() - 1) as f64).cos();
            let phase = std::f64::consts::TAU * bin as f64 * sample as f64 / mono.len() as f64;
            real += value * window * phase.cos();
            imaginary -= value * window * phase.sin();
        }
        let magnitude = (real * real + imaginary * imaginary).sqrt();
        let frequency = bin as f64 * sample_rate / mono.len() as f64;
        weighted += frequency * magnitude;
        total += magnitude;
    }
    weighted / total.max(f64::MIN_POSITIVE)
}

#[test]
fn stable_counts_and_all_parameters_round_trip() {
    unsafe {
        assert_eq!(gooey_engine_instrument_count(), 5);
        assert_eq!(gooey_engine_instrument_type_count(), 6);
        assert_eq!(gooey_engine_fm_percussion_param_count(), 40);

        let engine = gooey_engine_new(48_000.0);
        gooey_engine_set_channel_instrument_type(engine, 2, INSTRUMENT_FM_PERCUSSION);
        for param in 0..FM_PERCUSSION_PARAM_COUNT {
            gooey_engine_set_channel_param(engine, 2, param, 0.234);
            let expected = match param {
                FM_PERCUSSION_PARAM_OSC1_WAVEFORM | FM_PERCUSSION_PARAM_OSC2_WAVEFORM => {
                    FM_PERCUSSION_WAVEFORM_TRIANGLE
                }
                FM_PERCUSSION_PARAM_OSC1_TRACKING
                | FM_PERCUSSION_PARAM_OSC2_TRACKING
                | FM_PERCUSSION_PARAM_FILTER_MODE => 0.0,
                FM_PERCUSSION_PARAM_RING_MODE => FM_PERCUSSION_RING_OFF,
                _ => 0.234,
            };
            assert_eq!(gooey_engine_get_channel_param(engine, 2, param), expected);
        }

        for (value, expected) in [
            (0.0, FM_PERCUSSION_WAVEFORM_SINE),
            (1.0 / 3.0, FM_PERCUSSION_WAVEFORM_TRIANGLE),
            (2.0 / 3.0, FM_PERCUSSION_WAVEFORM_SQUARE),
            (1.0, FM_PERCUSSION_WAVEFORM_METAL),
        ] {
            gooey_engine_set_channel_param(engine, 2, FM_PERCUSSION_PARAM_OSC1_WAVEFORM, value);
            assert_eq!(
                gooey_engine_get_channel_param(engine, 2, FM_PERCUSSION_PARAM_OSC1_WAVEFORM),
                expected
            );
        }
        for (value, expected) in [
            (0.0, FM_PERCUSSION_RING_OFF),
            (0.5, FM_PERCUSSION_RING),
            (1.0, FM_PERCUSSION_CROSS_RING),
        ] {
            gooey_engine_set_channel_param(engine, 2, FM_PERCUSSION_PARAM_RING_MODE, value);
            assert_eq!(
                gooey_engine_get_channel_param(engine, 2, FM_PERCUSSION_PARAM_RING_MODE),
                expected
            );
        }

        gooey_engine_set_channel_param(engine, 2, FM_PERCUSSION_PARAM_VOLUME, 2.0);
        assert_eq!(
            gooey_engine_get_channel_param(engine, 2, FM_PERCUSSION_PARAM_VOLUME),
            1.0
        );
        gooey_engine_set_channel_param(engine, 2, FM_PERCUSSION_PARAM_VOLUME, f32::NAN);
        assert_eq!(
            gooey_engine_get_channel_param(engine, 2, FM_PERCUSSION_PARAM_VOLUME),
            1.0
        );
        assert!(gooey_engine_get_channel_param(engine, 99, 0).is_nan());
        assert!(gooey_engine_get_channel_param(engine, 2, 40).is_nan());
        gooey_engine_free(engine);
    }
}

#[test]
fn fm_swap_preserves_strip_state_and_multiple_copies_are_independent() {
    unsafe {
        let engine = gooey_engine_new(48_000.0);
        gooey_engine_set_instrument_gain(engine, 0, 0.42);
        gooey_engine_set_instrument_pan(engine, 0, 0.8);
        gooey_engine_sequencer_set_instrument_step(engine, 0, 7, true);
        gooey_engine_set_channel_instrument_type(engine, 0, INSTRUMENT_FM_PERCUSSION);
        gooey_engine_set_channel_instrument_type(engine, 3, INSTRUMENT_FM_PERCUSSION);

        assert_eq!(gooey_engine_get_instrument_gain(engine, 0), 0.42);
        assert_eq!(gooey_engine_get_instrument_pan(engine, 0), 0.8);
        assert!(gooey_engine_sequencer_get_instrument_step_enabled(
            engine, 0, 7
        ));
        assert_eq!(
            gooey_engine_get_channel_instrument_type(engine, 0),
            INSTRUMENT_FM_PERCUSSION
        );
        assert_eq!(
            gooey_engine_get_channel_instrument_type(engine, 3),
            INSTRUMENT_FM_PERCUSSION
        );

        gooey_engine_blend_enable(engine, 0);
        gooey_engine_blend_enable(engine, 3);
        gooey_engine_blend_set_position(engine, 0, 0.0, 0.0);
        gooey_engine_blend_set_position(engine, 3, 1.0, 1.0);
        assert_ne!(
            gooey_engine_get_channel_param(engine, 0, FM_PERCUSSION_PARAM_INDEX),
            gooey_engine_get_channel_param(engine, 3, FM_PERCUSSION_PARAM_INDEX)
        );
        assert!(gooey_engine_load_fm_percussion_preset(
            engine,
            0,
            FM_PERCUSSION_PRESET_ZAP
        ));
        assert!(!gooey_engine_load_fm_percussion_preset(engine, 1, 0));
        gooey_engine_free(engine);
    }
}

#[test]
fn manual_notes_change_sound_without_changing_base_pitch() {
    unsafe {
        let low = render_trigger(36, 1.0);
        let high = render_trigger(84, 1.0);
        assert!(low.iter().any(|sample| sample.abs() > 1e-5));
        assert!(high.iter().any(|sample| sample.abs() > 1e-5));
        let difference: f64 = low
            .iter()
            .zip(&high)
            .map(|(a, b)| (*a as f64 - *b as f64).abs())
            .sum();
        assert!(difference > 1.0);

        let engine = gooey_engine_new(48_000.0);
        gooey_engine_set_channel_instrument_type(engine, 0, INSTRUMENT_FM_PERCUSSION);
        gooey_engine_set_channel_param(engine, 0, FM_PERCUSSION_PARAM_BASE_PITCH, 0.31);
        assert!(gooey_engine_trigger_channel_note(engine, 0, 96, 0.75));
        let mut output = vec![0.0; 512 * 2];
        gooey_engine_render(engine, output.as_mut_ptr(), 512);
        assert_eq!(
            gooey_engine_get_channel_param(engine, 0, FM_PERCUSSION_PARAM_BASE_PITCH),
            0.31
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn sequencer_note_does_not_overwrite_the_base_patch() {
    unsafe {
        let engine = gooey_engine_new(48_000.0);
        gooey_engine_set_channel_instrument_type(engine, 0, INSTRUMENT_FM_PERCUSSION);
        gooey_engine_set_channel_param(engine, 0, FM_PERCUSSION_PARAM_BASE_PITCH, 0.27);
        gooey_engine_sequencer_set_instrument_step_with_velocity(engine, 0, 0, true, 0.8);
        gooey_engine_sequencer_set_instrument_step_note(engine, 0, 0, 91);
        gooey_engine_sequencer_start(engine);
        let mut output = vec![0.0; 1_024 * 2];
        gooey_engine_render(engine, output.as_mut_ptr(), 1_024);
        assert!(output.iter().any(|sample| sample.abs() > 1e-5));
        assert_eq!(
            gooey_engine_get_channel_param(engine, 0, FM_PERCUSSION_PARAM_BASE_PITCH),
            0.27
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn default_velocity_level_route_changes_rms() {
    unsafe {
        let soft = render_trigger(60, 0.2);
        let hard = render_trigger(60, 1.0);
        let rms = |samples: &[f32]| {
            (samples
                .iter()
                .map(|sample| *sample as f64 * *sample as f64)
                .sum::<f64>()
                / samples.len() as f64)
                .sqrt()
        };
        assert!(rms(&hard) > rms(&soft) * 2.0);
        assert!(
            (spectral_centroid(&hard, 48_000.0) - spectral_centroid(&soft, 48_000.0)).abs() > 5.0
        );
    }
}

#[test]
fn continuous_parameters_are_lfo_destinations_and_categories_ignore_lfos() {
    unsafe {
        let engine = gooey_engine_new(48_000.0);
        gooey_engine_set_channel_instrument_type(engine, 0, INSTRUMENT_FM_PERCUSSION);
        gooey_engine_set_lfo_amount(engine, 0, 0.0);
        gooey_engine_set_lfo_offset(engine, 0, 0.2);
        gooey_engine_set_lfo_enabled(engine, 0, true);
        let categories = [
            FM_PERCUSSION_PARAM_OSC1_WAVEFORM,
            FM_PERCUSSION_PARAM_OSC1_TRACKING,
            FM_PERCUSSION_PARAM_OSC2_WAVEFORM,
            FM_PERCUSSION_PARAM_OSC2_TRACKING,
            FM_PERCUSSION_PARAM_RING_MODE,
            FM_PERCUSSION_PARAM_FILTER_MODE,
        ];
        let mut frame = [0.0_f32; 2];
        for param in 0..FM_PERCUSSION_PARAM_COUNT {
            gooey_engine_clear_lfo_routes(engine, 0);
            gooey_engine_set_channel_param(engine, 0, param, 0.0);
            assert_ne!(
                gooey_engine_add_lfo_route(engine, 0, 0, param, 1.0),
                LFO_INVALID
            );
            gooey_engine_render(engine, frame.as_mut_ptr(), 1);
            let actual = gooey_engine_get_channel_param(engine, 0, param);
            if categories.contains(&param) {
                assert_eq!(actual, 0.0, "categorical parameter {param}");
            } else {
                assert!((actual - 0.6).abs() < 1e-6, "continuous parameter {param}");
            }
        }
        gooey_engine_free(engine);
    }
}
