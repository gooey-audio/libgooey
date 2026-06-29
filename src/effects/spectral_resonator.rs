//! Spectral resonator effect (STFT-based).
//!
//! Breaks the input into spectral partials with a short-time Fourier transform,
//! then emphasizes and *resonates* the frequency bins that align with a tunable
//! fundamental and its harmonic series. A per-bin magnitude memory with a decay
//! coefficient lets that energy ring over time, and per-bin phase accumulation
//! keeps the ring phase-coherent — producing tuned, metallic, pad-like
//! resonances (conceptually similar to Ableton's Spectral Resonator).
//!
//! Built on [`StftProcessor`](crate::utils::stft::StftProcessor); see that
//! module for the windowing/overlap-add details. Note this effect adds
//! `FFT_SIZE` (1024) samples of latency — ~23 ms at 44.1 kHz — to wherever it
//! sits in the chain. There is no latency compensation across effects.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU32, Ordering};

use rustfft::num_complex::Complex;

use crate::effects::Effect;
use crate::frame::StereoFrame;
use crate::utils::smoother::SmoothedParam;
use crate::utils::stft::StftProcessor;

/// FFT size for the spectral transform (fixed for v1).
const FFT_SIZE: usize = 1024;
/// Highest bin index (inclusive) of the non-redundant half-spectrum.
const NYQUIST_BIN: usize = FFT_SIZE / 2;

/// Threshold for flushing denormal numbers to zero.
const DENORMAL_THRESHOLD: f32 = 1e-15;

/// Fundamental frequency range, in Hz.
const MIN_FREQUENCY: f32 = 20.0;
const MAX_FREQUENCY: f32 = 4000.0;

/// Per-channel mutable state (wrapped in `UnsafeCell` for interior mutability).
struct ResonatorState {
    stft: StftProcessor,

    /// Dry signal delayed by `FFT_SIZE` so the wet/dry mix is phase-aligned with
    /// the STFT latency.
    dry_delay: Vec<f32>,
    dry_pos: usize,

    /// Per-bin resonant magnitude memory (`NYQUIST_BIN + 1` bins).
    mag_mem: Vec<f32>,
    /// Per-bin phase accumulator driving the coherent ring.
    phase_acc: Vec<f32>,

    freq_smoothed: SmoothedParam,
    resonance_smoothed: SmoothedParam,
    sharpness_smoothed: SmoothedParam,
    mix_smoothed: SmoothedParam,
}

/// Spectral resonator.
///
/// Parameters (all lock-free via atomics, smoothed per-sample):
/// - `frequency`: fundamental in Hz (20–4000). Resonant peaks sit on this and
///   its integer harmonics.
/// - `resonance`: 0–1, ring/sustain time (mapped to a per-frame magnitude decay).
/// - `sharpness`: 0–1, narrowness of each resonant peak (1 = pure ring, 0 =
///   lets more of the input through).
/// - `mix`: 0–1 dry/wet.
pub struct SpectralResonator {
    sample_rate: f32,
    state: UnsafeCell<[ResonatorState; 2]>,

    frequency_target: AtomicU32,
    resonance_target: AtomicU32,
    sharpness_target: AtomicU32,
    mix_target: AtomicU32,
}

// SAFETY: interior `UnsafeCell` state is only mutated from the audio thread
// (`process`/`process_stereo`); parameter updates from other threads go through
// lock-free atomics. This mirrors the established pattern in `DelayEffect`.
unsafe impl Send for SpectralResonator {}
unsafe impl Sync for SpectralResonator {}

impl SpectralResonator {
    /// Create a new spectral resonator.
    pub fn new(sample_rate: f32, frequency: f32, resonance: f32, sharpness: f32, mix: f32) -> Self {
        let frequency_c = frequency.clamp(MIN_FREQUENCY, MAX_FREQUENCY);
        let resonance_c = resonance.clamp(0.0, 1.0);
        let sharpness_c = sharpness.clamp(0.0, 1.0);
        let mix_c = mix.clamp(0.0, 1.0);

        let make_state = || ResonatorState {
            stft: StftProcessor::new(FFT_SIZE, sample_rate),
            dry_delay: vec![0.0; FFT_SIZE],
            dry_pos: 0,
            mag_mem: vec![0.0; NYQUIST_BIN + 1],
            phase_acc: vec![0.0; NYQUIST_BIN + 1],
            freq_smoothed: SmoothedParam::new(
                frequency_c,
                MIN_FREQUENCY,
                MAX_FREQUENCY,
                sample_rate,
                30.0,
            ),
            resonance_smoothed: SmoothedParam::new(resonance_c, 0.0, 1.0, sample_rate, 30.0),
            sharpness_smoothed: SmoothedParam::new(sharpness_c, 0.0, 1.0, sample_rate, 30.0),
            mix_smoothed: SmoothedParam::new(mix_c, 0.0, 1.0, sample_rate, 30.0),
        };

        Self {
            sample_rate,
            state: UnsafeCell::new([make_state(), make_state()]),
            frequency_target: AtomicU32::new(frequency_c.to_bits()),
            resonance_target: AtomicU32::new(resonance_c.to_bits()),
            sharpness_target: AtomicU32::new(sharpness_c.to_bits()),
            mix_target: AtomicU32::new(mix_c.to_bits()),
        }
    }

