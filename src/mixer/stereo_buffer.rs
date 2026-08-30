//! Stereo, immutable, reference-counted sample data for loop playback.
//!
//! Unlike the granulator's mono [`crate::instruments::SampleBuffer`], a loop
//! channel keeps both channels so a stereo loop plays back with its original
//! image intact. Fractional read positions are resolved with the shared
//! [`cubic_interpolate`] so varispeed / sample-rate conversion stays click-free.
//!
//! # Storage precision
//!
//! Samples are held either as `f32` or as `i16`. Most sample packs — and every
//! WAV this crate writes by default — are 16-bit at the source, so widening
//! them to `f32` doubles memory for no added information. That is affordable
//! for a handful of loops and ruinous for a multi-sampled instrument, where a
//! full piano runs to hundreds of megabytes. [`StereoSampleBuffer::from_wav`]
//! therefore keeps 16-bit sources 16-bit, and only widens when the source
//! genuinely carries more (24/32-bit or float WAVs, or generated audio).
//!
//! Reads go through a monomorphized cubic interpolator, so precision is chosen
//! once per call rather than branching on every tap.

use std::sync::Arc;

use crate::frame::StereoFrame;
use crate::utils::cubic_interpolate;

/// One stored sample, widened to `f32` on read.
///
/// Implemented for the two storage precisions so the interpolator can be
/// generic over them instead of branching per tap.
pub trait StoredSample: Copy {
    fn widen(self) -> f32;
}

impl StoredSample for f32 {
    #[inline]
    fn widen(self) -> f32 {
        self
    }
}

impl StoredSample for i16 {
    #[inline]
    fn widen(self) -> f32 {
        // Divide by 32767 so full-scale positive reaches exactly 1.0, matching
        // how the WAV readers elsewhere in the crate scale integer PCM.
        self as f32 / i16::MAX as f32
    }
}

/// Sample storage, at whichever precision the source actually carried.
#[derive(Clone, Debug)]
enum Storage {
    /// Full precision. Generated audio, and 24/32-bit or float sources.
    F32 { left: Arc<[f32]>, right: Arc<[f32]> },
    /// Half the memory, and lossless when the source was already 16-bit.
    I16 { left: Arc<[i16]>, right: Arc<[i16]> },
}

impl Storage {
    fn len(&self) -> usize {
        match self {
            Storage::F32 { left, .. } => left.len(),
            Storage::I16 { left, .. } => left.len(),
        }
    }

    /// Bytes of sample data held, for memory reporting.
    fn bytes(&self) -> usize {
        match self {
            Storage::F32 { left, right } => (left.len() + right.len()) * 4,
            Storage::I16 { left, right } => (left.len() + right.len()) * 2,
        }
    }
}

