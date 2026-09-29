//! Macro LFOs (continuous, tempo-synced cycling of a macro) through the C ABI.

use gooey::ffi::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

const SR: f32 = 48_000.0;
const SHAPES: [u32; 4] = [
    MACRO_LFO_SHAPE_SINE,
    MACRO_LFO_SHAPE_TRIANGLE,
    MACRO_LFO_SHAPE_SAW,
    MACRO_LFO_SHAPE_SQUARE,
];
/// Accepted rates, 1/16 note through 4 bars.
const RATES: [u32; 7] = [
    LFO_TIMING_SIXTEENTH,
    LFO_TIMING_EIGHTH,
    LFO_TIMING_QUARTER,
    LFO_TIMING_HALF,
    LFO_TIMING_ONE_BAR,
    LFO_TIMING_TWO_BARS,
    LFO_TIMING_FOUR_BARS,
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

    fn render_seconds(&self, seconds: f64) {
        self.render_frames((seconds * SR as f64).round() as usize);
    }

    fn cutoff(&self) -> f32 {
        unsafe {
            gooey_engine_get_global_effect_param(self.0, EFFECT_LOWPASS_FILTER, FILTER_PARAM_CUTOFF)
        }
    }

    fn hihat_tone(&self) -> f32 {
        unsafe { gooey_engine_get_hihat_param(self.0, HIHAT_PARAM_TONE) }
    }

    fn poly_cutoff(&self) -> f32 {
        unsafe { gooey_engine_poly_get_param(self.0, POLY_PARAM_FILTER_CUTOFF) }
    }

    fn value(&self, macro_index: u32) -> f32 {
        unsafe { gooey_engine_macro_get_value(self.0, macro_index) }
    }

    fn phase(&self, macro_index: u32) -> f32 {
        unsafe { gooey_engine_macro_lfo_get_phase(self.0, macro_index) }
    }

    fn state(&self, macro_index: u32) -> u32 {
        unsafe { gooey_engine_macro_lfo_get_state(self.0, macro_index) }
    }

    fn start(&self, macro_index: u32, shape: u32, rate: u32) {
        unsafe {
            assert!(gooey_engine_macro_lfo_set_shape(self.0, macro_index, shape));
            assert!(gooey_engine_macro_lfo_set_rate(self.0, macro_index, rate));
            assert!(gooey_engine_macro_lfo_start(self.0, macro_index));
        }
    }

    fn map(&self, macro_index: u32, kind: u32, index: u32, param: u32, from: f32, to: f32) {
        unsafe {
            assert!(gooey_engine_macro_add_mapping(
                self.0,
                macro_index,
                kind,
                index,
                param,
                from,
                to
            ));
        }
    }

    fn map_hihat_tone(&self, macro_index: u32, from: f32, to: f32) {
        self.map(
            macro_index,
            PARAM_TARGET_DRUM,
            INSTRUMENT_HIHAT,
            HIHAT_PARAM_TONE,
            from,
            to,
        );
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { gooey_engine_free(self.0) }
    }
}

fn close(actual: f32, expected: f32, tolerance: f32) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected} ± {tolerance}, got {actual}"
    );
}

/// Distance between two cycle phases, treating 0 and 1 as the same point.
fn phase_close(actual: f32, expected: f64, tolerance: f64) {
    let delta = (actual as f64 - expected).rem_euclid(1.0);
    assert!(
        delta.min(1.0 - delta) <= tolerance,
        "expected phase {expected} ± {tolerance}, got {actual}"
    );
}

/// Reference waveforms, written independently of the engine.
fn wave(shape: u32, phase: f64) -> f32 {
    let phase = phase.rem_euclid(1.0);
    let value = match shape {
        MACRO_LFO_SHAPE_SINE => 0.5 - 0.5 * (2.0 * std::f64::consts::PI * phase).cos(),
        MACRO_LFO_SHAPE_TRIANGLE => {
            if phase < 0.5 {
                2.0 * phase
            } else {
                2.0 - 2.0 * phase
            }
        }
        MACRO_LFO_SHAPE_SAW => phase,
        MACRO_LFO_SHAPE_SQUARE => {
            if phase < 0.5 {
                0.0
            } else {
                1.0
            }
        }
        _ => unreachable!(),
    };
    value as f32
}

