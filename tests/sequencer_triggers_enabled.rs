// Integration test for `gooey_engine_set_sequencer_triggers_enabled` /
// `gooey_engine_get_sequencer_triggers_enabled`. Drives the FFI surface that the
// Ripple AUv3 uses for its "SEQ" toggle: when disabled, the internal sequencer must
// keep its clock phase-locked (position keeps advancing) but stop firing instruments
// and emitting MIDI, while host-supplied triggers must keep sounding.

use gooey::ffi::{
    gooey_engine_drain_midi_events, gooey_engine_free, gooey_engine_get_sequencer_triggers_enabled,
    gooey_engine_new, gooey_engine_render, gooey_engine_sequencer_get_beat_position,
    gooey_engine_sequencer_set_step, gooey_engine_sequencer_start,
    gooey_engine_set_sequencer_triggers_enabled, gooey_engine_trigger_instrument_with_velocity,
    GooeyMidiEvent,
};

const SAMPLE_RATE: f32 = 48_000.0;
/// One second of audio — long enough to span at least one 16th-note step at any
/// sane BPM, so a step boundary is guaranteed to occur within the buffer.
const ONE_SECOND: usize = SAMPLE_RATE as usize;
const AUDIBLE: f32 = 0.01;
const SILENCE: f32 = 0.001;

/// Render `frames` samples and return the buffer.
unsafe fn render_n(engine: *mut gooey::ffi::GooeyEngine, frames: usize) -> Vec<f32> {
    // Interleaved stereo: two output samples per frame.
    let mut buffer = vec![0.0_f32; frames * 2];
    gooey_engine_render(engine, buffer.as_mut_ptr(), frames as u32);
    buffer
}

/// Drain all pending MIDI events into a Vec of (instrument_index, sample_offset).
unsafe fn drain_midi(engine: *mut gooey::ffi::GooeyEngine) -> Vec<(u32, u32)> {
    let mut events: Vec<GooeyMidiEvent> = (0..64)
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

fn max_amplitude(buffer: &[f32]) -> f32 {
    buffer.iter().map(|s| s.abs()).fold(0.0_f32, f32::max)
}

#[test]
fn disabling_triggers_mutes_sequencer_but_keeps_clock_and_host_input() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);

        // Default is enabled (sequencer plays on engine creation).
        assert!(
            gooey_engine_get_sequencer_triggers_enabled(engine),
            "sequencer triggers should default to enabled"
        );

        // Enable kick step 0 and start the sequencer.
        gooey_engine_sequencer_set_step(engine, 0, true);
        gooey_engine_sequencer_start(engine);

        // --- 1. Sequencer enabled: produces MIDI + audio. ---
        let buf = render_n(engine, ONE_SECOND);
        let events = drain_midi(engine);
        assert!(
            events.iter().any(|(ch, _)| *ch == 0),
            "enabled sequencer should fire the kick; got {events:?}"
        );
        assert!(
            max_amplitude(&buf) > AUDIBLE,
            "enabled sequencer should produce audible output"
        );

        // --- 2. Disable triggers: clock keeps advancing, but silence + no MIDI. ---
        gooey_engine_set_sequencer_triggers_enabled(engine, false);
        assert!(!gooey_engine_get_sequencer_triggers_enabled(engine));

        // The sequencer position must keep moving while muted (phase-locked clock).
        // Render a sub-step buffer so the beat position changes without wrapping a
        // full bar (beat position is bar-relative, so compare for inequality).
        let beat_before = gooey_engine_sequencer_get_beat_position(engine);
        let _ = render_n(engine, 256);
        let beat_after = gooey_engine_sequencer_get_beat_position(engine);
        assert!(
            (beat_after - beat_before).abs() > f64::EPSILON,
            "clock must keep advancing while triggers are disabled \
             (before={beat_before}, after={beat_after})"
        );
        let _ = drain_midi(engine);

        // Settle buffer: let any voices triggered while enabled ring out to silence.
        let _ = render_n(engine, ONE_SECOND);
        let settle_events = drain_midi(engine);
        assert!(
            settle_events.is_empty(),
            "disabled sequencer must not emit MIDI; got {settle_events:?}"
        );

        // Measurement buffer: now genuinely silent (tails decayed) and still no MIDI.
        let muted_buf = render_n(engine, ONE_SECOND);
        let muted_events = drain_midi(engine);
        assert!(
            muted_events.is_empty(),
            "disabled sequencer must not emit MIDI; got {muted_events:?}"
        );
        assert!(
            max_amplitude(&muted_buf) < SILENCE,
            "disabled sequencer should produce silence, got {}",
            max_amplitude(&muted_buf)
        );

        // --- 3. Host MIDI input still sounds while the sequencer is muted. ---
        gooey_engine_trigger_instrument_with_velocity(engine, 0, 1.0);
        let host_buf = render_n(engine, ONE_SECOND / 4);
        assert!(
            max_amplitude(&host_buf) > AUDIBLE,
            "host-supplied trigger must keep sounding while the sequencer is muted"
        );
        let _ = drain_midi(engine);

        // --- 4. Re-enable: sequencer triggers resume. ---
        gooey_engine_set_sequencer_triggers_enabled(engine, true);
        assert!(gooey_engine_get_sequencer_triggers_enabled(engine));

        let resumed_buf = render_n(engine, ONE_SECOND);
        let resumed_events = drain_midi(engine);
        assert!(
            resumed_events.iter().any(|(ch, _)| *ch == 0),
            "re-enabled sequencer should fire the kick again; got {resumed_events:?}"
        );
        assert!(
            max_amplitude(&resumed_buf) > AUDIBLE,
            "re-enabled sequencer should produce audible output again"
        );

        gooey_engine_free(engine);
    }
}
