/// Offline bounce demo — renders a 4-bar loop with kick, snare, hihat, and tom
/// through effects and LFO modulation, then writes it to a WAV file.
use std::path::Path;

use gooey::bounce::{bounce_to_wav, BounceLength, WavConfig};
use gooey::effects::{DelayEffect, DelayTiming, SoftLimiter, TubeCompressor, TubeSaturation};
use gooey::engine::{Engine, Lfo, MusicalDivision, Sequencer, SequencerStep};
use gooey::instruments::{HiHat2, KickDrum, SnareDrum, Tom2};

fn main() {
    let sample_rate = 44100.0;
    let bpm = 128.0;

    let mut engine = Engine::new(sample_rate);
    engine.set_bpm(bpm);
    engine.set_master_gain(0.7);

    // --- Instruments ---
    engine.add_instrument("kick", Box::new(KickDrum::new(sample_rate)));
    engine.add_instrument("snare", Box::new(SnareDrum::new(sample_rate)));
    engine.add_instrument("hihat", Box::new(HiHat2::new(sample_rate)));
    engine.add_instrument("tom", Box::new(Tom2::new(sample_rate)));

    // --- Sequencer patterns (16 steps = 1 bar) ---

    // Kick: four-on-the-floor
    let kick_pattern: Vec<SequencerStep> = (0..16)
        .map(|i| SequencerStep {
            enabled: i % 4 == 0,
            velocity: if i == 0 { 1.0 } else { 0.85 },
            blend: None,
            note: None,
        })
        .collect();
    let mut kick_seq = Sequencer::with_velocity_pattern(bpm, sample_rate, kick_pattern, "kick");
    kick_seq.set_swing(0.55);
    engine.add_sequencer(kick_seq);

    // Snare: beats 2 and 4, ghost note on the "e" of 2
    let snare_steps = [
        (4, 1.0),  // beat 2
        (5, 0.3),  // ghost note
        (12, 0.9), // beat 4
    ];
    let snare_pattern: Vec<SequencerStep> = (0..16)
        .map(|i| {
            let hit = snare_steps.iter().find(|(s, _)| *s == i);
            SequencerStep {
                enabled: hit.is_some(),
                velocity: hit.map(|(_, v)| *v).unwrap_or(0.0),
                blend: None,
                note: None,
            }
        })
        .collect();
    let snare_seq = Sequencer::with_velocity_pattern(bpm, sample_rate, snare_pattern, "snare");
    engine.add_sequencer(snare_seq);

    // Hi-hat: every 8th note with alternating velocity (open/closed feel)
    let hihat_pattern: Vec<SequencerStep> = (0..16)
        .map(|i| SequencerStep {
            enabled: i % 2 == 0,
            velocity: if i % 4 == 0 { 0.9 } else { 0.5 },
            blend: None,
            note: None,
        })
        .collect();
    let mut hihat_seq = Sequencer::with_velocity_pattern(bpm, sample_rate, hihat_pattern, "hihat");
    hihat_seq.set_swing(0.58);
    engine.add_sequencer(hihat_seq);

    // Tom: a fill on beat 4 of every bar
    let tom_steps = [(13, 0.7), (14, 0.8), (15, 0.9)];
    let tom_pattern: Vec<SequencerStep> = (0..16)
        .map(|i| {
            let hit = tom_steps.iter().find(|(s, _)| *s == i);
            SequencerStep {
                enabled: hit.is_some(),
                velocity: hit.map(|(_, v)| *v).unwrap_or(0.0),
                blend: None,
                note: None,
            }
        })
        .collect();
    let tom_seq = Sequencer::with_velocity_pattern(bpm, sample_rate, tom_pattern, "tom");
    engine.add_sequencer(tom_seq);

    // --- LFOs ---

    // Slow LFO on hihat decay (one bar cycle)
    let lfo = Lfo::new_synced(MusicalDivision::OneBar, bpm, sample_rate);
    let lfo_idx = engine.add_lfo(lfo);
    engine
        .map_lfo_to_parameter(lfo_idx, "hihat", "decay", 0.3)
        .unwrap();

    // Faster LFO on kick punch (half note)
    let lfo2 = Lfo::new_synced(MusicalDivision::Half, bpm, sample_rate);
    let lfo2_idx = engine.add_lfo(lfo2);
    engine
        .map_lfo_to_parameter(lfo2_idx, "kick", "punch", 0.2)
        .unwrap();

    // --- Global effects chain ---
    engine.clear_global_effects();
    engine.add_global_effect(Box::new(TubeSaturation::new(sample_rate, 0.3, 0.4, 0.5)));
    engine.add_global_effect(Box::new(DelayEffect::new(
        sample_rate,
        DelayTiming::Eighth,
        bpm,
        0.35,
        0.2,
        8000.0,
    )));
    engine.add_global_effect(Box::new(TubeCompressor::new(
        sample_rate,
        -12.0,
        4.0,
        10.0,
        100.0,
        0.6,
    )));
    engine.add_global_effect(Box::new(SoftLimiter::new(0.95)));

    // --- Bounce ---
    let bars = 4;
    let path = Path::new("bounce_demo.wav");
    println!("Bouncing {bars} bars at {bpm} BPM to {path:?}...");

    match bounce_to_wav(
        &mut engine,
        BounceLength::Bars(bars),
        path,
        WavConfig::default(),
    ) {
        Ok(()) => {
            let duration_secs = bars as f64 * 4.0 * 60.0 / bpm as f64;
            println!("Done! Wrote {duration_secs:.1}s of audio to {path:?}");
        }
        Err(e) => eprintln!("Error: {e}"),
    }
}