/// Cycle length in frames at `bpm`.
fn cycle_frames(rate: u32, bpm: f64) -> f64 {
    let beats = match rate {
        LFO_TIMING_SIXTEENTH => 0.25,
        LFO_TIMING_EIGHTH => 0.5,
        LFO_TIMING_QUARTER => 1.0,
        LFO_TIMING_HALF => 2.0,
        LFO_TIMING_ONE_BAR => 4.0,
        LFO_TIMING_TWO_BARS => 8.0,
        LFO_TIMING_FOUR_BARS => 16.0,
        _ => unreachable!(),
    };
    beats * 60.0 / bpm * SR as f64
}

#[test]
fn settings_default_round_trip_and_reject_invalid_arguments() {
    let engine = Engine::new();
    unsafe {
        for macro_index in 0..MACRO_COUNT {
            assert_eq!(
                gooey_engine_macro_lfo_get_shape(engine.0, macro_index),
                MACRO_LFO_SHAPE_SINE
            );
            assert_eq!(
                gooey_engine_macro_lfo_get_rate(engine.0, macro_index),
                LFO_TIMING_ONE_BAR
            );
            assert_eq!(engine.state(macro_index), MACRO_LFO_STATE_STOPPED);
            assert_eq!(gooey_engine_macro_lfo_get_value(engine.0, macro_index), 0.0);
            assert_eq!(engine.phase(macro_index), 0.0);
        }

        for (i, (&shape, &rate)) in SHAPES.iter().cycle().zip(&RATES).enumerate() {
            let macro_index = i as u32;
            assert!(gooey_engine_macro_lfo_set_shape(
                engine.0,
                macro_index,
                shape
            ));
            assert!(gooey_engine_macro_lfo_set_rate(engine.0, macro_index, rate));
        }
        for (i, (&shape, &rate)) in SHAPES.iter().cycle().zip(&RATES).enumerate() {
            let macro_index = i as u32;
            assert_eq!(
                gooey_engine_macro_lfo_get_shape(engine.0, macro_index),
                shape
            );
            assert_eq!(gooey_engine_macro_lfo_get_rate(engine.0, macro_index), rate);
        }
        // Settings are per macro.
        assert_eq!(
            gooey_engine_macro_lfo_get_shape(engine.0, 7),
            MACRO_LFO_SHAPE_SINE
        );

        assert!(!gooey_engine_macro_lfo_set_shape(engine.0, 0, 4));
        assert!(!gooey_engine_macro_lfo_set_rate(
            engine.0,
            0,
            LFO_TIMING_THIRTY_SECOND
        ));
        assert!(!gooey_engine_macro_lfo_set_rate(engine.0, 0, 99));
        assert_eq!(
            gooey_engine_macro_lfo_get_shape(engine.0, 0),
            MACRO_LFO_SHAPE_SINE
        );

        let bad = MACRO_COUNT;
        assert!(!gooey_engine_macro_lfo_set_shape(engine.0, bad, 0));
        assert!(!gooey_engine_macro_lfo_set_rate(engine.0, bad, 0));
        assert!(!gooey_engine_macro_lfo_start(engine.0, bad));
        assert!(!gooey_engine_macro_lfo_stop(engine.0, bad));
        assert!(!gooey_engine_macro_lfo_reset_phase(engine.0, bad));
        assert_eq!(
            gooey_engine_macro_lfo_get_shape(engine.0, bad),
            AUTOMATION_INVALID
        );
        assert_eq!(
            gooey_engine_macro_lfo_get_rate(engine.0, bad),
            AUTOMATION_INVALID
        );
        assert_eq!(engine.state(bad), AUTOMATION_INVALID);
        assert!(gooey_engine_macro_lfo_get_value(engine.0, bad).is_nan());
        assert!(engine.phase(bad).is_nan());

        let null = std::ptr::null::<GooeyEngine>();
        assert!(!gooey_engine_macro_lfo_set_shape(null, 0, 0));
        assert!(!gooey_engine_macro_lfo_start(null, 0));
        assert!(!gooey_engine_macro_lfo_stop(null, 0));
        assert!(!gooey_engine_macro_lfo_stop_all(null));
        assert!(!gooey_engine_macro_lfo_reset_phase(null, 0));
        assert_eq!(
            gooey_engine_macro_lfo_get_state(null, 0),
            AUTOMATION_INVALID
        );
        assert!(gooey_engine_macro_lfo_get_value(null, 0).is_nan());
    }
}

