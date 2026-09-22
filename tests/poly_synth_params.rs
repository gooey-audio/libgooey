//! End-to-end coverage for the expressive poly-synth C surface.

use std::ptr;

use gooey::ffi::*;

const SR: f32 = 44_100.0;

fn approx_eq(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 1e-6,
        "expected {expected}, got {actual}"
    );
}

fn render(engine: *mut GooeyEngine, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0; frames * 2];
    unsafe { gooey_engine_render(engine, output.as_mut_ptr(), frames as u32) };
    output
}

fn samples_per_step(bpm: f32) -> usize {
    ((60.0 / bpm) / 4.0 * SR) as usize
}

unsafe fn gate_config(engine: *const GooeyEngine, preset: u32) -> GooeyPolyGateConfig {
    let mut config = GooeyPolyGateConfig {
        pattern: u32::MAX,
        depth: f32::NAN,
        smoothing: f32::NAN,
    };
    assert!(gooey_engine_poly_get_preset_gate(
        engine,
        preset,
        &mut config
    ));
    config
}

unsafe fn active_gate_config(engine: *const GooeyEngine) -> GooeyPolyGateConfig {
    let mut config = GooeyPolyGateConfig {
        pattern: u32::MAX,
        depth: f32::NAN,
        smoothing: f32::NAN,
    };
    assert!(gooey_engine_poly_get_gate(engine, &mut config));
    config
}

fn frame_energy(samples: &[f32], start: usize, end: usize) -> f32 {
    samples
        .chunks_exact(2)
        .skip(start)
        .take(end - start)
        .map(|frame| frame[0] * frame[0] + frame[1] * frame[1])
        .sum()
}

#[test]
fn third_inversion_and_drop2_have_stable_ids_and_render_through_ffi() {
    assert_eq!(VOICING_THIRD_INVERSION, 3);
    assert_eq!(VOICING_DROP2, 5);

    unsafe {
        for voicing in [VOICING_THIRD_INVERSION, VOICING_DROP2] {
            let engine = gooey_engine_new(SR);
            gooey_engine_poly_trigger_chord(
                engine,
                0, // C
                SCALE_MAJOR,
                0,
                voicing,
                POLY_PRESET_DEFAULT,
                4,
                0.8,
            );
            let peak = render(engine, 1024)
                .into_iter()
                .map(f32::abs)
                .fold(0.0_f32, f32::max);
            assert!(peak > 0.001, "voicing id {voicing} should render audio");
            gooey_engine_free(engine);
        }
    }
}

#[test]
fn all_thirty_active_parameters_round_trip_and_survive_retrigger() {
    unsafe {
        let engine = gooey_engine_new(SR);
        assert_eq!(POLY_PARAM_COUNT, 30);
        assert_eq!(gooey_engine_poly_get_preset(engine), POLY_PRESET_DEFAULT);

        for param in 0..POLY_PARAM_COUNT {
            let value = (param + 1) as f32 / (POLY_PARAM_COUNT + 1) as f32;
            assert!(gooey_engine_poly_set_param(engine, param, value));
            approx_eq(gooey_engine_poly_get_param(engine, param), value);
            approx_eq(
                gooey_engine_poly_get_preset_param(engine, POLY_PRESET_DEFAULT, param),
                value,
            );
        }

        gooey_engine_poly_trigger_chord(
            engine,
            0,
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            POLY_PRESET_DEFAULT,
            4,
            0.8,
        );
        for param in 0..POLY_PARAM_COUNT {
            let expected = (param + 1) as f32 / (POLY_PARAM_COUNT + 1) as f32;
            approx_eq(gooey_engine_poly_get_param(engine, param), expected);
        }
        gooey_engine_free(engine);
    }
}

