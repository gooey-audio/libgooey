//! A single stereo loop-player channel: buffer playback with loop start/end,
//! speed, a click-free gain fader, mute/solo intent, and its own effect chain.
//!
//! The playback cursor advances by `speed * (source_sr / engine_sr)` per output
//! sample. The `source_sr / engine_sr` term handles sample-rate conversion via
//! the buffer's cubic interpolation; `speed` is the user varispeed control. This
//! is the hook for the future tempo-warp phase — warping simply multiplies the
//! advance by `engine_bpm / source_bpm` (see the plan's "Tempo warping" phase).

use crate::frame::StereoFrame;
use crate::mixer::effect_chain::EffectChain;
use crate::mixer::stereo_buffer::StereoSampleBuffer;
use crate::utils::SmoothedParam;

/// Smoothing time for the user gain fader and the mute/solo gate (ms).
const FADER_SMOOTH_MS: f32 = 15.0;
/// Maximum varispeed magnitude (forward or reverse).
const MAX_SPEED: f32 = 4.0;
/// Maximum user fader gain (allows a little boost above unity).
const MAX_GAIN: f32 = 2.0;

pub struct LoopChannel {
    buffer: Option<StereoSampleBuffer>,
    /// Playback position in source frames (fractional).
    cursor: f64,
    /// Loop window as normalized positions in `[0, 1]` of the buffer length.
    loop_start: f32,
    loop_end: f32,
    playing: bool,
    speed: f32,
    /// User fader.
    gain: SmoothedParam,
    /// Mute/solo gate (0 = silent, 1 = audible), set by the owning [`Mixer`].
    active_gain: SmoothedParam,
    muted: bool,
    soloed: bool,
    effects: EffectChain,
}

