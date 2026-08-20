//! Integration coverage for the FFI sampler rack API.

use gooey::ffi::*;

const SR: f32 = 44_100.0;

fn render(engine: *mut GooeyEngine, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0; frames * 2];
    unsafe { gooey_engine_render(engine, output.as_mut_ptr(), frames as u32) };
    output
}

fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0_f32, f32::max)
}

#[test]
fn registration_keeps_legacy_sources_and_has_a_fixed_limit() {
    unsafe {
        let engine = gooey_engine_new(SR);
        assert_eq!(SOURCE_COUNT, 5);
        assert_eq!(SOURCE_SAMPLER_BASE, 5);
        for rack in 0..SAMPLER_RACK_MAX {
            assert_eq!(gooey_engine_sampler_register(engine), rack as i32);
            assert_eq!(
                gooey_engine_sampler_get_source_id(engine, rack),
                SOURCE_SAMPLER_BASE + rack
            );
        }
        assert_eq!(gooey_engine_sampler_register(engine), -1);
        assert_eq!(
            gooey_engine_mixer_get_source_route(engine, SOURCE_DRUMKIT),
            0
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn registered_rack_can_route_after_default_graph_reset() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let rack = gooey_engine_sampler_register(engine) as u32;
        let source = gooey_engine_sampler_get_source_id(engine, rack);
        gooey_engine_mixer_reset_default_layout(engine);
        assert!(gooey_engine_mixer_route_source(engine, source, 3));
        assert_eq!(gooey_engine_mixer_get_source_route(engine, source), 3);
        gooey_engine_free(engine);
    }
}

#[test]
fn loaded_slot_can_be_routed_triggered_and_sequenced() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let rack = gooey_engine_sampler_register(engine) as u32;
        let source = gooey_engine_sampler_get_source_id(engine, rack);
        assert!(gooey_engine_mixer_route_source(engine, source, 3));

        let pcm = vec![0.5_f32; 4096];
        assert!(gooey_engine_sampler_set_slot_buffer(
            engine,
            rack,
            0,
            pcm.as_ptr(),
            4096,
            1,
            SR
        ));
        assert!(gooey_engine_sampler_slot_is_loaded(engine, rack, 0));
        assert_eq!(gooey_engine_sampler_slot_frames(engine, rack, 0), 4096);
        assert_eq!(gooey_engine_sampler_slot_channels(engine, rack, 0), 1);
        assert_eq!(gooey_engine_sampler_slot_sample_rate(engine, rack, 0), SR);
        assert!(gooey_engine_sampler_trigger(engine, rack, 0, 0.8));
        assert!(peak(&render(engine, 256)) > 0.01);

        assert!(gooey_engine_sampler_set_step(engine, rack, 0, true, 0, 1.0));
        let mut enabled = false;
        let mut slot = 99;
        let mut velocity = 0.0;
        assert!(gooey_engine_sampler_get_step(
            engine,
            rack,
            0,
            &mut enabled,
            &mut slot,
            &mut velocity
        ));
        assert!(enabled && slot == 0 && (velocity - 1.0).abs() < f32::EPSILON);
        gooey_engine_sequencer_start(engine);
        assert!(peak(&render(engine, 256)) > 0.01);

        assert!(gooey_engine_sampler_clear_slot(engine, rack, 0));
        assert!(!gooey_engine_sampler_slot_is_loaded(engine, rack, 0));
        assert!(!gooey_engine_sampler_trigger(engine, rack, 0, 1.0));
        gooey_engine_free(engine);
    }
}

