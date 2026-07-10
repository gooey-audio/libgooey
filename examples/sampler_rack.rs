//! Sampler Rack CLI validation (no external audio files required).
//!
//! Run with:
//! `cargo run --example sampler_rack --features native,crossterm`
//!
//! The program synthesizes two short PCM pads, routes the registered rack to a
//! graph track, renders one bar, then records and queries a manual pad hit.

use gooey::ffi::*;

fn pad(sample_rate: f32, hz: f32) -> Vec<f32> {
    let frames = (sample_rate * 0.18) as usize;
    (0..frames)
        .map(|i| {
            let t = i as f32 / sample_rate;
            let envelope = (1.0 - i as f32 / frames as f32).powi(2);
            (t * hz * std::f32::consts::TAU).sin() * envelope * 0.7
        })
        .collect()
}

fn main() {
    const SAMPLE_RATE: f32 = 44_100.0;
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        let rack = gooey_engine_sampler_register(engine);
        assert!(rack >= 0, "sampler rack registration failed");
        let rack = rack as u32;
        assert!(gooey_engine_mixer_route_source(
            engine,
            gooey_engine_sampler_get_source_id(engine, rack),
            3, // existing Loops bus in the default layout
        ));

        let low = pad(SAMPLE_RATE, 110.0);
        let high = pad(SAMPLE_RATE, 440.0);
        assert!(gooey_engine_sampler_set_slot_buffer(
            engine,
            rack,
            0,
            low.as_ptr(),
            low.len() as u32,
            1,
            SAMPLE_RATE
        ));
        assert!(gooey_engine_sampler_set_slot_buffer(
            engine,
            rack,
            1,
            high.as_ptr(),
            high.len() as u32,
            1,
            SAMPLE_RATE
        ));
        for step in 0..16 {
            let enabled = step % 4 == 0 || step % 4 == 2;
            assert!(gooey_engine_sampler_set_step(
                engine,
                rack,
                step,
                enabled,
                (step / 2 % 2) as u32,
                0.9
            ));
        }

        gooey_engine_set_master_gain(engine, 1.0);
        gooey_engine_perf_set_record_mode(engine, PERF_RECORD_MODE_OVERDUB);
        gooey_engine_perf_set_record_armed(engine, true);
        gooey_engine_sequencer_start(engine);
        let mut output = vec![0.0_f32; (SAMPLE_RATE as usize * 2) * 2]; // one stereo bar at 120 BPM
        gooey_engine_render(engine, output.as_mut_ptr(), (SAMPLE_RATE as u32) * 2);
        let peak = output
            .iter()
            .map(|sample| sample.abs())
            .fold(0.0_f32, f32::max);

        gooey_engine_sampler_trigger(engine, rack, 1, 0.7);
        println!(
            "Sampler rack {rack} rendered: peak={peak:.3}, recorded_sampler_hits={}",
            gooey_engine_perf_get_sampler_event_count(engine)
        );
        println!("Expected: non-zero peak and recorded_sampler_hits=1.");
        gooey_engine_free(engine);
    }
}