#[test]
fn lfo_drives_mapped_parameters_between_from_and_to() {
    let engine = Engine::new();
    engine.map_hihat_tone(0, 0.2, 0.8);
    // Inverted mapping in engineering units.
    engine.map(
        1,
        PARAM_TARGET_GLOBAL_EFFECT,
        EFFECT_LOWPASS_FILTER,
        FILTER_PARAM_CUTOFF,
        5000.0,
        1000.0,
    );
    engine.start(0, MACRO_LFO_SHAPE_TRIANGLE, LFO_TIMING_QUARTER);
    engine.start(1, MACRO_LFO_SHAPE_SINE, LFO_TIMING_HALF);
    assert_eq!(engine.state(0), MACRO_LFO_STATE_RUNNING);
    assert_eq!(engine.value(0), 0.0, "an LFO starts at macro 0");

    // A quarter-note triangle peaks half a cycle (0.25 s) in.
    engine.render_seconds(0.25);
    phase_close(engine.phase(0), 0.5, 1e-4);
    close(engine.value(0), 1.0, 1e-3);
    close(engine.hihat_tone(), 0.8, 1e-3);

    let (mut low, mut high) = (f32::MAX, f32::MIN);
    for _ in 0..40 {
        engine.render_seconds(0.037);
        for (macro_index, shape) in [(0, MACRO_LFO_SHAPE_TRIANGLE), (1, MACRO_LFO_SHAPE_SINE)] {
            let value = engine.value(macro_index);
            close(value, wave(shape, engine.phase(macro_index) as f64), 1e-5);
            close(
                unsafe { gooey_engine_macro_lfo_get_value(engine.0, macro_index) },
                value,
                0.0,
            );
        }
        let tone_macro = engine.value(0);
        close(engine.hihat_tone(), 0.2 + 0.6 * tone_macro, 1e-5);
        let cutoff_macro = engine.value(1);
        close(engine.cutoff(), 5000.0 - 4000.0 * cutoff_macro, 0.02);
        low = low.min(cutoff_macro);
        high = high.max(cutoff_macro);
    }
    assert!(
        low < 0.05 && high > 0.95,
        "sweeps the full range: {low}..{high}"
    );
}

#[test]
fn all_sixteen_macros_cycle_independently() {
    let engine = Engine::new();
    for macro_index in 0..MACRO_COUNT {
        let shape = SHAPES[macro_index as usize % SHAPES.len()];
        let rate = RATES[macro_index as usize % RATES.len()];
        engine.start(macro_index, shape, rate);
    }
    // 0.3 s is a whole number of 32-frame control ticks.
    let frames = 14_400;
    engine.render_frames(frames);
    for macro_index in 0..MACRO_COUNT {
        let shape = SHAPES[macro_index as usize % SHAPES.len()];
        let rate = RATES[macro_index as usize % RATES.len()];
        let expected = frames as f64 / cycle_frames(rate, 120.0);
        phase_close(engine.phase(macro_index), expected, 1e-5);
        close(
            engine.value(macro_index),
            wave(shape, engine.phase(macro_index) as f64),
            1e-5,
        );
        assert_eq!(engine.state(macro_index), MACRO_LFO_STATE_RUNNING);
    }
}