#[test]
fn editable_presets_are_isolated_selectable_and_resettable() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let factory_pad =
            gooey_engine_poly_get_preset_param(engine, POLY_PRESET_PAD, POLY_PARAM_STEREO_WIDTH);
        assert!(gooey_engine_poly_set_preset_param(
            engine,
            POLY_PRESET_PAD,
            POLY_PARAM_STEREO_WIDTH,
            0.13,
        ));
        assert!(gooey_engine_poly_set_preset_param(
            engine,
            POLY_PRESET_DEFAULT,
            POLY_PARAM_STEREO_WIDTH,
            0.77,
        ));
        approx_eq(
            gooey_engine_poly_get_param(engine, POLY_PARAM_STEREO_WIDTH),
            0.77,
        );

        assert!(gooey_engine_poly_set_preset(engine, POLY_PRESET_PAD));
        approx_eq(
            gooey_engine_poly_get_param(engine, POLY_PARAM_STEREO_WIDTH),
            0.13,
        );
        assert!(gooey_engine_poly_reset_preset(engine, POLY_PRESET_PAD));
        approx_eq(
            gooey_engine_poly_get_param(engine, POLY_PARAM_STEREO_WIDTH),
            factory_pad,
        );
        assert!(!gooey_engine_poly_set_preset(engine, POLY_PRESET_COUNT));
        assert_eq!(gooey_engine_poly_get_preset(engine), POLY_PRESET_PAD);
        gooey_engine_free(engine);
    }
}

#[test]
fn gate_configs_are_isolated_selectable_clamped_and_resettable() {
    assert_eq!(POLY_GATE_PATTERN_OFF, 0);
    assert_eq!(POLY_GATE_PATTERN_STRAIGHT_EIGHTHS, 1);
    assert_eq!(POLY_GATE_PATTERN_OFFBEAT_EIGHTHS, 2);
    assert_eq!(POLY_GATE_PATTERN_CHOPPER, 3);
    assert_eq!(POLY_GATE_PATTERN_TRANCE, 4);
    assert_eq!(POLY_GATE_PATTERN_SYNCOPATED, 5);
    assert_eq!(POLY_GATE_PATTERN_COUNT, 6);

    unsafe {
        let engine = gooey_engine_new(SR);
        let factory = gate_config(engine, POLY_PRESET_PAD);
        assert_eq!(factory.pattern, POLY_GATE_PATTERN_OFF);
        approx_eq(factory.depth, 1.0);
        approx_eq(factory.smoothing, 0.1);

        assert!(gooey_engine_poly_set_preset_gate(
            engine,
            POLY_PRESET_PAD,
            GooeyPolyGateConfig {
                pattern: POLY_GATE_PATTERN_TRANCE,
                depth: 2.0,
                smoothing: -1.0,
            },
        ));
        let pad = gate_config(engine, POLY_PRESET_PAD);
        assert_eq!(pad.pattern, POLY_GATE_PATTERN_TRANCE);
        approx_eq(pad.depth, 1.0);
        approx_eq(pad.smoothing, 0.0);
        assert_eq!(
            gate_config(engine, POLY_PRESET_DEFAULT).pattern,
            POLY_GATE_PATTERN_OFF
        );

        assert!(gooey_engine_poly_set_preset(engine, POLY_PRESET_PAD));
        let mut active = GooeyPolyGateConfig {
            pattern: 0,
            depth: 0.0,
            smoothing: 0.0,
        };
        assert!(gooey_engine_poly_get_gate(engine, &mut active));
        assert_eq!(active.pattern, POLY_GATE_PATTERN_TRANCE);

        assert!(gooey_engine_poly_set_gate(
            engine,
            GooeyPolyGateConfig {
                pattern: POLY_GATE_PATTERN_SYNCOPATED,
                depth: 0.6,
                smoothing: 0.4,
            },
        ));
        assert!(gooey_engine_poly_get_gate(engine, &mut active));
        assert_eq!(active.pattern, POLY_GATE_PATTERN_SYNCOPATED);
        approx_eq(active.depth, 0.6);
        approx_eq(active.smoothing, 0.4);

        assert!(gooey_engine_poly_reset_preset(engine, POLY_PRESET_PAD));
        let reset = gate_config(engine, POLY_PRESET_PAD);
        assert_eq!(reset.pattern, POLY_GATE_PATTERN_OFF);
        approx_eq(reset.depth, 1.0);
        approx_eq(reset.smoothing, 0.1);
        gooey_engine_free(engine);
    }
}

