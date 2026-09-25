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
fn amp_envelope_round_trips_and_defaults_to_one_second_hold() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let rack = gooey_engine_sampler_register(engine) as u32;

        // Defaults: 1 ms attack, 1 s hold, 50 ms release.
        let (mut a, mut h, mut r) = (-1.0_f32, -1.0_f32, -1.0_f32);
        assert!(gooey_engine_sampler_get_amp_envelope(
            engine, rack, &mut a, &mut h, &mut r
        ));
        assert!((a - 0.001).abs() < 1e-6);
        assert!((h - 1.0).abs() < 1e-6);
        assert!((r - 0.05).abs() < 1e-6);

        // A custom configuration round-trips exactly.
        assert!(gooey_engine_sampler_set_amp_envelope(
            engine, rack, 0.01, 2.5, 0.2
        ));
        assert!(gooey_engine_sampler_get_amp_envelope(
            engine, rack, &mut a, &mut h, &mut r
        ));
        assert!((a - 0.01).abs() < 1e-6 && (h - 2.5).abs() < 1e-6 && (r - 0.2).abs() < 1e-6);

        gooey_engine_free(engine);
    }
}

#[test]
fn amp_envelope_rejects_invalid_input_without_mutation() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let rack = gooey_engine_sampler_register(engine) as u32;
        assert!(gooey_engine_sampler_set_amp_envelope(
            engine, rack, 0.01, 0.5, 0.1
        ));

        for (a, h, r) in [
            (-0.01, 0.5, 0.1),
            (0.01, -0.5, 0.1),
            (0.01, 0.5, -0.1),
            (f32::NAN, 0.5, 0.1),
            (0.01, f32::INFINITY, 0.1),
        ] {
            assert!(!gooey_engine_sampler_set_amp_envelope(
                engine, rack, a, h, r
            ));
        }
        // An invalid rack index is also rejected.
        assert!(!gooey_engine_sampler_set_amp_envelope(
            engine,
            SAMPLER_RACK_MAX + 1,
            0.01,
            0.5,
            0.1
        ));

        // The last valid configuration survived every rejected update.
        let (mut a, mut h, mut r) = (0.0_f32, 0.0_f32, 0.0_f32);
        assert!(gooey_engine_sampler_get_amp_envelope(
            engine, rack, &mut a, &mut h, &mut r
        ));
        assert!((a - 0.01).abs() < 1e-6 && (h - 0.5).abs() < 1e-6 && (r - 0.1).abs() < 1e-6);

        // Null output pointers fail the getter.
        assert!(!gooey_engine_sampler_get_amp_envelope(
            engine,
            rack,
            std::ptr::null_mut(),
            &mut h,
            &mut r
        ));

        gooey_engine_free(engine);
    }
}

#[test]
fn default_hold_caps_a_long_sample_but_a_longer_hold_extends_it() {
    unsafe {
        // A 3-second DC pad — far longer than the default ~1.051 s envelope.
        let frames = (SR * 3.0) as usize;
        let pcm = vec![0.5_f32; frames];

        // Default envelope: the tail past ~1.051 s must be silent (capped).
        let engine = gooey_engine_new(SR);
        let rack = gooey_engine_sampler_register(engine) as u32;
        let source = gooey_engine_sampler_get_source_id(engine, rack);
        assert!(gooey_engine_mixer_route_source(engine, source, 3));
        assert!(gooey_engine_sampler_set_slot_buffer(
            engine,
            rack,
            0,
            pcm.as_ptr(),
            frames as u32,
            1,
            SR
        ));
        assert!(gooey_engine_sampler_trigger(engine, rack, 0, 1.0));
        let _ = render(engine, (SR * 1.3) as usize); // render through the cap
        assert!(
            peak(&render(engine, 4_096)) < 1e-4,
            "default hold must cap a long sample"
        );
        gooey_engine_free(engine);

        // A 2.5 s hold keeps the same pad sounding well past 1.3 s.
        let engine = gooey_engine_new(SR);
        let rack = gooey_engine_sampler_register(engine) as u32;
        let source = gooey_engine_sampler_get_source_id(engine, rack);
        assert!(gooey_engine_mixer_route_source(engine, source, 3));
        assert!(gooey_engine_sampler_set_slot_buffer(
            engine,
            rack,
            0,
            pcm.as_ptr(),
            frames as u32,
            1,
            SR
        ));
        assert!(gooey_engine_sampler_set_amp_envelope(
            engine, rack, 0.001, 2.5, 0.05
        ));
        assert!(gooey_engine_sampler_trigger(engine, rack, 0, 1.0));
        let _ = render(engine, (SR * 1.3) as usize);
        assert!(
            peak(&render(engine, 4_096)) > 0.01,
            "a longer hold must keep the pad sounding"
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn shortening_hold_on_an_active_hit_releases_without_a_click() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let rack = gooey_engine_sampler_register(engine) as u32;
        let source = gooey_engine_sampler_get_source_id(engine, rack);
        assert!(gooey_engine_mixer_route_source(engine, source, 3));
        let frames = (SR * 3.0) as usize;
        let pcm = vec![0.5_f32; frames];
        assert!(gooey_engine_sampler_set_slot_buffer(
            engine,
            rack,
            0,
            pcm.as_ptr(),
            frames as u32,
            1,
            SR
        ));
        // Long hold, then trigger and settle into the sustained portion.
        assert!(gooey_engine_sampler_set_amp_envelope(
            engine, rack, 0.001, 5.0, 0.1
        ));
        assert!(gooey_engine_sampler_trigger(engine, rack, 0, 1.0));
        let _ = render(engine, 8_192);

        // Shorten the hold while the hit sounds — this begins release now.
        assert!(gooey_engine_sampler_set_amp_envelope(
            engine, rack, 0.001, 0.0, 0.1
        ));
        let tail = render(engine, (SR * 0.2) as usize);
        let max_step = tail
            .chunks(2)
            .map(|f| f[0])
            .collect::<Vec<_>>()
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0_f32, f32::max);
        assert!(max_step < 0.05, "release must not click: {max_step}");
        // And the release actually completes to silence.
        assert!(
            peak(&render(engine, 4_096)) < 1e-4,
            "release reaches silence"
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
        let host_now = 1_000u64;
        gooey_engine_set_render_host_time(engine, host_now, 1.0);
        gooey_engine_sequencer_start_at_host_time(engine, host_now + 100, 0.0);
        let output = render(engine, 256);
        assert!(output[..100 * 2].iter().all(|sample| *sample == 0.0));
        assert!(peak(&output[100 * 2..]) > 0.001);
        gooey_engine_free(engine);
    }
}
