//! Visual aliasing validation: render spectrum and spectrogram PNGs so aliasing
//! artifacts can be *seen* and proven, not just heard.
//!
//! Run with:
//! `cargo run --release --example aliasing_plots --features plots`
//!
//! Render a single targeted spectrum instead of the full suite:
//! `cargo run --release --example aliasing_plots --features plots -- --waveform saw --freq 2200`
//!
//! Outputs (PNGs + source WAVs) land in `aliasing-plots/` (gitignored).
//!
//! What to look for:
//! - Steady-tone spectra: real partials sit on the green harmonic markers. Any spike
//!   *between* the markers is aliasing (a partial folded back below Nyquist).
//! - Sweep spectrograms: real content sweeps upward; aliasing shows as lines sweeping
//!   *downward*, reflecting off the Nyquist ceiling.

use std::error::Error;
use std::f64::consts::TAU;
use std::path::{Path, PathBuf};

use gooey::envelope::ADSRConfig;
use gooey::gen::polyblep::{polyblep_saw, polyblep_square};
use gooey::gen::{Oscillator, Waveform};
use gooey::utils::{Oversampler, OversamplingMode};

use plotters::prelude::*;
use rustfft::{num_complex::Complex, FftPlanner};

const SAMPLE_RATE: f32 = 48_000.0;
/// Drive used for the nonlinear (tanh) anti-alias demonstration. Matches the spirit of
/// `examples/antialias_validation.rs`.
const DRIVE: f32 = 10.0;

// ---------------------------------------------------------------------------
// Signal generation
// ---------------------------------------------------------------------------

/// Render a steady tone from the *shipping* `Oscillator` (exercises PolyBLEP saw/square,
/// Gibbs-tapered triangle, etc.). The envelope is held at full sustain and never released,
/// so the captured region has constant amplitude. A warmup is discarded so the attack ramp
/// does not pollute the spectrum.
fn render_osc_tone(waveform: Waveform, freq: f32, n: usize, antialias: bool) -> Vec<f32> {
    let warmup = 4096usize;
    let mut osc = Oscillator::new(SAMPLE_RATE, freq);
    osc.waveform = waveform;
    osc.set_antialias(antialias);
    osc.set_adsr(ADSRConfig::new(0.001, 0.001, 1.0, 0.001));
    osc.set_volume(1.0);
    osc.trigger(0.0);

    let mut out = Vec::with_capacity(n);
    for i in 0..(warmup + n) {
        let t = i as f64 / SAMPLE_RATE as f64;
        let s = osc.tick(t);
        if i >= warmup {
            out.push(s);
        }
    }
    out
}

/// Render a steady tone at a fixed frequency via an `f64` phase accumulator, evaluating
/// `gen(phase, phase_inc)` per sample. Used for the controlled naive-vs-band-limited
/// comparison: the only difference between the two is the generator, not the (jitter-free)
/// phase source.
fn render_tone(freq: f32, n: usize, mut gen: impl FnMut(f64, f64) -> f32) -> Vec<f32> {
    let dt = freq as f64 / SAMPLE_RATE as f64;
    let mut phase = 0.0f64;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(gen(phase, dt));
        phase = (phase + dt).rem_euclid(1.0);
    }
    out
}

/// Render a fixed-frequency oscillator generated at `mode`'s oversampling factor and
/// decimated back to the base rate via the same half-band `Oversampler` the engine uses for
/// nonlinear effects. The oscillator is the *signal source* (the oversampler's input is
/// unused), so this is classic oversampled generation: synthesize fast, low-pass, decimate.
/// `square`/`antialias` pick which generator runs at the oversampled rate.
fn render_osc_oversampled(
    square: bool,
    antialias: bool,
    mode: OversamplingMode,
    freq: f32,
    n: usize,
) -> Vec<f32> {
    let factor = mode.factor();
    let dt = freq as f64 / SAMPLE_RATE as f64;
    let sub_dt = dt / factor as f64; // phase step at the oversampled rate
    let mut phase = 0.0f64;
    let mut os = Oversampler::new(mode);

    let warmup = 4096usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..(warmup + n) {
        let s = os.process(0.0, |_| {
            let v = if square {
                if antialias {
                    polyblep_square(phase, sub_dt)
                } else if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            } else if antialias {
                polyblep_saw(phase, sub_dt)
            } else {
                (2.0 * phase - 1.0) as f32
            };
            phase = (phase + sub_dt).rem_euclid(1.0);
            v
        });
        if i >= warmup {
            out.push(s);
        }
    }
    out
}

