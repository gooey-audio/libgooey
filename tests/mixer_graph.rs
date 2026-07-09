//! End-to-end tests for the host-facing mixer graph FFI.

use std::ffi::{CStr, CString};

use gooey::ffi::*;

const SAMPLE_RATE: f32 = 44_100.0;

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap()
}

fn stereo_sine(seconds: f32, hz: f32) -> Vec<f32> {
    let frames = (SAMPLE_RATE * seconds) as usize;
    let mut out = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let s = (i as f32 / SAMPLE_RATE * hz * std::f32::consts::TAU).sin() * 0.5;
        out.push(s);
        out.push(s);
    }
    out
}

unsafe fn load_loop(engine: *mut GooeyEngine, hz: f32) {
    let samples = stereo_sine(0.5, hz);
    let frames = (samples.len() / 2) as u32;
    assert!(gooey_engine_loop_load(
        engine,
        0,
        samples.as_ptr(),
        frames,
        2,
        SAMPLE_RATE,
    ));
    gooey_engine_loop_set_playing(engine, 0, true);
}

unsafe fn render_peak(engine: *mut GooeyEngine, frames: usize) -> f32 {
    let mut buffer = vec![0.0_f32; frames * 2];
    gooey_engine_render(engine, buffer.as_mut_ptr(), frames as u32);
    buffer.iter().fold(0.0, |acc, sample| {
        assert!(sample.is_finite(), "render output must stay finite");
        acc.max(sample.abs())
    })
}

unsafe fn render_tail_peak(engine: *mut GooeyEngine, frames: usize) -> f32 {
    let mut buffer = vec![0.0_f32; frames * 2];
    gooey_engine_render(engine, buffer.as_mut_ptr(), frames as u32);
    buffer[frames..].iter().fold(0.0, |acc, sample| {
        assert!(sample.is_finite(), "render output must stay finite");
        acc.max(sample.abs())
    })
}

unsafe fn bounce(engine: *mut GooeyEngine, bars: u32) -> Vec<f32> {
    let mut len = 0;
    let ptr = gooey_engine_bounce_to_buffer(engine, bars, &mut len);
    assert!(!ptr.is_null(), "bounce should return a buffer");
    let out = std::slice::from_raw_parts(ptr, len as usize).to_vec();
    gooey_engine_free_buffer(ptr, len);
    out
}

fn max_abs(buffer: &[f32]) -> f32 {
    buffer.iter().map(|sample| sample.abs()).fold(0.0, f32::max)
}

unsafe fn track_name(engine: *const GooeyEngine, track: u32) -> String {
    let ptr = gooey_engine_mixer_get_track_name(engine, track);
    assert!(!ptr.is_null(), "track {track} should have a name");
    CStr::from_ptr(ptr).to_str().unwrap().to_owned()
}