#[test]
fn lfo_keeps_cycling_whether_or_not_the_transport_runs() {
    let engine = Engine::new();
    engine.start(0, MACRO_LFO_SHAPE_SAW, LFO_TIMING_ONE_BAR);
    engine.render_seconds(1.0);
    phase_close(engine.phase(0), 0.5, 1e-4);
    unsafe { gooey_engine_sequencer_start(engine.0) };
    engine.render_seconds(0.5);
    phase_close(engine.phase(0), 0.75, 1e-4);
    unsafe { gooey_engine_sequencer_stop(engine.0) };
    engine.render_seconds(0.25);
    phase_close(engine.phase(0), 0.875, 1e-4);
    assert_eq!(engine.state(0), MACRO_LFO_STATE_RUNNING);
}

#[test]
fn tempo_changes_keep_phase_and_change_speed() {
    let engine = Engine::new();
    engine.map_hihat_tone(0, 0.0, 1.0);
    engine.start(0, MACRO_LFO_SHAPE_SAW, LFO_TIMING_ONE_BAR);
    engine.render_seconds(1.0);
    phase_close(engine.phase(0), 0.5, 1e-4);
    let before = engine.hihat_tone();

    // Halving the tempo: no jump, then one beat per second.
    unsafe { gooey_engine_set_bpm(engine.0, 60.0) };
    engine.render_frames(32);
    close(engine.hihat_tone(), before, 1e-3);
    engine.render_seconds(1.0);
    phase_close(engine.phase(0), 0.75, 1e-3);

    unsafe { gooey_engine_set_bpm(engine.0, 240.0) };
    engine.render_seconds(0.125);
    phase_close(engine.phase(0), 0.875, 1e-3);
    close(engine.hihat_tone(), engine.value(0), 1e-5);
}

#[test]
fn retrigger_and_reset_restart_the_cycle() {
    let engine = Engine::new();
    engine.start(0, MACRO_LFO_SHAPE_TRIANGLE, LFO_TIMING_HALF);
    engine.render_seconds(0.3);
    assert!(engine.value(0) > 0.2);

    unsafe { assert!(gooey_engine_macro_lfo_start(engine.0, 0)) };
    assert_eq!(engine.value(0), 0.0, "restart reports macro 0 at once");
    engine.render_frames(512);
    phase_close(
        engine.phase(0),
        512.0 / cycle_frames(LFO_TIMING_HALF, 120.0),
        1e-6,
    );

    engine.render_seconds(0.2);
    unsafe { assert!(gooey_engine_macro_lfo_reset_phase(engine.0, 0)) };
    assert_eq!(engine.phase(0), 0.0);
    engine.render_frames(512);
    phase_close(
        engine.phase(0),
        512.0 / cycle_frames(LFO_TIMING_HALF, 120.0),
        1e-6,
    );
    assert_eq!(engine.state(0), MACRO_LFO_STATE_RUNNING);

    // Reset does nothing to a stopped LFO.
    unsafe { assert!(gooey_engine_macro_lfo_stop(engine.0, 0)) };
    engine.render_seconds(0.01);
    let held = engine.value(0);
    unsafe { assert!(gooey_engine_macro_lfo_reset_phase(engine.0, 0)) };
    engine.render_seconds(0.1);
    assert_eq!(engine.state(0), MACRO_LFO_STATE_STOPPED);
    assert_eq!(engine.value(0), held);
}

#[test]
fn settings_change_a_running_lfo_without_losing_phase() {
    let engine = Engine::new();
    engine.start(0, MACRO_LFO_SHAPE_SAW, LFO_TIMING_ONE_BAR);
    engine.render_seconds(0.5);
    phase_close(engine.phase(0), 0.25, 1e-4);
    unsafe {
        assert!(gooey_engine_macro_lfo_set_rate(
            engine.0,
            0,
            LFO_TIMING_QUARTER
        ));
        assert!(gooey_engine_macro_lfo_set_shape(
            engine.0,
            0,
            MACRO_LFO_SHAPE_SQUARE
        ));
    }
    // A quarter-note cycle now: 0.25 s is half a cycle.
    engine.render_seconds(0.25);
    phase_close(engine.phase(0), 0.75, 1e-3);
    assert_eq!(engine.value(0), 1.0);
}