/// Render an exponential frequency sweep, evaluating `gen(phase, phase_inc)` per sample.
/// The phase is integrated by a proper accumulator so the sweep itself introduces no
/// artifacts (unlike the oscillator's time-based phase, which jumps when frequency changes).
fn render_sweep(
    start_hz: f32,
    end_hz: f32,
    dur_s: f32,
    mut gen: impl FnMut(f64, f64) -> f32,
) -> Vec<f32> {
    let n = (dur_s * SAMPLE_RATE) as usize;
    let ratio = (end_hz / start_hz) as f64;
    let mut phase = 0.0f64;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let p = i as f64 / n as f64;
        let freq = start_hz as f64 * ratio.powf(p);
        let dt = freq / SAMPLE_RATE as f64;
        out.push(gen(phase, dt));
        phase = (phase + dt).rem_euclid(1.0);
    }
    out
}

/// Render a sine sweep pushed through a tanh nonlinearity at the given oversampling mode.
/// Higher oversampling should visibly shrink the downward-sweeping alias bands.
fn render_nonlinear_sweep(
    mode: OversamplingMode,
    start_hz: f32,
    end_hz: f32,
    dur_s: f32,
) -> Vec<f32> {
    let n = (dur_s * SAMPLE_RATE) as usize;
    let ratio = (end_hz / start_hz) as f64;
    let mut phase = 0.0f64;
    let mut os = Oversampler::new(mode);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let p = i as f64 / n as f64;
        let freq = start_hz as f64 * ratio.powf(p);
        let dt = freq / SAMPLE_RATE as f64;
        let input = (TAU * phase).sin() as f32 * 0.8;
        out.push(os.process(input, |x| (x * DRIVE).tanh()));
        phase = (phase + dt).rem_euclid(1.0);
    }
    out
}

// ---------------------------------------------------------------------------
// FFT analysis (Hann window -> rustfft -> magnitude -> dB), mirroring the recipe in
// src/visualization/spectrogram.rs but without the GUI-gated `visualization` feature.
// ---------------------------------------------------------------------------

fn hann(i: usize, n: usize) -> f32 {
    0.5 * (1.0 - (TAU as f32 * i as f32 / n as f32).cos())
}

/// Magnitude spectrum of the last `fft_size` samples, in dB, peak-normalized to 0 dB.
/// Bin `i` corresponds to frequency `i * SAMPLE_RATE / fft_size`.
fn spectrum_db(samples: &[f32], fft_size: usize, planner: &mut FftPlanner<f32>) -> Vec<f32> {
    let start = samples.len().saturating_sub(fft_size);
    let frame = &samples[start..];
    let mut buf: Vec<Complex<f32>> = (0..fft_size)
        .map(|i| {
            let s = frame.get(i).copied().unwrap_or(0.0);
            Complex::new(s * hann(i, fft_size), 0.0)
        })
        .collect();

    let fft = planner.plan_fft_forward(fft_size);
    fft.process(&mut buf);

    let num_bins = fft_size / 2;
    let mags: Vec<f32> = buf[..num_bins]
        .iter()
        .map(|c| (c.re * c.re + c.im * c.im).sqrt())
        .collect();
    let max = mags.iter().copied().fold(1e-30f32, f32::max);
    mags.iter()
        .map(|m| 20.0 * (m / max + 1e-12).log10())
        .collect()
}