#[test]
fn performance_replay_uses_the_engines_edited_preset_copy() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let bpm = 120.0;
        gooey_engine_set_bpm(engine, bpm);
        gooey_engine_perf_set_record_mode(engine, PERF_RECORD_MODE_PUNCH_OUT);
        gooey_engine_perf_set_record_armed(engine, true);
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 64);

        gooey_engine_poly_trigger_chord(
            engine,
            0,
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            POLY_PRESET_PAD,
            4,
            0.8,
        );
        let _ = render(engine, samples_per_step(bpm));
        gooey_engine_poly_release(engine);
        assert_eq!(gooey_engine_perf_get_event_count(engine), 1);

        // Edit the recorded preset after capture, select another sound, and
        // then cross the loop boundary. Playback must resolve the stable
        // preset id through this engine's editable preset bank.
        assert!(gooey_engine_poly_set_preset_param(
            engine,
            POLY_PRESET_PAD,
            POLY_PARAM_STEREO_WIDTH,
            0.19,
        ));
        assert!(gooey_engine_poly_set_preset_gate(
            engine,
            POLY_PRESET_PAD,
            GooeyPolyGateConfig {
                pattern: POLY_GATE_PATTERN_CHOPPER,
                depth: 0.71,
                smoothing: 0.23,
            },
        ));
        assert!(gooey_engine_poly_set_preset(engine, POLY_PRESET_DEFAULT));
        let _ = render(engine, samples_per_step(bpm) * 16 + 512);

        assert_eq!(gooey_engine_poly_get_preset(engine), POLY_PRESET_PAD);
        approx_eq(
            gooey_engine_poly_get_param(engine, POLY_PARAM_STEREO_WIDTH),
            0.19,
        );
        let replayed_gate = active_gate_config(engine);
        assert_eq!(replayed_gate.pattern, POLY_GATE_PATTERN_CHOPPER);
        approx_eq(replayed_gate.depth, 0.71);
        approx_eq(replayed_gate.smoothing, 0.23);
        gooey_engine_free(engine);
    }
}

#[test]
fn all_eight_modulation_routes_round_trip_and_clear() {
    unsafe {
        let engine = gooey_engine_new(SR);
        assert_eq!(POLY_MOD_ROUTE_COUNT, 8);
        for slot in 0..POLY_MOD_ROUTE_COUNT {
            let expected = GooeyPolyModRoute {
                enabled: slot % 2 == 0,
                source: if slot % 2 == 0 {
                    POLY_MOD_SOURCE_VELOCITY
                } else {
                    POLY_MOD_SOURCE_KEY_POSITION
                },
                destination: slot % POLY_PARAM_COUNT,
                depth: -0.7 + slot as f32 * 0.2,
                curve: slot as f32 / (POLY_MOD_ROUTE_COUNT - 1) as f32,
                key_scale: 0.4 - slot as f32 * 0.1,
            };
            assert!(gooey_engine_poly_set_mod_route(
                engine,
                POLY_PRESET_KEYS,
                slot,
                expected,
            ));
            let mut actual = GooeyPolyModRoute {
                enabled: false,
                source: 0,
                destination: 0,
                depth: 0.0,
                curve: 0.0,
                key_scale: 0.0,
            };
            assert!(gooey_engine_poly_get_mod_route(
                engine,
                POLY_PRESET_KEYS,
                slot,
                &mut actual,
            ));
            assert_eq!(actual.enabled, expected.enabled);
            assert_eq!(actual.source, expected.source);
            assert_eq!(actual.destination, expected.destination);
            approx_eq(actual.depth, expected.depth);
            approx_eq(actual.curve, expected.curve);
            approx_eq(actual.key_scale, expected.key_scale);
        }

        assert!(gooey_engine_poly_clear_mod_route(
            engine,
            POLY_PRESET_KEYS,
            3,
        ));
        let mut cleared = GooeyPolyModRoute {
            enabled: true,
            source: 99,
            destination: 99,
            depth: 1.0,
            curve: 1.0,
            key_scale: 1.0,
        };
        assert!(gooey_engine_poly_get_mod_route(
            engine,
            POLY_PRESET_KEYS,
            3,
            &mut cleared,
        ));
        assert!(!cleared.enabled);
        approx_eq(cleared.depth, 0.0);
        gooey_engine_free(engine);
    }
}

