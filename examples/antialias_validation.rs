//! Generate off/2x/4x artifacts and measurements for nonlinear anti-aliasing.
//!
//! Run with:
//! `cargo run --release --example antialias_validation --no-default-features --features bounce`

use gooey::utils::{Oversampler, OversamplingMode};
use std::f64::consts::TAU;
use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

const SAMPLE_RATE: f32 = 48_000.0;
const DRIVE: f32 = 10.0;
const WARMUP_SAMPLES: usize = 1_024;
const MEASUREMENT_SAMPLES: usize = 4_800;

fn shape(input: f32) -> f32 {
    (input * DRIVE).tanh()
}

fn coherent_sine(sample: usize) -> f32 {
    (std::f32::consts::TAU * 10_000.0 * sample as f32 / SAMPLE_RATE).sin() * 0.8
}

fn bin_power(samples: &[f32], frequency: f32) -> f64 {
    let phase_step = TAU * frequency as f64 / SAMPLE_RATE as f64;
    let (real, imag) =
        samples
            .iter()
            .enumerate()
            .fold((0.0_f64, 0.0_f64), |(real, imag), (i, &x)| {
                let phase = phase_step * i as f64;
                (real + x as f64 * phase.cos(), imag - x as f64 * phase.sin())
            });
    real * real + imag * imag
}

fn combined_alias_power(samples: &[f32]) -> f64 {
    [2_000.0, 18_000.0, 22_000.0]
        .iter()
        .map(|&frequency| bin_power(samples, frequency))
        .sum()
}

struct ModeRenders {
    off: Vec<f32>,
    x2: Vec<f32>,
    x4: Vec<f32>,
}

fn render_alias_measurement() -> ModeRenders {
    let mut off = Oversampler::new(OversamplingMode::Off);
    let mut x2 = Oversampler::new(OversamplingMode::X2);
    let mut x4 = Oversampler::new(OversamplingMode::X4);
    let mut renders = ModeRenders {
        off: Vec::with_capacity(MEASUREMENT_SAMPLES),
        x2: Vec::with_capacity(MEASUREMENT_SAMPLES),
        x4: Vec::with_capacity(MEASUREMENT_SAMPLES),
    };

    for i in 0..WARMUP_SAMPLES + MEASUREMENT_SAMPLES {
        let input = coherent_sine(i);
        let off_output = off.process(input, shape);
        let x2_output = x2.process(input, shape);
        let x4_output = x4.process(input, shape);
        if i >= WARMUP_SAMPLES {
            renders.off.push(off_output);
            renders.x2.push(x2_output);
            renders.x4.push(x4_output);
        }
    }

    renders
}

fn render_sweeps() -> ModeRenders {
    let duration_seconds = 5.0;
    let samples = (duration_seconds * SAMPLE_RATE) as usize;
    let start_frequency = 1_000.0_f32;
    let end_frequency = 20_000.0_f32;
    let frequency_ratio = end_frequency / start_frequency;
    let mut phase = 0.0_f32;
    let mut off = Oversampler::new(OversamplingMode::Off);
    let mut x2 = Oversampler::new(OversamplingMode::X2);
    let mut x4 = Oversampler::new(OversamplingMode::X4);
    let mut renders = ModeRenders {
        off: Vec::with_capacity(samples),
        x2: Vec::with_capacity(samples),
        x4: Vec::with_capacity(samples),
    };

    for i in 0..samples {
        let progress = i as f32 / samples as f32;
        let frequency = start_frequency * frequency_ratio.powf(progress);
        phase = (phase + std::f32::consts::TAU * frequency / SAMPLE_RATE)
            .rem_euclid(std::f32::consts::TAU);
        let input = phase.sin() * 0.8;
        renders.off.push(off.process(input, shape));
        renders.x2.push(x2.process(input, shape));
        renders.x4.push(x4.process(input, shape));
    }

    renders
}

fn write_float_wav(path: &Path, samples: &[f32]) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE as u32,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(|error| error.to_string())?;
    for &sample in samples {
        writer
            .write_sample(sample)
            .map_err(|error| error.to_string())?;
    }
    writer.finalize().map_err(|error| error.to_string())
}

fn benchmark_mode(mode: OversamplingMode) -> std::time::Duration {
    const SAMPLES: usize = 5_000_000;
    let mut oversampler = Oversampler::new(mode);

    let start = Instant::now();
    let mut output = 0.0_f32;
    for i in 0..SAMPLES {
        output = oversampler.process(black_box(coherent_sine(i)), shape);
    }
    black_box(output);
    start.elapsed()
}

fn benchmark() {
    const SAMPLES: usize = 5_000_000;
    let off_elapsed = benchmark_mode(OversamplingMode::Off);
    let x2_elapsed = benchmark_mode(OversamplingMode::X2);
    let x4_elapsed = benchmark_mode(OversamplingMode::X4);

    println!(
        "Off throughput: {:.2} ns/sample",
        off_elapsed.as_nanos() as f64 / SAMPLES as f64
    );
    println!(
        "2x throughput: {:.2} ns/sample ({:.2}x off cost)",
        x2_elapsed.as_nanos() as f64 / SAMPLES as f64,
        x2_elapsed.as_secs_f64() / off_elapsed.as_secs_f64()
    );
    println!(
        "4x throughput: {:.2} ns/sample ({:.2}x off cost)",
        x4_elapsed.as_nanos() as f64 / SAMPLES as f64,
        x4_elapsed.as_secs_f64() / off_elapsed.as_secs_f64()
    );
}

fn main() -> Result<(), String> {
    let output_dir = Path::new(".context/antialias-validation");
    std::fs::create_dir_all(output_dir).map_err(|error| error.to_string())?;

    let measurement = render_alias_measurement();
    let off_alias_power = combined_alias_power(&measurement.off);
    let x2_reduction_db = 10.0 * (off_alias_power / combined_alias_power(&measurement.x2)).log10();
    let x4_reduction_db = 10.0 * (off_alias_power / combined_alias_power(&measurement.x4)).log10();
    println!("2x known-bin alias reduction versus off: {x2_reduction_db:.2} dB");
    println!("4x known-bin alias reduction versus off: {x4_reduction_db:.2} dB");

    let sweeps = render_sweeps();
    let base_path = output_dir.join("base-rate-sweep.wav");
    let x2_path = output_dir.join("oversampled-2x-sweep.wav");
    let x4_path = output_dir.join("oversampled-4x-sweep.wav");
    write_float_wav(&base_path, &sweeps.off)?;
    write_float_wav(&x2_path, &sweeps.x2)?;
    write_float_wav(&x4_path, &sweeps.x4)?;
    println!("Wrote {}", base_path.display());
    println!("Wrote {}", x2_path.display());
    println!("Wrote {}", x4_path.display());

    benchmark();
    Ok(())
}
