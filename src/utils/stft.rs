//! Short-time Fourier transform (STFT) processor with weighted overlap-add.
//!
//! This bridges block-based spectral processing into libgooey's strictly
//! per-sample [`Effect`](crate::effects::Effect) interface: you feed one input
//! sample and get one output sample back, while internally the processor
//! accumulates frames, runs an FFT every hop, lets a caller-supplied closure
//! modify the complex spectrum, runs the inverse FFT, and overlap-adds the
//! windowed result into an output ring.
//!
//! ## Windowing / reconstruction
//!
//! A periodic Hann window is applied on BOTH analysis and synthesis (a `w²`
//! weighted-overlap-add, WOLA). At 75% overlap (hop = `fft_size / 4`) the sum
//! of the squared Hann windows is constant, satisfying the constant-overlap-add
//! (COLA) condition, so a no-op spectral modification reconstructs the input
//! exactly (delayed by `fft_size` samples). The normalization constant — which
//! folds in both the COLA window sum and rustfft's unnormalized inverse `1/N`
//! factor — is computed numerically at construction so it stays correct if the
//! FFT size or overlap changes.
//!
//! ## Latency
//!
//! Output is delayed by exactly `fft_size` samples (one analysis window). The
//! first `fft_size` output samples are silence (pre-roll). There is no
//! latency-compensation mechanism in the effect chain, so any effect built on
//! this adds `fft_size` samples of latency to wherever it sits.
//!
//! ## Real-time safety
//!
//! All buffers and FFT plans are allocated in [`StftProcessor::new`]. The
//! per-sample path uses `process_with_scratch` with preallocated scratch, so it
//! never allocates on the audio thread.

use std::sync::Arc;

use rustfft::{num_complex::Complex, Fft, FftPlanner};

/// Overlap factor: hop = fft_size / OVERLAP. 4 → 75% overlap (standard WOLA).
const OVERLAP: usize = 4;

/// Single-channel STFT / weighted-overlap-add processor.
///
/// One instance holds the state for one audio channel; stereo effects keep two.
/// The forward/inverse FFT plans are shared via `Arc` (the `Fft` itself is
/// `Sync`), but every buffer below is per-instance, so two channels never alias.
pub struct StftProcessor {
    fft_size: usize,
    hop: usize,

    fft_fwd: Arc<dyn Fft<f32>>,
    fft_inv: Arc<dyn Fft<f32>>,

    /// Periodic Hann window, length `fft_size`. Used for both analysis and
    /// synthesis.
    window: Vec<f32>,
    /// Combined normalization: `1 / (Σ w² · fft_size)`. The `Σ w²` term is the
    /// COLA window-overlap sum; the `fft_size` term cancels rustfft's
    /// unnormalized inverse transform.
    cola_scale: f32,

    /// Ring of the most recent `fft_size` input samples.
    history: Vec<f32>,
    hist_write: usize,

    /// Complex FFT work buffer (`fft_size`).
    spectrum: Vec<Complex<f32>>,
    fwd_scratch: Vec<Complex<f32>>,
    inv_scratch: Vec<Complex<f32>>,

    /// Output overlap-add ring. Length `fft_size + hop` guarantees the slot for
    /// an emitted (and cleared) output position is not reused by a future frame
    /// before that position has been read out.
    out_buf: Vec<f32>,

    /// Number of input samples consumed so far. Drives frame triggering and the
    /// `fft_size`-sample output delay. `u64` so it never wraps in practice.
    n_in: u64,
}

