//! Numeric guardrail for oscillator anti-aliasing.
//!
//! Companion to `examples/aliasing_plots.rs` (which renders the visual proof). These tests
//! assert that the band-limited PolyBLEP generators put dramatically less energy into
//! aliased (inter-harmonic) bins than the naive baselines.
//!
//! The signals are generated with *coherent* sampling — the fundamental fits a whole number
//! of cycles in the analysis window — so a plain rectangular-window DFT has no spectral
//! leakage and every component lands on an exact bin. That lets us measure alias energy
//! without rustfft or windowing.

use std::f64::consts::TAU;

use gooey::envelope::ADSRConfig;
use gooey::gen::polyblep::{polyblep_saw, polyblep_square};
use gooey::gen::{Oscillator, Waveform};
use gooey::utils::{Oversampler, OversamplingMode};

const SAMPLE_RATE: f32 = 48_000.0;
const N: usize = 8192;
/// Cycles-per-window of the fundamental. Prime, and chosen so `SAMPLE_RATE / f0` is
/// non-integer, which keeps folded aliases off the true harmonic bins.
const J: usize = 367;

fn fundamental_hz() -> f32 {
    J as f32 * SAMPLE_RATE / N as f32
}

/// Power in DFT bin `k` (|X_k|^2) via direct evaluation.
fn bin_power(x: &[f32], k: usize) -> f64 {
    let n = x.len();
    let step = TAU * k as f64 / n as f64;
    let (mut re, mut im) = (0.0_f64, 0.0_f64);
    for (i, &s) in x.iter().enumerate() {
        let phase = step * i as f64;
        re += s as f64 * phase.cos();
        im -= s as f64 * phase.sin();
    }
    re * re + im * im
}

/// Ratio of aliased/non-harmonic power to true-harmonic power.
///
/// `signal_bins` are the bins of the intended harmonics. Total positive-frequency power comes
/// from Parseval (`sum|X_k|^2 = N * sum x^2`), so alias power = total − DC − harmonics.
fn alias_to_signal_ratio(x: &[f32], signal_bins: &[usize]) -> f64 {
    let n = x.len();
    let sumsq: f64 = x.iter().map(|&s| (s as f64) * (s as f64)).sum();
    let dc = {
        let s: f64 = x.iter().map(|&v| v as f64).sum();
        s * s
    };
    let total_positive = (n as f64 * sumsq - dc) / 2.0;
    let signal: f64 = signal_bins.iter().map(|&k| bin_power(x, k)).sum();
    let alias = (total_positive - signal).max(0.0);
    alias / signal.max(1e-30)
}