#[test]
fn invalid_poly_inputs_leave_state_unchanged() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let before = gooey_engine_poly_get_param(engine, POLY_PARAM_VOLUME);
        assert!(!gooey_engine_poly_set_param(
            engine,
            POLY_PARAM_VOLUME,
            f32::NAN,
        ));
        assert!(!gooey_engine_poly_set_param(engine, POLY_PARAM_COUNT, 0.5));
        approx_eq(
            gooey_engine_poly_get_param(engine, POLY_PARAM_VOLUME),
            before,
        );
        assert!(gooey_engine_poly_get_param(engine, POLY_PARAM_COUNT).is_nan());

        let invalid_source = GooeyPolyModRoute {
            enabled: true,
            source: 99,
            destination: POLY_PARAM_VOLUME,
            depth: 0.5,
            curve: 0.5,
            key_scale: 0.0,
        };
        assert!(!gooey_engine_poly_set_mod_route(
            engine,
            POLY_PRESET_DEFAULT,
            0,
            invalid_source,
        ));
        assert!(!gooey_engine_poly_get_mod_route(
            engine,
            POLY_PRESET_DEFAULT,
            0,
            ptr::null_mut(),
        ));
        assert!(!gooey_engine_poly_set_param(
            ptr::null_mut(),
            POLY_PARAM_VOLUME,
            0.5,
        ));
        assert!(gooey_engine_poly_get_param(ptr::null(), POLY_PARAM_VOLUME).is_nan());

        let before_gate = gate_config(engine, POLY_PRESET_DEFAULT);
        for invalid in [
            GooeyPolyGateConfig {
                pattern: POLY_GATE_PATTERN_COUNT,
                depth: 0.5,
                smoothing: 0.5,
            },
            GooeyPolyGateConfig {
                pattern: POLY_GATE_PATTERN_TRANCE,
                depth: f32::NAN,
                smoothing: 0.5,
            },
            GooeyPolyGateConfig {
                pattern: POLY_GATE_PATTERN_TRANCE,
                depth: 0.5,
                smoothing: f32::INFINITY,
            },
        ] {
            assert!(!gooey_engine_poly_set_gate(engine, invalid));
        }
        let after_gate = gate_config(engine, POLY_PRESET_DEFAULT);
        assert_eq!(after_gate.pattern, before_gate.pattern);
        approx_eq(after_gate.depth, before_gate.depth);
        approx_eq(after_gate.smoothing, before_gate.smoothing);
        assert!(!gooey_engine_poly_set_gate(
            ptr::null_mut(),
            GooeyPolyGateConfig {
                pattern: POLY_GATE_PATTERN_TRANCE,
                depth: 1.0,
                smoothing: 0.0,
            },
        ));
        assert!(!gooey_engine_poly_get_gate(engine, ptr::null_mut()));
        assert!(!gooey_engine_poly_get_preset_gate(
            engine,
            POLY_PRESET_COUNT,
            ptr::null_mut()
        ));
        gooey_engine_free(engine);
    }
}

