// Integration tests for `gooey_engine_sequencer_start_at_host_time` and
// `gooey_engine_set_render_host_time`. Drives the engine through the FFI
// surface that iOS/Ripple uses for Ableton Link Sync Start/Stop.

use gooey::ffi::{
    gooey_engine_drain_midi_events, gooey_engine_free, gooey_engine_new, gooey_engine_render,
    gooey_engine_sequencer_set_beat_position, gooey_engine_sequencer_set_step,
    gooey_engine_sequencer_start, gooey_engine_sequencer_start_at_host_time,
    gooey_engine_set_render_host_time, GooeyMidiEvent,
};

const SAMPLE_RATE: f32 = 48_000.0;
const BUF_FRAMES: usize = 256;

/// Drain all pending MIDI events from the engine into a Vec.
unsafe fn drain_midi(engine: *mut gooey::ffi::GooeyEngine) -> Vec<(u32, u32)> {
    let mut events: Vec<GooeyMidiEvent> = (0..32)
        .map(|_| GooeyMidiEvent {
            instrument_index: u32::MAX,
            velocity: 0.0,
            sample_offset: 0,
        })
        .collect();
    let count =
        gooey_engine_drain_midi_events(engine, events.as_mut_ptr(), events.len() as u32) as usize;
    events
        .into_iter()
        .take(count)
        .map(|e| (e.instrument_index, e.sample_offset))
        .collect()
}

/// Render one buffer of `BUF_FRAMES` samples and return the buffer.
unsafe fn render_buf(engine: *mut gooey::ffi::GooeyEngine) -> Vec<f32> {
    let mut buffer = vec![0.0_f32; BUF_FRAMES];
    gooey_engine_render(engine, buffer.as_mut_ptr(), BUF_FRAMES as u32);
    buffer
}

#[test]
fn arm_in_past_fires_immediately() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        // Enable kick step 0 so a fired step produces a MIDI event we can detect
        gooey_engine_sequencer_set_step(engine, 0, true);

        let host_now: u64 = 1_000_000_000;
        let host_ticks_per_sample = 1_000_000.0_f64 / SAMPLE_RATE as f64; // arbitrary positive
        gooey_engine_set_render_host_time(engine, host_now, host_ticks_per_sample);
        // Arm at a host time that's already in the past
        gooey_engine_sequencer_start_at_host_time(engine, host_now - 1_000, 0.0);

        let _audio = render_buf(engine);
        let events = drain_midi(engine);
        assert!(
            events.iter().any(|(ch, off)| *ch == 0 && *off == 0),
            "kick step 0 should fire on sample 0 of the first buffer; got {:?}",
            events
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn arm_far_future_keeps_buffer_silent() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        gooey_engine_sequencer_set_step(engine, 0, true);

        let host_now: u64 = 1_000_000_000;
        let host_ticks_per_sample = 1_000_000.0_f64 / SAMPLE_RATE as f64;
        gooey_engine_set_render_host_time(engine, host_now, host_ticks_per_sample);

        // Arm one full second in the future at this fake clock rate
        let one_second_in_ticks = (host_ticks_per_sample * SAMPLE_RATE as f64) as u64;
        gooey_engine_sequencer_start_at_host_time(engine, host_now + one_second_in_ticks, 0.0);

        let audio = render_buf(engine);
        // Whole buffer must be silent
        assert!(
            audio.iter().all(|s| *s == 0.0),
            "armed-but-not-yet-firing buffer must be entirely silent"
        );
        // No MIDI events emitted
        let events = drain_midi(engine);
        assert!(
            events.is_empty(),
            "no MIDI events should fire while still armed; got {:?}",
            events
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn arm_fires_mid_buffer() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        gooey_engine_sequencer_set_step(engine, 0, true);

        // Pick host_ticks_per_sample = 1.0 so 100 host ticks == 100 samples.
        let host_now: u64 = 1_000_000_000;
        let host_ticks_per_sample = 1.0_f64;
        gooey_engine_set_render_host_time(engine, host_now, host_ticks_per_sample);

        // Arm 100 samples into the future
        gooey_engine_sequencer_start_at_host_time(engine, host_now + 100, 0.0);

        let _audio = render_buf(engine);
        let events = drain_midi(engine);
        assert!(
            events.iter().any(|(ch, off)| *ch == 0 && *off == 100),
            "kick step 0 should fire at sample offset 100; got {:?}",
            events
        );
    }
}

