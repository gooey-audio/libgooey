use gooey::bounce::{bounce_to_buffer, BounceLength};
use gooey::engine::{Engine, Sequencer};
use gooey::instruments::KickDrum;

fn setup_engine() -> Engine {
    let sample_rate = 44100.0;
    let bpm = 120.0;
    let mut engine = Engine::new(sample_rate);
    engine.set_bpm(bpm);

    let kick = KickDrum::new(sample_rate);
    engine.add_instrument("kick", Box::new(kick));

    // 4-on-the-floor: steps 0, 4, 8, 12 enabled
    let mut pattern = vec![false; 16];
    pattern[0] = true;
    pattern[4] = true;
    pattern[8] = true;
    pattern[12] = true;
    let seq = Sequencer::with_pattern(bpm, sample_rate, pattern, "kick");
    engine.add_sequencer(seq);

    engine
}

#[test]
fn test_bounce_correct_length() {
    let mut engine = setup_engine();
    let buffer = bounce_to_buffer(&mut engine, BounceLength::Bars(1));

    // At 120 BPM, 1 bar = 4 beats = 2 seconds = 88200 samples
    assert_eq!(buffer.len(), 88200);

    let buffer = bounce_to_buffer(&mut engine, BounceLength::Bars(2));
    assert_eq!(buffer.len(), 176400);
}

#[test]
fn test_bounce_beats_length() {
    let mut engine = setup_engine();
    let buffer = bounce_to_buffer(&mut engine, BounceLength::Beats(2.0));

    // 2 beats at 120 BPM = 1 second = 44100 samples
    assert_eq!(buffer.len(), 44100);
}

#[test]
fn test_bounce_produces_audio() {
    let mut engine = setup_engine();
    let buffer = bounce_to_buffer(&mut engine, BounceLength::Bars(1));

    // Should have non-zero samples (kick drum is playing)
    let max_abs = buffer.iter().map(|s| s.abs()).fold(0.0_f32, f32::max);
    assert!(
        max_abs > 0.01,
        "Expected audible output, got max amplitude {max_abs}"
    );
}

#[test]
fn test_bounce_deterministic() {
    // Two fresh engines with identical setup should produce identical output
    let mut engine1 = setup_engine();
    let mut engine2 = setup_engine();
    let buffer1 = bounce_to_buffer(&mut engine1, BounceLength::Bars(1));
    let buffer2 = bounce_to_buffer(&mut engine2, BounceLength::Bars(1));

    assert_eq!(buffer1.len(), buffer2.len());
    assert_eq!(buffer1, buffer2, "Bounces should be identical");
}

#[test]
fn test_bounce_silent_when_no_pattern() {
    let sample_rate = 44100.0;
    let mut engine = Engine::new(sample_rate);
    engine.set_bpm(120.0);

    let kick = KickDrum::new(sample_rate);
    engine.add_instrument("kick", Box::new(kick));

    // Sequencer with no steps enabled
    let pattern = vec![false; 16];
    let seq = Sequencer::with_pattern(120.0, sample_rate, pattern, "kick");
    engine.add_sequencer(seq);

    let buffer = bounce_to_buffer(&mut engine, BounceLength::Bars(1));
    let max_abs = buffer.iter().map(|s| s.abs()).fold(0.0_f32, f32::max);
    assert!(
        max_abs < 0.001,
        "Expected near-silence, got max amplitude {max_abs}"
    );
}