/// Sliding-window spectrogram. Returns one column per hop; each column is `fft_size/2`
/// magnitudes in dB, normalized so the global peak across the whole signal is 0 dB.
fn spectrogram(
    samples: &[f32],
    fft_size: usize,
    hop: usize,
    planner: &mut FftPlanner<f32>,
) -> Vec<Vec<f32>> {
    let num_bins = fft_size / 2;
    let fft = planner.plan_fft_forward(fft_size);

    let mut cols: Vec<Vec<f32>> = Vec::new();
    let mut global_max = 1e-30f32;
    let mut start = 0;
    while start + fft_size <= samples.len() {
        let frame = &samples[start..start + fft_size];
        let mut buf: Vec<Complex<f32>> = (0..fft_size)
            .map(|i| Complex::new(frame[i] * hann(i, fft_size), 0.0))
            .collect();
        fft.process(&mut buf);
        let mags: Vec<f32> = buf[..num_bins]
            .iter()
            .map(|c| (c.re * c.re + c.im * c.im).sqrt())
            .collect();
        for &m in &mags {
            if m > global_max {
                global_max = m;
            }
        }
        cols.push(mags);
        start += hop;
    }

    for mags in &mut cols {
        for m in mags.iter_mut() {
            *m = 20.0 * (*m / global_max + 1e-12).log10();
        }
    }
    cols
}

// ---------------------------------------------------------------------------
// Plotting
// ---------------------------------------------------------------------------

