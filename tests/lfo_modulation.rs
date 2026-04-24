// Integration tests for LFO modulation on all instruments
// Based on the lfo_verify example code

use gooey::engine::{Engine, Lfo, Modulatable, MusicalDivision};
use gooey::instruments::{HiHat, HiHat2, KickDrum, SnareDrum, Tom2, TomDrum};

#[test]
fn test_kick_drum_modulation() {
    let sample_rate = 44100.0;
    let bpm = 120.0;

    let mut engine = Engine::new(sample_rate);
    engine.set_bpm(bpm);
    engine.add_instrument("kick", Box::new(KickDrum::new(sample_rate)));

    let lfo = Lfo::new_synced(MusicalDivision::OneBar, bpm, sample_rate);
    let lfo_idx = engine.add_lfo(lfo);

    // Test all KickDrum parameters (using new normalized 0-1 parameter names)
    let params = [
        "frequency",
        "punch",
        "sub",
        "click",
        "oscillator_decay",
        "tuning",
    ];
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
    let params = ["pitch", "decay", "attack", "tone"];
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
    let result2 = engine.map_lfo_to_parameter(lfo2_idx, "kick", "tuning", 0.5);

    assert!(result1.is_ok(), "First LFO mapping should succeed");
    assert!(result2.is_ok(), "Second LFO mapping should succeed");
}

// ---------------------------------------------------------------------------
// Audio-output regression tests
//
// The tests above only verify that LFO routing accepts a parameter name.
// These tests render actual audio and confirm that LFO modulation produces
// different output than the unmodulated baseline. They guard the bug where
// instrument code snapshotted parameters at trigger time and never re-read
// them, leaving LFO modulation silently inert.
// ---------------------------------------------------------------------------

const SR: f32 = 44100.0;
const RENDER_SAMPLES: usize = 4096;
const DIFF_THRESHOLD: f32 = 1e-3;

/// Render `RENDER_SAMPLES` samples of a kick after applying a single bipolar
/// modulation value to `param`. Returns the rendered buffer.
fn render_kick_with_mod(param: &str, mod_value: f32) -> Vec<f32> {
    let mut kick = KickDrum::new(SR);
    // Apply the modulation BEFORE trigger so the smoothed param has time to
    // settle for trigger-time reads. Subsequent ticks keep applying the same
    // modulation so per-sample reads pick it up too.
    kick.apply_modulation(param, mod_value).unwrap();
    // Settle smoothers
    for _ in 0..2048 {
        kick.tick(0.0);
    }
    kick.trigger_with_velocity(0.0, 1.0);
    let mut out = Vec::with_capacity(RENDER_SAMPLES);
    for i in 0..RENDER_SAMPLES {
        kick.apply_modulation(param, mod_value).unwrap();
        let t = i as f64 / SR as f64;
        out.push(kick.tick(t));
    }
    out
}

fn render_snare_with_mod(param: &str, mod_value: f32) -> Vec<f32> {
    let mut snare = SnareDrum::new(SR);
    snare.apply_modulation(param, mod_value).unwrap();
    for _ in 0..2048 {
        snare.tick(0.0);
    }
    snare.trigger_with_velocity(0.0, 1.0);
    let mut out = Vec::with_capacity(RENDER_SAMPLES);
    for i in 0..RENDER_SAMPLES {
        snare.apply_modulation(param, mod_value).unwrap();
        let t = i as f64 / SR as f64;
        out.push(snare.tick(t));
    }
    out
}

/// Mean-absolute difference between two equal-length buffers.
fn mean_abs_diff(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    let total: f32 = a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum();
    total / a.len() as f32
}

#[test]
fn audio_kick_frequency_lfo_changes_output() {
    // Regression: kick `frequency` was cached in `triggered_frequency` at
    // trigger time, so LFO modulation never reached the audio.
    let low = render_kick_with_mod("frequency", -1.0);
    let high = render_kick_with_mod("frequency", 1.0);
    let diff = mean_abs_diff(&low, &high);
    assert!(
        diff > DIFF_THRESHOLD,
        "kick frequency LFO produced no audible change (mean abs diff = {})",
        diff
    );
}

#[test]
fn audio_kick_oscillator_decay_lfo_changes_output() {
    // Regression: kick `oscillator_decay` was baked into ADSR configs at
    // trigger time. tick() now re-applies decay times each sample.
    let low = render_kick_with_mod("oscillator_decay", -1.0);
    let high = render_kick_with_mod("oscillator_decay", 1.0);
    let diff = mean_abs_diff(&low, &high);
    assert!(
        diff > DIFF_THRESHOLD,
        "kick oscillator_decay LFO produced no audible change (mean abs diff = {})",
        diff
    );
}

