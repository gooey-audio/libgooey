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