fn render(naive: bool, square: bool) -> Vec<f32> {
    let dt = fundamental_hz() as f64 / SAMPLE_RATE as f64;
    let mut phase = 0.0_f64;
    let mut out = Vec::with_capacity(N);
    for _ in 0..N {
        let s = match (naive, square) {
            (true, false) => (2.0 * phase - 1.0) as f32,
            (true, true) => {
                if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            (false, false) => polyblep_saw(phase, dt),
            (false, true) => polyblep_square(phase, dt),
        };
        out.push(s);
        phase = (phase + dt).rem_euclid(1.0);
    }
    out
}

/// Harmonic bins up to Nyquist. Saw uses all harmonics; square uses odd harmonics only.
fn signal_bins(square: bool) -> Vec<usize> {
    let nyquist_bin = N / 2;
    (1..)
        .map(|m| (m, m * J))
        .take_while(|&(_, bin)| bin <= nyquist_bin)
        .filter(|&(m, _)| !square || m % 2 == 1)
        .map(|(_, bin)| bin)
        .collect()
}

#[test]
fn polyblep_saw_suppresses_aliasing() {
    let bins = signal_bins(false);
    let naive = alias_to_signal_ratio(&render(true, false), &bins);
    let bandlimited = alias_to_signal_ratio(&render(false, false), &bins);
    println!(
        "saw @ {:.1} Hz: naive alias/signal = {:.4}, band-limited = {:.4}",
        fundamental_hz(),
        naive,
        bandlimited
    );

    // Sanity: the naive baseline must actually alias, or the test setup is wrong.
    assert!(
        naive > 0.02,
        "naive saw should show measurable aliasing, got {naive}"
    );
    // PolyBLEP must cut alias energy by at least ~6 dB versus naive (it is typically far more),
    // and leave the band-limited output essentially clean.
    assert!(
        bandlimited < naive * 0.25,
        "band-limited saw alias/signal {bandlimited} not <0.25x naive {naive}"
    );
    assert!(
        bandlimited < 0.01,
        "band-limited saw alias/signal {bandlimited} should be near-clean (<0.01)"
    );
}

/// Render a coherent tone straight from the shipping `Oscillator`, exercising the public
/// `set_antialias` toggle end-to-end. The fundamental does a whole number of cycles per N
/// samples, so the analysis stays coherent regardless of the warmup offset.
fn render_osc(waveform: Waveform, antialias: bool) -> Vec<f32> {
    let mut osc = Oscillator::new(SAMPLE_RATE, fundamental_hz());
    osc.waveform = waveform;
    osc.set_antialias(antialias);
    osc.set_adsr(ADSRConfig::new(0.0005, 0.0005, 1.0, 0.0005));
    osc.set_volume(1.0);
    osc.trigger(0.0);

    let warmup = 2048usize;
    let mut out = Vec::with_capacity(N);
    for i in 0..(warmup + N) {
        let s = osc.tick(i as f64 / SAMPLE_RATE as f64);
        if i >= warmup {
            out.push(s);
        }
    }
    out
}

#[test]
fn oscillator_antialias_toggle_changes_aliasing() {
    // Defaults to band-limited.
    assert!(Oscillator::new(SAMPLE_RATE, 440.0).is_antialiased());

    let bins = signal_bins(false);
    let antialiased = alias_to_signal_ratio(&render_osc(Waveform::Saw, true), &bins);
    let naive = alias_to_signal_ratio(&render_osc(Waveform::Saw, false), &bins);
    println!(
        "Oscillator saw @ {:.1} Hz: antialias=on alias/signal = {:.4}, antialias=off = {:.4}",
        fundamental_hz(),
        antialiased,
        naive
    );

    // Turning anti-aliasing off must measurably increase alias energy.
    assert!(
        naive > antialiased * 2.0,
        "naive oscillator saw ({naive}) should alias far more than band-limited ({antialiased})"
    );
}

/// Generate a naive square at `mode`'s oversampling factor and decimate via the engine's
/// half-band `Oversampler` (the oscillator is the signal source; the input is unused).
fn render_oversampled_naive_square(mode: OversamplingMode) -> Vec<f32> {
    let sub_dt = (fundamental_hz() as f64 / SAMPLE_RATE as f64) / mode.factor() as f64;
    let mut phase = 0.0f64;
    let mut os = Oversampler::new(mode);

    let warmup = 4096usize;
    let mut out = Vec::with_capacity(N);
    for i in 0..(warmup + N) {
        let s = os.process(0.0, |_| {
            let v = if phase < 0.5 { 1.0f32 } else { -1.0 };
            phase = (phase + sub_dt).rem_euclid(1.0);
            v
        });
        if i >= warmup {
            out.push(s);
        }
    }
    out
}

#[test]
fn oversampling_reduces_naive_square_aliasing() {
    let bins = signal_bins(true); // square: odd harmonics
    let off = alias_to_signal_ratio(
        &render_oversampled_naive_square(OversamplingMode::Off),
        &bins,
    );
    let x4 = alias_to_signal_ratio(
        &render_oversampled_naive_square(OversamplingMode::X4),
        &bins,
    );
    println!("naive square generation: oversampling off alias/signal = {off:.4}, 4x = {x4:.4}");

    // Off should be the fully-aliased naive baseline.
    assert!(off > 0.02, "naive square (off) should alias, got {off}");
    // Oversampled generation must cut alias energy substantially.
    assert!(
        x4 < off * 0.5,
        "4x oversampled square ({x4}) should roughly halve naive alias energy ({off}) or better"
    );
}

#[test]
fn polyblep_square_suppresses_aliasing() {
    let bins = signal_bins(true);
    let naive = alias_to_signal_ratio(&render(true, true), &bins);
    let bandlimited = alias_to_signal_ratio(&render(false, true), &bins);
    println!(
        "square @ {:.1} Hz: naive alias/signal = {:.4}, band-limited = {:.4}",
        fundamental_hz(),
        naive,
        bandlimited
    );

    assert!(
        naive > 0.02,
        "naive square should show measurable aliasing, got {naive}"
    );
    assert!(
        bandlimited < naive * 0.25,
        "band-limited square alias/signal {bandlimited} not <0.25x naive {naive}"
    );
    assert!(
        bandlimited < 0.01,
        "band-limited square alias/signal {bandlimited} should be near-clean (<0.01)"
    );
}
