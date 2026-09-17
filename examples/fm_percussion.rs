//! Offline Noir-inspired FM percussion demo.
//!
//! Writes `fm_percussion.wav`: a monophonic 16-step pattern whose notes,
//! velocities, and preset-blend coordinates change on every hit.

use std::ffi::CString;

use gooey::ffi::*;

fn main() {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "fm_percussion.wav".to_string());
    let output = CString::new(output).expect("output path must not contain a NUL byte");

    unsafe {
        let engine = gooey_engine_new(48_000.0);
        assert!(!engine.is_null());
        gooey_engine_set_bpm(engine, 136.0);
        gooey_engine_set_channel_instrument_type(engine, 0, INSTRUMENT_FM_PERCUSSION);
        gooey_engine_blend_enable(engine, 0);

        let hits = [
            (0, 36, 1.00, 0.00, 0.00),
            (3, 55, 0.48, 0.25, 0.78),
            (6, 72, 0.82, 0.55, 0.20),
            (7, 91, 0.32, 1.00, 0.00),
            (10, 43, 0.92, 0.15, 0.42),
            (12, 67, 0.64, 0.72, 0.66),
            (15, 99, 0.74, 1.00, 1.00),
        ];
        for (step, note, velocity, x, y) in hits {
            gooey_engine_sequencer_set_instrument_step_with_velocity(
                engine, 0, step, true, velocity,
            );
            gooey_engine_sequencer_set_instrument_step_note(engine, 0, step, note);
            gooey_engine_sequencer_set_instrument_step_blend(engine, 0, step, x, y);
        }

        if !gooey_engine_bounce_to_wav(engine, 4, output.as_ptr()) {
            gooey_engine_free(engine);
            panic!("failed to write FM percussion WAV");
        }
        gooey_engine_free(engine);
    }

    println!("Wrote {}", output.to_string_lossy());
}
