//! End-to-end coverage for the bass-loop clip: notes land on the shared
//! transport grid, replacements keep phase, and clearing silences the voice.

use gooey::ffi::*;

const SR: f32 = 48_000.0;
const BUFFER: usize = 256;
/// 120 BPM at 48 kHz: one quarter note is 24 000 samples, one tick 250.
const SAMPLES_PER_TICK: u64 = 250;

fn note(start_tick: u32, duration_ticks: u32, midi_note: u8) -> GooeyBassLoopEvent {
    GooeyBassLoopEvent {
        start_tick,
        duration_ticks,
        midi_note,
        velocity: 0.9,
    }
}

unsafe fn started_engine() -> *mut GooeyEngine {
    let engine = gooey_engine_new(SR);
    gooey_engine_set_bpm(engine, 120.0);
    engine
}

/// Render `frames` samples in host-sized buffers, returning the absolute sample
/// positions of bass triggers and the peak output level.
unsafe fn render_collecting(
    engine: *mut GooeyEngine,
    start_sample: u64,
    frames: usize,
) -> (Vec<u64>, f32) {
    let mut output = vec![0.0_f32; BUFFER * GOOEY_OUTPUT_CHANNELS as usize];
    let mut events: [GooeyMidiEvent; 64] = std::array::from_fn(|_| GooeyMidiEvent {
        instrument_index: 0,
        velocity: 0.0,
        sample_offset: 0,
    });
    let mut hits = Vec::new();
    let mut peak = 0.0_f32;
    let mut rendered = 0;
    while rendered < frames {
        gooey_engine_render(engine, output.as_mut_ptr(), BUFFER as u32);
        let count = gooey_engine_drain_midi_events(engine, events.as_mut_ptr(), 64);
        for event in &events[..count as usize] {
            if event.instrument_index == INSTRUMENT_BASS {
                hits.push(start_sample + (rendered + event.sample_offset as usize) as u64);
            }
        }
        peak = output
            .iter()
            .fold(peak, |peak, sample| peak.max(sample.abs()));
        rendered += BUFFER;
    }
    (hits, peak)
}

#[test]
fn notes_fire_on_the_transport_grid_every_pass() {
    unsafe {
        let engine = started_engine();
        let clip = [note(0, 96, 36), note(192, 96, 43)];
        let generation = gooey_engine_bass_loop_replace(engine, clip.as_ptr(), 2, 384);
        assert_ne!(generation, 0);
        gooey_engine_sequencer_start(engine);

        let (hits, peak) = render_collecting(engine, 0, 2 * 384 * SAMPLES_PER_TICK as usize);
        assert_eq!(
            gooey_engine_bass_loop_get_applied_generation(engine),
            generation
        );
        let expected: Vec<u64> = [0, 192, 384, 576]
            .iter()
            .map(|tick| tick * SAMPLES_PER_TICK)
            .collect();
        assert_eq!(hits.len(), expected.len(), "hits: {hits:?}");
        for (hit, want) in hits.iter().zip(&expected) {
            assert!(hit.abs_diff(*want) <= 1, "hit {hit} expected {want}");
        }
        assert!(peak > 0.01, "bass clip should be audible, peak {peak}");
        gooey_engine_free(engine);
    }
}

#[test]
fn replacing_mid_loop_keeps_phase_and_clearing_silences() {
    unsafe {
        let engine = started_engine();
        let first = [note(0, 96, 36)];
        assert_ne!(
            gooey_engine_bass_loop_replace(engine, first.as_ptr(), 1, 384),
            0
        );
        gooey_engine_sequencer_start(engine);
        // ~tick 120, rounded to whole host buffers so absolute positions line up.
        let warmup = (120 * SAMPLES_PER_TICK as usize / BUFFER) * BUFFER;
        let _ = render_collecting(engine, 0, warmup);

        // A new clip staged at tick ~120 plays its tick-288 note on the
        // original grid rather than restarting from the replacement point.
        let second = [note(288, 48, 40)];
        assert_ne!(
            gooey_engine_bass_loop_replace(engine, second.as_ptr(), 1, 384),
            0
        );
        let (hits, _) = render_collecting(engine, warmup as u64, 264 * SAMPLES_PER_TICK as usize);
        assert_eq!(hits.len(), 1, "hits: {hits:?}");
        assert!(hits[0].abs_diff(288 * SAMPLES_PER_TICK) <= 1);

        assert_ne!(gooey_engine_bass_loop_clear(engine), 0);
        let (hits, _) = render_collecting(engine, 0, 2 * 384 * SAMPLES_PER_TICK as usize);
        assert!(hits.is_empty(), "cleared clip still fired: {hits:?}");
        gooey_engine_free(engine);
    }
}

#[test]
fn rejects_malformed_clips() {
    unsafe {
        let engine = started_engine();
        let overlapping = [note(0, 200, 36), note(96, 48, 38)];
        assert_eq!(
            gooey_engine_bass_loop_replace(engine, overlapping.as_ptr(), 2, 384),
            0
        );
        let outside = [note(384, 48, 36)];
        assert_eq!(
            gooey_engine_bass_loop_replace(engine, outside.as_ptr(), 1, 384),
            0
        );
        assert_eq!(
            gooey_engine_bass_loop_replace(engine, outside.as_ptr(), 1, 0),
            0
        );
        assert_eq!(
            gooey_engine_bass_loop_replace(engine, std::ptr::null(), 1, 384),
            0
        );
        assert_eq!(
            gooey_engine_bass_loop_replace(std::ptr::null(), outside.as_ptr(), 1, 384),
            0
        );
        // An empty clip is valid and simply silent.
        assert_ne!(
            gooey_engine_bass_loop_replace(engine, std::ptr::null(), 0, 384),
            0
        );
        gooey_engine_free(engine);
    }
}