#[test]
fn default_layout_names_routes_and_source_constants() {
    unsafe {
        assert_eq!(SOURCE_DRUMKIT, 0);
        assert_eq!(SOURCE_BASS, 1);
        assert_eq!(SOURCE_POLYSYNTH, 2);
        assert_eq!(SOURCE_GRANULATOR, 3);
        assert_eq!(SOURCE_LOOPMIXER, 4);
        assert_eq!(SOURCE_COUNT, 5);

        let engine = gooey_engine_new(SAMPLE_RATE);
        assert_eq!(gooey_engine_mixer_get_track_count(engine), 4);
        assert_eq!(track_name(engine, 0), "Drums");
        assert_eq!(track_name(engine, 1), "Bass");
        assert_eq!(track_name(engine, 2), "Synth");
        assert_eq!(track_name(engine, 3), "Loops");
        assert!(gooey_engine_mixer_get_track_name(engine, 99).is_null());

        assert_eq!(
            gooey_engine_mixer_get_source_route(engine, SOURCE_DRUMKIT),
            0
        );
        assert_eq!(gooey_engine_mixer_get_source_route(engine, SOURCE_BASS), 1);
        assert_eq!(
            gooey_engine_mixer_get_source_route(engine, SOURCE_POLYSYNTH),
            2
        );
        assert_eq!(
            gooey_engine_mixer_get_source_route(engine, SOURCE_GRANULATOR),
            3
        );
        assert_eq!(
            gooey_engine_mixer_get_source_route(engine, SOURCE_LOOPMIXER),
            3
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn layout_editing_round_trips_and_reset_restores_defaults() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);

        gooey_engine_mixer_clear_layout(engine);
        assert_eq!(gooey_engine_mixer_get_track_count(engine), 0);
        assert_eq!(
            gooey_engine_mixer_get_source_route(engine, SOURCE_DRUMKIT),
            -1
        );

        let drums = cstr("Drums A");
        let track = gooey_engine_mixer_add_track(engine, drums.as_ptr());
        assert_eq!(track, 0);
        assert_eq!(track_name(engine, 0), "Drums A");
        assert_eq!(gooey_engine_mixer_find_track(engine, drums.as_ptr()), 0);

        let renamed = cstr("Kit Bus");
        assert!(gooey_engine_mixer_set_track_name(
            engine,
            0,
            renamed.as_ptr()
        ));
        assert_eq!(track_name(engine, 0), "Kit Bus");
        assert_eq!(gooey_engine_mixer_find_track(engine, drums.as_ptr()), -1);
        assert_eq!(gooey_engine_mixer_find_track(engine, renamed.as_ptr()), 0);

        gooey_engine_mixer_reset_default_layout(engine);
        assert_eq!(gooey_engine_mixer_get_track_count(engine), 4);
        assert_eq!(track_name(engine, 0), "Drums");
        assert_eq!(
            gooey_engine_mixer_get_source_route(engine, SOURCE_DRUMKIT),
            0
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn routing_and_invalid_inputs_are_safe() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        let bus = cstr("Extra");
        let extra = gooey_engine_mixer_add_track(engine, bus.as_ptr()) as u32;

        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_LOOPMIXER,
            extra
        ));
        assert_eq!(
            gooey_engine_mixer_get_source_route(engine, SOURCE_LOOPMIXER),
            extra as i32
        );
        assert!(gooey_engine_mixer_unroute_source(engine, SOURCE_LOOPMIXER));
        assert_eq!(
            gooey_engine_mixer_get_source_route(engine, SOURCE_LOOPMIXER),
            -1
        );
        assert!(!gooey_engine_mixer_unroute_source(engine, SOURCE_LOOPMIXER));
        assert!(!gooey_engine_mixer_route_source(engine, SOURCE_COUNT, 0));
        assert!(!gooey_engine_mixer_route_source(engine, SOURCE_DRUMKIT, 99));

        assert_eq!(gooey_engine_mixer_get_track_count(std::ptr::null()), 0);
        assert_eq!(
            gooey_engine_mixer_add_track(std::ptr::null_mut(), bus.as_ptr()),
            -1
        );
        assert_eq!(gooey_engine_mixer_add_track(engine, std::ptr::null()), -1);
        assert!(!gooey_engine_mixer_set_track_name(
            engine,
            0,
            std::ptr::null()
        ));
        assert_eq!(gooey_engine_mixer_find_track(engine, std::ptr::null()), -1);
        assert!(!gooey_engine_mixer_route_source(
            std::ptr::null_mut(),
            SOURCE_DRUMKIT,
            0
        ));
        assert_eq!(
            gooey_engine_mixer_get_source_route(std::ptr::null(), SOURCE_DRUMKIT),
            -1
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn track_strip_controls_round_trip_and_clamp() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);

        gooey_engine_mixer_set_track_gain(engine, 0, 3.0);
        assert_eq!(gooey_engine_mixer_get_track_gain(engine, 0), 2.0);
        gooey_engine_mixer_set_track_gain(engine, 0, -1.0);
        assert_eq!(gooey_engine_mixer_get_track_gain(engine, 0), 0.0);
        assert_eq!(gooey_engine_mixer_get_track_gain(engine, 99), 1.0);

        gooey_engine_mixer_set_track_pan(engine, 0, 2.0);
        assert_eq!(gooey_engine_mixer_get_track_pan(engine, 0), 1.0);
        gooey_engine_mixer_set_track_pan(engine, 0, -1.0);
        assert_eq!(gooey_engine_mixer_get_track_pan(engine, 0), 0.0);
        assert_eq!(gooey_engine_mixer_get_track_pan(engine, 99), 0.5);

        gooey_engine_mixer_set_track_mute(engine, 0, true);
        gooey_engine_mixer_set_track_solo(engine, 0, true);
        assert!(gooey_engine_mixer_get_track_mute(engine, 0));
        assert!(gooey_engine_mixer_get_track_solo(engine, 0));
        assert!(!gooey_engine_mixer_get_track_mute(engine, 99));
        assert!(!gooey_engine_mixer_get_track_solo(engine, 99));
        assert_eq!(gooey_engine_mixer_get_track_peak(engine, 99), 0.0);

        gooey_engine_free(engine);
    }
}