/// "Hot" colormap: black -> red -> yellow -> white as `t` goes 0..1 (louder = brighter).
fn heat(t: f32) -> (u8, u8, u8) {
    let r = (t * 3.0).clamp(0.0, 1.0);
    let g = (t * 3.0 - 1.0).clamp(0.0, 1.0);
    let b = (t * 3.0 - 2.0).clamp(0.0, 1.0);
    ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

fn plot_spectrum(
    path: &Path,
    title: &str,
    spec_db: &[f32],
    fft_size: usize,
    markers: Option<f32>,
) -> Result<(), Box<dyn Error>> {
    let root = BitMapBackend::new(path, (1200, 600)).into_drawing_area();
    root.fill(&WHITE)?;
    let nyquist = SAMPLE_RATE / 2.0;

    let mut chart = ChartBuilder::on(&root)
        .caption(title, ("sans-serif", 24))
        .margin(15)
        .x_label_area_size(45)
        .y_label_area_size(60)
        .build_cartesian_2d(0f32..nyquist, -120f32..5f32)?;

    chart
        .configure_mesh()
        .x_desc("Frequency (Hz)")
        .y_desc("Magnitude (dB)")
        .draw()?;

    // Vertical markers at the true harmonic frequencies. Real partials land on these lines;
    // aliased components land between them.
    if let Some(f0) = markers {
        let mut k = 1;
        while (k as f32) * f0 < nyquist {
            let f = (k as f32) * f0;
            chart.draw_series(std::iter::once(PathElement::new(
                vec![(f, -120f32), (f, 5f32)],
                ShapeStyle::from(RGBColor(0, 150, 0).mix(0.25)).stroke_width(1),
            )))?;
            k += 1;
        }
    }

    let bin_hz = SAMPLE_RATE / fft_size as f32;
    chart.draw_series(LineSeries::new(
        spec_db
            .iter()
            .enumerate()
            .map(|(i, &db)| (i as f32 * bin_hz, db)),
        BLUE.stroke_width(1),
    ))?;

    root.present()?;
    Ok(())
}

fn spectrogram_rgb(cols: &[Vec<f32>], num_bins: usize, w: u32, h: u32) -> Vec<u8> {
    let num_cols = cols.len().max(1);
    // Display floor: bins at/below this map to black so only the stronger partials and
    // alias lines show. -72 dB keeps the leakage background dark while preserving foldback.
    let floor = -72.0f32;
    let mut buf = vec![0u8; (w * h * 3) as usize];
    let h_div = (h.max(2) - 1) as f32;
    for py in 0..h {
        // py = 0 is the top of the image, which we map to Nyquist (high frequency).
        let frac = 1.0 - (py as f32) / h_div;
        let bin = ((frac * (num_bins as f32 - 1.0)).round() as usize).min(num_bins - 1);
        for px in 0..w {
            let col = (((px as f32 / w as f32) * num_cols as f32) as usize).min(num_cols - 1);
            let db = cols[col].get(bin).copied().unwrap_or(floor);
            let t = ((db - floor) / (0.0 - floor)).clamp(0.0, 1.0);
            let (r, g, b) = heat(t);
            let idx = ((py * w + px) * 3) as usize;
            buf[idx] = r;
            buf[idx + 1] = g;
            buf[idx + 2] = b;
        }
    }
    buf
}

fn plot_spectrogram(
    path: &Path,
    title: &str,
    cols: &[Vec<f32>],
    fft_size: usize,
    dur_s: f32,
) -> Result<(), Box<dyn Error>> {
    let root = BitMapBackend::new(path, (1200, 600)).into_drawing_area();
    root.fill(&WHITE)?;
    let nyquist = SAMPLE_RATE / 2.0;

    let mut chart = ChartBuilder::on(&root)
        .caption(title, ("sans-serif", 24))
        .margin(15)
        .x_label_area_size(45)
        .y_label_area_size(60)
        .build_cartesian_2d(0f32..dur_s, 0f32..nyquist)?;

    chart
        .configure_mesh()
        .disable_mesh()
        .x_desc("Time (s)")
        .y_desc("Frequency (Hz)")
        .draw()?;

    let (w_px, h_px) = chart.plotting_area().dim_in_pixel();
    let img = spectrogram_rgb(cols, fft_size / 2, w_px, h_px);
    let elem = BitMapElement::with_owned_buffer((0f32, nyquist), (w_px, h_px), img)
        .ok_or("spectrogram bitmap buffer size mismatch")?;
    chart.draw_series(std::iter::once(elem))?;

    root.present()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WAV export (for cross-checking in an external meter)
// ---------------------------------------------------------------------------

fn write_wav(path: &Path, samples: &[f32]) -> Result<(), Box<dyn Error>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE as u32,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for &s in samples {
        writer.write_sample(s)?;
    }
    writer.finalize()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Misc helpers / CLI
// ---------------------------------------------------------------------------

fn waveform_name(w: Waveform) -> &'static str {
    match w {
        Waveform::Sine => "sine",
        Waveform::Square => "square",
        Waveform::Saw => "saw",
        Waveform::Triangle => "triangle",
        Waveform::RingMod => "ringmod",
        Waveform::Noise => "noise",
    }
}

fn parse_waveform(s: &str) -> Option<Waveform> {
    match s.to_ascii_lowercase().as_str() {
        "sine" => Some(Waveform::Sine),
        "square" => Some(Waveform::Square),
        "saw" => Some(Waveform::Saw),
        "triangle" => Some(Waveform::Triangle),
        "ringmod" | "ring_mod" => Some(Waveform::RingMod),
        "noise" => Some(Waveform::Noise),
        _ => None,
    }
}

/// Harmonic markers only make sense for the pitched, harmonically-structured waveforms.
fn marker_for(w: Waveform, f0: f32) -> Option<f32> {
    match w {
        Waveform::Sine | Waveform::Square | Waveform::Saw | Waveform::Triangle => Some(f0),
        Waveform::RingMod | Waveform::Noise => None,
    }
}

/// Parse `--waveform <name> --freq <hz>` for single-shot mode. Returns None if not requested.
fn parse_single_shot(args: &[String]) -> Option<(Waveform, f32)> {
    let mut waveform = None;
    let mut freq = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--waveform" => {
                waveform = args.get(i + 1).and_then(|s| parse_waveform(s));
                i += 2;
            }
            "--freq" => {
                freq = args.get(i + 1).and_then(|s| s.parse::<f32>().ok());
                i += 2;
            }
            _ => i += 1,
        }
    }
    match (waveform, freq) {
        (Some(w), Some(f)) => Some((w, f)),
        _ => None,
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let out = PathBuf::from("aliasing-plots");
    std::fs::create_dir_all(&out)?;
    let mut planner = FftPlanner::<f32>::new();

    const SPEC_FFT: usize = 1 << 14; // 16384
    const SPEC_N: usize = 1 << 15; // 32768 captured samples
    const SG_FFT: usize = 2048;
    const SG_HOP: usize = 512;
    const SWEEP_DUR: f32 = 5.0;

    let args: Vec<String> = std::env::args().collect();

    // Single-shot: render just one spectrum and exit.
    if let Some((w, freq)) = parse_single_shot(&args) {
        let sig = render_osc_tone(w, freq, SPEC_N, true);
        let db = spectrum_db(&sig, SPEC_FFT, &mut planner);
        let name = format!("spectrum-{}-{}hz.png", waveform_name(w), freq as u32);
        let title = format!(
            "{} @ {:.0} Hz (sr {:.0} Hz)",
            waveform_name(w),
            freq,
            SAMPLE_RATE
        );
        plot_spectrum(&out.join(&name), &title, &db, SPEC_FFT, marker_for(w, freq))?;
        println!("Wrote {}", out.join(&name).display());
        return Ok(());
    }

    // ---- Full default suite ----
    println!("Rendering aliasing plots to {}", out.display());

    // 1) Steady-tone spectra at a high fundamental where harmonics fold.
    let f0 = 2200.0f32;

    // 1a) Controlled comparison: identical (jitter-free) phase source, naive vs band-limited.
    // Real partials land on the green harmonic markers; aliasing shows as spikes between them.
    type ToneGen = (&'static str, &'static str, Box<dyn FnMut(f64, f64) -> f32>);
    let tone_gens: Vec<ToneGen> = vec![
        (
            "sine",
            "sine (reference, band-limited by nature)",
            Box::new(|phase, _| (TAU * phase).sin() as f32),
        ),
        (
            "saw-naive",
            "saw (NAIVE / aliased)",
            Box::new(|phase, _| (2.0 * phase - 1.0) as f32),
        ),
        (
            "saw-bandlimited",
            "saw (PolyBLEP band-limited)",
            Box::new(polyblep_saw),
        ),
        (
            "square-naive",
            "square (NAIVE / aliased)",
            Box::new(|phase, _| if phase < 0.5 { 1.0 } else { -1.0 }),
        ),
        (
            "square-bandlimited",
            "square (PolyBLEP band-limited)",
            Box::new(polyblep_square),
        ),
    ];
    for (slug, label, gen) in tone_gens {
        let sig = render_tone(f0, SPEC_N, gen);
        let db = spectrum_db(&sig, SPEC_FFT, &mut planner);
        let name = format!("spectrum-{}-{}hz.png", slug, f0 as u32);
        let title = format!("{} @ {:.0} Hz, sr {:.0} Hz", label, f0, SAMPLE_RATE);
        plot_spectrum(&out.join(&name), &title, &db, SPEC_FFT, Some(f0))?;
        println!("  wrote {}", name);
    }

    // 1b) The shipping Oscillator path (all 6 waveforms, as actually used by the engine).
    // Note: the Oscillator reconstructs phase from f32 time, which adds its own low-level
    // broadband floor on top of any aliasing — these are the "as-shipped" reality, not the
    // isolated band-limiting algorithm shown in 1a.
    for w in [
        Waveform::Sine,
        Waveform::Saw,
        Waveform::Square,
        Waveform::Triangle,
        Waveform::RingMod,
        Waveform::Noise,
    ] {
        let sig = render_osc_tone(w, f0, SPEC_N, true);
        let db = spectrum_db(&sig, SPEC_FFT, &mut planner);
        let name = format!("osc-{}-{}hz.png", waveform_name(w), f0 as u32);
        let title = format!(
            "{} (Oscillator, as shipped) @ {:.0} Hz, sr {:.0} Hz",
            waveform_name(w),
            f0,
            SAMPLE_RATE
        );
        plot_spectrum(&out.join(&name), &title, &db, SPEC_FFT, marker_for(w, f0))?;
        println!("  wrote {}", name);
    }

    // 1c) The same Oscillator with anti-aliasing turned OFF (set_antialias(false)) for the
    // waveforms that have a band-limited variant. Compare each against its `osc-*` twin above.
    for w in [Waveform::Saw, Waveform::Square, Waveform::Triangle] {
        let sig = render_osc_tone(w, f0, SPEC_N, false);
        let db = spectrum_db(&sig, SPEC_FFT, &mut planner);
        let name = format!("osc-{}-naive-{}hz.png", waveform_name(w), f0 as u32);
        let title = format!(
            "{} (Oscillator, antialias OFF) @ {:.0} Hz, sr {:.0} Hz",
            waveform_name(w),
            f0,
            SAMPLE_RATE
        );
        plot_spectrum(&out.join(&name), &title, &db, SPEC_FFT, marker_for(w, f0))?;
        println!("  wrote {}", name);
    }

    // 1d) Oversampled generation: synthesize the oscillator at 2x/4x and decimate through the
    // engine's half-band filters. For the naive generator this is an alternative to PolyBLEP
    // (aliasing shrinks as the factor rises); for the PolyBLEP generator it shrinks the
    // residual aliasing PolyBLEP leaves behind. Compare each triplet against its Off plot.
    for (square, wave) in [(false, "saw"), (true, "square")] {
        for (antialias, meth, meth_label) in [(false, "naive", "naive"), (true, "blep", "PolyBLEP")]
        {
            for (mode, ms) in [
                (OversamplingMode::Off, "off"),
                (OversamplingMode::X2, "2x"),
                (OversamplingMode::X4, "4x"),
            ] {
                let sig = render_osc_oversampled(square, antialias, mode, f0, SPEC_N);
                let db = spectrum_db(&sig, SPEC_FFT, &mut planner);
                let name = format!("spectrum-{}-{}-os-{}-{}hz.png", wave, meth, ms, f0 as u32);
                let title = format!(
                    "{} ({}) — {} oversampling @ {:.0} Hz, sr {:.0} Hz",
                    wave, meth_label, ms, f0, SAMPLE_RATE
                );
                plot_spectrum(&out.join(&name), &title, &db, SPEC_FFT, Some(f0))?;
                println!("  wrote {}", name);
            }
        }
    }

    // 2) Sweep spectrograms (the foldback picture). Oscillators: naive vs band-limited.
    let sweep_start = 200.0f32;
    let sweep_end = 8000.0f32;

    let osc_sweeps: [(&str, Vec<f32>); 4] = [
        (
            "saw-naive",
            render_sweep(sweep_start, sweep_end, SWEEP_DUR, |phase, _| {
                (2.0 * phase - 1.0) as f32
            }),
        ),
        (
            "saw-bandlimited",
            render_sweep(sweep_start, sweep_end, SWEEP_DUR, |phase, dt| {
                polyblep_saw(phase, dt)
            }),
        ),
        (
            "square-naive",
            render_sweep(sweep_start, sweep_end, SWEEP_DUR, |phase, _| {
                if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }),
        ),
        (
            "square-bandlimited",
            render_sweep(sweep_start, sweep_end, SWEEP_DUR, |phase, dt| {
                polyblep_square(phase, dt)
            }),
        ),
    ];

    for (label, sig) in &osc_sweeps {
        let cols = spectrogram(sig, SG_FFT, SG_HOP, &mut planner);
        let name = format!("spectrogram-{}-sweep.png", label);
        let title = format!(
            "{} sweep {:.0}-{:.0} Hz, sr {:.0} Hz",
            label, sweep_start, sweep_end, SAMPLE_RATE
        );
        plot_spectrogram(&out.join(&name), &title, &cols, SG_FFT, SWEEP_DUR)?;
        write_wav(&out.join(format!("{}-sweep.wav", label)), sig)?;
        println!("  wrote {} (+wav)", name);
    }

    // 3) Nonlinear (tanh) sweep at Off / 2x / 4x oversampling.
    let nl_start = 1000.0f32;
    let nl_end = 20000.0f32;
    for (mode, label) in [
        (OversamplingMode::Off, "off"),
        (OversamplingMode::X2, "2x"),
        (OversamplingMode::X4, "4x"),
    ] {
        let sig = render_nonlinear_sweep(mode, nl_start, nl_end, SWEEP_DUR);
        let cols = spectrogram(&sig, SG_FFT, SG_HOP, &mut planner);
        let name = format!("spectrogram-tanh-{}.png", label);
        let title = format!(
            "tanh(drive={:.0}) sine sweep {:.0}-{:.0} Hz, oversampling {}",
            DRIVE, nl_start, nl_end, label
        );
        plot_spectrogram(&out.join(&name), &title, &cols, SG_FFT, SWEEP_DUR)?;
        write_wav(&out.join(format!("tanh-{}-sweep.wav", label)), &sig)?;
        println!("  wrote {} (+wav)", name);
    }

    println!(
        "Done. Open the PNGs in {} to inspect aliasing.",
        out.display()
    );
    Ok(())
}