    /// Set the fundamental frequency in Hz (clamped to 20–4000).
    pub fn set_frequency(&self, hz: f32) {
        self.frequency_target.store(
            hz.clamp(MIN_FREQUENCY, MAX_FREQUENCY).to_bits(),
            Ordering::Relaxed,
        );
    }
    pub fn get_frequency(&self) -> f32 {
        f32::from_bits(self.frequency_target.load(Ordering::Relaxed))
    }

    /// Set the resonance/ring amount (0–1).
    pub fn set_resonance(&self, v: f32) {
        self.resonance_target
            .store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
    pub fn get_resonance(&self) -> f32 {
        f32::from_bits(self.resonance_target.load(Ordering::Relaxed))
    }

    /// Set the peak sharpness (0–1).
    pub fn set_sharpness(&self, v: f32) {
        self.sharpness_target
            .store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
    pub fn get_sharpness(&self) -> f32 {
        f32::from_bits(self.sharpness_target.load(Ordering::Relaxed))
    }

    /// Set the dry/wet mix (0–1).
    pub fn set_mix(&self, v: f32) {
        self.mix_target
            .store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
    pub fn get_mix(&self) -> f32 {
        f32::from_bits(self.mix_target.load(Ordering::Relaxed))
    }

    /// Inherent latency in samples (`FFT_SIZE`).
    pub fn latency_samples(&self) -> usize {
        FFT_SIZE
    }

    fn process_one(&self, state: &mut ResonatorState, input: f32) -> f32 {
        let input = if input.is_finite() { input } else { 0.0 };

        // Pull atomic targets into the smoothers and advance one sample.
        state.freq_smoothed.set_target(f32::from_bits(
            self.frequency_target.load(Ordering::Relaxed),
        ));
        state.resonance_smoothed.set_target(f32::from_bits(
            self.resonance_target.load(Ordering::Relaxed),
        ));
        state.sharpness_smoothed.set_target(f32::from_bits(
            self.sharpness_target.load(Ordering::Relaxed),
        ));
        state
            .mix_smoothed
            .set_target(f32::from_bits(self.mix_target.load(Ordering::Relaxed)));

        let frequency = state.freq_smoothed.tick();
        let resonance = state.resonance_smoothed.tick();
        let sharpness = state.sharpness_smoothed.tick();
        let mix = state.mix_smoothed.tick();

        // Map resonance (0–1) to a per-frame magnitude decay. A squared
        // complement puts most of the knob travel in the long-tail region and
        // caps below 1.0 so the ring always eventually dies.
        let decay = (1.0 - (1.0 - resonance) * (1.0 - resonance)).min(0.999);
        // Map sharpness (0–1) to a Gaussian peak half-width in bins.
        let bw_bins = (1.0 - sharpness) * 8.0 + 0.7;
        let bin_hz = self.sample_rate / FFT_SIZE as f32;
        // Per-frame phase advance for bin k is `phase_inc * k`.
        let phase_inc = 2.0 * std::f32::consts::PI * state.stft.hop() as f32 / FFT_SIZE as f32;

        // Split the borrow so the closure can hold `mag_mem`/`phase_acc` while
        // the STFT processor borrows its own fields.
        let ResonatorState {
            stft,
            mag_mem,
            phase_acc,
            ..
        } = state;

        let mut modify = |spec: &mut [Complex<f32>]| {
            for k in 0..=NYQUIST_BIN {
                // Resonant gain: 1.0 at a harmonic, falling off as a Gaussian
                // with the bin-distance to the nearest harmonic. DC is removed.
                let gain = if k == 0 || frequency <= 0.0 {
                    0.0
                } else {
                    let bin_freq = k as f32 * bin_hz;
                    let harmonic = (bin_freq / frequency).round().max(1.0);
                    let dist_bins = (bin_freq - harmonic * frequency).abs() / bin_hz;
                    let z = dist_bins / bw_bins;
                    (-z * z).exp()
                };

                let input_mag = spec[k].norm();
                let target = input_mag * gain;
                // Resonant feedback: hold the larger of the new excitation and
                // the decayed previous magnitude.
                let m = target.max(mag_mem[k] * decay);
                mag_mem[k] = m;

                // Advance the per-bin phase and synthesize the resonant bin.
                let ph =
                    (phase_acc[k] + phase_inc * k as f32).rem_euclid(2.0 * std::f32::consts::PI);
                phase_acc[k] = ph;

                if k == 0 {
                    spec[0] = Complex::new(0.0, 0.0);
                } else if k == NYQUIST_BIN {
                    // Nyquist bin must be real for a real inverse transform.
                    spec[k] = Complex::new(m * ph.cos(), 0.0);
                } else {
                    let bin = Complex::new(m * ph.cos(), m * ph.sin());
                    spec[k] = bin;
                    // Maintain Hermitian symmetry so the IFFT output is real.
                    spec[FFT_SIZE - k] = bin.conj();
                }
            }
        };

        let wet = stft.process_sample(input, &mut modify);

        // Delay the dry path by FFT_SIZE to align with the wet latency.
        let dry = state.dry_delay[state.dry_pos];
        state.dry_delay[state.dry_pos] = input;
        state.dry_pos = (state.dry_pos + 1) % FFT_SIZE;

        let mut out = dry * (1.0 - mix) + wet * mix;
        if !out.is_finite() || out.abs() < DENORMAL_THRESHOLD {
            out = 0.0;
        }
        out
    }
}

impl Effect for SpectralResonator {
    fn process(&self, input: f32) -> f32 {
        let states = unsafe { &mut *self.state.get() };
        self.process_one(&mut states[0], input)
    }

    fn process_stereo(&self, input: StereoFrame) -> StereoFrame {
        let states = unsafe { &mut *self.state.get() };
        StereoFrame {
            l: self.process_one(&mut states[0], input.l),
            r: self.process_one(&mut states[1], input.r),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44_100.0;

    #[test]
    fn params_are_clamped() {
        let fx = SpectralResonator::new(SR, 220.0, 0.5, 0.5, 0.5);
        fx.set_frequency(99_999.0);
        assert_eq!(fx.get_frequency(), MAX_FREQUENCY);
        fx.set_frequency(1.0);
        assert_eq!(fx.get_frequency(), MIN_FREQUENCY);
        fx.set_resonance(2.0);
        assert_eq!(fx.get_resonance(), 1.0);
        fx.set_sharpness(-1.0);
        assert_eq!(fx.get_sharpness(), 0.0);
        fx.set_mix(5.0);
        assert_eq!(fx.get_mix(), 1.0);
    }

    #[test]
    fn nan_input_yields_finite_output() {
        let fx = SpectralResonator::new(SR, 220.0, 0.8, 0.8, 1.0);
        for _ in 0..(FFT_SIZE * 4) {
            let out = fx.process(f32::NAN);
            assert!(out.is_finite());
        }
    }

    #[test]
    fn output_stays_bounded_under_sustained_input() {
        let fx = SpectralResonator::new(SR, 220.0, 1.0, 0.9, 1.0);
        let mut max_abs = 0.0_f32;
        for i in 0..(SR as usize) {
            let x = (2.0 * std::f32::consts::PI * 220.0 * i as f32 / SR).sin();
            let out = fx.process(x);
            assert!(out.is_finite());
            max_abs = max_abs.max(out.abs());
        }
        assert!(max_abs < 100.0, "output blew up: {max_abs}");
    }

    #[test]
    fn mix_zero_passes_delayed_dry() {
        let fx = SpectralResonator::new(SR, 220.0, 0.9, 0.9, 0.0);
        let input: Vec<f32> = (0..(FFT_SIZE * 3))
            .map(|i| (2.0 * std::f32::consts::PI * 330.0 * i as f32 / SR).sin())
            .collect();
        let output: Vec<f32> = input.iter().map(|&x| fx.process(x)).collect();
        // With mix=0 the output is the dry signal delayed by FFT_SIZE.
        let mut max_err = 0.0_f32;
        for i in FFT_SIZE..input.len() {
            max_err = max_err.max((output[i] - input[i - FFT_SIZE]).abs());
        }
        assert!(max_err < 1e-6, "dry path not delayed cleanly: {max_err}");
    }

    #[test]
    fn mono_input_keeps_channels_identical() {
        let fx = SpectralResonator::new(SR, 220.0, 0.8, 0.7, 1.0);
        for i in 0..(FFT_SIZE * 4) {
            let x = (2.0 * std::f32::consts::PI * 220.0 * i as f32 / SR).sin();
            let out = fx.process_stereo(StereoFrame::mono(x));
            assert_eq!(out.l, out.r, "channels diverged at {i}");
        }
    }
}
