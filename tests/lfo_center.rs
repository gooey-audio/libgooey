//! LFO routes swing a parameter around its own (knob) value, so moving the
//! parameter while it is routed moves the center of the sweep.

use gooey::ffi::*;

const KICK_CHANNEL: u32 = INSTRUMENT_KICK;

fn approx_eq(a: f32, b: f32) {
    assert!(
        (a - b).abs() < 1e-4,
        "expected {b}, got {a} (delta {})",
        (a - b).abs()
    );
}

struct Engine(*mut GooeyEngine);

impl Engine {
    fn new() -> Self {
        Self(gooey_engine_new(44100.0))
    }

    fn set(&self, channel: u32, param: u32, value: f32) {
        unsafe { gooey_engine_set_channel_param(self.0, channel, param, value) };
    }

    fn param(&self, channel: u32, param: u32) -> f32 {
        unsafe { gooey_engine_get_channel_param(self.0, channel, param) }
    }

    fn modulated(&self, channel: u32, param: u32) -> f32 {
        unsafe { gooey_engine_get_channel_param_modulated(self.0, channel, param) }
    }

    /// Route `lfo` to a parameter with a constant output of `output`
    /// (amount 0, offset `output`) at full depth. Returns the route ID.
    fn constant_lfo(&self, lfo: u32, channel: u32, param: u32, output: f32) -> u32 {
        unsafe {
            gooey_engine_set_lfo_amount(self.0, lfo, 0.0);
            gooey_engine_set_lfo_offset(self.0, lfo, output);
            gooey_engine_set_lfo_enabled(self.0, lfo, true);
            gooey_engine_add_lfo_route(self.0, lfo, channel, param, 1.0)
        }
    }

    fn render(&self, frames: usize) {
        let mut buffer = vec![0.0_f32; frames * 2];
        unsafe { gooey_engine_render(self.0, buffer.as_mut_ptr(), frames as u32) };
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { gooey_engine_free(self.0) };
    }
}

#[test]
fn moving_the_knob_moves_the_center() {
    let engine = Engine::new();
    engine.set(KICK_CHANNEL, KICK_PARAM_PUNCH, 0.3);
    engine.constant_lfo(0, KICK_CHANNEL, KICK_PARAM_PUNCH, 0.8);
    engine.render(64);
    approx_eq(engine.modulated(KICK_CHANNEL, KICK_PARAM_PUNCH), 0.7);

    engine.set(KICK_CHANNEL, KICK_PARAM_PUNCH, 0.1);
    engine.render(64);
    approx_eq(engine.modulated(KICK_CHANNEL, KICK_PARAM_PUNCH), 0.5);
}

#[test]
fn getter_returns_the_center_while_modulated() {
    let engine = Engine::new();
    engine.set(KICK_CHANNEL, KICK_PARAM_DECAY, 0.25);
    engine.constant_lfo(0, KICK_CHANNEL, KICK_PARAM_DECAY, -0.4);
    engine.render(64);
    approx_eq(engine.param(KICK_CHANNEL, KICK_PARAM_DECAY), 0.25);
    approx_eq(engine.modulated(KICK_CHANNEL, KICK_PARAM_DECAY), 0.05);
    approx_eq(
        unsafe { gooey_engine_get_kick_param(engine.0, KICK_PARAM_DECAY) },
        0.25,
    );
}

#[test]
fn every_instrument_swings_around_its_value() {
    let engine = Engine::new();
    unsafe {
        gooey_engine_set_channel_instrument_type(engine.0, 1, INSTRUMENT_RESONATOR);
        gooey_engine_set_channel_instrument_type(engine.0, 2, INSTRUMENT_TWIN_CORE);
    }
    let targets = [
        (KICK_CHANNEL, KICK_PARAM_FREQUENCY),
        (1, RESONATOR_PARAM_DECAY),
        (2, TWIN_CORE_PARAM_TUNE),
        (INSTRUMENT_TOM, TOM_PARAM_TUNE),
        (INSTRUMENT_BASS, BASS_PARAM_FILTER_CUTOFF),
    ];
    for (lfo, &(channel, param)) in targets.iter().enumerate() {
        engine.set(channel, param, 0.4);
        // Negative output: tom used to clip the lower half of the sweep.
        engine.constant_lfo(lfo as u32, channel, param, -0.4);
    }
    engine.render(64);
    for &(channel, param) in &targets {
        approx_eq(engine.modulated(channel, param), 0.2);
        approx_eq(engine.param(channel, param), 0.4);
    }
}

