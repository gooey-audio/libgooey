// Integration tests for LFO modulation on all instruments
// Based on the lfo_verify example code

use gooey::engine::{Engine, Lfo, MusicalDivision};
use gooey::instruments::{HiHat, KickDrum, SnareDrum, TomDrum};

#[test]
fn test_kick_drum_modulation() {
    let sample_rate = 44100.0;
    let bpm = 120.0;

    let mut engine = Engine::new(sample_rate);
    engine.set_bpm(bpm);
    engine.add_instrument("kick", Box::new(KickDrum::new(sample_rate)));

    let lfo = Lfo::new_synced(MusicalDivision::OneBar, bpm, sample_rate);
    let lfo_idx = engine.add_lfo(lfo);

    // Test all KickDrum parameters
    let params = ["frequency", "punch", "sub", "click", "snap", "decay", "pitch_envelope"];
    for param in params {
        let result = engine.map_lfo_to_parameter(lfo_idx, "kick", param, 1.0);
        assert!(
            result.is_ok(),
            "KickDrum parameter '{}' should be modulatable",
            param
        );
    }
}

#[test]
fn test_snare_drum_modulation() {
    let sample_rate = 44100.0;
    let bpm = 120.0;

    let mut engine = Engine::new(sample_rate);
    engine.set_bpm(bpm);
    engine.add_instrument("snare", Box::new(SnareDrum::new(sample_rate)));

    let lfo = Lfo::new_synced(MusicalDivision::OneBar, bpm, sample_rate);
    let lfo_idx = engine.add_lfo(lfo);

    // Test all SnareDrum parameters
    let params = [
        "frequency",
        "tonal",
        "noise",
        "crack",
        "decay",
        "pitch_drop",
    ];
    for param in params {
        let result = engine.map_lfo_to_parameter(lfo_idx, "snare", param, 1.0);
        assert!(
            result.is_ok(),
            "SnareDrum parameter '{}' should be modulatable",
            param
        );
    }
}

#[test]
fn test_hihat_modulation() {
    let sample_rate = 44100.0;
    let bpm = 120.0;

    let mut engine = Engine::new(sample_rate);
    engine.set_bpm(bpm);
    engine.add_instrument("hihat", Box::new(HiHat::new(sample_rate)));

    let lfo = Lfo::new_synced(MusicalDivision::OneBar, bpm, sample_rate);
    let lfo_idx = engine.add_lfo(lfo);

    // Test all HiHat parameters
    let params = ["decay", "brightness", "resonance", "frequency", "volume"];
    for param in params {
        let result = engine.map_lfo_to_parameter(lfo_idx, "hihat", param, 1.0);
        assert!(
            result.is_ok(),
            "HiHat parameter '{}' should be modulatable",
            param
        );
    }
}

#[test]
fn test_tom_drum_modulation() {
    let sample_rate = 44100.0;
    let bpm = 120.0;

    let mut engine = Engine::new(sample_rate);
    engine.set_bpm(bpm);
    engine.add_instrument("tom", Box::new(TomDrum::new(sample_rate)));

    let lfo = Lfo::new_synced(MusicalDivision::OneBar, bpm, sample_rate);
    let lfo_idx = engine.add_lfo(lfo);

    // Test all TomDrum parameters
    let params = ["frequency", "tonal", "punch", "decay", "pitch_drop"];
    for param in params {
        let result = engine.map_lfo_to_parameter(lfo_idx, "tom", param, 1.0);
        assert!(
            result.is_ok(),
            "TomDrum parameter '{}' should be modulatable",
            param
        );
    }
}

#[test]
fn test_invalid_parameter_returns_error() {
    let sample_rate = 44100.0;
    let bpm = 120.0;

    let mut engine = Engine::new(sample_rate);
    engine.set_bpm(bpm);
    engine.add_instrument("kick", Box::new(KickDrum::new(sample_rate)));

    let lfo = Lfo::new_synced(MusicalDivision::OneBar, bpm, sample_rate);
    let lfo_idx = engine.add_lfo(lfo);

    // Test that invalid parameter returns an error
    let result = engine.map_lfo_to_parameter(lfo_idx, "kick", "invalid_param", 1.0);
    assert!(
        result.is_err(),
        "Mapping to invalid parameter should return an error"
    );
}

#[test]
fn test_invalid_instrument_returns_error() {
    let sample_rate = 44100.0;
    let bpm = 120.0;

    let mut engine = Engine::new(sample_rate);
    engine.set_bpm(bpm);

    let lfo = Lfo::new_synced(MusicalDivision::OneBar, bpm, sample_rate);
    let lfo_idx = engine.add_lfo(lfo);

    // Test that invalid instrument returns an error
    let result = engine.map_lfo_to_parameter(lfo_idx, "nonexistent", "frequency", 1.0);
    assert!(
        result.is_err(),
        "Mapping to nonexistent instrument should return an error"
    );
}

#[test]
fn test_multiple_lfos_on_same_instrument() {
    let sample_rate = 44100.0;
    let bpm = 120.0;

    let mut engine = Engine::new(sample_rate);
    engine.set_bpm(bpm);
    engine.add_instrument("kick", Box::new(KickDrum::new(sample_rate)));

    // Add multiple LFOs
    let lfo1 = Lfo::new_synced(MusicalDivision::OneBar, bpm, sample_rate);
    let lfo1_idx = engine.add_lfo(lfo1);

    let lfo2 = Lfo::new_synced(MusicalDivision::Quarter, bpm, sample_rate);
    let lfo2_idx = engine.add_lfo(lfo2);

    // Map different parameters to different LFOs
    let result1 = engine.map_lfo_to_parameter(lfo1_idx, "kick", "frequency", 1.0);
    let result2 = engine.map_lfo_to_parameter(lfo2_idx, "kick", "pitch_envelope", 0.5);

    assert!(result1.is_ok(), "First LFO mapping should succeed");
    assert!(result2.is_ok(), "Second LFO mapping should succeed");
}