#[test]
fn gate_bypasses_when_stopped_and_chops_transport_synced_steps() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let bpm = 120.0;
        gooey_engine_set_bpm(engine, bpm);
        assert!(gooey_engine_poly_set_gate(
            engine,
            GooeyPolyGateConfig {
                // Beat zero is closed in the offbeat pattern.
                pattern: POLY_GATE_PATTERN_OFFBEAT_EIGHTHS,
                depth: 1.0,
                smoothing: 0.0,
            },
        ));
        gooey_engine_poly_trigger_chord(
            engine,
            0,
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            POLY_PRESET_DEFAULT,
            4,
            1.0,
        );
        let stopped = render(engine, 2048);
        let stopped_energy = frame_energy(&stopped, 512, 2048);
        assert!(
            stopped_energy > 1e-5,
            "stopped gate must bypass: {stopped_energy}"
        );

        gooey_engine_sequencer_set_beat_position(engine, 0.0);
        gooey_engine_sequencer_start(engine);
        let running_closed = render(engine, 2048);
        let running_energy = frame_energy(&running_closed, 0, 2048);
        assert!(
            running_energy < stopped_energy * 1e-4,
            "closed transport step should mute: stopped={stopped_energy} running={running_energy}"
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn gate_open_and_closed_steps_follow_the_transport_grid() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let bpm = 120.0;
        let step = samples_per_step(bpm);
        gooey_engine_set_bpm(engine, bpm);
        assert!(gooey_engine_poly_set_gate(
            engine,
            GooeyPolyGateConfig {
                pattern: POLY_GATE_PATTERN_STRAIGHT_EIGHTHS,
                depth: 1.0,
                smoothing: 0.0,
            },
        ));
        gooey_engine_poly_trigger_chord(
            engine,
            0,
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            POLY_PRESET_DEFAULT,
            4,
            1.0,
        );
        gooey_engine_sequencer_start(engine);
        let samples = render(engine, step * 2 + 32);
        let open = frame_energy(&samples, step / 2, step);
        let closed = frame_energy(&samples, step + 16, step * 2);
        assert!(open > 1e-5, "open step should pass the synth: {open}");
        assert!(
            closed < open * 1e-4,
            "closed step should be much quieter: open={open} closed={closed}"
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn ffi_graph_preserves_the_poly_synth_native_stereo_image() {
    unsafe {
        let engine = gooey_engine_new(SR);
        for (param, value) in [
            (POLY_PARAM_OSC_A_WAVEFORM, 0.0),
            (POLY_PARAM_OSC_B_WAVEFORM, 1.0),
            (POLY_PARAM_OSC_A_LEVEL, 1.0),
            (POLY_PARAM_OSC_B_LEVEL, 1.0),
            (POLY_PARAM_DETUNE, 1.0),
            (POLY_PARAM_STEREO_WIDTH, 1.0),
            (POLY_PARAM_AMP_ATTACK, 0.0),
            (POLY_PARAM_AMP_SUSTAIN, 1.0),
            (POLY_PARAM_FILTER_CUTOFF, 1.0),
            (POLY_PARAM_FILTER_ENV_AMOUNT, 0.5),
            (POLY_PARAM_SATURATION, 0.0),
        ] {
            assert!(gooey_engine_poly_set_param(engine, param, value));
        }
        gooey_engine_poly_trigger_chord(
            engine,
            0,
            SCALE_MAJOR,
            0,
            VOICING_ROOT_POSITION,
            POLY_PRESET_DEFAULT,
            4,
            1.0,
        );
        let samples = render(engine, 4096);
        let mut energy = 0.0;
        let mut side_energy = 0.0;
        for frame in samples.chunks_exact(2) {
            energy += frame[0] * frame[0] + frame[1] * frame[1];
            let side = frame[0] - frame[1];
            side_energy += side * side;
        }
        assert!(energy > 0.001, "poly source should be audible: {energy}");
        assert!(
            side_energy > 0.001,
            "width and independent waveforms should reach the graph: {side_energy}"
        );
        gooey_engine_free(engine);
    }
}