#[test]
fn manual_sampler_hits_record_but_sequencer_hits_do_not() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let rack = gooey_engine_sampler_register(engine) as u32;
        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_SAMPLER_BASE + rack,
            3
        ));
        let pcm = vec![0.35_f32; 4096];
        assert!(gooey_engine_sampler_set_slot_buffer(
            engine,
            rack,
            0,
            pcm.as_ptr(),
            4096,
            1,
            SR
        ));
        assert!(gooey_engine_sampler_set_step(engine, rack, 0, true, 0, 1.0));
        gooey_engine_perf_set_record_mode(engine, PERF_RECORD_MODE_OVERDUB);
        gooey_engine_perf_set_record_armed(engine, true);
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 128);
        assert!(gooey_engine_perf_is_recording(engine));
        assert_eq!(
            gooey_engine_perf_get_sampler_event_count(engine),
            0,
            "sequencer must not record itself"
        );
        assert!(gooey_engine_sampler_trigger(engine, rack, 0, 0.7));
        assert_eq!(gooey_engine_perf_get_sampler_event_count(engine), 1);
        let mut start = 0;
        let mut got_rack = 99;
        let mut got_slot = 99;
        let mut velocity = 0.0;
        assert!(gooey_engine_perf_get_sampler_event(
            engine,
            0,
            &mut start,
            &mut got_rack,
            &mut got_slot,
            &mut velocity
        ));
        assert_eq!((got_rack, got_slot), (rack, 0));
        assert!((velocity - 0.7).abs() < 0.001);
        gooey_engine_free(engine);
    }
}

#[test]
fn recorded_manual_hit_replays_on_the_next_loop() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let rack = gooey_engine_sampler_register(engine) as u32;
        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_SAMPLER_BASE + rack,
            3
        ));
        let pcm = vec![0.5_f32; 4096];
        assert!(gooey_engine_sampler_set_slot_buffer(
            engine,
            rack,
            0,
            pcm.as_ptr(),
            4096,
            1,
            SR
        ));
        gooey_engine_perf_set_record_mode(engine, PERF_RECORD_MODE_OVERDUB);
        gooey_engine_perf_set_record_armed(engine, true);
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 128);
        assert!(gooey_engine_sampler_trigger(engine, rack, 0, 1.0));
        // Render the live hit out, then cross the one-bar wrap. The tail has
        // no live event left, so a non-zero result proves clip replay.
        let _ = render(engine, 5000);
        let loop_crossing = render(engine, 84_000);
        assert!(
            peak(&loop_crossing[80_000 * 2..]) > 0.01,
            "recorded hit should replay at loop start"
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn sampler_sequence_starts_when_host_time_arm_fires() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let rack = gooey_engine_sampler_register(engine) as u32;
        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_SAMPLER_BASE + rack,
            3
        ));
        let pcm = vec![0.5_f32; 4096];
        assert!(gooey_engine_sampler_set_slot_buffer(
            engine,
            rack,
            0,
            pcm.as_ptr(),
            pcm.len() as u32,
            1,
            SR
        ));
        assert!(gooey_engine_sampler_set_step(engine, rack, 0, true, 0, 1.0));
        assert!(gooey_engine_sampler_start_pattern(
            engine,
            rack,
            CLIP_QUANTIZE_BAR
        ));
        let host_now = 1_000u64;
        gooey_engine_set_render_host_time(engine, host_now, 1.0);
        gooey_engine_sequencer_start_at_host_time(engine, host_now + 100, 0.0);
        let output = render(engine, 256);
        assert!(output[..100 * 2].iter().all(|sample| *sample == 0.0));
        assert!(peak(&output[100 * 2..]) > 0.001);
        gooey_engine_free(engine);
    }
}