#[test]
fn stop_holds_the_last_reported_value() {
    let engine = Engine::new();
    engine.map_hihat_tone(0, 0.2, 0.8);
    engine.start(0, MACRO_LFO_SHAPE_SINE, LFO_TIMING_HALF);
    engine.render_seconds(0.13);
    let reported = engine.value(0);
    assert!(reported > 0.05 && reported < 0.95, "{reported}");

    unsafe { assert!(gooey_engine_macro_lfo_stop(engine.0, 0)) };
    assert_eq!(engine.state(0), MACRO_LFO_STATE_STOPPED);
    assert_eq!(engine.value(0), reported);
    engine.render_seconds(1.0);
    assert_eq!(engine.value(0), reported);
    assert_eq!(
        unsafe { gooey_engine_macro_lfo_get_value(engine.0, 0) },
        reported
    );
    close(engine.hihat_tone(), 0.2 + 0.6 * reported, 1e-6);
    // Stopping again is a harmless no-op.
    unsafe { assert!(gooey_engine_macro_lfo_stop(engine.0, 0)) };
    engine.render_seconds(0.1);
    assert_eq!(engine.value(0), reported);
}

#[test]
fn stop_all_holds_every_macro() {
    let engine = Engine::new();
    for macro_index in [0, 5, 15] {
        engine.start(macro_index, MACRO_LFO_SHAPE_SAW, LFO_TIMING_HALF);
    }
    engine.render_seconds(0.3);
    let held: Vec<f32> = [0, 5, 15].iter().map(|&m| engine.value(m)).collect();
    unsafe { assert!(gooey_engine_macro_lfo_stop_all(engine.0)) };
    engine.render_seconds(0.3);
    for (&macro_index, &value) in [0, 5, 15].iter().zip(&held) {
        assert_eq!(engine.state(macro_index), MACRO_LFO_STATE_STOPPED);
        assert_eq!(engine.value(macro_index), value);
    }
}

#[test]
fn motion_trigger_takes_the_macro_from_its_lfo() {
    let engine = Engine::new();
    engine.map_hihat_tone(0, 0.0, 1.0);
    engine.start(0, MACRO_LFO_SHAPE_SINE, LFO_TIMING_QUARTER);
    engine.render_seconds(0.1);
    unsafe {
        assert!(gooey_engine_motion_configure(engine.0, 0, 0, 1.0));
        assert!(gooey_engine_motion_set_duration(
            engine.0,
            0,
            MOTION_DURATION_MS,
            100.0
        ));
        assert!(gooey_engine_motion_trigger(engine.0, 0));
    }
    assert_eq!(engine.state(0), MACRO_LFO_STATE_STOPPED);
    engine.render_seconds(0.2);
    assert_eq!(
        unsafe { gooey_engine_motion_get_state(engine.0, 0) },
        MOTION_STATE_IDLE
    );
    assert_eq!(engine.value(0), 1.0);
    engine.render_seconds(0.3);
    assert_eq!(engine.value(0), 1.0, "the LFO does not resume");
    close(engine.hihat_tone(), 1.0, 1e-6);
}

#[test]
fn manual_value_takes_the_macro_from_its_lfo() {
    let engine = Engine::new();
    engine.map_hihat_tone(0, 0.0, 1.0);
    engine.start(0, MACRO_LFO_SHAPE_SQUARE, LFO_TIMING_EIGHTH);
    engine.render_seconds(0.1);
    unsafe { assert!(gooey_engine_macro_set_value(engine.0, 0, 0.3)) };
    assert_eq!(engine.state(0), MACRO_LFO_STATE_STOPPED);
    engine.render_seconds(0.5);
    close(engine.value(0), 0.3, 1e-6);
    close(engine.hihat_tone(), 0.3, 1e-6);
}