#[test]
fn sweep_clamps_to_the_parameter_range() {
    let engine = Engine::new();
    engine.set(KICK_CHANNEL, KICK_PARAM_PUNCH, 0.9);
    engine.constant_lfo(0, KICK_CHANNEL, KICK_PARAM_PUNCH, 0.8);
    engine.render(64);
    approx_eq(engine.modulated(KICK_CHANNEL, KICK_PARAM_PUNCH), 1.0);
    // Clamping does not lose the center.
    engine.render(64);
    approx_eq(engine.param(KICK_CHANNEL, KICK_PARAM_PUNCH), 0.9);
}

#[test]
fn routes_to_one_parameter_add_up() {
    let engine = Engine::new();
    engine.set(KICK_CHANNEL, KICK_PARAM_PUNCH, 0.5);
    engine.constant_lfo(0, KICK_CHANNEL, KICK_PARAM_PUNCH, 0.2);
    engine.constant_lfo(1, KICK_CHANNEL, KICK_PARAM_PUNCH, 0.4);
    engine.render(64);
    approx_eq(engine.modulated(KICK_CHANNEL, KICK_PARAM_PUNCH), 0.8);
}

#[test]
fn removing_the_route_restores_the_center() {
    let engine = Engine::new();
    engine.set(KICK_CHANNEL, KICK_PARAM_PUNCH, 0.3);
    let route = engine.constant_lfo(0, KICK_CHANNEL, KICK_PARAM_PUNCH, 0.8);
    engine.render(64);
    unsafe { assert!(gooey_engine_remove_lfo_route(engine.0, 0, route)) };
    engine.render(64);
    approx_eq(engine.modulated(KICK_CHANNEL, KICK_PARAM_PUNCH), 0.3);
}

#[test]
fn disabling_the_lfo_restores_the_center() {
    let engine = Engine::new();
    engine.set(KICK_CHANNEL, KICK_PARAM_PUNCH, 0.3);
    engine.constant_lfo(0, KICK_CHANNEL, KICK_PARAM_PUNCH, 0.8);
    engine.render(64);
    unsafe { gooey_engine_set_lfo_enabled(engine.0, 0, false) };
    engine.render(64);
    approx_eq(engine.modulated(KICK_CHANNEL, KICK_PARAM_PUNCH), 0.3);
}

#[test]
fn a_write_after_unrouting_is_kept() {
    let engine = Engine::new();
    engine.set(KICK_CHANNEL, KICK_PARAM_PUNCH, 0.3);
    engine.constant_lfo(0, KICK_CHANNEL, KICK_PARAM_PUNCH, 0.8);
    engine.render(64);
    unsafe { gooey_engine_clear_lfo_routes(engine.0, 0) };
    engine.set(KICK_CHANNEL, KICK_PARAM_PUNCH, 0.6);
    engine.render(64);
    approx_eq(engine.modulated(KICK_CHANNEL, KICK_PARAM_PUNCH), 0.6);
}

#[test]
fn blend_moves_the_center_of_an_unlocked_param() {
    let engine = Engine::new();
    unsafe {
        gooey_engine_blend_enable(engine.0, KICK_CHANNEL);
        gooey_engine_blend_set_position(engine.0, KICK_CHANNEL, 0.0, 0.0);
    }
    engine.constant_lfo(0, KICK_CHANNEL, KICK_PARAM_FREQUENCY, 0.2);
    engine.render(64);
    unsafe { gooey_engine_blend_set_position(engine.0, KICK_CHANNEL, 1.0, 1.0) };
    let blended = engine.modulated(KICK_CHANNEL, KICK_PARAM_FREQUENCY);
    engine.render(64);
    approx_eq(engine.param(KICK_CHANNEL, KICK_PARAM_FREQUENCY), blended);
    approx_eq(
        engine.modulated(KICK_CHANNEL, KICK_PARAM_FREQUENCY),
        (blended + 0.1).min(1.0),
    );
}

#[test]
fn changing_instrument_type_forgets_the_center() {
    let engine = Engine::new();
    engine.set(KICK_CHANNEL, KICK_PARAM_PUNCH, 0.3);
    engine.constant_lfo(0, KICK_CHANNEL, KICK_PARAM_PUNCH, 0.0);
    engine.render(64);
    unsafe {
        gooey_engine_clear_lfo_routes(engine.0, 0);
        gooey_engine_set_channel_instrument_type(engine.0, KICK_CHANNEL, INSTRUMENT_SNARE);
    }
    // The same index now names a snare parameter; the kick's center must not
    // be written into it.
    let fresh = engine.param(KICK_CHANNEL, KICK_PARAM_PUNCH);
    engine.render(64);
    approx_eq(engine.param(KICK_CHANNEL, KICK_PARAM_PUNCH), fresh);
}
