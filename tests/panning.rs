//! Integration tests for per-instrument stereo panning on the native engine.
//!
//! Pan is applied at the stereo seam ([`Engine::tick_stereo`]) with an
//! equal-power law: 0.0 = hard left, 0.5 = center, 1.0 = hard right. The mono
//! [`Engine::tick`] path ignores pan entirely.

use gooey::engine::Engine;
use gooey::instruments::KickDrum;

const SAMPLE_RATE: f32 = 44_100.0;

/// Trigger a kick and return the summed (left, right) energy over `frames`.
fn render_kick_energy(pan: f32) -> (f64, f64) {
    let mut engine = Engine::new(SAMPLE_RATE);
    engine.add_instrument("kick", Box::new(KickDrum::new(SAMPLE_RATE)));
    engine.set_instrument_pan("kick", pan);

    let step = 1.0 / SAMPLE_RATE as f64;
    let mut t = 0.0_f64;

    // Settle the smoothed pan before triggering, so the kick's loud attack is
    // measured at the final pan position, not mid-ramp. The one-pole smoother
    // needs several thousand samples to snap fully onto the target.
    for _ in 0..8192 {
        engine.tick_stereo(t);
        t += step;
    }

    engine.trigger_instrument("kick");

    let mut left = 0.0_f64;
    let mut right = 0.0_f64;
    for _ in 0..4096 {
        let frame = engine.tick_stereo(t);
        left += (frame.l * frame.l) as f64;
        right += (frame.r * frame.r) as f64;
        t += step;
    }
    (left, right)
}

#[test]
fn default_pan_is_centered() {
    assert_eq!(Engine::new(SAMPLE_RATE).instrument_pan("kick"), 0.5);
}

#[test]
fn set_and_get_pan_round_trips_and_clamps() {
    let mut engine = Engine::new(SAMPLE_RATE);
    engine.add_instrument("kick", Box::new(KickDrum::new(SAMPLE_RATE)));

    engine.set_instrument_pan("kick", 0.25);
    assert_eq!(engine.instrument_pan("kick"), 0.25);

    engine.set_instrument_pan("kick", -1.0);
    assert_eq!(engine.instrument_pan("kick"), 0.0);

    engine.set_instrument_pan("kick", 2.0);
    assert_eq!(engine.instrument_pan("kick"), 1.0);
}

#[test]
fn hard_left_pan_favors_the_left_channel() {
    let (left, right) = render_kick_energy(0.0);
    assert!(left > 0.0, "expected audible left output");
    assert!(
        right < left * 1e-6,
        "hard-left pan should silence the right channel (left={left}, right={right})"
    );
}

#[test]
fn hard_right_pan_favors_the_right_channel() {
    let (left, right) = render_kick_energy(1.0);
    assert!(right > 0.0, "expected audible right output");
    assert!(
        left < right * 1e-6,
        "hard-right pan should silence the left channel (left={left}, right={right})"
    );
}

#[test]
fn center_pan_is_balanced() {
    let (left, right) = render_kick_energy(0.5);
    assert!(left > 0.0 && right > 0.0);
    assert!(
        (left - right).abs() < left * 1e-6,
        "center pan should be balanced (left={left}, right={right})"
    );
}