/// Shared stereo sample data. Cloning is cheap (two `Arc` bumps).
#[derive(Clone, Debug)]
pub struct StereoSampleBuffer {
    storage: Storage,
    sample_rate: f32,
    /// The tempo the source material was authored/recorded at, if known. Used
    /// by [`crate::mixer::loop_channel::LoopChannel`]'s tempo-warp modes to
    /// compute a warp ratio against the engine's BPM. `None` disables warping
    /// for this buffer regardless of the channel's pitch mode.
    source_bpm: Option<f32>,
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
            storage: Storage::F32 {
                left: Arc::from(left.into_boxed_slice()),
                right: Arc::from(right.into_boxed_slice()),
            },
            sample_rate,
            source_bpm: None,
        })
    }

    /// Build from de-interleaved 16-bit channels, storing them as-is.
    ///
    /// Use this for material that was 16-bit at the source: it halves memory
    /// and loses nothing. Widening to `f32` first and calling
    /// [`Self::from_channels`] would store the same information in twice the
    /// space.
    pub fn from_channels_i16(
        left: Vec<i16>,
        right: Vec<i16>,
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

        Ok(Self {
            storage: Storage::I16 {
                left: Arc::from(left.into_boxed_slice()),
                right: Arc::from(right.into_boxed_slice()),
            },
            sample_rate,
            source_bpm: None,
        })
    }

    /// Build from interleaved 16-bit PCM, keeping it 16-bit. Mono is duplicated
    /// to both sides; more than two channels keeps 0 and 1.
    pub fn from_interleaved_i16(
        samples: &[i16],
        channels: usize,
        sample_rate: f32,
    ) -> Result<Self, String> {
        if channels == 0 {
            return Err("StereoSampleBuffer requires at least one channel".to_string());
        }
        let frames = samples.len() / channels.max(1);
        if frames == 0 {
            return Err("StereoSampleBuffer requires at least one full frame".to_string());
        }

        let mut left = Vec::with_capacity(frames);
        let mut right = Vec::with_capacity(frames);
        for frame in samples.chunks_exact(channels) {
            left.push(frame[0]);
            right.push(if channels == 1 { frame[0] } else { frame[1] });
        }

        Self::from_channels_i16(left, right, sample_rate)
    }

    /// Bytes of sample data held. Cheap way for a host to budget the memory a
    /// loaded pack occupies.
    pub fn memory_bytes(&self) -> usize {
        self.storage.bytes()
    }

    /// Whether this buffer is stored at 16-bit precision.
    pub fn is_compact(&self) -> bool {
        matches!(self.storage, Storage::I16 { .. })
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
    ///
    /// A 16-bit source is stored 16-bit, halving memory at no cost in fidelity.
    /// Deeper and float sources are widened to `f32`.
    #[cfg(feature = "bounce")]
    pub fn from_wav(path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        Self::from_wav_trimmed(path, None)
    }

    /// Like [`Self::from_wav`], but stops after `max_seconds` of audio.
    ///
    /// The truncation happens while decoding, so a trimmed sample never
    /// occupies its full length in memory even briefly — which is the whole
    /// point when loading a few hundred of them.
    ///
    /// Cutting mid-decay leaves a step at the end of the buffer, so a caller
    /// that trims audible material is expected to fade the tail on playback
    /// (see [`crate::instruments::multisample::SampleZone::fade_out_frames`]);
    /// this function only limits length.
    #[cfg(feature = "bounce")]
    pub fn from_wav_trimmed(
        path: impl AsRef<std::path::Path>,
        max_seconds: Option<f32>,
    ) -> Result<Self, String> {
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
        // Resolve the limit now that the file's own rate is known.
        let max_samples = match max_seconds {
            Some(secs) if secs.is_finite() && secs > 0.0 => {
                ((secs * spec.sample_rate as f32).ceil() as usize).saturating_mul(channels)
            }
            _ => usize::MAX,
        };

        // 16-bit integer is the common case for sample packs, and the one worth
        // keeping narrow. Everything else falls through to the f32 path below.
        if spec.sample_format == hound::SampleFormat::Int
            && (9..=16).contains(&spec.bits_per_sample)
        {
            let interleaved = reader
                .samples::<i16>()
                .take(max_samples)
                .map(|s| s.map_err(|e| format!("Failed to read WAV sample: {e}")))
                .collect::<Result<Vec<_>, _>>()?;
            if interleaved.is_empty() {
                return Err("WAV contains no samples".to_string());
            }
            return Self::from_interleaved_i16(&interleaved, channels, spec.sample_rate as f32);
        }

        let interleaved = match spec.sample_format {
            hound::SampleFormat::Float => reader
                .samples::<f32>()
                .take(max_samples)
                .map(|s| s.map_err(|e| format!("Failed to read WAV sample: {e}")))
                .collect::<Result<Vec<_>, _>>()?,
            hound::SampleFormat::Int => match spec.bits_per_sample {
                0 => return Err("WAV bit depth must be greater than zero".to_string()),
                1..=8 => {
                    let scale = ((1_i32 << (spec.bits_per_sample - 1)) - 1) as f32;
                    reader
                        .samples::<i8>()
                        .take(max_samples)
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
                        .take(max_samples)
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
        self.storage.len()
    }

    pub fn is_empty(&self) -> bool {
        self.storage.len() == 0
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Tag this buffer with the tempo its source material was authored at.
    /// Pass `None` to clear the tag (disables tempo-warp modes for this buffer).
    pub fn set_source_bpm(&mut self, bpm: Option<f32>) {
        self.source_bpm = bpm.filter(|b| b.is_finite() && *b > 0.0);
    }

    /// The tagged source BPM, if any.
    pub fn source_bpm(&self) -> Option<f32> {
        self.source_bpm
    }

    #[inline]
    fn tap_clamped<S: StoredSample>(channel: &[S], index: isize) -> f32 {
        let last = channel.len() as isize - 1;
        channel[index.clamp(0, last) as usize].widen()
    }

    #[inline]
    fn tap_wrapped<S: StoredSample>(channel: &[S], index: isize) -> f32 {
        let len = channel.len() as isize;
        channel[index.rem_euclid(len) as usize].widen()
    }

    /// Cubic read over one channel pair at whichever precision they are stored.
    /// Generic rather than branching per tap, so each precision compiles to its
    /// own straight-line loop.
    #[inline]
    fn cubic<S: StoredSample>(
        left: &[S],
        right: &[S],
        index: isize,
        frac: f32,
        wrap: bool,
    ) -> StereoFrame {
        let read = |channel: &[S]| {
            let tap = |i: isize| {
                if wrap {
                    Self::tap_wrapped(channel, i)
                } else {
                    Self::tap_clamped(channel, i)
                }
            };
            cubic_interpolate(
                tap(index - 1),
                tap(index),
                tap(index + 1),
                tap(index + 2),
                frac,
            )
        };
        StereoFrame {
            l: read(left),
            r: read(right),
        }
    }

    /// The single stored frame, for the degenerate one-frame buffer.
    #[inline]
    fn single_frame(&self) -> StereoFrame {
        match &self.storage {
            Storage::F32 { left, right } => StereoFrame {
                l: left[0],
                r: right[0],
            },
            Storage::I16 { left, right } => StereoFrame {
                l: left[0].widen(),
                r: right[0].widen(),
            },
        }
    }

    /// Read a stereo frame at a fractional frame position using cubic
    /// interpolation. The position is clamped into the valid range; callers
    /// that loop are responsible for wrapping `position` before calling.
    #[inline]
    pub fn read_interpolated(&self, position: f64) -> StereoFrame {
        if self.len() == 1 {
            return self.single_frame();
        }

        let last = (self.len() - 1) as f64;
        let position = position.clamp(0.0, last);
        let index = position.floor() as isize;
        let frac = (position - index as f64) as f32;

        match &self.storage {
            Storage::F32 { left, right } => Self::cubic(left, right, index, frac, false),
            Storage::I16 { left, right } => Self::cubic(left, right, index, frac, false),
        }
    }

    /// Read a stereo frame like [`Self::read_interpolated`], but with the cubic
    /// interpolation taps wrapping around the buffer ends
    /// (`index.rem_euclid(len)`) instead of clamping. Used by
    /// [`crate::mixer::loop_channel::LoopChannel`] only when the active loop
    /// window wraps the buffer end, so the seam between the last and first
    /// frames stays continuous.
    #[inline]
    pub fn read_wrapped(&self, position: f64) -> StereoFrame {
        if self.len() == 1 {
            return self.single_frame();
        }

        let len = self.len() as f64;
        // Fold the read position into [0, len) so the integer index and each of
        // its cubic neighbors resolve to a valid mod-len tap.
        let position = position.rem_euclid(len);
        let index = position.floor() as isize;
        let frac = (position - index as f64) as f32;

        match &self.storage {
            Storage::F32 { left, right } => Self::cubic(left, right, index, frac, true),
            Storage::I16 { left, right } => Self::cubic(left, right, index, frac, true),
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

    #[test]
    fn i16_storage_halves_memory() {
        let frames = 1000;
        let wide = StereoSampleBuffer::from_channels(vec![0.5; frames], vec![0.5; frames], 44100.0)
            .unwrap();
        let compact = StereoSampleBuffer::from_channels_i16(
            vec![16383; frames],
            vec![16383; frames],
            44100.0,
        )
        .unwrap();

        assert_eq!(wide.memory_bytes(), frames * 2 * 4);
        assert_eq!(compact.memory_bytes(), frames * 2 * 2);
        assert!(compact.is_compact() && !wide.is_compact());
        assert_eq!(compact.len(), wide.len());
    }

    #[test]
    fn i16_and_f32_storage_read_the_same_signal() {
        // Same waveform stored both ways must interpolate to the same values,
        // within 16-bit quantization.
        let frames = 512;
        let wave: Vec<f32> = (0..frames).map(|i| (i as f32 / 32.0).sin() * 0.8).collect();
        let quantized: Vec<i16> = wave
            .iter()
            .map(|&s| (s * i16::MAX as f32).round() as i16)
            .collect();

        let wide = StereoSampleBuffer::from_channels(wave.clone(), wave.clone(), 44100.0).unwrap();
        let compact =
            StereoSampleBuffer::from_channels_i16(quantized.clone(), quantized, 44100.0).unwrap();

        for step in 0..400 {
            let pos = step as f64 * 1.27 + 3.0;
            let a = wide.read_interpolated(pos);
            let b = compact.read_interpolated(pos);
            assert!(
                (a.l - b.l).abs() < 1e-3 && (a.r - b.r).abs() < 1e-3,
                "at {pos}: f32 {a:?} vs i16 {b:?}"
            );
            let a = wide.read_wrapped(pos);
            let b = compact.read_wrapped(pos);
            assert!((a.l - b.l).abs() < 1e-3, "wrapped at {pos}");
        }
    }

    #[test]
    fn i16_full_scale_maps_to_unity() {
        let buf =
            StereoSampleBuffer::from_channels_i16(vec![i16::MAX; 4], vec![i16::MIN; 4], 44100.0)
                .unwrap();
        let f = buf.read_interpolated(1.0);
        assert!((f.l - 1.0).abs() < 1e-6, "{f:?}");
        // i16::MIN is one step past -1.0; it must not blow past it meaningfully.
        assert!(f.r < -0.999 && f.r >= -1.001, "{f:?}");
    }

    #[test]
    fn i16_single_frame_buffer_reads_without_panicking() {
        let buf =
            StereoSampleBuffer::from_channels_i16(vec![16383], vec![-16383], 44100.0).unwrap();
        let f = buf.read_interpolated(7.5);
        assert!(
            (f.l - 0.5).abs() < 1e-3 && (f.r + 0.5).abs() < 1e-3,
            "{f:?}"
        );
        assert_eq!(buf.read_wrapped(-3.2).l, f.l);
    }

    #[test]
    fn interleaved_i16_splits_and_duplicates_like_the_f32_path() {
        let stereo =
            StereoSampleBuffer::from_interleaved_i16(&[i16::MAX, i16::MIN, 0, 0], 2, 48000.0)
                .unwrap();
        assert_eq!(stereo.len(), 2);
        assert_eq!(stereo.sample_rate(), 48000.0);
        let f = stereo.read_interpolated(0.0);
        assert!((f.l - 1.0).abs() < 1e-6 && f.r < -0.999);

        let mono = StereoSampleBuffer::from_interleaved_i16(&[16383, 8191, 0], 1, 44100.0).unwrap();
        assert_eq!(mono.len(), 3);
        let f = mono.read_interpolated(0.0);
        assert_eq!(f.l, f.r, "mono should duplicate to both sides");
    }
}
