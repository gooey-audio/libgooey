use std::f64::consts::TAU;

use gooey::effects::{Effect, FeedbackWaveshaper, TubeSaturation};

const SAMPLE_RATE: f32 = 48_000.0;
const N: usize = 8192;
const WARMUP: usize = 8192;
const FUNDAMENTAL_BIN: usize = 37;
const INPUT_AMP: f32 = 0.5;

fn fundamental_hz() -> f32 {
    FUNDAMENTAL_BIN as f32 * SAMPLE_RATE / N as f32
}

fn render_input() -> Vec<f32> {
    let mut out = Vec::with_capacity(N);
    for i in WARMUP..(WARMUP + N) {
        let phase = TAU * FUNDAMENTAL_BIN as f64 * i as f64 / N as f64;
        out.push(phase.sin() as f32 * INPUT_AMP);
    }
    out
}

fn render_saturation(drive: f32, warmth: f32) -> Vec<f32> {
    let sat = TubeSaturation::new(SAMPLE_RATE, drive, warmth, 1.0);
    let mut out = Vec::with_capacity(N);
    for i in 0..(WARMUP + N) {
        let phase = TAU * fundamental_hz() as f64 * i as f64 / SAMPLE_RATE as f64;
        let sample = phase.sin() as f32 * INPUT_AMP;
        let processed = sat.process(sample);
        if i >= WARMUP {
            out.push(processed);
        }
    }
    out
}

fn render_feedback(drive: f32, feedback: f32) -> Vec<f32> {
    let mut ws = FeedbackWaveshaper::new(SAMPLE_RATE, drive, feedback, 2000.0, 1.0);
    let mut out = Vec::with_capacity(N);
    for i in 0..(WARMUP + N) {
        let phase = TAU * fundamental_hz() as f64 * i as f64 / SAMPLE_RATE as f64;
        let sample = phase.sin() as f32 * INPUT_AMP;
        let processed = ws.process(sample);
        if i >= WARMUP {
            out.push(processed);
        }
    }
    out
}

fn rms(samples: &[f32]) -> f64 {
    let mean_square = samples
        .iter()
        .map(|&s| {
            let s = s as f64;
            s * s
        })
        .sum::<f64>()
        / samples.len() as f64;
    mean_square.sqrt()
}

fn gain_db(processed: &[f32], dry: &[f32]) -> f64 {
    20.0 * (rms(processed) / rms(dry).max(1e-30)).log10()
}

fn bin_power(samples: &[f32], bin: usize) -> f64 {
    let n = samples.len();
    let step = TAU * bin as f64 / n as f64;
    let (mut re, mut im) = (0.0_f64, 0.0_f64);
    for (i, &s) in samples.iter().enumerate() {
        let phase = step * i as f64;
        re += s as f64 * phase.cos();
        im -= s as f64 * phase.sin();
    }
    re * re + im * im
}

fn harmonic_distortion(samples: &[f32]) -> f64 {
    let fundamental = bin_power(samples, FUNDAMENTAL_BIN).max(1e-30);
    let harmonic_power = (2..=10)
        .map(|harmonic| FUNDAMENTAL_BIN * harmonic)
        .take_while(|&bin| bin < N / 2)
        .map(|bin| bin_power(samples, bin))
        .sum::<f64>();
    (harmonic_power / fundamental).sqrt()
}

#[test]
fn max_feedback_matches_saturation_gain_and_distortion() {
    let dry = render_input();
    let saturation = render_saturation(1.0, 0.5);
    let feedback = render_feedback(100.0, 0.98);

    let saturation_gain_db = gain_db(&saturation, &dry);
    let feedback_gain_db = gain_db(&feedback, &dry);
    let gain_diff_db = feedback_gain_db - saturation_gain_db;

    assert!(
        gain_diff_db.abs() <= 1.5,
        "max feedback gain ({feedback_gain_db:.2} dB) should be within 1.5 dB of saturation ({saturation_gain_db:.2} dB); diff={gain_diff_db:.2} dB"
    );

    let saturation_distortion = harmonic_distortion(&saturation);
    let feedback_distortion = harmonic_distortion(&feedback);

    assert!(
        feedback_distortion >= saturation_distortion * 0.9,
        "max feedback distortion ({feedback_distortion:.4}) should be at least 90% of saturation ({saturation_distortion:.4})"
    );
}

#[test]
fn mid_feedback_stays_near_mid_saturation_gain() {
    let dry = render_input();
    let saturation = render_saturation(0.5, 0.4);
    let feedback = render_feedback(50.0, 0.49);

    let saturation_gain_db = gain_db(&saturation, &dry);
    let feedback_gain_db = gain_db(&feedback, &dry);
    let gain_diff_db = feedback_gain_db - saturation_gain_db;

    assert!(
        gain_diff_db.abs() <= 3.0,
        "mid feedback gain ({feedback_gain_db:.2} dB) should stay within 3 dB of mid saturation ({saturation_gain_db:.2} dB); diff={gain_diff_db:.2} dB"
    );
}
