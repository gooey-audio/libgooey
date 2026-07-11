//! A single stereo loop-player channel: buffer playback with loop start/end,
//! speed, a click-free gain fader, mute/solo intent, and its own effect chain.
//!
//! The playback cursor advances by `speed * (source_sr / engine_sr)` per output
//! sample. The `source_sr / engine_sr` term handles sample-rate conversion via
//! the buffer's cubic interpolation; `speed` is the user varispeed control. This
//! is the hook for the future tempo-warp phase — warping simply multiplies the
//! advance by `engine_bpm / source_bpm` (see the plan's "Tempo warping" phase).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

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

/// A resolved loop window in physical frame coordinates.
///
/// Normally `[lo, hi)`. When the start marker is pushed past the end marker
/// (`loop_end < loop_start`), the window wraps the buffer end and plays the
/// union `[lo, len) ∪ [0, hi)` — rotating the phrase downbeat. `span` is the
/// total playable length (`hi - lo`, or `len - lo + hi` when wrapped).
///
/// Playback math on the wrap branch runs in *virtual* coordinates `[0, span)`
/// via [`LoopWindow::to_virtual`]/[`LoopWindow::to_physical`]; the non-wrap
/// branch stays in physical frames so its float expressions are byte-identical
/// to the pre-wrap engine.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LoopWindow {
    /// Loop start in physical frames.
    pub(crate) lo: f64,
    /// Loop end in physical frames.
    pub(crate) hi: f64,
    /// Playable span in frames: `hi - lo`, or `len - lo + hi` when wrapped.
    pub(crate) span: f64,
    /// Whether the window wraps the buffer end (`hi < lo`).
    pub(crate) wraps: bool,
    /// Buffer length in frames.
    pub(crate) len: f64,
}

impl LoopWindow {
    /// Map a physical frame position to its virtual offset in `[0, span)`.
    #[inline]
    pub(crate) fn to_virtual(self, p: f64) -> f64 {
        (p - self.lo).rem_euclid(self.len)
    }

    /// Map a virtual offset back to a physical frame position in `[0, len)`.
    #[inline]
    pub(crate) fn to_physical(self, v: f64) -> f64 {
        (self.lo + v).rem_euclid(self.len)
    }

    /// Whether a physical position lies inside the playable region.
    #[inline]
    fn contains(&self, p: f64) -> bool {
        if self.wraps {
            p >= self.lo || p < self.hi
        } else {
            p >= self.lo && p < self.hi
        }
    }

    /// Fold a physical position into the window: returned unchanged when it is
    /// already inside, otherwise snapped to the nearer edge of the playable
    /// region (the loop start `lo` or end `hi`). On the wrap branch the gap is
    /// the contiguous physical range `[hi, lo)`, so plain distance picks the
    /// nearer edge.
    #[inline]
    fn fold(&self, p: f64) -> f64 {
        if self.contains(p) {
            return p;
        }
        if self.wraps {
            if (p - self.hi) <= (self.lo - p) {
                self.hi
            } else {
                self.lo
            }
        } else {
            p.clamp(self.lo, self.hi)
        }
    }
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
    /// A buffer staged (from the main thread) to atomically replace `buffer` at
    /// the next bar-grid boundary. Taken by the audio thread in `advance()`.
    /// `has_pending` gates the audio thread so the common (nothing-queued) path
    /// never touches the mutex; `pending_divisions` is the swap grid (loop region
    /// split into this many equal segments — bar count for bar-quantized swaps).
    pending: Mutex<Option<StereoSampleBuffer>>,
    pending_divisions: AtomicU32,
    has_pending: AtomicBool,
    swaps_completed: AtomicU32,
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
            pending: Mutex::new(None),
            pending_divisions: AtomicU32::new(1),
            has_pending: AtomicBool::new(false),
            swaps_completed: AtomicU32::new(0),
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
                let buffer = self.buffer.as_ref().unwrap();
                let window = self.window(buffer.len() as f64);
                // Read through the wrapping cubic taps only when the window
                // wraps the buffer end, so the seam stays continuous; the
                // common non-wrap read is unchanged.
                let frame = if window.wraps {
                    buffer.read_wrapped(self.cursor)
                } else {
                    buffer.read_interpolated(self.cursor)
                };
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
        let window = self.window(len);
        let sr_ratio = buffer.sample_rate() as f64 / engine_sample_rate.max(1.0) as f64;
        let warp = self.warp_ratio();
        let speed = self.speed as f64;