impl StftProcessor {
    /// Create a processor for the given FFT size. `fft_size` must be a multiple
    /// of [`OVERLAP`] (4); 1024 is the typical choice (~23 ms latency at 44.1 kHz).
    pub fn new(fft_size: usize, _sample_rate: f32) -> Self {
        debug_assert!(
            fft_size % OVERLAP == 0,
            "fft_size must be divisible by the overlap factor"
        );
        let hop = fft_size / OVERLAP;

        let mut planner = FftPlanner::<f32>::new();
        let fft_fwd = planner.plan_fft_forward(fft_size);
        let fft_inv = planner.plan_fft_inverse(fft_size);

        // Periodic Hann window (matches src/visualization/spectrogram.rs).
        let window: Vec<f32> = (0..fft_size)
            .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / fft_size as f32).cos()))
            .collect();

        // COLA window-overlap sum at one output position: with analysis +
        // synthesis Hann at this hop the per-position gain is Σ_k w[k·hop]².
        // (Constant across positions for valid COLA configurations.)
        let mut wsum = 0.0_f32;
        let mut k = 0;
        while k < fft_size {
            wsum += window[k] * window[k];
            k += hop;
        }
        let cola_scale = 1.0 / (wsum * fft_size as f32);

        let fwd_scratch = vec![Complex::new(0.0, 0.0); fft_fwd.get_inplace_scratch_len()];
        let inv_scratch = vec![Complex::new(0.0, 0.0); fft_inv.get_inplace_scratch_len()];

        Self {
            fft_size,
            hop,
            fft_fwd,
            fft_inv,
            window,
            cola_scale,
            history: vec![0.0; fft_size],
            hist_write: 0,
            spectrum: vec![Complex::new(0.0, 0.0); fft_size],
            fwd_scratch,
            inv_scratch,
            out_buf: vec![0.0; fft_size + hop],
            n_in: 0,
        }
    }

    /// The hop size in samples (distance between successive analysis frames).
    pub fn hop(&self) -> usize {
        self.hop
    }

    /// Inherent latency in samples (`fft_size`).
    pub fn latency_samples(&self) -> usize {
        self.fft_size
    }

    /// Push one input sample and return one output sample (delayed by
    /// `fft_size`). On every hop boundary the analysis/modify/synthesis cycle
    /// runs; `modify` receives the full complex spectrum (length `fft_size`) to
    /// mutate in place. The output sample corresponds to the input fed
    /// `fft_size` samples ago; the first `fft_size` outputs are silence.
    #[inline]
    pub fn process_sample<F: FnMut(&mut [Complex<f32>])>(
        &mut self,
        input: f32,
        modify: &mut F,
    ) -> f32 {
        let n = self.n_in;
        let out_len = self.out_buf.len();

        // 1. Store the input in the history ring; hist_write now points at the
        //    oldest retained sample.
        self.history[self.hist_write] = if input.is_finite() { input } else { 0.0 };
        self.hist_write = (self.hist_write + 1) % self.fft_size;

        // 2. Emit (and clear) the output for absolute position n - fft_size.
        //    Negative positions are pre-roll silence.
        let out = if n >= self.fft_size as u64 {
            let pos = n - self.fft_size as u64;
            let idx = (pos % out_len as u64) as usize;
            let v = self.out_buf[idx];
            self.out_buf[idx] = 0.0;
            v
        } else {
            0.0
        };

        // 3. On a hop boundary, the frame ending at input index n is complete.
        if (n + 1) % self.hop as u64 == 0 {
            self.run_frame(n, modify);
        }

        self.n_in += 1;
        out
    }

    /// Run one analysis → modify → synthesis frame for the block ending at input
    /// index `frame_end`. The reconstructed block maps to output positions
    /// `[frame_end - fft_size + 1 ..= frame_end]`.
    fn run_frame<F: FnMut(&mut [Complex<f32>])>(&mut self, frame_end: u64, modify: &mut F) {
        // Copy the fft_size most recent inputs (oldest → newest) into the
        // complex buffer, applying the analysis window.
        let start = self.hist_write; // oldest retained sample
        for n in 0..self.fft_size {
            let s = self.history[(start + n) % self.fft_size];
            self.spectrum[n] = Complex::new(s * self.window[n], 0.0);
        }

        self.fft_fwd
            .process_with_scratch(&mut self.spectrum, &mut self.fwd_scratch);

        modify(&mut self.spectrum);

        self.fft_inv
            .process_with_scratch(&mut self.spectrum, &mut self.inv_scratch);

        // Overlap-add the windowed, normalized result into the output ring. The
        // first output sample of this block is at absolute position
        // frame_end - fft_size + 1, which is negative for the first few frames
        // (their leading part lands in the discarded pre-roll); skip those.
        let out_len = self.out_buf.len() as i64;
        let block_start = frame_end as i64 + 1 - self.fft_size as i64;
        for n in 0..self.fft_size {
            let pos = block_start + n as i64;
            if pos < 0 {
                continue;
            }
            let y = self.spectrum[n].re * self.window[n] * self.cola_scale;
            let idx = (pos % out_len) as usize;
            self.out_buf[idx] += y;
        }
    }

    /// Clear all internal state (buffers and indices) back to silence.
    pub fn reset(&mut self) {
        self.history.iter_mut().for_each(|s| *s = 0.0);
        self.out_buf.iter_mut().for_each(|s| *s = 0.0);
        self.hist_write = 0;
        self.n_in = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FFT_SIZE: usize = 1024;
    const SR: f32 = 44_100.0;

    /// A no-op spectral modification must reconstruct the input exactly, delayed
    /// by `fft_size`. This is the load-bearing correctness test: it fails if the
    /// COLA normalization or the overlap-add index bookkeeping is wrong.
    #[test]
    fn reconstructs_input_delayed_by_fft_size() {
        let mut stft = StftProcessor::new(FFT_SIZE, SR);
        let mut noop = |_spec: &mut [Complex<f32>]| {};

        // 1 kHz sine input.
        let freq = 1000.0_f32;
        let n_samples = FFT_SIZE * 8;
        let input: Vec<f32> = (0..n_samples)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / SR).sin())
            .collect();

        let output: Vec<f32> = input
            .iter()
            .map(|&x| stft.process_sample(x, &mut noop))
            .collect();

        // After the pre-roll, output[i] should equal input[i - FFT_SIZE]. Allow
        // a small margin near the very start of the steady region for window
        // ramp-in, so compare from 2*FFT_SIZE onward.
        let mut max_err = 0.0_f32;
        for i in (2 * FFT_SIZE)..n_samples {
            let err = (output[i] - input[i - FFT_SIZE]).abs();
            max_err = max_err.max(err);
        }
        assert!(
            max_err < 1e-3,
            "reconstruction error too large: {max_err} (COLA/indexing bug)"
        );
    }

    #[test]
    fn silence_in_silence_out_no_nan() {
        let mut stft = StftProcessor::new(FFT_SIZE, SR);
        let mut noop = |_spec: &mut [Complex<f32>]| {};
        for _ in 0..(FFT_SIZE * 4) {
            let out = stft.process_sample(0.0, &mut noop);
            assert!(out.is_finite());
            assert_eq!(out, 0.0);
        }
    }

    #[test]
    fn first_fft_size_outputs_are_silent_preroll() {
        let mut stft = StftProcessor::new(FFT_SIZE, SR);
        let mut noop = |_spec: &mut [Complex<f32>]| {};
        // Impulse at t=0.
        let mut outputs = Vec::new();
        for i in 0..(FFT_SIZE * 3) {
            let x = if i == 0 { 1.0 } else { 0.0 };
            outputs.push(stft.process_sample(x, &mut noop));
        }
        // Pre-roll: the first fft_size outputs are silence.
        for (i, &o) in outputs.iter().take(FFT_SIZE).enumerate() {
            assert_eq!(o, 0.0, "expected pre-roll silence at {i}, got {o}");
        }
        // Energy must appear after the delay.
        let tail_energy: f32 = outputs[FFT_SIZE..].iter().map(|o| o.abs()).sum();
        assert!(tail_energy > 1e-3, "impulse energy missing after pre-roll");
    }

    #[test]
    fn reset_clears_state() {
        let mut stft = StftProcessor::new(FFT_SIZE, SR);
        let mut noop = |_spec: &mut [Complex<f32>]| {};
        for _ in 0..(FFT_SIZE * 2) {
            stft.process_sample(0.5, &mut noop);
        }
        stft.reset();
        // Immediately after reset the next fft_size outputs are pre-roll silence.
        for _ in 0..FFT_SIZE {
            assert_eq!(stft.process_sample(0.0, &mut noop), 0.0);
        }
    }
}
