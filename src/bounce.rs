//! Offline audio bounce/export
//!
//! Renders the engine's audio output to a buffer or WAV file faster than
//! real-time, without requiring audio hardware.

use crate::engine::Engine;

/// Specifies how long to render.
pub enum BounceLength {
    /// Number of bars in 4/4 time
    Bars(usize),
    /// Number of quarter-note beats
    Beats(f64),
    /// Exact number of samples
    Samples(usize),
}

impl BounceLength {
    /// Convert to a sample count given BPM and sample rate.
    fn to_samples(&self, bpm: f32, sample_rate: f32) -> usize {
        match self {
            BounceLength::Bars(bars) => {
                let samples_per_bar = 4.0 * (60.0 / bpm as f64) * sample_rate as f64;
                (*bars as f64 * samples_per_bar).round() as usize
            }
            BounceLength::Beats(beats) => {
                let samples_per_beat = (60.0 / bpm as f64) * sample_rate as f64;
                (beats * samples_per_beat).round() as usize
            }
            BounceLength::Samples(n) => *n,
        }
    }
}

/// Render the engine's current configuration to a sample buffer.
///
/// This resets all sequencers to beat 0, renders the requested length,
/// then stops all sequencers. The render runs as fast as possible (offline).
///
/// Returns a `Vec<f32>` of mono audio samples at the engine's sample rate.
pub fn bounce_to_buffer(engine: &mut Engine, length: BounceLength) -> Vec<f32> {
    let total_samples = length.to_samples(engine.bpm(), engine.sample_rate());
    let sample_rate = engine.sample_rate() as f64;

    engine.prepare_for_bounce();

    let mut buffer = Vec::with_capacity(total_samples);
    let mut current_time: f64 = 0.0;
    let time_step = 1.0 / sample_rate;

    for _ in 0..total_samples {
        buffer.push(engine.tick(current_time));
        current_time += time_step;
    }

    engine.stop_all_sequencers();

    buffer
}

/// Configuration for WAV file output.
#[cfg(feature = "bounce")]
pub struct WavConfig {
    /// Bit depth: 16 or 24. Defaults to 16.
    pub bit_depth: u16,
}

#[cfg(feature = "bounce")]
impl Default for WavConfig {
    fn default() -> Self {
        Self { bit_depth: 16 }
    }
}

/// Render the engine to a WAV file.
///
/// This calls [`bounce_to_buffer`] internally, then writes the result
/// to the specified path as a mono WAV file.
#[cfg(feature = "bounce")]
pub fn bounce_to_wav(
    engine: &mut Engine,
    length: BounceLength,
    path: &std::path::Path,
    config: WavConfig,
) -> Result<(), String> {
    if config.bit_depth != 16 && config.bit_depth != 24 {
        return Err(format!(
            "Unsupported bit depth: {}. Use 16 or 24.",
            config.bit_depth
        ));
    }

    let sample_rate = engine.sample_rate();
    let buffer = bounce_to_buffer(engine, length);

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: sample_rate as u32,
        bits_per_sample: config.bit_depth,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer =
        hound::WavWriter::create(path, spec).map_err(|e| format!("Failed to create WAV: {e}"))?;

    match config.bit_depth {
        16 => {
            let scale = i16::MAX as f32;
            for &sample in &buffer {
                let s = (sample * scale).round() as i16;
                writer
                    .write_sample(s)
                    .map_err(|e| format!("Failed to write sample: {e}"))?;
            }
        }
        24 => {
            let scale = 8_388_607.0_f32; // 2^23 - 1
            for &sample in &buffer {
                let s = (sample * scale).round() as i32;
                writer
                    .write_sample(s)
                    .map_err(|e| format!("Failed to write sample: {e}"))?;
            }
        }
        _ => unreachable!("bit depth validated above"),
    }

    writer
        .finalize()
        .map_err(|e| format!("Failed to finalize WAV: {e}"))?;

    Ok(())
}
