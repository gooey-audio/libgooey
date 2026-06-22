//! Stereo, immutable, reference-counted sample data for loop playback.
//!
//! Unlike the granulator's mono [`crate::instruments::SampleBuffer`], a loop
//! channel keeps both channels so a stereo loop plays back with its original
//! image intact. Fractional read positions are resolved with the shared
//! [`cubic_interpolate`] so varispeed / sample-rate conversion stays click-free.

use std::sync::Arc;

use crate::frame::StereoFrame;
use crate::utils::cubic_interpolate;

/// Shared stereo sample data. Cloning is cheap (two `Arc` bumps).
#[derive(Clone, Debug)]
pub struct StereoSampleBuffer {
    left: Arc<[f32]>,
    right: Arc<[f32]>,
    sample_rate: f32,
}

impl StereoSampleBuffer {
    /// Build from de-interleaved left/right channels. Both must be the same
    /// non-zero length and the sample rate must be positive and finite.
    pub fn from_channels(
        left: Vec<f32>,
        right: Vec<f32>,
        sample_rate: f32,
    ) -> Result<Self, String> {
        if left.is_empty() || right.is_empty() {
            return Err("StereoSampleBuffer requires at least one frame".to_string());
        }
        if left.len() != right.len() {
            return Err(format!(
                "StereoSampleBuffer channels must match: left={}, right={}",
                left.len(),
                right.len()
            ));
        }
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            return Err(format!("Invalid sample rate: {sample_rate}"));
        }
        if left.iter().chain(right.iter()).any(|s| !s.is_finite()) {
            return Err("StereoSampleBuffer samples must be finite".to_string());
        }

