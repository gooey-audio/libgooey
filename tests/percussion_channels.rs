//! Channel integration for the two factory percussion architectures.

use gooey::ffi::*;
use gooey::instruments::{TwinCorePercConfig, TwinCorePercVoice};

const SR: f32 = 44_100.0;
const CHANNEL: u32 = 1;

struct Engine(*mut GooeyEngine);

impl Engine {
    fn new(kind: u32) -> Self {
        let engine = gooey_engine_new(SR);
        unsafe { gooey_engine_set_channel_instrument_type(engine, CHANNEL, kind) };
        assert_eq!(
            unsafe { gooey_engine_get_channel_instrument_type(engine, CHANNEL) },
            kind
        );
        Self(engine)
    }

    fn param(&self, index: u32) -> f32 {
        unsafe { gooey_engine_get_channel_param(self.0, CHANNEL, index) }
    }

    fn render(&self, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        unsafe { gooey_engine_render(self.0, out.as_mut_ptr(), frames as u32) };
        assert!(out.iter().all(|sample| sample.is_finite()));
        out
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { gooey_engine_free(self.0) };
    }
}

fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|x| x * x).sum()
}

fn approx(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
}

#[test]
fn selectable_types_keep_five_addressable_channels() {
    assert_eq!(gooey_engine_instrument_count(), 7);
    assert_eq!(gooey_engine_channel_count(), 5);
    assert_eq!(gooey_engine_resonator_param_count(), 9);
    assert_eq!(gooey_engine_twin_core_param_count(), 14);
    let legacy = gooey_engine_new(SR);
    unsafe {
        assert!(!gooey_engine_set_channel_preset(
            legacy,
            CHANNEL,
            PERC_PRESET_SNARE
        ));
        assert_eq!(gooey_engine_get_channel_preset(legacy, CHANNEL), u32::MAX);
        gooey_engine_set_channel_tuning(legacy, CHANNEL, 0.8);
        gooey_engine_blend_enable(legacy, CHANNEL);
        gooey_engine_set_channel_instrument_type(legacy, CHANNEL, INSTRUMENT_RESONATOR);
        assert!(!gooey_engine_blend_is_enabled(legacy, CHANNEL));
        approx(gooey_engine_get_channel_tuning(legacy, CHANNEL), 0.8);
        gooey_engine_free(legacy);
    }
    for kind in [INSTRUMENT_RESONATOR, INSTRUMENT_TWIN_CORE] {
        let engine = Engine::new(kind);
        assert_eq!(
            unsafe { gooey_engine_get_channel_preset(engine.0, CHANNEL) },
            PERC_PRESET_KICK
        );
        unsafe { gooey_engine_set_channel_instrument_type(engine.0, 4, kind) };
        assert_eq!(
            unsafe { gooey_engine_get_channel_instrument_type(engine.0, 4) },
            kind
        );
        unsafe { gooey_engine_set_channel_instrument_type(engine.0, 5, kind) };
        assert_eq!(
            unsafe { gooey_engine_get_channel_instrument_type(engine.0, 5) },
            u32::MAX
        );
    }
}

#[test]
fn sequencer_triggers_both_types_with_velocity() {
    for kind in [INSTRUMENT_RESONATOR, INSTRUMENT_TWIN_CORE] {
        let render_hit = |velocity| {
            let engine = Engine::new(kind);
            unsafe {
                gooey_engine_sequencer_set_instrument_step_with_velocity(
                    engine.0, CHANNEL, 0, true, velocity,
                );
                gooey_engine_sequencer_start(engine.0);
            }
            engine.render(8192)
        };
        let loud = energy(&render_hit(1.0));
        let soft = energy(&render_hit(0.25));
        assert!(loud > 1e-6, "type {kind} was silent");
        assert!(
            soft > 0.0 && soft < loud,
            "type {kind}: soft {soft}, loud {loud}"
        );
    }
}

#[test]
fn channel_parameters_and_discrete_modes_round_trip() {
    for (kind, count) in [(INSTRUMENT_RESONATOR, 9), (INSTRUMENT_TWIN_CORE, 12)] {
        let engine = Engine::new(kind);
        for index in 0..count {
            unsafe { gooey_engine_set_channel_param(engine.0, CHANNEL, index, 0.37) };
            approx(engine.param(index), 0.37);
        }
        assert!(engine
            .param(if kind == INSTRUMENT_RESONATOR { 9 } else { 14 })
            .is_nan());
    }
    let engine = Engine::new(INSTRUMENT_TWIN_CORE);
    unsafe {
        gooey_engine_set_channel_param(engine.0, CHANNEL, TWIN_CORE_PARAM_BODY_MODE, 1.6);
        gooey_engine_set_channel_param(engine.0, CHANNEL, TWIN_CORE_PARAM_NOISE_MODE, 0.6);
    }
    approx(engine.param(TWIN_CORE_PARAM_BODY_MODE), 2.0);
    approx(engine.param(TWIN_CORE_PARAM_NOISE_MODE), 1.0);
    unsafe {
        gooey_engine_set_channel_param_lock(engine.0, CHANNEL, TWIN_CORE_PARAM_BODY_MODE, 0.1);
        gooey_engine_set_channel_param_lock(engine.0, CHANNEL, TWIN_CORE_PARAM_NOISE_MODE, 2.8);
    }
    approx(engine.param(TWIN_CORE_PARAM_BODY_MODE), 0.0);
    approx(engine.param(TWIN_CORE_PARAM_NOISE_MODE), 2.0);
}

