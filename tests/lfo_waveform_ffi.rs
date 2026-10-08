//! Engine LFO waveforms and live output through the C ABI.

use gooey::ffi::*;

const SR: f32 = 48_000.0;
const WAVEFORMS: [u32; 5] = [
    LFO_WAVEFORM_SINE,
    LFO_WAVEFORM_TRIANGLE,
    LFO_WAVEFORM_SAW,
    LFO_WAVEFORM_SQUARE,
    LFO_WAVEFORM_SAMPLE_HOLD,
];

struct Engine(*mut GooeyEngine);

impl Engine {
    fn new() -> Self {
        let engine = gooey_engine_new(SR);
        unsafe { gooey_engine_set_bpm(engine, 120.0) };
        Self(engine)
    }

    fn render_frames(&self, frames: usize) {
        let mut buffer = vec![0.0_f32; 512 * 2];
        let mut remaining = frames;
        while remaining > 0 {
            let chunk = remaining.min(512);
            unsafe { gooey_engine_render(self.0, buffer.as_mut_ptr(), chunk as u32) };
            remaining -= chunk;
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { gooey_engine_free(self.0) };
    }
}

#[test]
fn waveform_defaults_to_sine_and_round_trips() {
    let engine = Engine::new();
    for lfo in 0..LFO_COUNT as u32 {
        assert_eq!(
            unsafe { gooey_engine_get_lfo_waveform(engine.0, lfo) },
            LFO_WAVEFORM_SINE
        );
    }
    for waveform in WAVEFORMS {
        assert!(unsafe { gooey_engine_set_lfo_waveform(engine.0, 2, waveform) });
        assert_eq!(
            unsafe { gooey_engine_get_lfo_waveform(engine.0, 2) },
            waveform
        );
    }
}

#[test]
fn invalid_waveform_inputs_are_rejected() {
    let engine = Engine::new();
    unsafe {
        assert!(!gooey_engine_set_lfo_waveform(engine.0, 0, 5));
        assert_eq!(
            gooey_engine_get_lfo_waveform(engine.0, 0),
            LFO_WAVEFORM_SINE
        );
        assert!(!gooey_engine_set_lfo_waveform(
            engine.0,
            LFO_COUNT as u32,
            LFO_WAVEFORM_SAW
        ));
        assert_eq!(
            gooey_engine_get_lfo_waveform(engine.0, LFO_COUNT as u32),
            LFO_INVALID
        );
        assert!(!gooey_engine_set_lfo_waveform(
            std::ptr::null_mut(),
            0,
            LFO_WAVEFORM_SAW
        ));
        assert_eq!(
            gooey_engine_get_lfo_waveform(std::ptr::null(), 0),
            LFO_INVALID
        );
        assert_eq!(gooey_engine_get_lfo_value(std::ptr::null(), 0), 0.0);
        assert_eq!(gooey_engine_get_lfo_value(engine.0, LFO_COUNT as u32), 0.0);
    }
}

#[test]
fn value_is_zero_while_disabled() {
    let engine = Engine::new();
    engine.render_frames(4_800);
    assert_eq!(unsafe { gooey_engine_get_lfo_value(engine.0, 0) }, 0.0);
}

#[test]
fn square_value_follows_phase_with_amount_and_offset() {
    let engine = Engine::new();
    unsafe {
        gooey_engine_set_lfo_timing(engine.0, 0, LFO_TIMING_QUARTER);
        gooey_engine_set_lfo_waveform(engine.0, 0, LFO_WAVEFORM_SQUARE);
        gooey_engine_set_lfo_amount(engine.0, 0, 0.5);
        gooey_engine_set_lfo_offset(engine.0, 0, 0.25);
        gooey_engine_set_lfo_enabled(engine.0, 0, true);
    }
    // One quarter note at 120 BPM is 24,000 frames.
    engine.render_frames(6_000);
    assert!((unsafe { gooey_engine_get_lfo_value(engine.0, 0) } - 0.75).abs() < 1e-5);
    engine.render_frames(12_000);
    assert!((unsafe { gooey_engine_get_lfo_value(engine.0, 0) } + 0.25).abs() < 1e-5);
}

#[test]
fn sine_value_tracks_phase() {
    let engine = Engine::new();
    unsafe {
        gooey_engine_set_lfo_timing(engine.0, 1, LFO_TIMING_QUARTER);
        gooey_engine_set_lfo_enabled(engine.0, 1, true);
    }
    for _ in 0..8 {
        engine.render_frames(1_337);
        let (phase, value) = unsafe {
            (
                gooey_engine_get_lfo_phase(engine.0, 1),
                gooey_engine_get_lfo_value(engine.0, 1),
            )
        };
        // The value was computed one sample before the phase advanced.
        let previous = phase - 2.0 / SR;
        let expected = (previous * std::f32::consts::TAU).sin();
        assert!(
            (value - expected).abs() < 1e-3,
            "phase {phase}: {value} vs {expected}"
        );
    }
}