#[test]
fn pattern_start_is_bar_quantized_and_never_seeks_the_clip_transport() {
    unsafe {
        let engine = gooey_engine_new(SR);
        gooey_engine_set_bpm(engine, 60.0);
        let rack = gooey_engine_sampler_register(engine) as u32;
        let source = gooey_engine_sampler_get_source_id(engine, rack);
        assert!(gooey_engine_mixer_route_source(engine, source, 3));
        let pcm = vec![0.5_f32; 4096];
        assert!(gooey_engine_sampler_set_slot_buffer(
            engine,
            rack,
            0,
            pcm.as_ptr(),
            pcm.len() as u32,
            1,
            SR
        ));
        assert!(gooey_engine_sampler_set_step(engine, rack, 0, true, 0, 1.0));

        gooey_engine_sequencer_start(engine);
        let _ = render(engine, (SR / 10.0) as usize); // beat 0.1
        assert!(gooey_engine_sampler_start_pattern(
            engine,
            rack,
            CLIP_QUANTIZE_BAR
        ));
        assert_eq!(
            gooey_engine_sampler_get_pending_start_beat(engine, rack),
            4.0
        );
        assert!(!gooey_engine_sampler_is_pattern_running(engine, rack));

        let _ = render(engine, ((4.0 - 0.1) * SR as f64) as usize);
        assert!(!gooey_engine_sampler_is_pattern_running(engine, rack));
        let before = gooey_engine_transport_get_beat_position(engine);
        let _ = render(engine, 1);
        assert!(gooey_engine_sampler_is_pattern_running(engine, rack));
        assert!(
            peak(&render(engine, 64)) > 0.001,
            "step zero should fire on the boundary"
        );
        assert!(gooey_engine_transport_get_beat_position(engine) > before);

        assert!(gooey_engine_sampler_stop_pattern(engine, rack));
        assert!(!gooey_engine_sampler_is_pattern_running(engine, rack));
        assert!(gooey_engine_sampler_start_pattern(
            engine,
            rack,
            CLIP_QUANTIZE_BAR
        ));
        assert_eq!(
            gooey_engine_sampler_get_pending_start_beat(engine, rack),
            8.0
        );
        assert!(gooey_engine_sampler_cancel_pattern_start(engine, rack));
        assert_eq!(
            gooey_engine_sampler_get_pending_start_beat(engine, rack),
            -1.0
        );
        gooey_engine_free(engine);
    }
}

unsafe fn load_mono(engine: *mut GooeyEngine, rack: u32, slot: u32, frames: usize, value: f32) {
    let pcm = vec![value; frames];
    assert!(gooey_engine_sampler_set_slot_buffer(
        engine,
        rack,
        slot,
        pcm.as_ptr(),
        frames as u32,
        1,
        SR
    ));
}

#[test]
fn default_slot_params_leave_playback_unchanged() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let rack = gooey_engine_sampler_register(engine) as u32;
        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_SAMPLER_BASE + rack,
            3
        ));
        load_mono(engine, rack, 0, 4096, 0.5);
        assert!((gooey_engine_sampler_slot_gain(engine, rack, 0) - 1.0).abs() < f32::EPSILON);
        assert_eq!(gooey_engine_sampler_slot_pitch(engine, rack, 0), 0.0);
        assert!(gooey_engine_sampler_trigger(engine, rack, 0, 1.0));
        let out = render(engine, 4200);
        assert!(peak(&out) > 0.01);
        assert!(peak(&out[4150 * 2..]) < 0.01);
        gooey_engine_free(engine);
    }
}

#[test]
fn slot_gain_scales_output_and_clamps() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let rack = gooey_engine_sampler_register(engine) as u32;
        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_SAMPLER_BASE + rack,
            3
        ));
        load_mono(engine, rack, 0, 2048, 0.4);
        assert!(gooey_engine_sampler_set_slot_gain(engine, rack, 0, 2.0));
        assert!((gooey_engine_sampler_slot_gain(engine, rack, 0) - 2.0).abs() < f32::EPSILON);
        assert!(gooey_engine_sampler_set_slot_gain(engine, rack, 0, 5.0));
        assert!((gooey_engine_sampler_slot_gain(engine, rack, 0) - 2.0).abs() < f32::EPSILON);
        assert!(gooey_engine_sampler_trigger(engine, rack, 0, 1.0));
        let loud = peak(&render(engine, 256));
        assert!(loud > 0.01);
        let _ = render(engine, 4096);
        assert!(gooey_engine_sampler_set_slot_gain(engine, rack, 0, 0.25));
        assert!(gooey_engine_sampler_trigger(engine, rack, 0, 1.0));
        let quiet = peak(&render(engine, 256));
        assert!(quiet < loud);
        gooey_engine_free(engine);
    }
}