#[test]
fn lfo_routes_can_select_twin_core_modes_for_the_next_hit() {
    let engine = Engine::new(INSTRUMENT_TWIN_CORE);
    unsafe {
        gooey_engine_set_lfo_amount(engine.0, 0, 0.0);
        gooey_engine_set_lfo_offset(engine.0, 0, 1.0);
        assert_ne!(
            gooey_engine_add_lfo_route(engine.0, 0, CHANNEL, TWIN_CORE_PARAM_BODY_MODE, 1.0),
            LFO_INVALID
        );
        assert_ne!(
            gooey_engine_add_lfo_route(engine.0, 0, CHANNEL, TWIN_CORE_PARAM_NOISE_MODE, 1.0),
            LFO_INVALID
        );
        gooey_engine_set_lfo_enabled(engine.0, 0, true);
    }
    engine.render(64);
    approx(engine.param(TWIN_CORE_PARAM_BODY_MODE), 2.0);
    approx(engine.param(TWIN_CORE_PARAM_NOISE_MODE), 2.0);
    unsafe { gooey_engine_trigger_channel(engine.0, CHANNEL) };
    assert!(energy(&engine.render(8192)) > 1e-6);
}

#[test]
fn locks_lfos_presets_and_blends_follow_channel_rules() {
    for (kind, param) in [
        (INSTRUMENT_RESONATOR, RESONATOR_PARAM_NOISE),
        (INSTRUMENT_TWIN_CORE, TWIN_CORE_PARAM_HARMONICS),
    ] {
        let engine = Engine::new(kind);
        unsafe {
            gooey_engine_set_channel_param(engine.0, CHANNEL, param, 0.3);
            gooey_engine_set_channel_param_lock(engine.0, CHANNEL, param, 0.1);
            gooey_engine_blend_enable(engine.0, CHANNEL);
            gooey_engine_blend_set_position(engine.0, CHANNEL, 1.0, 1.0);
            gooey_engine_sequencer_set_instrument_step_with_velocity(
                engine.0, CHANNEL, 0, true, 1.0,
            );
            gooey_engine_sequencer_set_instrument_step_blend(engine.0, CHANNEL, 0, 0.0, 1.0);
            gooey_engine_sequencer_start(engine.0);
        }
        assert!(!unsafe { gooey_engine_blend_is_enabled(engine.0, CHANNEL) });
        engine.render(1024);
        approx(engine.param(param), 0.1);
        assert!(unsafe { gooey_engine_channel_param_is_locked(engine.0, CHANNEL, param) });

        unsafe {
            gooey_engine_set_lfo_amount(engine.0, 0, 0.0);
            gooey_engine_set_lfo_offset(engine.0, 0, 0.8);
            assert_ne!(
                gooey_engine_add_lfo_route(engine.0, 0, CHANNEL, param, 1.0),
                LFO_INVALID
            );
            gooey_engine_set_lfo_enabled(engine.0, 0, true);
        }
        engine.render(64);
        approx(engine.param(param), 0.1); // the getter exposes the locked base
        unsafe { gooey_engine_clear_channel_param_lock(engine.0, CHANNEL, param) };
        approx(engine.param(param), 0.1); // the getter exposes the LFO center
                                          // The LFO swings the live target around that center.
        approx(
            unsafe { gooey_engine_get_channel_param_modulated(engine.0, CHANNEL, param) },
            0.5,
        );

        unsafe {
            gooey_engine_set_channel_param_lock(engine.0, CHANNEL, param, 0.2);
            gooey_engine_set_channel_tuning(engine.0, CHANNEL, 0.73);
            gooey_engine_set_instrument_gain(engine.0, CHANNEL, 0.42);
            gooey_engine_set_instrument_mute(engine.0, CHANNEL, true);
            gooey_engine_set_instrument_solo(engine.0, CHANNEL, true);
            assert!(gooey_engine_set_channel_preset(
                engine.0,
                CHANNEL,
                PERC_PRESET_SNARE
            ));
        }
        assert_eq!(
            unsafe { gooey_engine_get_channel_preset(engine.0, CHANNEL) },
            PERC_PRESET_SNARE
        );
        assert!(!unsafe { gooey_engine_channel_param_is_locked(engine.0, CHANNEL, param) });
        approx(
            unsafe { gooey_engine_get_channel_tuning(engine.0, CHANNEL) },
            0.73,
        );
        approx(
            unsafe { gooey_engine_get_instrument_gain(engine.0, CHANNEL) },
            0.42,
        );
        assert!(unsafe { gooey_engine_get_instrument_mute(engine.0, CHANNEL) });
        assert!(unsafe { gooey_engine_get_instrument_solo(engine.0, CHANNEL) });
        assert!(unsafe {
            gooey_engine_sequencer_get_instrument_step_enabled(engine.0, CHANNEL, 0)
        });
        assert!(!unsafe { gooey_engine_set_channel_preset(engine.0, CHANNEL, 99) });
    }
}

