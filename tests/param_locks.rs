//! Per-channel parameter locks: locked values must survive preset blending.

use gooey::ffi::*;

const KICK_CHANNEL: u32 = INSTRUMENT_KICK;

fn approx_eq(a: f32, b: f32) {
    assert!(
        (a - b).abs() < 1e-6,
        "expected {b}, got {a} (delta {})",
        (a - b).abs()
    );
}

struct Engine(*mut GooeyEngine);

impl Engine {
    fn new() -> Self {
        Self(gooey_engine_new(44100.0))
    }

    /// Kick engine with blend enabled at the bottom-left corner.
    fn blended_kick() -> Self {
        let engine = Self::new();
        unsafe {
            gooey_engine_blend_enable(engine.0, KICK_CHANNEL);
            gooey_engine_blend_set_position(engine.0, KICK_CHANNEL, 0.0, 0.0);
        }
        engine
    }

    fn param(&self, channel: u32, param: u32) -> f32 {
        unsafe { gooey_engine_get_channel_param(self.0, channel, param) }
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

/// Unlocked kick value at blend position (x, y), from a fresh engine.
fn blended_value(param: u32, x: f32, y: f32) -> f32 {
    let engine = Engine::blended_kick();
    unsafe { gooey_engine_blend_set_position(engine.0, KICK_CHANNEL, x, y) };
    engine.param(KICK_CHANNEL, param)
}

#[test]
fn lock_survives_blend_set_position() {
    let engine = Engine::blended_kick();
    unsafe {
        gooey_engine_set_channel_param_lock(engine.0, KICK_CHANNEL, KICK_PARAM_PUNCH, 0.123);
        assert!(gooey_engine_channel_param_is_locked(
            engine.0,
            KICK_CHANNEL,
            KICK_PARAM_PUNCH
        ));
        gooey_engine_blend_set_position(engine.0, KICK_CHANNEL, 1.0, 1.0);

        approx_eq(engine.param(KICK_CHANNEL, KICK_PARAM_PUNCH), 0.123);
        // The instrument's own target holds the lock, not just the lock table.
        approx_eq(
            gooey_engine_get_kick_param(engine.0, KICK_PARAM_PUNCH),
            0.123,
        );
    }
    // Unlocked params still follow the blend.
    approx_eq(
        engine.param(KICK_CHANNEL, KICK_PARAM_NOISE_RESONANCE),
        blended_value(KICK_PARAM_NOISE_RESONANCE, 1.0, 1.0),
    );
}

#[test]
fn lock_survives_sequencer_trigger_with_step_blend() {
    let engine = Engine::blended_kick();
    unsafe {
        gooey_engine_set_channel_param_lock(engine.0, KICK_CHANNEL, KICK_PARAM_PUNCH, 0.123);
        gooey_engine_set_channel_param_lock(engine.0, KICK_CHANNEL, KICK_PARAM_AMP_DECAY, 0.9);
        gooey_engine_sequencer_set_instrument_step(engine.0, KICK_CHANNEL, 0, true);
        gooey_engine_sequencer_set_instrument_step_blend(engine.0, KICK_CHANNEL, 0, 1.0, 1.0);
        gooey_engine_sequencer_start(engine.0);
    }
    engine.render(512);

    // The step blend was applied on the hit (bottom-left and top-right
    // corners differ in noise resonance)...
    approx_eq(
        engine.param(KICK_CHANNEL, KICK_PARAM_NOISE_RESONANCE),
        blended_value(KICK_PARAM_NOISE_RESONANCE, 1.0, 1.0),
    );
    // ...but the locked params kept their values.
    unsafe {
        approx_eq(
            gooey_engine_get_kick_param(engine.0, KICK_PARAM_PUNCH),
            0.123,
        );
        approx_eq(
            gooey_engine_get_kick_param(engine.0, KICK_PARAM_AMP_DECAY),
            0.9,
        );
    }
}

#[test]
fn clearing_a_lock_restores_the_blended_value() {
    let engine = Engine::blended_kick();
    let blended_punch = engine.param(KICK_CHANNEL, KICK_PARAM_PUNCH);
    unsafe {
        gooey_engine_set_channel_param_lock(engine.0, KICK_CHANNEL, KICK_PARAM_PUNCH, 0.987);
        gooey_engine_set_channel_param_lock(engine.0, KICK_CHANNEL, KICK_PARAM_CLICK, 0.456);

        gooey_engine_clear_channel_param_lock(engine.0, KICK_CHANNEL, KICK_PARAM_PUNCH);
        assert!(!gooey_engine_channel_param_is_locked(
            engine.0,
            KICK_CHANNEL,
            KICK_PARAM_PUNCH
        ));
        approx_eq(engine.param(KICK_CHANNEL, KICK_PARAM_PUNCH), blended_punch);
        // Re-blending respects the remaining lock.
        approx_eq(
            gooey_engine_get_kick_param(engine.0, KICK_PARAM_CLICK),
            0.456,
        );
    }
}

#[test]
fn clearing_all_locks_restores_the_blend() {
    let engine = Engine::blended_kick();
    let blended_punch = engine.param(KICK_CHANNEL, KICK_PARAM_PUNCH);
    let blended_overdrive = engine.param(KICK_CHANNEL, KICK_PARAM_OVERDRIVE);
    unsafe {
        gooey_engine_set_channel_param_lock(engine.0, KICK_CHANNEL, KICK_PARAM_PUNCH, 0.987);
        gooey_engine_set_channel_param_lock(engine.0, KICK_CHANNEL, KICK_PARAM_OVERDRIVE, 0.654);
        gooey_engine_clear_channel_param_locks(engine.0, KICK_CHANNEL);

        for param in [KICK_PARAM_PUNCH, KICK_PARAM_OVERDRIVE] {
            assert!(!gooey_engine_channel_param_is_locked(
                engine.0,
                KICK_CHANNEL,
                param
            ));
        }
    }
    approx_eq(engine.param(KICK_CHANNEL, KICK_PARAM_PUNCH), blended_punch);
    approx_eq(
        engine.param(KICK_CHANNEL, KICK_PARAM_OVERDRIVE),
        blended_overdrive,
    );
}

#[test]
fn clearing_a_lock_without_blend_keeps_the_value() {
    let engine = Engine::new();
    unsafe {
        gooey_engine_set_channel_param_lock(engine.0, KICK_CHANNEL, KICK_PARAM_PUNCH, 0.321);
        gooey_engine_clear_channel_param_lock(engine.0, KICK_CHANNEL, KICK_PARAM_PUNCH);
    }
    approx_eq(engine.param(KICK_CHANNEL, KICK_PARAM_PUNCH), 0.321);
}

#[test]
fn instrument_change_clears_locks() {
    let engine = Engine::blended_kick();
    unsafe {
        gooey_engine_set_channel_param_lock(engine.0, KICK_CHANNEL, KICK_PARAM_PUNCH, 0.123);
        gooey_engine_set_channel_instrument_type(engine.0, KICK_CHANNEL, INSTRUMENT_SNARE);
        assert!(!gooey_engine_channel_param_is_locked(
            engine.0,
            KICK_CHANNEL,
            KICK_PARAM_PUNCH
        ));

        gooey_engine_set_channel_instrument_type(engine.0, KICK_CHANNEL, INSTRUMENT_KICK);
        assert!(!gooey_engine_channel_param_is_locked(
            engine.0,
            KICK_CHANNEL,
            KICK_PARAM_PUNCH
        ));
    }
    approx_eq(
        engine.param(KICK_CHANNEL, KICK_PARAM_PUNCH),
        blended_value(KICK_PARAM_PUNCH, 0.0, 0.0),
    );
}

#[test]
fn lfo_still_modulates_a_locked_param() {
    let engine = Engine::new();
    unsafe {
        gooey_engine_set_channel_param_lock(engine.0, KICK_CHANNEL, KICK_PARAM_PUNCH, 0.1);
        // A constant LFO output of 0.8 (bipolar) maps to a 0.9 target.
        gooey_engine_set_lfo_amount(engine.0, 0, 0.0);
        gooey_engine_set_lfo_offset(engine.0, 0, 0.8);
        gooey_engine_add_lfo_route(engine.0, 0, KICK_CHANNEL, KICK_PARAM_PUNCH, 1.0);
        gooey_engine_set_lfo_enabled(engine.0, 0, true);
    }
    engine.render(64);
    unsafe {
        approx_eq(gooey_engine_get_kick_param(engine.0, KICK_PARAM_PUNCH), 0.9);
        // The lock itself is untouched.
        assert!(gooey_engine_channel_param_is_locked(
            engine.0,
            KICK_CHANNEL,
            KICK_PARAM_PUNCH
        ));
    }
    approx_eq(engine.param(KICK_CHANNEL, KICK_PARAM_PUNCH), 0.1);
}

#[test]
fn get_channel_param_reaches_any_channel() {
    let engine = Engine::new();
    unsafe {
        // Channel 1 becomes a second kick; the legacy getter only sees channel 0.
        gooey_engine_set_channel_instrument_type(engine.0, 1, INSTRUMENT_KICK);
        gooey_engine_set_channel_param(engine.0, 1, KICK_PARAM_NOISE_CUTOFF, 0.37);
        gooey_engine_set_channel_param(engine.0, INSTRUMENT_BASS, BASS_PARAM_OVERDRIVE, 0.61);
    }
    approx_eq(engine.param(1, KICK_PARAM_NOISE_CUTOFF), 0.37);
    approx_eq(engine.param(INSTRUMENT_BASS, BASS_PARAM_OVERDRIVE), 0.61);
}

#[test]
fn invalid_inputs_are_rejected() {
    let engine = Engine::new();
    unsafe {
        assert!(gooey_engine_get_channel_param(std::ptr::null(), 0, 0).is_nan());
        assert!(!gooey_engine_channel_param_is_locked(
            std::ptr::null(),
            0,
            0
        ));
        gooey_engine_set_channel_param_lock(std::ptr::null_mut(), 0, 0, 0.5);
        gooey_engine_clear_channel_param_lock(std::ptr::null_mut(), 0, 0);
        gooey_engine_clear_channel_param_locks(std::ptr::null_mut(), 0);

        assert!(engine.param(99, 0).is_nan());
        assert!(engine.param(KICK_CHANNEL, 999).is_nan());
        // Hi-hat has no param 20, so a lock there is ignored.
        assert!(engine.param(INSTRUMENT_HIHAT, 20).is_nan());
        gooey_engine_set_channel_param_lock(engine.0, INSTRUMENT_HIHAT, 20, 0.5);
        assert!(!gooey_engine_channel_param_is_locked(
            engine.0,
            INSTRUMENT_HIHAT,
            20
        ));
        gooey_engine_set_channel_param_lock(engine.0, 99, 0, 0.5);
        assert!(!gooey_engine_channel_param_is_locked(engine.0, 99, 0));
        assert!(!gooey_engine_channel_param_is_locked(
            engine.0,
            KICK_CHANNEL,
            999
        ));
    }
}
