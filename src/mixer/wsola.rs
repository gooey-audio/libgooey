//! WSOLA (Waveform Similarity Overlap-Add) time-stretcher for
//! [`PitchMode::PreservePitch`](crate::mixer::loop_channel::PitchMode::PreservePitch).
//!
//! Unlike [`LoopChannel`](crate::mixer::loop_channel::LoopChannel)'s per-sample
//! cursor advance, this works in fixed-size *output* hops: every `hop_len`
//! output frames, [`WsolaStretcher::synthesize_next_hop`] extracts a
//! `window_len`-frame grain from the source buffer, searches a small window
//! around the tempo-warped "ideal" analysis position for the best-aligned
//! start (via normalized cross-correlation against the tail of the previous
//! grain), and overlap-adds the Hann-windowed result into a scratch buffer
//! that `LoopChannel::tick()` drains one frame at a time.
//!
//! The key trick that decouples tempo from pitch: within a single grain,
//! samples are read from the source at the *native* per-sample step (sample
//! rate conversion + user varispeed only, no tempo-warp) — so the grain's own
//! pitch is untouched. Only the *jump* from one grain's start to the next is
//! scaled by the tempo-warp ratio, which is what changes how much of the
//! source material is consumed per unit of output time.
//!
//! All buffers are preallocated at construction (or lazily on first use);
//! `synthesize_next_hop` performs no heap allocation, so it's real-time safe
//! to call from `LoopChannel::tick()`.

use crate::frame::StereoFrame;
use crate::mixer::stereo_buffer::StereoSampleBuffer;
use crate::utils::raised_sine_window;

/// Fixed synthesis (output-side) hop length, in milliseconds.
const HOP_MS: f32 = 20.0;
/// Search tolerance around the ideal analysis position, in milliseconds of
/// *source* audio (converted to source frames using the buffer's own sample
/// rate at search time).
const SEARCH_MS: f32 = 10.0;
/// Number of steps in the coarse pass of the coarse-to-fine correlation
/// search. Bounds worst-case search cost regardless of `SEARCH_MS`/sample rate.
const COARSE_STEPS: usize = 64;

pub(crate) struct WsolaStretcher {
    /// Output frames produced per hop (fixed for this stretcher's lifetime).
    hop_len: usize,
    /// Grain/analysis window length in output frames; `2 * hop_len` (50% overlap).
    window_len: usize,
    /// Precomputed Hann coefficients, length `window_len`.
    window: Vec<f32>,

    /// Next hop's synthesized output, drained one frame per `tick()` call.
    out_scratch: Vec<StereoFrame>,
    /// Reusable scratch for the windowed grain extracted each hop.
    grain_scratch: Vec<StereoFrame>,
    /// Windowed second half of the previous grain — both the crossfade
    /// carry-over for the next hop's overlap-add and the correlation
    /// reference (via `prev_tail_mono`) for finding the next grain's start.
    prev_tail: Vec<StereoFrame>,
    /// Mono (L+R) sum of `prev_tail`, precomputed once per hop for reuse
    /// across every candidate evaluated during the correlation search.
    prev_tail_mono: Vec<f32>,
    have_prev_tail: bool,

    /// Index of the next frame in `out_scratch` to emit; `>= hop_len` means
    /// the scratch buffer is exhausted and a new hop must be synthesized.
    drain_idx: usize,
    /// Source-frame position the most recently synthesized grain started at.
    /// Also reported to `LoopChannel` for `position_normalized()`.
    analysis_cursor: f64,
}

impl WsolaStretcher {
    /// Build a stretcher sized from the engine's sample rate, with playback
    /// starting at `initial_cursor` (source frames).
    pub(crate) fn new(engine_sample_rate: f32, initial_cursor: f64) -> Self {
        let sr = engine_sample_rate.max(1.0) as f64;
        let hop_len = ((HOP_MS as f64 / 1000.0) * sr).round().max(1.0) as usize;
        let window_len = hop_len * 2;

        // Periodic Hann (denominator `window_len`, not `window_len - 1`) so
        // that `window[i] + window[hop_len + i] == 1.0` for all `i` — the
        // constant-overlap-add property this 50%-overlap scheme relies on.
        let window: Vec<f32> = (0..window_len)
            .map(|i| raised_sine_window(i as f32 / window_len as f32, 2.0))
            .collect();

        Self {
            hop_len,
            window_len,
            window,
            out_scratch: vec![StereoFrame::default(); hop_len],
            grain_scratch: vec![StereoFrame::default(); window_len],
            prev_tail: vec![StereoFrame::default(); hop_len],
            prev_tail_mono: vec![0.0; hop_len],
            have_prev_tail: false,
            drain_idx: hop_len, // force a synth pass before the first drain()
            analysis_cursor: initial_cursor,
        }
    }

