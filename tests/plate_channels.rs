//! Channel integration for the modal plate voice.

use gooey::ffi::*;

const SR: f32 = 44_100.0;
const CHANNEL: u32 = 2;

struct Engine(*mut GooeyEngine);

impl Engine {
    fn new() -> Self {
        let engine = gooey_engine_new(SR);
        unsafe { gooey_engine_set_channel_instrument_type(engine, CHANNEL, INSTRUMENT_PLATE) };
        assert_eq!(
            unsafe { gooey_engine_get_channel_instrument_type(engine, CHANNEL) },
            INSTRUMENT_PLATE
        );
        Self(engine)
    }

    fn param(&self, index: u32) -> f32 {
        unsafe { gooey_engine_get_channel_param(self.0, CHANNEL, index) }
    }

    fn set_param(&self, index: u32, value: f32) {
        unsafe { gooey_engine_set_channel_param(self.0, CHANNEL, index, value) };
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

/// Sign changes on the left channel of an interleaved buffer.
fn zero_crossings(samples: &[f32]) -> usize {
    let left: Vec<f32> = samples.iter().step_by(2).copied().collect();
    left.windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count()
}

fn approx(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 1e-4, "{actual} != {expected}");
}

#[test]
fn plate_is_a_selectable_channel_type() {
    assert_eq!(gooey_engine_instrument_count(), 8);
    assert_eq!(gooey_engine_plate_voice_param_count(), 13);
    let engine = Engine::new();
    assert_eq!(
        unsafe { gooey_engine_get_channel_preset(engine.0, CHANNEL) },
        PLATE_VOICE_PRESET_RIDE
    );
    unsafe {
        gooey_engine_set_channel_instrument_type(engine.0, CHANNEL, INSTRUMENT_TWIN_CORE);
        assert_eq!(
            gooey_engine_get_channel_instrument_type(engine.0, CHANNEL),
            INSTRUMENT_TWIN_CORE
        );
        gooey_engine_set_channel_instrument_type(engine.0, CHANNEL, INSTRUMENT_PLATE);
        assert_eq!(
            gooey_engine_get_channel_instrument_type(engine.0, CHANNEL),
            INSTRUMENT_PLATE
        );
        // The bass strip can hold a plate too.
        gooey_engine_set_channel_instrument_type(engine.0, 4, INSTRUMENT_PLATE);
        assert_eq!(
            gooey_engine_get_channel_instrument_type(engine.0, 4),
            INSTRUMENT_PLATE
        );
    }
}

#[test]
fn sequencer_triggers_plate_with_velocity() {
    let render_hit = |velocity| {
        let engine = Engine::new();
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
    assert!(loud > 1e-6, "plate was silent");
    assert!(soft > 0.0 && soft < loud, "soft {soft}, loud {loud}");
}

#[test]
fn continuous_parameters_round_trip_normalized() {
    let engine = Engine::new();
    for index in 0..PLATE_VOICE_PARAM_MATERIAL {
        engine.set_param(index, 0.37);
        approx(engine.param(index), 0.37);
        engine.set_param(index, 2.0);
        approx(engine.param(index), 1.0);
        engine.set_param(index, f32::NAN);
        approx(engine.param(index), 1.0);
    }
    assert!(engine.param(13).is_nan());
}

#[test]
fn material_and_mode_count_use_raw_indices() {
    let engine = Engine::new();
    for material in PLATE_VOICE_MATERIAL_MEMBRANE..=PLATE_VOICE_MATERIAL_SHEET {
        engine.set_param(PLATE_VOICE_PARAM_MATERIAL, material as f32);
        approx(engine.param(PLATE_VOICE_PARAM_MATERIAL), material as f32);
    }
    engine.set_param(PLATE_VOICE_PARAM_MATERIAL, 2.6);
    approx(engine.param(PLATE_VOICE_PARAM_MATERIAL), 3.0);
    engine.set_param(PLATE_VOICE_PARAM_MATERIAL, 9.0);
    approx(engine.param(PLATE_VOICE_PARAM_MATERIAL), 4.0);

    for step in 0..8 {
        engine.set_param(PLATE_VOICE_PARAM_MODES, step as f32);
        approx(engine.param(PLATE_VOICE_PARAM_MODES), step as f32);
    }
    engine.set_param(PLATE_VOICE_PARAM_MODES, -3.0);
    approx(engine.param(PLATE_VOICE_PARAM_MODES), 0.0);
    engine.set_param(PLATE_VOICE_PARAM_MODES, 40.0);
    approx(engine.param(PLATE_VOICE_PARAM_MODES), 7.0);

    unsafe {
        gooey_engine_set_channel_param_lock(engine.0, CHANNEL, PLATE_VOICE_PARAM_MATERIAL, 0.4);
        gooey_engine_set_channel_param_lock(engine.0, CHANNEL, PLATE_VOICE_PARAM_MODES, 11.0);
    }
    approx(engine.param(PLATE_VOICE_PARAM_MATERIAL), 0.0);
    approx(engine.param(PLATE_VOICE_PARAM_MODES), 7.0);

    // Every material and the densest bank still render.
    for material in PLATE_VOICE_MATERIAL_MEMBRANE..=PLATE_VOICE_MATERIAL_SHEET {
        unsafe {
            gooey_engine_set_channel_param_lock(
                engine.0,
                CHANNEL,
                PLATE_VOICE_PARAM_MATERIAL,
                material as f32,
            );
            gooey_engine_trigger_channel(engine.0, CHANNEL);
        }
        assert!(energy(&engine.render(4096)) > 1e-6, "material {material}");
    }
}

#[test]
fn every_preset_loads_and_clears_locks_but_keeps_the_strip() {
    let engine = Engine::new();
    let param = PLATE_VOICE_PARAM_CASCADE;
    unsafe {
        gooey_engine_set_channel_tuning(engine.0, CHANNEL, 0.73);
        gooey_engine_set_instrument_gain(engine.0, CHANNEL, 0.42);
        gooey_engine_set_instrument_mute(engine.0, CHANNEL, true);
        gooey_engine_set_instrument_solo(engine.0, CHANNEL, true);
        gooey_engine_sequencer_set_instrument_step_with_velocity(engine.0, CHANNEL, 3, true, 1.0);
    }
    let mut pitches = Vec::new();
    for preset in PLATE_VOICE_PRESET_RIDE..=PLATE_VOICE_PRESET_GAMELAN {
        unsafe {
            gooey_engine_set_channel_param_lock(engine.0, CHANNEL, param, 0.05);
            assert!(gooey_engine_set_channel_preset(engine.0, CHANNEL, preset));
            assert_eq!(gooey_engine_get_channel_preset(engine.0, CHANNEL), preset);
            assert!(!gooey_engine_channel_param_is_locked(
                engine.0, CHANNEL, param
            ));
            approx(gooey_engine_get_channel_tuning(engine.0, CHANNEL), 0.73);
            approx(gooey_engine_get_instrument_gain(engine.0, CHANNEL), 0.42);
            assert!(gooey_engine_get_instrument_mute(engine.0, CHANNEL));
            assert!(gooey_engine_get_instrument_solo(engine.0, CHANNEL));
            assert!(gooey_engine_sequencer_get_instrument_step_enabled(
                engine.0, CHANNEL, 3
            ));
        }
        pitches.push(engine.param(PLATE_VOICE_PARAM_PITCH));
    }
    pitches.dedup();
    assert!(pitches.len() > 1, "presets did not change the voice");
    unsafe {
        assert!(!gooey_engine_set_channel_preset(engine.0, CHANNEL, 8));
        assert_eq!(
            gooey_engine_get_channel_preset(engine.0, CHANNEL),
            PLATE_VOICE_PRESET_GAMELAN
        );
    }
}

#[test]
fn blends_are_disabled_and_ignored() {
    let engine = Engine::new();
    engine.set_param(PLATE_VOICE_PARAM_TENSION, 0.3);
    unsafe {
        gooey_engine_blend_enable(engine.0, CHANNEL);
        gooey_engine_blend_set_position(engine.0, CHANNEL, 1.0, 1.0);
        gooey_engine_sequencer_set_instrument_step_with_velocity(engine.0, CHANNEL, 0, true, 1.0);
        gooey_engine_sequencer_set_instrument_step_blend(engine.0, CHANNEL, 0, 0.0, 1.0);
        gooey_engine_sequencer_start(engine.0);
        assert!(!gooey_engine_blend_is_enabled(engine.0, CHANNEL));
    }
    engine.render(1024);
    approx(engine.param(PLATE_VOICE_PARAM_TENSION), 0.3);

    // Switching to plate turns an enabled blend off.
    let engine = gooey_engine_new(SR);
    unsafe {
        gooey_engine_blend_enable(engine, CHANNEL);
        gooey_engine_set_channel_instrument_type(engine, CHANNEL, INSTRUMENT_PLATE);
        assert!(!gooey_engine_blend_is_enabled(engine, CHANNEL));
        gooey_engine_free(engine);
    }
}

#[test]
fn tuning_shifts_pitch_without_changing_the_pitch_control() {
    let hit = |tuning| {
        let engine = Engine::new();
        // A dark, soft, sparse membrane so the fundamental sets the crossings.
        for (param, value) in [
            (
                PLATE_VOICE_PARAM_MATERIAL,
                PLATE_VOICE_MATERIAL_MEMBRANE as f32,
            ),
            (PLATE_VOICE_PARAM_MODES, 0.0),
            (PLATE_VOICE_PARAM_PITCH, 0.3),
            (PLATE_VOICE_PARAM_HARDNESS, 0.0),
            (PLATE_VOICE_PARAM_TENSION, 0.0),
            (PLATE_VOICE_PARAM_CASCADE, 0.0),
            (PLATE_VOICE_PARAM_BRIGHTNESS, 0.0),
        ] {
            engine.set_param(param, value);
        }
        let base = engine.param(PLATE_VOICE_PARAM_PITCH);
        unsafe {
            gooey_engine_set_channel_tuning(engine.0, CHANNEL, tuning);
            gooey_engine_trigger_channel_with_velocity(engine.0, CHANNEL, 1.0);
        }
        approx(engine.param(PLATE_VOICE_PARAM_PITCH), base);
        zero_crossings(&engine.render(16_384))
    };
    let low = hit(0.0);
    let neutral = hit(0.5);
    let high = hit(1.0);
    assert!(
        low < neutral && 2 * high > 3 * neutral,
        "{low} {neutral} {high}"
    );
}

#[test]
fn lfo_swings_pickup_position_around_its_center() {
    let engine = Engine::new();
    let param = PLATE_VOICE_PARAM_PICKUP_POS;
    engine.set_param(param, 0.2);
    unsafe {
        gooey_engine_set_lfo_amount(engine.0, 0, 0.0);
        gooey_engine_set_lfo_offset(engine.0, 0, 0.6);
        assert_ne!(
            gooey_engine_add_lfo_route(engine.0, 0, CHANNEL, param, 1.0),
            LFO_INVALID
        );
        gooey_engine_set_lfo_enabled(engine.0, 0, true);
        gooey_engine_trigger_channel(engine.0, CHANNEL);
    }
    engine.render(64);
    approx(engine.param(param), 0.2);
    approx(
        unsafe { gooey_engine_get_channel_param_modulated(engine.0, CHANNEL, param) },
        0.5,
    );
}

#[test]
fn lfo_routes_select_material_and_mode_count_for_the_next_hit() {
    let engine = Engine::new();
    engine.set_param(PLATE_VOICE_PARAM_MATERIAL, 0.0);
    engine.set_param(PLATE_VOICE_PARAM_MODES, 0.0);
    unsafe {
        gooey_engine_set_lfo_amount(engine.0, 0, 0.0);
        gooey_engine_set_lfo_offset(engine.0, 0, 1.0);
        for param in [PLATE_VOICE_PARAM_MATERIAL, PLATE_VOICE_PARAM_MODES] {
            assert_ne!(
                gooey_engine_add_lfo_route(engine.0, 0, CHANNEL, param, 1.0),
                LFO_INVALID
            );
        }
        gooey_engine_set_lfo_enabled(engine.0, 0, true);
    }
    engine.render(64);
    let modulated =
        |param| unsafe { gooey_engine_get_channel_param_modulated(engine.0, CHANNEL, param) };
    approx(modulated(PLATE_VOICE_PARAM_MATERIAL), 4.0);
    approx(modulated(PLATE_VOICE_PARAM_MODES), 7.0);

    unsafe { gooey_engine_set_lfo_offset(engine.0, 0, -1.0) };
    engine.render(64);
    approx(modulated(PLATE_VOICE_PARAM_MATERIAL), 0.0);
    approx(modulated(PLATE_VOICE_PARAM_MODES), 0.0);

    unsafe { gooey_engine_set_lfo_offset(engine.0, 0, 0.0) };
    engine.render(64);
    approx(modulated(PLATE_VOICE_PARAM_MATERIAL), 2.0);

    unsafe { gooey_engine_trigger_channel(engine.0, CHANNEL) };
    assert!(energy(&engine.render(8192)) > 1e-6);
}
