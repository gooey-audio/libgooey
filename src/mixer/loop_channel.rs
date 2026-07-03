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
use crate::mixer::wsola::WsolaStretcher;
use crate::utils::SmoothedParam;

/// Smoothing time for the user gain fader and the mute/solo gate (ms).
const FADER_SMOOTH_MS: f32 = 15.0;
/// Maximum varispeed magnitude (forward or reverse).
const MAX_SPEED: f32 = 4.0;
/// Maximum user fader gain (allows a little boost above unity).
const MAX_GAIN: f32 = 2.0;
/// Engine BPM assumed until [`Mixer::set_bpm`](crate::mixer::Mixer::set_bpm) is
/// first called; matches `Mixer`'s own default so an untouched channel's warp
/// ratio is sensible if a host tags `source_bpm` before ever setting tempo.
const DEFAULT_ENGINE_BPM: f32 = 120.0;

/// How a loop channel reacts to engine BPM changes relative to its buffer's
/// tagged `source_bpm`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PitchMode {
    /// No BPM-driven warp; playback rate is controlled by `speed` alone
    /// (today's behavior).
    #[default]
    Off,
    /// Naive resample warp: cursor advance is scaled by `engine_bpm /
    /// source_bpm`, so tempo changes shift pitch (like varispeed).
    Resample,
    /// WSOLA time-stretch: tempo tracks `engine_bpm / source_bpm` without
    /// shifting pitch. Falls back to `Resample` behavior when `speed < 0`
    /// (reverse playback is not supported in this mode).
    PreservePitch,
}

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
    pitch_mode: PitchMode,
    /// Cached from the last [`Mixer::set_bpm`](crate::mixer::Mixer::set_bpm) call.
    engine_bpm: f32,
    /// Lazily constructed when `pitch_mode` becomes `PreservePitch`; dropped
    /// (freeing its buffers) when leaving that mode, or whenever the cursor
    /// is externally moved (`set_buffer`/`restart`/`set_position`) so it
    /// re-seeds at the new position instead of playing stale state.
    stretcher: Option<WsolaStretcher>,
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
            pitch_mode: PitchMode::default(),
            engine_bpm: DEFAULT_ENGINE_BPM,
            stretcher: None,
        }
    }

    // --- Playback ---------------------------------------------------------

    /// Generate the next stereo sample. The mute/solo gate is applied to the
    /// post-effect (wet) signal so muting fades a channel's full output — including
    /// effect tails — rather than chopping the dry input.
    pub fn tick(&mut self, engine_sample_rate: f32) -> StereoFrame {
        let dry = if self.playing && self.has_buffer() {
            if self.pitch_mode == PitchMode::PreservePitch && self.speed >= 0.0 {
                self.tick_preserve_pitch(engine_sample_rate)
            } else {
                let frame = self.buffer.as_ref().unwrap().read_interpolated(self.cursor);
                self.advance(engine_sample_rate);
                frame
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

    /// WSOLA time-stretch playback path (see [`crate::mixer::wsola`]). Drains
    /// one frame from the stretcher's output, resynthesizing a hop first if
    /// its scratch buffer is exhausted. Only reached when `pitch_mode ==
    /// PreservePitch` and `speed >= 0`; `tick()` falls back to the direct
    /// resample path otherwise.
    fn tick_preserve_pitch(&mut self, engine_sample_rate: f32) -> StereoFrame {
        let buffer = self.buffer.as_ref().unwrap().clone(); // cheap: two Arc bumps
        let len = buffer.len() as f64;
        let (lo, hi) = self.loop_bounds(len);
        let sr_ratio = buffer.sample_rate() as f64 / engine_sample_rate.max(1.0) as f64;
        let warp = self.warp_ratio();
        let speed = self.speed as f64;

        if self.stretcher.is_none() {
            self.stretcher = Some(WsolaStretcher::new(engine_sample_rate, self.cursor));
        }
        let stretcher = self.stretcher.as_mut().unwrap();
        if stretcher.needs_refill() {
            self.cursor = stretcher.synthesize_next_hop(&buffer, lo, hi, sr_ratio, speed, warp);
        }
        self.stretcher.as_mut().unwrap().drain()
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
        let warp = if self.pitch_mode == PitchMode::Resample {
            self.warp_ratio()
        } else {
            1.0
        };
        self.cursor += self.speed as f64 * ratio * warp;

        if self.cursor >= hi {
            self.cursor = lo + (self.cursor - lo).rem_euclid(span);
        } else if self.cursor < lo {
            // rem_euclid keeps the offset in [0, span); mirror it back from hi.
            self.cursor = hi - (lo - self.cursor).rem_euclid(span);
        }
    }

    /// The tempo-warp multiplier implied by `engine_bpm / source_bpm`, or
    /// `1.0` when warping doesn't apply (mode off, or the buffer has no
    /// tagged `source_bpm`). Shared by both `Resample` and `PreservePitch`
    /// modes — they differ only in how the ratio is applied to playback.
    fn warp_ratio(&self) -> f64 {
        if self.pitch_mode == PitchMode::Off {
            return 1.0;
        }
        match self
            .buffer
            .as_ref()
            .and_then(StereoSampleBuffer::source_bpm)
        {
            Some(source_bpm) if source_bpm > 0.0 && self.engine_bpm > 0.0 => {
                self.engine_bpm as f64 / source_bpm as f64
            }
            _ => 1.0,
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
        self.stretcher = None;
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

    pub fn set_pitch_mode(&mut self, mode: PitchMode) {
        if self.pitch_mode == PitchMode::PreservePitch && mode != PitchMode::PreservePitch {
            self.stretcher = None;
        }
        self.pitch_mode = mode;
    }

    /// Tag the currently loaded buffer with its source BPM (`None` clears the
    /// tag). No-op if no buffer is loaded. See [`StereoSampleBuffer::set_source_bpm`].
    pub fn set_source_bpm(&mut self, bpm: Option<f32>) {
        if let Some(buffer) = &mut self.buffer {
            buffer.set_source_bpm(bpm);
        }
    }

    /// The currently loaded buffer's tagged source BPM, if any.
    pub fn source_bpm(&self) -> Option<f32> {
        self.buffer
            .as_ref()
            .and_then(StereoSampleBuffer::source_bpm)
    }

    /// Cache the engine's current BPM. Called by [`Mixer::set_bpm`](crate::mixer::Mixer::set_bpm).
    pub(crate) fn set_engine_bpm(&mut self, bpm: f32) {
        self.engine_bpm = bpm;
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
            self.stretcher = None;
        }
    }

    /// Set the playhead to a normalized [0, 1] position of the full buffer,
    /// clamped into the active loop window. Inverse of `position_normalized()`.
    /// No-op if no buffer is loaded.
    pub fn set_position(&mut self, normalized: f32) {
        if let Some(buffer) = &self.buffer {
            let len = buffer.len() as f64;
            let (lo, hi) = self.loop_bounds(len);
            let target = normalized.clamp(0.0, 1.0) as f64 * len;
            self.cursor = target.clamp(lo, hi);
            self.stretcher = None;
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

    pub fn pitch_mode(&self) -> PitchMode {
        self.pitch_mode
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
    fn set_position_round_trips() {
        let mut ch = LoopChannel::new(SR);
        ch.set_buffer(ramp_buffer(100));
        ch.set_position(0.42);
        assert!((ch.position_normalized() - 0.42).abs() < 1e-2);
    }

    #[test]
    fn set_position_clamps_into_loop_window() {
        let mut ch = LoopChannel::new(SR);
        ch.set_buffer(ramp_buffer(100));
        ch.set_loop_start(0.25);
        ch.set_loop_end(0.75);
        ch.set_position(0.9); // outside window -> clamped to hi
        let p = ch.position_normalized();
        assert!((0.25..=0.75 + 1e-3).contains(&p), "pos {p} out of window");
        ch.set_position(0.0); // below window -> clamped to lo
        assert!(ch.position_normalized() >= 0.25 - 1e-3);
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