    /// Whether `out_scratch` is exhausted and a new hop must be synthesized
    /// before the next `drain()`.
    pub(crate) fn needs_refill(&self) -> bool {
        self.drain_idx >= self.hop_len
    }

    /// Emit the next frame from `out_scratch`. Caller must ensure
    /// `!needs_refill()` (or accept silence past the end of the buffer).
    pub(crate) fn drain(&mut self) -> StereoFrame {
        let frame = self
            .out_scratch
            .get(self.drain_idx)
            .copied()
            .unwrap_or_default();
        self.drain_idx += 1;
        frame
    }

    /// Synthesize the next hop's worth of output into `out_scratch` and reset
    /// the drain cursor. Returns the new `analysis_cursor` (source frames) so
    /// the caller can mirror it into `LoopChannel::cursor` for position
    /// reporting.
    ///
    /// `sr_ratio` is `buffer.sample_rate() / engine_sample_rate` (the same
    /// sample-rate-conversion term `LoopChannel::advance()` uses). `speed` is
    /// the channel's varispeed (only `>= 0` is meaningful here — the caller
    /// is responsible for falling back to resample-mode playback for reverse
    /// speeds). `warp` is `engine_bpm / source_bpm` (or `1.0` if warping
    /// doesn't apply) — the tempo-warp ratio, applied only to the hop-to-hop
    /// jump, never to the within-grain sample step (that's what preserves
    /// pitch).
    pub(crate) fn synthesize_next_hop(
        &mut self,
        buffer: &StereoSampleBuffer,
        loop_lo: f64,
        loop_hi: f64,
        sr_ratio: f64,
        speed: f64,
        warp: f64,
    ) -> f64 {
        // Native per-output-sample source step: sample-rate conversion and
        // user varispeed only — no warp. This is what determines the pitch
        // heard within a grain.
        let step = (sr_ratio * speed.max(0.0)).max(1e-6);
        let hop_source_span = self.hop_len as f64 * step;
        let grain_source_span = (self.window_len as f64 - 1.0) * step + 1.0;
        let max_start = (loop_hi - grain_source_span).max(loop_lo);

        // The hop-to-hop jump is scaled by `warp`: this is the only place
        // tempo-warp enters, decoupling source-consumption rate (tempo) from
        // the per-grain sample step (pitch).
        let raw_target = self.analysis_cursor + hop_source_span * warp.max(0.0);
        let (search_center, wrapped) = if raw_target > max_start || max_start <= loop_lo {
            (loop_lo, true)
        } else {
            (raw_target.max(loop_lo), false)
        };
        if wrapped {
            // A grain here would read past the loop end (or the loop window
            // is too small to fit one). Restart from the loop start with a
            // fresh, non-crossfaded grain rather than correlating across the
            // seam — a single ~one-hop discontinuity at the loop point is
            // the accepted v1 tradeoff (see plan doc).
            self.have_prev_tail = false;
        }

        let best_start = if self.have_prev_tail {
            self.search_best_start(buffer, search_center, step, loop_lo, max_start)
        } else {
            search_center
        };

        // Extract and window the new grain.
        for (i, slot) in self.grain_scratch.iter_mut().enumerate() {
            let pos = (best_start + i as f64 * step).clamp(loop_lo, loop_hi);
            let raw = buffer.read_interpolated(pos);
            let w = self.window[i];
            *slot = StereoFrame {
                l: raw.l * w,
                r: raw.r * w,
            };
        }

        // Overlap-add: first half of the new grain against the carried tail
        // of the previous one (silence if this is the first hop / a
        // just-wrapped seam — a brief fade-in rather than a hard onset).
        for i in 0..self.hop_len {
            let prev = if self.have_prev_tail {
                self.prev_tail[i]
            } else {
                StereoFrame::default()
            };
            let new = self.grain_scratch[i];
            self.out_scratch[i] = StereoFrame {
                l: prev.l + new.l,
                r: prev.r + new.r,
            };
        }

        // Carry the new grain's second half forward as next hop's tail /
        // correlation reference.
        for i in 0..self.hop_len {
            self.prev_tail[i] = self.grain_scratch[self.hop_len + i];
            let f = self.prev_tail[i];
            self.prev_tail_mono[i] = f.l + f.r;
        }

        self.have_prev_tail = true;
        self.drain_idx = 0;
        self.analysis_cursor = best_start;
        best_start
    }