        Ok(Self {
            left: Arc::from(left.into_boxed_slice()),
            right: Arc::from(right.into_boxed_slice()),
            sample_rate,
        })
    }

    /// Build from an interleaved frame buffer with `channels` samples per frame.
    /// A mono source (`channels == 1`) is duplicated to both sides; a source
    /// with two or more channels uses channels 0 and 1 as left/right.
    pub fn from_interleaved(
        samples: &[f32],
        channels: usize,
        sample_rate: f32,
    ) -> Result<Self, String> {
        if channels == 0 {
            return Err("StereoSampleBuffer requires at least one channel".to_string());
        }
        if samples.is_empty() {
            return Err("StereoSampleBuffer requires at least one sample".to_string());
        }

        let frames = samples.len() / channels;
        if frames == 0 {
            return Err("StereoSampleBuffer requires at least one full frame".to_string());
        }

        let mut left = Vec::with_capacity(frames);
        let mut right = Vec::with_capacity(frames);
        for frame in samples.chunks_exact(channels) {
            if channels == 1 {
                left.push(frame[0]);
                right.push(frame[0]);
            } else {
                left.push(frame[0]);
                right.push(frame[1]);
            }
        }

        Self::from_channels(left, right, sample_rate)
    }

    /// Load a (mono or stereo) WAV file, preserving the stereo image.
    /// Mono files are duplicated to both channels; files with more than two
    /// channels keep channels 0 and 1.
    #[cfg(feature = "bounce")]
    pub fn from_wav(path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        let mut reader = hound::WavReader::open(path.as_ref())
            .map_err(|e| format!("Failed to open WAV: {e}"))?;
        let spec = reader.spec();
        if spec.channels == 0 {
            return Err("WAV must have at least one channel".to_string());
        }
        if spec.sample_rate == 0 {
            return Err("WAV sample rate must be greater than zero".to_string());
        }

        let channels = spec.channels as usize;
        let interleaved = match spec.sample_format {
            hound::SampleFormat::Float => reader
                .samples::<f32>()
                .map(|s| s.map_err(|e| format!("Failed to read WAV sample: {e}")))
                .collect::<Result<Vec<_>, _>>()?,
            hound::SampleFormat::Int => match spec.bits_per_sample {
                0 => return Err("WAV bit depth must be greater than zero".to_string()),
                1..=8 => {
                    let scale = ((1_i32 << (spec.bits_per_sample - 1)) - 1) as f32;
                    reader
                        .samples::<i8>()
                        .map(|s| {
                            s.map(|v| v as f32 / scale)
                                .map_err(|e| format!("Failed to read WAV sample: {e}"))
                        })
                        .collect::<Result<Vec<_>, _>>()?
                }
                9..=16 => {
                    let scale = ((1_i32 << (spec.bits_per_sample - 1)) - 1) as f32;
                    reader
                        .samples::<i16>()
                        .map(|s| {
                            s.map(|v| v as f32 / scale)
                                .map_err(|e| format!("Failed to read WAV sample: {e}"))
                        })
                        .collect::<Result<Vec<_>, _>>()?
                }
                17..=32 => {
                    let scale = ((1_i64 << (spec.bits_per_sample - 1)) - 1) as f32;
                    reader
                        .samples::<i32>()
                        .map(|s| {
                            s.map(|v| v as f32 / scale)
                                .map_err(|e| format!("Failed to read WAV sample: {e}"))
                        })
                        .collect::<Result<Vec<_>, _>>()?
                }
                bits => return Err(format!("Unsupported WAV bit depth: {bits}")),
            },
        };

        if interleaved.is_empty() {
            return Err("WAV contains no samples".to_string());
        }

        Self::from_interleaved(&interleaved, channels, spec.sample_rate as f32)
    }

    /// Number of stereo frames in the buffer.
    pub fn len(&self) -> usize {
        self.left.len()
    }

    pub fn is_empty(&self) -> bool {
        self.left.is_empty()
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    #[inline]
    fn channel_clamped(channel: &[f32], index: isize) -> f32 {
        let last = channel.len() as isize - 1;
        channel[index.clamp(0, last) as usize]
    }

    /// Read a stereo frame at a fractional frame position using cubic
    /// interpolation. The position is clamped into the valid range; callers
    /// that loop are responsible for wrapping `position` before calling.
    #[inline]
    pub fn read_interpolated(&self, position: f64) -> StereoFrame {
        if self.left.len() == 1 {
            return StereoFrame {
                l: self.left[0],
                r: self.right[0],
            };
        }

        let last = (self.left.len() - 1) as f64;
        let position = position.clamp(0.0, last);
        let index = position.floor() as isize;
        let frac = (position - index as f64) as f32;

        let read = |channel: &[f32]| {
            let p0 = Self::channel_clamped(channel, index - 1);
            let p1 = Self::channel_clamped(channel, index);
            let p2 = Self::channel_clamped(channel, index + 1);
            let p3 = Self::channel_clamped(channel, index + 2);
            cubic_interpolate(p0, p1, p2, p3, frac)
        };

        StereoFrame {
            l: read(&self.left),
            r: read(&self.right),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_interleaved_mono_duplicates_to_both_channels() {
        let buf = StereoSampleBuffer::from_interleaved(&[0.1, 0.2, 0.3], 1, 44100.0).unwrap();
        assert_eq!(buf.len(), 3);
        let f = buf.read_interpolated(1.0);
        assert_eq!(f.l, 0.2);
        assert_eq!(f.r, 0.2);
    }

    #[test]
    fn from_interleaved_stereo_splits_left_right() {
        // frames: (1.0, -1.0), (0.5, -0.5)
        let buf =
            StereoSampleBuffer::from_interleaved(&[1.0, -1.0, 0.5, -0.5], 2, 48000.0).unwrap();
        assert_eq!(buf.len(), 2);
        assert_eq!(buf.sample_rate(), 48000.0);
        let f = buf.read_interpolated(0.0);
        assert_eq!(f.l, 1.0);
        assert_eq!(f.r, -1.0);
    }

    #[test]
    fn mismatched_channels_rejected() {
        assert!(StereoSampleBuffer::from_channels(vec![0.0, 0.1], vec![0.0], 44100.0).is_err());
    }

    #[test]
    fn non_finite_rejected() {
        assert!(StereoSampleBuffer::from_channels(vec![f32::NAN], vec![0.0], 44100.0).is_err());
    }
}