#[test]
fn audio_snare_decay_lfo_changes_output() {
    // Regression: snare `decay` was baked into multiple ADSR configs at
    // trigger time. tick() now re-applies the dependent decay/release times.
    let low = render_snare_with_mod("decay", -1.0);
    let high = render_snare_with_mod("decay", 1.0);
    let diff = mean_abs_diff(&low, &high);
    assert!(
        diff > DIFF_THRESHOLD,
        "snare decay LFO produced no audible change (mean abs diff = {})",
        diff
    );
}

#[test]
fn audio_kick_pitch_targets_no_longer_modulatable() {
    // pitch_envelope_amount and pitch_start_ratio were removed as LFO
    // targets — `tuning` covers live pitch modulation instead.
    let mut kick = KickDrum::new(SR);
    assert!(kick.apply_modulation("pitch_envelope_amount", 1.0).is_err());
    assert!(kick.apply_modulation("pitch_start_ratio", 1.0).is_err());
    assert!(kick.apply_modulation("tuning", 1.0).is_ok());
}

// ---------------------------------------------------------------------------
// Trigger-time pickup tests for envelopes that can't be live-updated.
//
// HiHat2 uses MaxCurveEnvelope which has no live-update API, so attack/decay
// modulation is intentionally trigger-time only. Tom2 stores decay as a plain
// f32 and rebuilds its MaxCurveEnvelope at each trigger, so the same applies.
// What we verify here is that the *next* trigger picks up the LFO-modulated
// value, which is the documented contract for these instruments.
// ---------------------------------------------------------------------------

#[test]
fn audio_hihat_attack_pickup_at_next_trigger() {
    // Two identical hihat instances triggered with attack at -1 vs +1 should
    // differ. The smoother for `attack` is read at trigger time.
    fn render_hihat(attack_mod: f32) -> Vec<f32> {
        let mut h = HiHat2::new(SR);
        h.apply_modulation("attack", attack_mod).unwrap();
        for _ in 0..2048 {
            h.tick(0.0);
        }
        let mut out = Vec::with_capacity(RENDER_SAMPLES);
        h.apply_modulation("attack", attack_mod).unwrap();
        h.trigger_with_velocity(0.0, 1.0);
        for i in 0..RENDER_SAMPLES {
            h.apply_modulation("attack", attack_mod).unwrap();
            let t = i as f64 / SR as f64;
            out.push(h.tick(t));
        }
        out
    }

    let short = render_hihat(-1.0);
    let long = render_hihat(1.0);
    let diff = mean_abs_diff(&short, &long);
    assert!(
        diff > DIFF_THRESHOLD,
        "hihat attack LFO produced no audible change at next trigger (mean abs diff = {})",
        diff
    );
}

#[test]
fn audio_hihat_decay_pickup_at_next_trigger() {
    fn render_hihat(decay_mod: f32) -> Vec<f32> {
        let mut h = HiHat2::new(SR);
        h.apply_modulation("decay", decay_mod).unwrap();
        for _ in 0..2048 {
            h.tick(0.0);
        }
        let mut out = Vec::with_capacity(RENDER_SAMPLES);
        h.apply_modulation("decay", decay_mod).unwrap();
        h.trigger_with_velocity(0.0, 1.0);
        for i in 0..RENDER_SAMPLES {
            h.apply_modulation("decay", decay_mod).unwrap();
            let t = i as f64 / SR as f64;
            out.push(h.tick(t));
        }
        out
    }

    let short = render_hihat(-1.0);
    let long = render_hihat(1.0);
    let diff = mean_abs_diff(&short, &long);
    assert!(
        diff > DIFF_THRESHOLD,
        "hihat decay LFO produced no audible change at next trigger (mean abs diff = {})",
        diff
    );
}

#[test]
fn audio_tom2_decay_pickup_at_next_trigger() {
    // Tom2 reads `self.decay` directly when rebuilding its MaxCurveEnvelope on
    // trigger. Setting different decay values before each trigger must produce
    // audibly different envelopes.
    use gooey::engine::Instrument;
    fn render_tom(decay_value: f32) -> Vec<f32> {
        let mut t = Tom2::new(SR);
        t.set_decay(decay_value);
        let mut out = Vec::with_capacity(RENDER_SAMPLES);
        t.trigger_with_velocity(0.0, 1.0);
        for i in 0..RENDER_SAMPLES {
            let now = i as f64 / SR as f64;
            out.push(t.tick(now));
        }
        out
    }

    let short = render_tom(5.0);
    let long = render_tom(95.0);
    let diff = mean_abs_diff(&short, &long);
    assert!(
        diff > DIFF_THRESHOLD,
        "tom2 decay change between triggers produced no audible difference (mean abs diff = {})",
        diff
    );
}