#[test]
fn lfo_start_takes_the_macro_from_its_motion() {
    let engine = Engine::new();
    unsafe {
        assert!(gooey_engine_motion_configure(engine.0, 3, 0, 1.0));
        assert!(gooey_engine_motion_set_duration(
            engine.0,
            3,
            MOTION_DURATION_MS,
            2000.0
        ));
        assert!(gooey_engine_motion_trigger(engine.0, 3));
    }
    engine.render_seconds(0.1);
    engine.start(0, MACRO_LFO_SHAPE_SAW, LFO_TIMING_QUARTER);
    engine.render_seconds(0.1);
    assert_eq!(
        unsafe { gooey_engine_motion_get_state(engine.0, 3) },
        MOTION_STATE_IDLE
    );
    close(
        engine.value(0),
        wave(MACRO_LFO_SHAPE_SAW, engine.phase(0) as f64),
        1e-5,
    );
    phase_close(engine.phase(0), 0.2, 1e-3);
}

#[test]
fn capture_commit_stops_the_macro_lfo() {
    let engine = Engine::new();
    engine.start(0, MACRO_LFO_SHAPE_SINE, LFO_TIMING_QUARTER);
    engine.render_seconds(0.1);
    unsafe {
        assert!(gooey_engine_macro_capture_begin(engine.0));
        gooey_engine_set_hihat_param(engine.0, HIHAT_PARAM_TONE, 0.9);
        assert_eq!(
            gooey_engine_macro_capture_commit(engine.0, 0, MACRO_CAPTURE_REPLACE),
            1
        );
    }
    assert_eq!(engine.state(0), MACRO_LFO_STATE_STOPPED);
    engine.render_seconds(0.3);
    assert_eq!(engine.value(0), 0.0);
}

#[test]
fn poly_projection_follows_lfo_ownership() {
    let engine = Engine::new();
    engine.map(0, PARAM_TARGET_POLY, 0, POLY_PARAM_FILTER_CUTOFF, 0.9, 0.1);
    unsafe { assert!(gooey_engine_macro_set_value(engine.0, 0, 0.5)) };
    close(engine.poly_cutoff(), 0.5, 1e-6);

    // While running, the projection holds the start value (macro 0).
    engine.start(0, MACRO_LFO_SHAPE_TRIANGLE, LFO_TIMING_HALF);
    close(engine.poly_cutoff(), 0.9, 1e-6);
    engine.render_seconds(0.3);
    close(engine.poly_cutoff(), 0.9, 1e-6);

    // Stopping projects the held value, which a later unrelated preset edit
    // keeps.
    let held = engine.value(0);
    assert!(held > 0.1, "{held}");
    unsafe { assert!(gooey_engine_macro_lfo_stop(engine.0, 0)) };
    let mapped = 0.9 - 0.8 * held;
    close(engine.poly_cutoff(), mapped, 1e-6);
    unsafe {
        assert!(gooey_engine_poly_set_param(
            engine.0,
            POLY_PARAM_VOLUME,
            0.4
        ))
    };
    engine.render_seconds(0.05);
    close(engine.poly_cutoff(), mapped, 1e-6);
    assert_eq!(engine.value(0), held);
}

#[test]
fn routed_drum_lfo_still_wins_over_a_macro_lfo() {
    let engine = Engine::new();
    engine.map_hihat_tone(0, 0.0, 0.2);
    engine.start(0, MACRO_LFO_SHAPE_SAW, LFO_TIMING_QUARTER);
    unsafe {
        // A flat routed LFO (amount 0, offset 0) holds the tone at mid-range.
        gooey_engine_set_lfo_amount(engine.0, 0, 0.0);
        gooey_engine_set_lfo_offset(engine.0, 0, 0.0);
        assert_ne!(
            gooey_engine_add_lfo_route(engine.0, 0, INSTRUMENT_HIHAT, HIHAT_PARAM_TONE, 1.0),
            LFO_INVALID
        );
        gooey_engine_set_lfo_enabled(engine.0, 0, true);
    }
    for _ in 0..10 {
        engine.render_seconds(0.043);
        close(engine.hihat_tone(), 0.5, 1e-6);
    }
    assert!(engine.value(0) > 0.0, "the macro itself keeps cycling");
}

