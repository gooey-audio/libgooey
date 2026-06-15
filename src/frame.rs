//! Stereo audio frame: the engine's output currency.
//!
//! libgooey's instruments and effects are mono (one `f32` per sample). This
//! type is the single place the signal becomes two-channel — the "stereo seam".
//! Today the seam produces dual-mono frames via [`StereoFrame::mono`] (left ==
//! right), but the rest of the output path (CPAL device write, FFI interleaved
//! buffer, tests) already treats the signal as stereo. Future work that adds
//! per-instrument panning or stereo effects only has to change how frames are
//! produced, not how they are consumed.

/// A single stereo sample: a left and a right channel value.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StereoFrame {
    pub l: f32,
    pub r: f32,
}

impl StereoFrame {
    /// Build a frame from a single mono sample, placing it equally on both
    /// channels. This is the "stereo seam": the one conversion point from the
    /// mono signal path to the stereo output path.
    pub const fn mono(x: f32) -> Self {
        Self { l: x, r: x }
    }

    /// Collapse the frame to a single mono sample (average of both channels).
    /// Used when the output device has a single channel and when feeding the
    /// visualization buffer, which expects one value per sample.
    pub fn downmix(self) -> f32 {
        0.5 * (self.l + self.r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_places_sample_on_both_channels() {
        let f = StereoFrame::mono(0.42);
        assert_eq!(f.l, 0.42);
        assert_eq!(f.r, 0.42);
    }

    #[test]
    fn downmix_of_a_mono_frame_is_the_original_sample() {
        let x = -0.3;
        assert_eq!(StereoFrame::mono(x).downmix(), x);
    }

    #[test]
    fn downmix_averages_the_two_channels() {
        let f = StereoFrame { l: 1.0, r: 0.0 };
        assert_eq!(f.downmix(), 0.5);
    }

    #[test]
    fn default_is_silence() {
        assert_eq!(StereoFrame::default(), StereoFrame { l: 0.0, r: 0.0 });
    }
}
