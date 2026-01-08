// Integration tests for basic Engine functionality

use gooey::engine::{Engine, Sequencer};
use gooey::instruments::{HiHat, KickDrum, SnareDrum};

#[test]
fn test_engine_creation() {
    let sample_rate = 44100.0;
    let engine = Engine::new(sample_rate);

    assert_eq!(engine.sample_rate(), sample_rate);
    assert_eq!(engine.bpm(), 120.0); // Default BPM
}

#[test]
fn test_add_instruments() {
    let sample_rate = 44100.0;
    let mut engine = Engine::new(sample_rate);

    // Add instruments
    engine.add_instrument("kick", Box::new(KickDrum::new(sample_rate)));
    engine.add_instrument("snare", Box::new(SnareDrum::new(sample_rate)));
    engine.add_instrument("hihat", Box::new(HiHat::new(sample_rate)));

    // Engine should be functional (this is a smoke test)
    let sample = engine.tick(0.0);
    assert!(
        sample.is_finite(),
        "Engine tick should produce finite audio sample"
    );
}

#[test]
fn test_trigger_instrument() {
    let sample_rate = 44100.0;
    let mut engine = Engine::new(sample_rate);

    engine.add_instrument("kick", Box::new(KickDrum::new(sample_rate)));

    // Trigger the instrument
    engine.trigger_instrument("kick");

    // Tick the engine and verify we get non-zero audio
    let mut found_audio = false;
    for i in 0..1000 {
        let sample = engine.tick(i as f32 / sample_rate);
        if sample.abs() > 0.001 {
            found_audio = true;
            break;
        }
    }

    assert!(
        found_audio,
        "Triggered instrument should produce audible output"
    );
}

#[test]
fn test_bpm_setting() {
    let sample_rate = 44100.0;
    let mut engine = Engine::new(sample_rate);

    // Test setting BPM
    engine.set_bpm(140.0);
    assert_eq!(engine.bpm(), 140.0);

    engine.set_bpm(80.0);
    assert_eq!(engine.bpm(), 80.0);
}

#[test]
fn test_add_sequencer() {
    let sample_rate = 44100.0;
    let mut engine = Engine::new(sample_rate);

    engine.add_instrument("kick", Box::new(KickDrum::new(sample_rate)));

    // Create and add a sequencer
    let pattern = vec![true, false, true, false];
    let sequencer = Sequencer::with_pattern(120.0, sample_rate, pattern, "kick");
    engine.add_sequencer(sequencer);

    assert_eq!(engine.sequencer_count(), 1);
}

#[test]
fn test_sequencer_access() {
    let sample_rate = 44100.0;
    let mut engine = Engine::new(sample_rate);

    engine.add_instrument("kick", Box::new(KickDrum::new(sample_rate)));

    let pattern = vec![true, false, true, false];
    let sequencer = Sequencer::with_pattern(120.0, sample_rate, pattern, "kick");
    engine.add_sequencer(sequencer);

    // Access sequencer
    assert!(engine.sequencer(0).is_some());
    assert!(engine.sequencer_mut(0).is_some());
    assert!(engine.sequencer(1).is_none()); // Out of bounds
}

#[test]
fn test_global_effects() {
    let sample_rate = 44100.0;
    let engine = Engine::new(sample_rate);

    // Engine should have at least the default BrickWallLimiter
    assert!(
        engine.global_effect_count() >= 1,
        "Engine should have default global effects"
    );
}

#[test]
fn test_multiple_instruments_mix() {
    let sample_rate = 44100.0;
    let mut engine = Engine::new(sample_rate);

    // Add multiple instruments
    engine.add_instrument("kick", Box::new(KickDrum::new(sample_rate)));
    engine.add_instrument("snare", Box::new(SnareDrum::new(sample_rate)));

    // Trigger both
    engine.trigger_instrument("kick");
    engine.trigger_instrument("snare");

    // Both should produce audio when mixed
    let mut found_audio = false;
    for i in 0..1000 {
        let sample = engine.tick(i as f32 / sample_rate);
        if sample.abs() > 0.001 {
            found_audio = true;
            break;
        }
    }

    assert!(
        found_audio,
        "Multiple triggered instruments should produce mixed output"
    );
}