#[test]
fn concurrent_control_calls_while_rendering_stay_coherent() {
    let engine = Engine::new();
    engine.map_hihat_tone(0, 0.1, 0.9);
    engine.map(0, PARAM_TARGET_POLY, 0, POLY_PARAM_FILTER_CUTOFF, 0.9, 0.1);
    unsafe {
        assert!(gooey_engine_motion_configure(engine.0, 0, 0, 1.0));
        assert!(gooey_engine_motion_set_duration(
            engine.0,
            0,
            MOTION_DURATION_MS,
            5.0
        ));
    }

    // Raw pointers are not `Send`; the threads share the address, as a host's
    // audio and UI threads share the engine.
    let address = engine.0 as usize;
    let rendering = Arc::new(AtomicBool::new(true));
    let barrier = Arc::new(Barrier::new(3));

    let render = {
        let (rendering, barrier) = (rendering.clone(), barrier.clone());
        thread::spawn(move || {
            let engine = address as *mut GooeyEngine;
            let mut buffer = vec![0.0_f32; 256 * 2];
            barrier.wait();
            while rendering.load(Ordering::Relaxed) {
                unsafe { gooey_engine_render(engine, buffer.as_mut_ptr(), 256) };
                assert!(buffer.iter().all(|sample| sample.is_finite()));
            }
        })
    };
    let lfo_host = {
        let barrier = barrier.clone();
        thread::spawn(move || {
            let engine = address as *const GooeyEngine;
            barrier.wait();
            for i in 0..2000_u32 {
                let macro_index = i % 3;
                unsafe {
                    gooey_engine_macro_lfo_set_shape(engine, macro_index, SHAPES[i as usize % 4]);
                    gooey_engine_macro_lfo_set_rate(engine, macro_index, RATES[i as usize % 7]);
                    gooey_engine_macro_lfo_start(engine, macro_index);
                    gooey_engine_macro_lfo_reset_phase(engine, macro_index);
                    if i % 5 == 0 {
                        gooey_engine_macro_lfo_stop(engine, macro_index);
                    }
                    let _ = gooey_engine_macro_lfo_get_state(engine, macro_index);
                    let _ = gooey_engine_macro_lfo_get_value(engine, macro_index);
                    let _ = gooey_engine_macro_lfo_get_phase(engine, macro_index);
                }
            }
        })
    };
    let macro_host = {
        let barrier = barrier.clone();
        thread::spawn(move || {
            let engine = address as *const GooeyEngine;
            barrier.wait();
            for i in 0..2000_u32 {
                unsafe {
                    gooey_engine_macro_set_value(engine, i % 2, (i % 100) as f32 / 100.0);
                    if i % 7 == 0 {
                        gooey_engine_motion_trigger(engine, 0);
                    }
                    if i % 11 == 0 {
                        gooey_engine_macro_lfo_stop_all(engine);
                    }
                    let _ = gooey_engine_macro_get_value(engine, 0);
                }
            }
        })
    };
    lfo_host.join().unwrap();
    macro_host.join().unwrap();
    rendering.store(false, Ordering::Relaxed);
    render.join().unwrap();

    // The engine is still fully controllable and coherent afterwards.
    unsafe {
        assert!(gooey_engine_motion_stop_all(engine.0));
        assert!(gooey_engine_macro_lfo_stop_all(engine.0));
    }
    engine.render_seconds(0.05);
    engine.start(0, MACRO_LFO_SHAPE_SINE, LFO_TIMING_QUARTER);
    engine.render_seconds(0.17);
    let held = engine.value(0);
    unsafe { assert!(gooey_engine_macro_lfo_stop(engine.0, 0)) };
    engine.render_seconds(0.1);
    for macro_index in 0..MACRO_COUNT {
        assert_eq!(engine.state(macro_index), MACRO_LFO_STATE_STOPPED);
    }
    assert_eq!(engine.value(0), held);
    close(engine.hihat_tone(), 0.1 + 0.8 * held, 1e-6);
    close(engine.poly_cutoff(), 0.9 - 0.8 * held, 1e-6);
}
