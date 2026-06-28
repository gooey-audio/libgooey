//! Stereo audio frame: the engine's output currency.
//!
//! libgooey's instruments and effects are mono (one `f32` per sample). This
//! type is the single place the signal becomes two-channel — the "stereo seam".
//! The seam either places a mono sample equally on both channels via
//! [`StereoFrame::mono`] (left == right) or spreads each instrument across the
//! field with an equal-power pan via [`StereoFrame::panned`]; the rest of the
//! output path (CPAL device write, FFI interleaved buffer, tests) already
//! treats the signal as stereo, so it only ever has to change how frames are
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

    /// Pan a mono sample to a stereo frame using an equal-power law.
    /// `pan` is clamped to `[0, 1]`: 0.0 = hard left, 0.5 = center, 1.0 = hard
    /// right. Center is -3 dB per channel (`0.707`), keeping constant power
    /// across the sweep.
    pub fn panned(x: f32, pan: f32) -> Self {
        let angle = pan.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2;
        Self {
            l: x * angle.cos(),
            r: x * angle.sin(),
        }
    }

    /// Collapse the frame to a single mono sample (average of both channels).
    /// Used when the output device has a single channel and when feeding the
    /// visualization buffer, which expects one value per sample.
    pub fn downmix(self) -> f32 {
        0.5 * (self.l + self.r)
    }

    /// Scale both channels by a single gain factor.
    #[inline]
    pub fn scaled(self, gain: f32) -> Self {
        Self {
            l: self.l * gain,
            r: self.r * gain,
        }
    }
}

impl std::ops::Add for StereoFrame {
    type Output = StereoFrame;

    #[inline]
    fn add(self, rhs: StereoFrame) -> StereoFrame {
        StereoFrame {
            l: self.l + rhs.l,
            r: self.r + rhs.r,
        }
    }
}

impl std::ops::AddAssign for StereoFrame {
    #[inline]
    fn add_assign(&mut self, rhs: StereoFrame) {
        self.l += rhs.l;
        self.r += rhs.r;
    }
}

impl std::ops::Mul<f32> for StereoFrame {
    type Output = StereoFrame;

    #[inline]
    fn mul(self, gain: f32) -> StereoFrame {
        self.scaled(gain)
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

    #[test]
    fn panned_hard_left_silences_right() {
        let f = StereoFrame::panned(0.8, 0.0);
        assert!((f.l - 0.8).abs() < 1e-6);
        assert!(f.r.abs() < 1e-6);
    }

    #[test]
    fn panned_hard_right_silences_left() {
        let f = StereoFrame::panned(0.8, 1.0);
        assert!(f.l.abs() < 1e-6);
        assert!((f.r - 0.8).abs() < 1e-6);
    }

    #[test]
    fn panned_center_is_equal_and_minus_three_db() {
        let f = StereoFrame::panned(1.0, 0.5);
        assert!((f.l - f.r).abs() < 1e-6);
        assert!((f.l - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
    }

    #[test]
    fn panned_preserves_power_across_sweep() {
        let x = 0.6;
        for pan in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let f = StereoFrame::panned(x, pan);
            assert!((f.l * f.l + f.r * f.r - x * x).abs() < 1e-5, "pan {pan}");
        }
    }

    #[test]
    fn panned_clamps_out_of_range() {
        assert_eq!(
            StereoFrame::panned(0.5, -1.0),
            StereoFrame::panned(0.5, 0.0)
        );
        assert_eq!(StereoFrame::panned(0.5, 2.0), StereoFrame::panned(0.5, 1.0));
    }
}