#[test]
fn track_effect_chain_edit_operations() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);

        assert_eq!(
            gooey_engine_track_effect_add(engine, 0, EFFECT_LOWPASS_FILTER),
            0
        );
        assert_eq!(gooey_engine_track_effect_add(engine, 0, EFFECT_DELAY), 1);
        assert_eq!(gooey_engine_track_effect_add(engine, 0, EFFECT_REVERB), 2);
        assert_eq!(gooey_engine_track_effect_count(engine, 0), 3);

        assert!(gooey_engine_track_effect_move(engine, 0, 2, 0));
        assert_eq!(
            gooey_engine_track_effect_type_at(engine, 0, 0),
            EFFECT_REVERB as i32
        );
        assert_eq!(
            gooey_engine_track_effect_type_at(engine, 0, 1),
            EFFECT_LOWPASS_FILTER as i32
        );

        gooey_engine_track_effect_set_param(engine, 0, 1, FILTER_PARAM_CUTOFF, 300.0);
        assert!(gooey_engine_track_effect_remove(engine, 0, 1));
        assert_eq!(gooey_engine_track_effect_count(engine, 0), 2);
        assert!(!gooey_engine_track_effect_remove(engine, 0, 99));

        gooey_engine_track_effect_clear(engine, 0);
        assert_eq!(gooey_engine_track_effect_count(engine, 0), 0);
        assert_eq!(gooey_engine_track_effect_type_at(engine, 0, 0), -1);
        assert_eq!(gooey_engine_track_effect_add(engine, 99, EFFECT_DELAY), -1);

        gooey_engine_free(engine);
    }
}

#[test]
fn track_gain_and_mute_silence_only_their_routed_source() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        load_loop(engine, 220.0);

        let audible_loop = render_tail_peak(engine, 8192);
        assert!(audible_loop > 1e-3);

        gooey_engine_mixer_set_track_mute(engine, 3, true);
        let muted_loop = render_tail_peak(engine, 8192);
        assert!(
            muted_loop < audible_loop * 0.02,
            "muted loop track should silence loop source: audible {audible_loop}, muted {muted_loop}"
        );

        gooey_engine_trigger_instrument(engine, INSTRUMENT_KICK);
        let kick = render_peak(engine, 2048);
        assert!(
            kick > 1e-4,
            "drum track should remain audible while loop track is muted: {kick}"
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn rerouting_source_changes_which_track_controls_it() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        load_loop(engine, 330.0);

        let default = render_tail_peak(engine, 8192);
        assert!(default > 1e-3);

        gooey_engine_mixer_set_track_gain(engine, 3, 0.0);
        let muted_default_route = render_tail_peak(engine, 8192);
        assert!(muted_default_route < default * 0.02);

        let new_track = gooey_engine_mixer_add_track(engine, cstr("Loop Bus").as_ptr()) as u32;
        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_LOOPMIXER,
            new_track
        ));
        let rerouted = render_tail_peak(engine, 8192);
        assert!(
            rerouted > default * 0.5,
            "new track at unity should restore rerouted loop: default {default}, rerouted {rerouted}"
        );

        gooey_engine_mixer_set_track_gain(engine, new_track, 0.0);
        let muted_new_route = render_tail_peak(engine, 8192);
        assert!(muted_new_route < rerouted * 0.02);

        gooey_engine_free(engine);
    }
}