impl LoopChannel {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            buffer: None,
            cursor: 0.0,
            loop_start: 0.0,
            loop_end: 1.0,
            playing: false,
            speed: 1.0,
            gain: SmoothedParam::new(1.0, 0.0, MAX_GAIN, sample_rate, FADER_SMOOTH_MS),
            active_gain: SmoothedParam::new(1.0, 0.0, 1.0, sample_rate, FADER_SMOOTH_MS),
            muted: false,
            soloed: false,
            effects: EffectChain::new(),
        }
    }

    // --- Playback ---------------------------------------------------------

    /// Generate the next stereo sample. The mute/solo gate is applied to the
    /// post-effect (wet) signal so muting fades a channel's full output — including
    /// effect tails — rather than chopping the dry input.
    pub fn tick(&mut self, engine_sample_rate: f32) -> StereoFrame {
        let dry = if self.playing {
            match &self.buffer {
                Some(buffer) if !buffer.is_empty() => {
                    let frame = buffer.read_interpolated(self.cursor);
                    self.advance(engine_sample_rate);
                    frame
                }
                _ => StereoFrame::default(),
            }
        } else {
            StereoFrame::default()
        };

        // Advance the gain smoothers every sample so their timeline is stable
        // regardless of whether the channel is currently playing.
        let gained = dry.scaled(self.gain.tick());
        let wet = self.effects.process(gained);
        wet.scaled(self.active_gain.tick())
    }

    /// Advance the cursor by one output sample, wrapping inside the loop window.
    /// Handles forward and reverse (negative `speed`) playback.
    fn advance(&mut self, engine_sample_rate: f32) {
        let Some(buffer) = &self.buffer else {
            return;
        };
        let len = buffer.len() as f64;
        let (lo, hi) = self.loop_bounds(len);
        let span = (hi - lo).max(1.0);

        let ratio = buffer.sample_rate() as f64 / engine_sample_rate.max(1.0) as f64;
        self.cursor += self.speed as f64 * ratio;

        if self.cursor >= hi {
            self.cursor = lo + (self.cursor - lo).rem_euclid(span);
        } else if self.cursor < lo {
            // rem_euclid keeps the offset in [0, span); mirror it back from hi.
            self.cursor = hi - (lo - self.cursor).rem_euclid(span);
        }
    }

    /// Resolve the normalized loop window to `[lo, hi)` frame positions.
    fn loop_bounds(&self, len: f64) -> (f64, f64) {
        let a = (self.loop_start as f64 * len).clamp(0.0, len);
        let b = (self.loop_end as f64 * len).clamp(0.0, len);
        (a.min(b), a.max(b))
    }

    // --- Setters ----------------------------------------------------------

    /// Load (or replace) this channel's loop and reset the cursor to the loop start.
    pub fn set_buffer(&mut self, buffer: StereoSampleBuffer) {
        let len = buffer.len() as f64;
        self.buffer = Some(buffer);
        let (lo, _) = self.loop_bounds(len);
        self.cursor = lo;
    }

    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    pub fn set_gain(&mut self, gain: f32) {
        self.gain.set_target(gain.clamp(0.0, MAX_GAIN));
    }

    pub fn set_loop_start(&mut self, normalized: f32) {
        self.loop_start = normalized.clamp(0.0, 1.0);
    }

    pub fn set_loop_end(&mut self, normalized: f32) {
        self.loop_end = normalized.clamp(0.0, 1.0);
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.clamp(-MAX_SPEED, MAX_SPEED);
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    pub fn set_soloed(&mut self, soloed: bool) {
        self.soloed = soloed;
    }

    /// Restart playback from the loop start.
    pub fn restart(&mut self) {
        if let Some(buffer) = &self.buffer {
            let (lo, _) = self.loop_bounds(buffer.len() as f64);
            self.cursor = lo;
        }
    }

    /// Set the mute/solo gate target. Called by the [`Mixer`] each block once
    /// the cross-channel solo state is known.
    pub(crate) fn set_active(&mut self, audible: bool) {
        self.active_gain.set_target(if audible { 1.0 } else { 0.0 });
    }

    // --- Getters ----------------------------------------------------------

    pub fn has_buffer(&self) -> bool {
        self.buffer.as_ref().is_some_and(|b| !b.is_empty())
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn gain(&self) -> f32 {
        self.gain.target()
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    pub fn is_soloed(&self) -> bool {
        self.soloed
    }

    pub fn speed(&self) -> f32 {
        self.speed
    }

    pub fn loop_start(&self) -> f32 {
        self.loop_start
    }

    pub fn loop_end(&self) -> f32 {
        self.loop_end
    }

    /// Current playhead as a normalized `[0, 1]` position in the buffer (0 if
    /// no buffer is loaded).
    pub fn position_normalized(&self) -> f32 {
        match &self.buffer {
            Some(buffer) if buffer.len() > 1 => (self.cursor / (buffer.len() as f64)) as f32,
            _ => 0.0,
        }
    }

    /// Mutable access to the per-channel effect chain.
    pub fn effects_mut(&mut self) -> &mut EffectChain {
        &mut self.effects
    }

    /// Shared access to the per-channel effect chain.
    pub fn effects(&self) -> &EffectChain {
        &self.effects
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44_100.0;

    fn ramp_buffer(frames: usize) -> StereoSampleBuffer {
        let left: Vec<f32> = (0..frames).map(|i| i as f32).collect();
        let right = left.clone();
        StereoSampleBuffer::from_channels(left, right, SR).unwrap()
    }

    #[test]
    fn silent_until_playing() {
        let mut ch = LoopChannel::new(SR);
        ch.set_buffer(ramp_buffer(100));
        // Not playing yet.
        assert_eq!(ch.tick(SR), StereoFrame::default());
    }

    #[test]
    fn cursor_wraps_within_loop_window() {
        let mut ch = LoopChannel::new(SR);
        ch.set_buffer(ramp_buffer(10));
        ch.set_loop_start(0.0);
        ch.set_loop_end(0.5); // window = frames [0, 5)
        ch.set_playing(true);
        // Run well past the window; cursor must always stay < 5.
        for _ in 0..50 {
            ch.tick(SR);
            assert!(ch.position_normalized() < 0.5 + 1e-3);
        }
    }

    #[test]
    fn set_buffer_starts_at_loop_start() {
        let mut ch = LoopChannel::new(SR);
        ch.set_loop_start(0.5);
        ch.set_loop_end(1.0);
        ch.set_buffer(ramp_buffer(100));
        // cursor should be at 0.5 * 100 = 50.
        assert!((ch.position_normalized() - 0.5).abs() < 1e-3);
    }

    #[test]
    fn reverse_playback_stays_in_window() {
        let mut ch = LoopChannel::new(SR);
        ch.set_buffer(ramp_buffer(20));
        ch.set_loop_start(0.25);
        ch.set_loop_end(0.75);
        ch.set_speed(-1.0);
        ch.set_playing(true);
        for _ in 0..100 {
            ch.tick(SR);
            let p = ch.position_normalized();
            assert!((0.25..0.75 + 1e-2).contains(&p), "pos {p} out of window");
        }
    }
}