#[test]
fn set_beat_position_cancels_pending_arm() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        gooey_engine_sequencer_set_step(engine, 0, true);

        let host_now: u64 = 1_000_000_000;
        gooey_engine_set_render_host_time(engine, host_now, 1.0);

        // Arm well into the future
        gooey_engine_sequencer_start_at_host_time(engine, host_now + 10_000, 0.0);
        // Cancel via set_beat_position
        gooey_engine_sequencer_set_beat_position(engine, 0.0);

        // Without an arm, sequencer is stopped. Render produces silence and no MIDI.
        let audio = render_buf(engine);
        assert!(audio.iter().all(|s| *s == 0.0));
        let events = drain_midi(engine);
        assert!(
            events.is_empty(),
            "set_beat_position must cancel the arm and not produce events; got {:?}",
            events
        );

        // Now manually start; first render should fire step 0
        gooey_engine_sequencer_start(engine);
        let _audio = render_buf(engine);
        let events = drain_midi(engine);
        assert!(
            events.iter().any(|(ch, off)| *ch == 0 && *off == 0),
            "after manual start, sequencer should be running; got {:?}",
            events
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn set_beat_position_does_not_fire_intermediate_steps() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        // Enable several steps; if set_beat_position fired intermediates,
        // multiple MIDI events would appear.
        for step in 0..16u32 {
            gooey_engine_sequencer_set_step(engine, step, true);
        }

        // Jump from beat 0.0 to beat 2.5 (would cross 10 step boundaries
        // if it fired them).
        gooey_engine_sequencer_set_beat_position(engine, 2.5);

        // Render one sample without starting — set_beat_position alone
        // must never produce triggers.
        let mut sample = [0.0_f32; 1];
        gooey_engine_render(engine, sample.as_mut_ptr(), 1);
        let events = drain_midi(engine);
        assert!(
            events.is_empty(),
            "set_beat_position must not fire any intermediate steps; got {:?}",
            events
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn arm_without_host_clock_falls_back_to_immediate() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        gooey_engine_sequencer_set_step(engine, 0, true);

        // Note: we never call gooey_engine_set_render_host_time. The
        // documented fail-safe is to fire immediately.
        gooey_engine_sequencer_start_at_host_time(engine, 1_000_000_000, 0.0);

        let _audio = render_buf(engine);
        let events = drain_midi(engine);
        assert!(
            events.iter().any(|(ch, off)| *ch == 0 && *off == 0),
            "without a host clock reference the arm must fire immediately; got {:?}",
            events
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn arm_lands_at_specified_beat_position() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        // Only enable step 4 (= beat 1.0) so we can verify the cursor
        // landed where requested.
        gooey_engine_sequencer_set_step(engine, 4, true);

        let host_now: u64 = 1_000_000_000;
        gooey_engine_set_render_host_time(engine, host_now, 1.0);
        // Arm 50 samples in the future to land on beat 1.0 (step 4).
        gooey_engine_sequencer_start_at_host_time(engine, host_now + 50, 1.0);

        let _audio = render_buf(engine);
        let events = drain_midi(engine);
        assert!(
            events.iter().any(|(ch, off)| *ch == 0 && *off == 50),
            "step 4 should fire at sample 50 (kick channel 0); got {:?}",
            events
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn arm_persists_across_silent_buffers() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        gooey_engine_sequencer_set_step(engine, 0, true);

        // Place the arm 1.5 buffers into the future at host_ticks_per_sample = 1.0.
        let host_ticks_per_sample = 1.0_f64;
        let mut host_now: u64 = 1_000_000_000;
        let arm_offset_samples = (BUF_FRAMES + BUF_FRAMES / 2) as u64;
        let arm_host_time = host_now + arm_offset_samples;

        // Arm first, then drive renders with advancing host clock.
        gooey_engine_set_render_host_time(engine, host_now, host_ticks_per_sample);
        gooey_engine_sequencer_start_at_host_time(engine, arm_host_time, 0.0);

        // Buffer 1: completely silent.
        gooey_engine_set_render_host_time(engine, host_now, host_ticks_per_sample);
        let audio_1 = render_buf(engine);
        assert!(
            audio_1.iter().all(|s| *s == 0.0),
            "buffer 1 should be entirely silent (arm not yet reached)"
        );
        let events_1 = drain_midi(engine);
        assert!(
            events_1.is_empty(),
            "buffer 1 should produce no MIDI events"
        );

        host_now += BUF_FRAMES as u64;

        // Buffer 2: arm fires at sample BUF_FRAMES/2.
        gooey_engine_set_render_host_time(engine, host_now, host_ticks_per_sample);
        let _audio_2 = render_buf(engine);
        let events_2 = drain_midi(engine);
        let expected_offset = (BUF_FRAMES / 2) as u32;
        assert!(
            events_2
                .iter()
                .any(|(ch, off)| *ch == 0 && *off == expected_offset),
            "buffer 2 should fire kick at sample {}; got {:?}",
            expected_offset,
            events_2
        );

        gooey_engine_free(engine);
    }
}