#[test]
fn tuning_changes_audio_without_changing_pitch_control() {
    for (kind, pitch_param) in [
        (INSTRUMENT_RESONATOR, RESONATOR_PARAM_PITCH),
        (INSTRUMENT_TWIN_CORE, TWIN_CORE_PARAM_TUNE),
    ] {
        let hit = |tuning| {
            let engine = Engine::new(kind);
            let base = engine.param(pitch_param);
            unsafe {
                gooey_engine_set_channel_tuning(engine.0, CHANNEL, tuning);
                gooey_engine_trigger_channel_with_velocity(engine.0, CHANNEL, 1.0);
            }
            approx(engine.param(pitch_param), base);
            approx(
                unsafe { gooey_engine_get_channel_tuning(engine.0, CHANNEL) },
                tuning,
            );
            engine.render(8192)
        };
        let low = hit(0.0);
        let neutral = hit(0.5);
        let high = hit(1.0);
        assert_ne!(low, neutral);
        assert_ne!(high, neutral);
    }
}

#[test]
fn mixer_gain_mute_solo_and_peaks_reach_new_voices() {
    for kind in [INSTRUMENT_RESONATOR, INSTRUMENT_TWIN_CORE] {
        let engine = Engine::new(kind);
        unsafe { gooey_engine_trigger_channel(engine.0, CHANNEL) };
        assert!(energy(&engine.render(8192)) > 1e-6);
        let mut peaks = [0.0; 5];
        unsafe { gooey_engine_get_channel_peaks(engine.0, peaks.as_mut_ptr(), 5) };
        assert!(peaks[CHANNEL as usize] > 0.0);
        unsafe { gooey_engine_set_instrument_gain(engine.0, CHANNEL, 0.0) };
        engine.render(4096); // let the gain smoother settle
        unsafe { gooey_engine_trigger_channel(engine.0, CHANNEL) };
        assert!(energy(&engine.render(8192)) < 1e-9);
        unsafe {
            gooey_engine_set_instrument_gain(engine.0, CHANNEL, 1.0);
            gooey_engine_set_instrument_mute(engine.0, CHANNEL, true);
        }
        engine.render(4096);
        unsafe { gooey_engine_trigger_channel(engine.0, CHANNEL) };
        assert!(energy(&engine.render(8192)) < 1e-9);
        unsafe {
            gooey_engine_set_instrument_solo(engine.0, CHANNEL, true);
            gooey_engine_trigger_channel(engine.0, CHANNEL);
        }
        assert!(energy(&engine.render(8192)) > 1e-6); // solo overrides mute
    }
}

#[test]
fn twin_core_exponential_sliders_preserve_bounds_and_round_trip() {
    let mut voice = TwinCorePercVoice::with_config(SR, TwinCorePercConfig::kick());
    for (index, min, max) in [
        (0, 20.0, 2_000.0),
        (2, 0.002, 20.0),
        (4, 0.002, 8.0),
        (8, 20.0, SR * 0.45),
        (9, 0.002, 20.0),
    ] {
        for (normalized, expected) in [(0.0, min), (0.5, (min * max).sqrt()), (1.0, max)] {
            voice.set_parameter_normalized(index, normalized);
            let physical = match index {
                0 => voice.config_targets().master_tune_hz,
                2 => voice.config_targets().length_seconds,
                4 => voice.config_targets().fm_decay_seconds,
                8 => voice.config_targets().noise_filter_hz,
                _ => voice.config_targets().noise_decay_seconds,
            };
            assert!((physical - expected).abs() / expected < 1e-5);
            assert!((voice.parameter_normalized(index).unwrap() - normalized).abs() < 1e-5);
        }
    }
}