    /// Coarse-to-fine normalized cross-correlation search for the source
    /// position (within `[center - Δ, center + Δ]`, clamped to
    /// `[loop_lo, max_start]`) whose next `hop_len` samples best match
    /// `prev_tail_mono`. Cost is bounded to `~(COARSE_STEPS + 2*refine)`
    /// candidate evaluations regardless of `Δ`, so it stays well inside the
    /// per-hop real-time budget.
    fn search_best_start(
        &self,
        buffer: &StereoSampleBuffer,
        center: f64,
        step: f64,
        loop_lo: f64,
        max_start: f64,
    ) -> f64 {
        let radius = ((SEARCH_MS as f64 / 1000.0) * buffer.sample_rate() as f64)
            .round()
            .max(1.0);
        let lo_bound = (center - radius).max(loop_lo);
        let hi_bound = (center + radius).min(max_start);
        if hi_bound <= lo_bound {
            return center.clamp(loop_lo, max_start);
        }

        let score_at = |start: f64| -> f32 {
            let mut num = 0.0f32;
            let mut ref_energy = 0.0f32;
            let mut cand_energy = 0.0f32;
            for (i, reference) in self.prev_tail_mono.iter().enumerate() {
                let pos = (start + i as f64 * step).clamp(loop_lo, max_start + step);
                let raw = buffer.read_interpolated(pos);
                let cand = raw.l + raw.r;
                num += cand * reference;
                ref_energy += reference * reference;
                cand_energy += cand * cand;
            }
            if ref_energy <= f32::EPSILON || cand_energy <= f32::EPSILON {
                0.0
            } else {
                num / (ref_energy.sqrt() * cand_energy.sqrt())
            }
        };

        let span = hi_bound - lo_bound;
        let coarse_stride = (span / COARSE_STEPS as f64).max(1.0);

        let mut best = lo_bound;
        let mut best_score = f32::MIN;
        let mut c = lo_bound;
        while c <= hi_bound {
            let s = score_at(c);
            if s > best_score {
                best_score = s;
                best = c;
            }
            c += coarse_stride;
        }

        let refine_lo = (best - coarse_stride).max(lo_bound);
        let refine_hi = (best + coarse_stride).min(hi_bound);
        let mut c = refine_lo;
        while c <= refine_hi {
            let s = score_at(c);
            if s > best_score {
                best_score = s;
                best = c;
            }
            c += 1.0;
        }

        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn sine_buffer(seconds: f32, hz: f32, sample_rate: f32) -> StereoSampleBuffer {
        let frames = (sample_rate * seconds) as usize;
        let left: Vec<f32> = (0..frames)
            .map(|i| (i as f32 / sample_rate * hz * std::f32::consts::TAU).sin())
            .collect();
        let right = left.clone();
        StereoSampleBuffer::from_channels(left, right, sample_rate).unwrap()
    }

    #[test]
    fn hop_and_window_are_consistent() {
        let s = WsolaStretcher::new(SR, 0.0);
        assert_eq!(s.window_len, s.hop_len * 2);
        assert_eq!(s.window.len(), s.window_len);
        assert_eq!(s.out_scratch.len(), s.hop_len);
    }

    #[test]
    fn window_satisfies_constant_overlap_add() {
        let s = WsolaStretcher::new(SR, 0.0);
        for i in 0..s.hop_len {
            let sum = s.window[i] + s.window[s.hop_len + i];
            assert!((sum - 1.0).abs() < 1e-4, "COLA violated at {i}: {sum}");
        }
    }

    #[test]
    fn synthesize_produces_finite_output() {
        let buffer = sine_buffer(2.0, 220.0, SR);
        let mut s = WsolaStretcher::new(SR, 0.0);
        let loop_hi = buffer.len() as f64;
        for _ in 0..50 {
            s.synthesize_next_hop(&buffer, 0.0, loop_hi, 1.0, 1.0, 1.0);
            for frame in &s.out_scratch {
                assert!(frame.l.is_finite() && frame.r.is_finite());
            }
        }
    }

    #[test]
    fn warp_ratio_advances_analysis_cursor_faster() {
        let buffer = sine_buffer(4.0, 220.0, SR);
        let loop_hi = buffer.len() as f64;

        let mut slow = WsolaStretcher::new(SR, 0.0);
        let mut fast = WsolaStretcher::new(SR, 0.0);
        for _ in 0..20 {
            slow.synthesize_next_hop(&buffer, 0.0, loop_hi, 1.0, 1.0, 1.0);
            fast.synthesize_next_hop(&buffer, 0.0, loop_hi, 1.0, 1.0, 2.0);
        }
        assert!(
            fast.analysis_cursor > slow.analysis_cursor,
            "2x warp should consume source material faster: fast={} slow={}",
            fast.analysis_cursor,
            slow.analysis_cursor
        );
    }
}