#[test]
fn slot_pitch_changes_playback_length() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let rack = gooey_engine_sampler_register(engine) as u32;
        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_SAMPLER_BASE + rack,
            3
        ));
        load_mono(engine, rack, 0, 2000, 0.5);
        assert!(gooey_engine_sampler_set_slot_pitch(engine, rack, 0, 12.0));
        assert!(gooey_engine_sampler_trigger(engine, rack, 0, 1.0));
        let out = render(engine, 1200);
        assert!(peak(&out[..800 * 2]) > 0.01);
        assert!(peak(&out[1100 * 2..]) < 0.01);
        gooey_engine_free(engine);
    }
}

#[test]
fn slot_envelope_shapes_amplitude() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let rack = gooey_engine_sampler_register(engine) as u32;
        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_SAMPLER_BASE + rack,
            3
        ));
        load_mono(engine, rack, 0, 44_100, 0.5);
        assert!(gooey_engine_sampler_set_slot_envelope(
            engine, rack, 0, 0.0, 0.01, 0.0, 0.01
        ));
        let mut a = 0.0;
        let mut d = 0.0;
        let mut s = 0.0;
        let mut r = 0.0;
        assert!(gooey_engine_sampler_get_slot_envelope(
            engine, rack, 0, &mut a, &mut d, &mut s, &mut r
        ));
        assert!((a - 0.001).abs() < 1e-6);
        assert!((s - 0.0).abs() < f32::EPSILON);
        assert!(gooey_engine_sampler_trigger(engine, rack, 0, 1.0));
        let out = render(engine, 8000);
        assert!(peak(&out[..2000 * 2]) > 0.01);
        assert!(peak(&out[4000 * 2..]) < 0.01);
        gooey_engine_free(engine);
    }
}

#[test]
fn slot_params_latch_at_trigger() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let rack = gooey_engine_sampler_register(engine) as u32;
        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_SAMPLER_BASE + rack,
            3
        ));
        load_mono(engine, rack, 0, 4096, 1.0);
        assert!(gooey_engine_sampler_trigger(engine, rack, 0, 1.0));
        let first = peak(&render(engine, 32));
        assert!(gooey_engine_sampler_set_slot_gain(engine, rack, 0, 0.1));
        let mid = peak(&render(engine, 32));
        assert!(
            (mid - first).abs() < 0.2,
            "in-flight hit should ignore live gain"
        );
        let _ = render(engine, 8192);
        assert!(gooey_engine_sampler_trigger(engine, rack, 0, 1.0));
        let next = peak(&render(engine, 64));
        assert!(next < first * 0.3);
        gooey_engine_free(engine);
    }
}

#[test]
fn invalid_rack_or_slot_rejected() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let rack = gooey_engine_sampler_register(engine) as u32;
        assert!(!gooey_engine_sampler_set_slot_gain(engine, 99, 0, 1.0));
        assert!(!gooey_engine_sampler_set_slot_gain(engine, rack, 99, 1.0));
        assert!(!gooey_engine_sampler_set_slot_gain(
            std::ptr::null_mut(),
            rack,
            0,
            1.0
        ));
        assert!(!gooey_engine_sampler_set_slot_gain(
            engine,
            rack,
            0,
            f32::NAN
        ));
        assert!(!gooey_engine_sampler_set_slot_pitch(
            engine,
            rack,
            0,
            f32::INFINITY
        ));
        assert_eq!(gooey_engine_sampler_slot_gain(engine, 99, 0), -1.0);
        assert!(!gooey_engine_sampler_set_slot_envelope(
            engine, 99, 0, 0.0, 0.0, 1.0, 0.01
        ));
        let mut a = 0.0;
        assert!(!gooey_engine_sampler_get_slot_envelope(
            engine,
            rack,
            0,
            std::ptr::null_mut(),
            &mut a,
            &mut a,
            &mut a
        ));
        gooey_engine_free(engine);
    }
}