#[test]
fn track_effect_changes_only_audio_routed_to_that_track() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        load_loop(engine, 6_000.0);

        let dry = render_tail_peak(engine, 8192);
        assert!(dry > 1e-3);

        let slot = gooey_engine_track_effect_add(engine, 3, EFFECT_LOWPASS_FILTER);
        assert_eq!(slot, 0);
        gooey_engine_track_effect_set_param(engine, 3, 0, FILTER_PARAM_CUTOFF, 300.0);
        let _ = render_tail_peak(engine, 8192);
        let filtered = render_tail_peak(engine, 8192);
        assert!(
            filtered < dry * 0.6,
            "track lowpass should attenuate bright loop: dry {dry}, filtered {filtered}"
        );

        let dry_track = gooey_engine_mixer_add_track(engine, cstr("Dry Loop").as_ptr()) as u32;
        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_LOOPMIXER,
            dry_track
        ));
        let rerouted_dry = render_tail_peak(engine, 8192);
        assert!(
            rerouted_dry > filtered * 1.5,
            "rerouted loop should bypass old track effect: filtered {filtered}, rerouted {rerouted_dry}"
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn track_peaks_report_and_reset_after_render() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);

        gooey_engine_trigger_instrument(engine, INSTRUMENT_KICK);
        let audible = render_peak(engine, 2048);
        assert!(audible > 1e-4);

        let peak = gooey_engine_mixer_get_track_peak(engine, 0);
        assert!(peak > 1e-4, "drum track peak should be nonzero: {peak}");
        assert_eq!(gooey_engine_mixer_get_track_peak(engine, 0), 0.0);

        gooey_engine_free(engine);
    }
}

#[test]
fn offline_bounce_snaps_recent_mixer_strip_changes() {
    unsafe {
        let gain_engine = gooey_engine_new(SAMPLE_RATE);
        gooey_engine_sequencer_set_step(gain_engine, 0, true);
        gooey_engine_mixer_set_track_gain(gain_engine, 0, 0.0);
        let gain_bounce = bounce(gain_engine, 1);
        assert_eq!(gain_bounce.len(), 88_200);
        assert!(
            max_abs(&gain_bounce) < 1e-6,
            "track gain should silence from sample zero in offline bounce, peak {}",
            max_abs(&gain_bounce)
        );
        gooey_engine_free(gain_engine);

        let mute_engine = gooey_engine_new(SAMPLE_RATE);
        gooey_engine_sequencer_set_step(mute_engine, 0, true);
        gooey_engine_mixer_set_track_mute(mute_engine, 0, true);
        let mute_bounce = bounce(mute_engine, 1);
        assert!(
            max_abs(&mute_bounce) < 1e-6,
            "track mute should silence from sample zero in offline bounce, peak {}",
            max_abs(&mute_bounce)
        );
        gooey_engine_free(mute_engine);

        let solo_engine = gooey_engine_new(SAMPLE_RATE);
        gooey_engine_sequencer_set_step(solo_engine, 0, true);
        gooey_engine_mixer_set_track_solo(solo_engine, 1, true);
        let solo_bounce = bounce(solo_engine, 1);
        assert!(
            max_abs(&solo_bounce) < 1e-6,
            "soloing another track should silence drums from sample zero in offline bounce, peak {}",
            max_abs(&solo_bounce)
        );
        gooey_engine_free(solo_engine);
    }
}