        if self.stretcher.is_none() {
            self.stretcher = Some(WsolaStretcher::new(engine_sample_rate, self.cursor));
        }
        let prev = self.cursor;
        let mut wrapped = false;
        let stretcher = self.stretcher.as_mut().unwrap();
        if stretcher.needs_refill() {
            self.cursor = stretcher.synthesize_next_hop(&buffer, &window, sr_ratio, speed, warp);
            // Forward-only path (speed >= 0), so a cursor that moved backward can
            // only mean the hop wrapped the loop window. Under a wrap window the
            // physical cursor isn't monotonic across the buffer seam, so compare
            // virtual offsets there.
            wrapped = if window.wraps {
                window.to_virtual(self.cursor) < window.to_virtual(prev)
            } else {
                self.cursor < prev
            };
        }
        let out = self.stretcher.as_mut().unwrap().drain();
        // Land any queued audition swap at its bar-grid boundary. `advance` handles
        // this for the resample/off paths; the WSOLA path bypasses `advance`, so we
        // check here too — otherwise a queued swap could never land while a channel
        // is in PreservePitch mode. `maybe_swap_pending` resets the stretcher on a
        // swap so the next tick re-seeds on the new buffer at its loop start.
        //
        // Note: `self.cursor` only advances on hop refills (~`wsola::HOP_MS`), so in
        // this mode the boundary check is at hop granularity — a queued swap can land
        // up to one hop after the exact grid sample, whereas the resample/off paths
        // are sample-accurate. This is accepted: the swap restarts the phrase from the
        // loop start (already a deliberate discontinuity), and sub-hop precision would
        // require splitting an overlap-add hop for no meaningful audible gain.
        let span = window.span.max(1.0);
        // Feed `maybe_swap_pending` virtual offsets. The non-wrap branch passes
        // `prev - lo` / `cursor - lo`, byte-identical to the previous internal
        // math so the swap grid lands on exactly the same samples.
        let (prev_v, cur_v) = if window.wraps {
            (window.to_virtual(prev), window.to_virtual(self.cursor))
        } else {
            (prev - window.lo, self.cursor - window.lo)
        };
        self.maybe_swap_pending(prev_v, cur_v, span, wrapped);
        out
    }

    /// Advance the cursor by one output sample, wrapping inside the loop window.
    /// Handles forward and reverse (negative `speed`) playback.
    fn advance(&mut self, engine_sample_rate: f32) {
        let (len, source_sr) = match &self.buffer {
            Some(buffer) => (buffer.len() as f64, buffer.sample_rate() as f64),
            None => return,
        };
        let window = self.window(len);
        let span = window.span.max(1.0);

        let ratio = source_sr / engine_sample_rate.max(1.0) as f64;
        let warp = if self.pitch_mode == PitchMode::Resample {
            self.warp_ratio()
        } else {
            1.0
        };
        let prev = self.cursor;
        let delta = self.speed as f64 * ratio * warp;

        let (prev_v, cur_v, wrapped) = if window.wraps {
            // Wrap branch: advance in virtual coordinates [0, span). rem_euclid
            // handles reverse (negative delta) for free.
            let prev_v = window.to_virtual(prev);
            let raw = prev_v + delta;
            let wrapped = !(0.0..span).contains(&raw);
            let cur_v = raw.rem_euclid(span);
            self.cursor = window.to_physical(cur_v);
            (prev_v, cur_v, wrapped)
        } else {
            // Non-wrap branch: physical-frame math, byte-identical to the
            // pre-wrap engine.
            let (lo, hi) = (window.lo, window.hi);
            self.cursor += delta;
            let mut wrapped = false;
            if self.cursor >= hi {
                self.cursor = lo + (self.cursor - lo).rem_euclid(span);
                wrapped = true;
            } else if self.cursor < lo {
                // rem_euclid keeps the offset in [0, span); mirror it back from hi.
                self.cursor = hi - (lo - self.cursor).rem_euclid(span);
                wrapped = true;
            }
            (prev - lo, self.cursor - lo, wrapped)
        };

        self.maybe_swap_pending(prev_v, cur_v, span, wrapped);
    }

    /// If a queued buffer is waiting and this sample crossed a bar-grid boundary
    /// (or wrapped the loop), swap it in and restart the phrase from the loop
    /// start — the sample-accurate, click-free downbeat swap. Gated by an atomic
    /// flag so the nothing-queued path never locks.
    fn maybe_swap_pending(&mut self, prev_v: f64, cur_v: f64, span: f64, wrapped: bool) {
        if !self.has_pending.load(Ordering::Acquire) {
            return;
        }
        let grid = self.pending_divisions.load(Ordering::Relaxed).max(1) as f64;
        let prev_idx = ((prev_v / span) * grid).floor();
        let new_idx = ((cur_v / span) * grid).floor();
        if !(wrapped || new_idx != prev_idx) {
            return;
        }
        // try_lock, not lock: only contends with a main-thread queue/cancel for the
        // few ns it holds the mutex, which never realistically coincides with a
        // boundary; a missed lock just defers the swap to the next boundary.
        if let Ok(mut pending) = self.pending.try_lock() {
            if let Some(buffer) = pending.take() {
                let new_lo = self.window(buffer.len() as f64).lo;
                self.buffer = Some(buffer);
                self.cursor = new_lo;
                // The buffer/cursor moved externally; drop any WSOLA stretcher so
                // the PreservePitch path re-seeds on the new buffer (mirrors
                // `set_buffer`/`restart`/`set_position`). No-op for other modes.
                self.stretcher = None;
                self.swaps_completed.fetch_add(1, Ordering::Relaxed);
                self.has_pending.store(false, Ordering::Release);
            }
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

    /// Resolve the normalized loop markers to a physical [`LoopWindow`]. When
    /// `loop_end < loop_start` the window wraps the buffer end into the union
    /// region `[lo, len) ∪ [0, hi)`.
    fn window(&self, len: f64) -> LoopWindow {
        let lo = (self.loop_start as f64 * len).clamp(0.0, len);
        let hi = (self.loop_end as f64 * len).clamp(0.0, len);
        let wraps = hi < lo;
        let span = if wraps { len - lo + hi } else { hi - lo };
        LoopWindow {
            lo,
            hi,
            span,
            wraps,
            len,
        }
    }

    // --- Setters ----------------------------------------------------------

    /// Load (or replace) this channel's loop and reset the cursor to the loop start.
    pub fn set_buffer(&mut self, buffer: StereoSampleBuffer) {
        let len = buffer.len() as f64;
        self.buffer = Some(buffer);
        self.cursor = self.window(len).lo;
        self.stretcher = None;
    }

    /// Drop the active sample buffer and reset playback state while preserving
    /// the channel strip (gain, mute/solo, and effects).
    pub fn clear_buffer(&mut self) {
        self.buffer = None;
        self.cursor = 0.0;
        self.playing = false;
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
        let Some(buffer) = &self.buffer else {
            return;
        };
        let len = buffer.len() as f64;
        self.cursor = self.window(len).lo;
        self.stretcher = None;
    }

    /// Set the playhead to a normalized [0, 1] position of the full buffer,
    /// folded into the active loop window. Inverse of `position_normalized()`.
    /// On a wrapped window a position that lands in the gap snaps to the nearer
    /// edge. No-op if no buffer is loaded.
    pub fn set_position(&mut self, normalized: f32) {
        let Some(buffer) = &self.buffer else {
            return;
        };
        let len = buffer.len() as f64;
        let window = self.window(len);
        let target = normalized.clamp(0.0, 1.0) as f64 * len;
        self.cursor = window.fold(target);
        self.stretcher = None;
    }

    /// Live-resize the loop window without a click or phrase restart. Sets the
    /// normalized markers, then folds the cursor into the new window: if the
    /// cursor is still inside the playable region it is kept and the WSOLA
    /// stretcher is preserved (continuous scrubbing); if it fell into the gap
    /// it snaps to the nearer edge and the stretcher resets (its ~20 ms
    /// fade-in keeps the jump click-free). Works with transport stopped too.
    pub fn set_loop_window(&mut self, start: f32, end: f32) {
        self.loop_start = start.clamp(0.0, 1.0);
        self.loop_end = end.clamp(0.0, 1.0);
        let Some(buffer) = &self.buffer else {
            return;
        };
        let len = buffer.len() as f64;
        let window = self.window(len);
        let folded = window.fold(self.cursor);
        if folded != self.cursor {
            self.cursor = folded;
            self.stretcher = None;
        }
    }

    /// Set the playhead to a musical phrase phase in `[0, 1)` of the loop
    /// *window* (phase 0 is the loop start), mapping through virtual/window
    /// coordinates so it lands correctly inside a wrapped region. Unlike
    /// [`Self::set_position`] (full-buffer coords), this is how the transport
    /// realigns an active clip's phase. No-op if no buffer is loaded.
    pub fn set_window_phase(&mut self, phase: f32) {
        let Some(buffer) = &self.buffer else {
            return;
        };
        let len = buffer.len() as f64;
        let window = self.window(len);
        let v = (phase as f64).rem_euclid(1.0) * window.span;
        self.cursor = window.to_physical(v);
        self.stretcher = None;
    }

    /// Stage a buffer to atomically replace `buffer` at the next bar-grid
    /// boundary. `divisions` splits the loop region into equal segments (bar
    /// count for bar-quantized swaps; 1 for whole-phrase). Replaces any buffer
    /// already queued. See [`Self::maybe_swap_pending`] for when it lands.
    pub fn queue_swap(&mut self, buffer: StereoSampleBuffer, divisions: u32) {
        self.pending_divisions
            .store(divisions.max(1), Ordering::Relaxed);
        *self.pending.lock().unwrap() = Some(buffer);
        self.has_pending.store(true, Ordering::Release);
    }

    /// Drop a pending queued swap. No-op if nothing is queued.
    pub fn cancel_queued_swap(&mut self) {
        self.has_pending.store(false, Ordering::Release);
        *self.pending.lock().unwrap() = None;
    }

    /// Number of queued swaps that have completed since creation.
    pub fn swaps_completed(&self) -> u32 {
        self.swaps_completed.load(Ordering::Relaxed)
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

    fn const_buffer(frames: usize, value: f32) -> StereoSampleBuffer {
        let left = vec![value; frames];
        let right = left.clone();
        StereoSampleBuffer::from_channels(left, right, SR).unwrap()
    }

    #[test]
    fn queued_swap_lands_at_first_division_boundary() {
        let mut ch = LoopChannel::new(SR);
        ch.set_buffer(ramp_buffer(100)); // A: values 0..100
        ch.set_playing(true);
        // Bar grid at frames 25/50/75; queue B (constant 1000).
        ch.queue_swap(const_buffer(100, 1000.0), 4);

        let mut swapped_at = None;
        for i in 0..48 {
            let f = ch.tick(SR);
            if f.l > 500.0 {
                swapped_at = Some(i);
                break;
            }
        }
        let idx = swapped_at.expect("swap should have landed");
        assert_eq!(ch.swaps_completed(), 1);
        // First boundary is frame 25 — the swap happens in advance() after the
        // read, so B is first heard on the tick just past 25.
        assert!(
            (24..=27).contains(&idx),
            "swapped at frame {idx}, expected ~25"
        );
    }

    #[test]
    fn queued_swap_divisions_one_only_swaps_at_wrap() {
        let mut ch = LoopChannel::new(SR);
        ch.set_buffer(ramp_buffer(100));
        ch.set_playing(true);
        ch.queue_swap(const_buffer(100, 1000.0), 1); // whole-phrase

        // Through most of the loop: no swap yet.
        for _ in 0..90 {
            let f = ch.tick(SR);
            assert!(f.l < 500.0, "swapped before the wrap");
        }
        assert_eq!(ch.swaps_completed(), 0);
        // Cross the wrap -> swap.
        for _ in 0..20 {
            ch.tick(SR);
        }
        assert_eq!(ch.swaps_completed(), 1);
    }

    #[test]
    fn cancel_drops_queued_swap() {
        let mut ch = LoopChannel::new(SR);
        ch.set_buffer(ramp_buffer(100));
        ch.set_playing(true);
        ch.queue_swap(const_buffer(100, 1000.0), 4);
        ch.cancel_queued_swap();
        for _ in 0..150 {
            let f = ch.tick(SR);
            assert!(f.l < 500.0, "swap fired after cancel");
        }
        assert_eq!(ch.swaps_completed(), 0);
    }

    #[test]
    fn requeue_replaces_pending_buffer() {
        let mut ch = LoopChannel::new(SR);
        ch.set_buffer(ramp_buffer(100));
        ch.set_playing(true);
        ch.queue_swap(const_buffer(100, -1000.0), 4); // B
        ch.queue_swap(const_buffer(100, 2000.0), 4); // C replaces B
        for _ in 0..40 {
            ch.tick(SR);
        }
        assert_eq!(ch.swaps_completed(), 1);
        // Now audible from C (~2000), never B (~-1000).
        let f = ch.tick(SR);
        assert!(f.l > 1500.0, "expected C after swap, got {}", f.l);
    }

    fn sine_buffer(frames: usize) -> StereoSampleBuffer {
        let left: Vec<f32> = (0..frames)
            .map(|i| (i as f32 / frames as f32 * std::f32::consts::TAU).sin())
            .collect();
        let right = left.clone();
        StereoSampleBuffer::from_channels(left, right, SR).unwrap()
    }

    #[test]
    fn wrapped_window_plays_union() {
        // start 0.75 / end 0.25 on 8 frames -> plays [6..8) ∪ [0..2). Markers
        // are exact in f32 so the wrapped positions land on integer frames.
        let mut ch = LoopChannel::new(SR);
        ch.set_loop_start(0.75);
        ch.set_loop_end(0.25);
        ch.set_buffer(ramp_buffer(8));
        ch.set_playing(true);
        let expected = [6.0, 7.0, 0.0, 1.0];
        for i in 0..16 {
            let f = ch.tick(SR);
            assert_eq!(f.l, expected[i % expected.len()], "tick {i}");
        }
    }

    #[test]
    fn non_wrapped_interior_window_exact_sequence_regression() {
        // Byte-identical guard: the non-wrap playback path must be unchanged.
        // 0.25/0.5 on 8 frames are exact in f32 -> window [2, 4), reads [2, 3].
        let mut ch = LoopChannel::new(SR);
        ch.set_loop_start(0.25);
        ch.set_loop_end(0.5);
        ch.set_buffer(ramp_buffer(8));
        ch.set_playing(true);
        let expected = [2.0, 3.0];
        for i in 0..16 {
            let f = ch.tick(SR);
            assert_eq!(f.l, expected[i % expected.len()], "tick {i}");
        }
    }

    #[test]
    fn wrapped_window_reverse_stays_in_union() {
        let mut ch = LoopChannel::new(SR);
        ch.set_loop_start(0.7);
        ch.set_loop_end(0.3);
        ch.set_buffer(ramp_buffer(10));
        ch.set_speed(-1.0);
        ch.set_playing(true);
        for _ in 0..200 {
            ch.tick(SR);
            let p = ch.position_normalized();
            assert!(
                !(0.3 + 1e-3..=0.7 - 1e-3).contains(&p),
                "pos {p} left the wrapped union"
            );
        }
    }

    #[test]
    fn set_position_folds_into_wrapped_window() {
        let mut ch = LoopChannel::new(SR);
        ch.set_loop_start(0.7);
        ch.set_loop_end(0.3);
        ch.set_buffer(ramp_buffer(10));
        // Inside the union -> kept.
        ch.set_position(0.1);
        assert!((ch.position_normalized() - 0.1).abs() < 1e-6);
        ch.set_position(0.9);
        assert!((ch.position_normalized() - 0.9).abs() < 1e-6);
        // In the gap [0.3, 0.7): snaps to the nearer edge.
        ch.set_position(0.45); // nearer hi = 0.3
        assert!((ch.position_normalized() - 0.3).abs() < 1e-6);
        ch.set_position(0.55); // nearer lo = 0.7
        assert!((ch.position_normalized() - 0.7).abs() < 1e-6);
    }

    #[test]
    fn set_window_phase_maps_across_seam() {
        // Exact f32 markers: window [6, 8) ∪ [0, 2) on 8 frames, span 4.
        let mut ch = LoopChannel::new(SR);
        ch.set_loop_start(0.75);
        ch.set_loop_end(0.25);
        ch.set_buffer(ramp_buffer(8));
        ch.set_window_phase(0.0); // loop start -> frame 6 (0.75)
        assert!((ch.position_normalized() - 0.75).abs() < 1e-6);
        // Half the 4-frame span crosses the buffer seam to frame 0.
        ch.set_window_phase(0.5);
        assert!((ch.position_normalized() - 0.0).abs() < 1e-6);
    }

    #[test]
    fn degenerate_wrapped_window_is_clamped() {
        // A wrap window whose raw span < 1 frame must stay finite and not hang.
        let mut ch = LoopChannel::new(SR);
        ch.set_loop_start(0.9);
        ch.set_loop_end(0.1);
        ch.set_buffer(ramp_buffer(4)); // lo=3.6, hi=0.4, span=0.8 -> clamped to 1.0
        ch.set_playing(true);
        for _ in 0..200 {
            let f = ch.tick(SR);
            assert!(f.l.is_finite() && f.r.is_finite());
            assert!(ch.position_normalized().is_finite());
        }
    }

    #[test]
    fn queued_swap_lands_in_wrapped_window() {
        let mut ch = LoopChannel::new(SR);
        ch.set_loop_start(0.7);
        ch.set_loop_end(0.3);
        ch.set_buffer(ramp_buffer(10)); // span = 6 virtual frames
        ch.set_playing(true);
        ch.queue_swap(const_buffer(10, 1000.0), 1); // whole-phrase swap at the wrap
        for _ in 0..5 {
            ch.tick(SR);
        }
        assert_eq!(ch.swaps_completed(), 0);
        // Cross the wrap -> swap lands.
        for _ in 0..4 {
            ch.tick(SR);
        }
        assert_eq!(ch.swaps_completed(), 1);
        // New buffer plays from its loop start (const 1000).
        let f = ch.tick(SR);
        assert!(f.l > 500.0, "expected swapped buffer, got {}", f.l);
    }

    #[test]
    fn preserve_pitch_wrapped_window_is_finite_and_bounded() {
        let mut ch = LoopChannel::new(SR);
        ch.set_loop_start(0.7);
        ch.set_loop_end(0.3);
        ch.set_buffer(sine_buffer(4000));
        ch.set_pitch_mode(PitchMode::PreservePitch);
        ch.set_playing(true);
        for _ in 0..8000 {
            let f = ch.tick(SR);
            assert!(f.l.is_finite() && f.r.is_finite());
            assert!(f.l.abs() < 4.0, "output blew up: {}", f.l);
            let p = ch.position_normalized();
            assert!(
                !(0.3 + 0.05..=0.7 - 0.05).contains(&p),
                "pos {p} left the wrapped union"
            );
        }
    }
}
