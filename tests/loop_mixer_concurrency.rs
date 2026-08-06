//! Regression coverage for concurrent loop controls and realtime rendering.

use gooey::mixer::{Mixer, PitchMode, StereoSampleBuffer};
use std::sync::{Arc, Barrier};
use std::thread;

const SR: f32 = 48_000.0;

fn sine(frames: usize) -> StereoSampleBuffer {
    let left: Vec<f32> = (0..frames).map(|i| (i as f32 * 0.02).sin()).collect();
    StereoSampleBuffer::from_channels(left.clone(), left, SR).unwrap()
}

#[test]
fn concurrent_control_and_render_is_crash_free() {
    let mut mixer = Mixer::new(SR);
    mixer.load(0, sine(4000));
    mixer.set_pitch_mode(0, PitchMode::PreservePitch);
    mixer.set_playing(0, true);

    // The producer owns only this handle, never a reference or pointer to the
    // live mixer. The audio thread remains the sole `Mixer` mutator.
    let control = mixer.control();
    let start = Arc::new(Barrier::new(2));
    let worker_start = start.clone();
    let control_thread = thread::spawn(move || {
        worker_start.wait();
        for _ in 0..512 {
            assert!(control.set_pitch_mode(0, PitchMode::PreservePitch));
            assert!(control.restart(0));
            assert!(control.load(0, sine(4000)));
            assert!(control.set_pitch_mode(0, PitchMode::Off));
            assert!(control.set_pitch_mode(0, PitchMode::PreservePitch));
            assert!(control.set_playing(0, true));
        }
    });

    start.wait();
    for _ in 0..20_000 {
        let frame = mixer.tick(SR);
        assert!(frame.l.is_finite() && frame.r.is_finite());
    }
    control_thread.join().unwrap();
    // Drain any commands that were produced near the end of the render loop.
    for _ in 0..128 {
        let frame = mixer.tick(SR);
        assert!(frame.l.is_finite() && frame.r.is_finite());
    }
}
