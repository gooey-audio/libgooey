//! Spring reverb effect using a series allpass chain with global feedback
//!
//! Models a spring reverb using a chain of allpass filters that simulate the
//! dispersive delay characteristics of a physical spring. A global feedback
//! loop with one-pole lowpass damping creates the reverb tail. This topology
//! preserves transients cleanly (allpasses are unity-gain) and produces the
//! characteristic "sproingy" dispersion of a spring reverb.

use crate::effects::Effect;
use crate::utils::smoother::SmoothedParam;

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU32, Ordering};

/// Threshold for flushing denormal numbers to zero
const DENORMAL_THRESHOLD: f32 = 1e-15;

/// Number of series allpass filters in the reverb chain
const NUM_ALLPASSES: usize = 6;

/// Allpass delay lengths in samples at 44100 Hz (prime numbers, increasing size).
/// Total round-trip ≈ 61ms, giving a natural spring bounce period.
const ALLPASS_DELAYS_44100: [usize; NUM_ALLPASSES] = [131, 251, 389, 521, 617, 787];

/// Per-allpass feedback coefficients (decreasing slightly for longer delays
/// to keep diffusion dense without ringing)
const ALLPASS_GAINS: [f32; NUM_ALLPASSES] = [0.70, 0.68, 0.65, 0.62, 0.60, 0.58];

/// Maximum global feedback (keeps reverb tail finite and stable)
const MAX_FEEDBACK: f32 = 0.95;

struct AllpassFilter {
    buffer: Vec<f32>,
    index: usize,
}

impl AllpassFilter {
    /// True Schroeder allpass: H(z) = (g + z^-N) / (1 + g·z^-N), unity gain for all frequencies.
    fn process(&mut self, input: f32, gain: f32) -> f32 {
        let delayed = self.buffer[self.index];
        let v = input - gain * delayed;
        let output = gain * v + delayed;
        self.buffer[self.index] = v;
        self.index = (self.index + 1) % self.buffer.len();
        output
    }
}

struct ReverbState {
    allpasses: [AllpassFilter; NUM_ALLPASSES],
    feedback_sample: f32,
    damping_filter_state: f32,
    decay_smoothed: SmoothedParam,
    mix_smoothed: SmoothedParam,
    damping_smoothed: SmoothedParam,
}

pub struct SpringReverbEffect {
    state: UnsafeCell<ReverbState>,
    decay_target: AtomicU32,
    mix_target: AtomicU32,
    damping_target: AtomicU32,
}

// SAFETY: UnsafeCell state is only accessed from the audio thread via process()
unsafe impl Send for SpringReverbEffect {}
unsafe impl Sync for SpringReverbEffect {}

impl SpringReverbEffect {
    pub fn new(sample_rate: f32, decay: f32, mix: f32, damping: f32) -> Self {
        let decay = decay.clamp(0.0, 1.0);
        let mix = mix.clamp(0.0, 1.0);
        let damping = damping.clamp(0.0, 1.0);

        let scale = sample_rate / 44100.0;

        let allpasses = std::array::from_fn(|i| {
            let len = ((ALLPASS_DELAYS_44100[i] as f32) * scale).max(1.0) as usize;
            AllpassFilter {
                buffer: vec![0.0; len],
                index: 0,
            }
        });

        Self {
            state: UnsafeCell::new(ReverbState {
                allpasses,
                feedback_sample: 0.0,
                damping_filter_state: 0.0,
                decay_smoothed: SmoothedParam::new_normalized(decay, sample_rate),
                mix_smoothed: SmoothedParam::new_normalized(mix, sample_rate),
                damping_smoothed: SmoothedParam::new_normalized(damping, sample_rate),
            }),
            decay_target: AtomicU32::new(decay.to_bits()),
            mix_target: AtomicU32::new(mix.to_bits()),
            damping_target: AtomicU32::new(damping.to_bits()),
        }
    }

    pub fn set_decay(&self, value: f32) {
        self.decay_target
            .store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn get_decay(&self) -> f32 {
        f32::from_bits(self.decay_target.load(Ordering::Relaxed))
    }

    pub fn set_mix(&self, value: f32) {
        self.mix_target
            .store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn get_mix(&self) -> f32 {
        f32::from_bits(self.mix_target.load(Ordering::Relaxed))
    }

    pub fn set_damping(&self, value: f32) {
        self.damping_target
            .store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn get_damping(&self) -> f32 {
        f32::from_bits(self.damping_target.load(Ordering::Relaxed))
    }
}

impl Effect for SpringReverbEffect {
    fn process(&self, input: f32) -> f32 {
        // SAFETY: process() is only called from the audio thread
        let state = unsafe { &mut *self.state.get() };

        let input = if input.is_finite() { input } else { 0.0 };

        // Update smoothed parameters from atomic targets
        state
            .decay_smoothed
            .set_target(f32::from_bits(self.decay_target.load(Ordering::Relaxed)));
        state
            .mix_smoothed
            .set_target(f32::from_bits(self.mix_target.load(Ordering::Relaxed)));
        state
            .damping_smoothed
            .set_target(f32::from_bits(self.damping_target.load(Ordering::Relaxed)));

        let decay = state.decay_smoothed.tick();
        let mix = state.mix_smoothed.tick();
        let damping = state.damping_smoothed.tick();

        // Exponential mapping: decay^0.4 spreads the usable range across the full knob.
        // At decay=0.1 → feedback≈0.36, decay=0.5 → feedback≈0.72, decay=1.0 → feedback=0.95
        let feedback = decay.powf(0.4) * MAX_FEEDBACK;

        // Damping coefficients for one-pole lowpass in feedback path
        let damp1 = damping;
        let damp2 = 1.0 - damping;

        // Mix feedback from previous iteration into input
        let mut signal = input + state.feedback_sample;

        // Series allpass chain — each allpass disperses the signal,
        // creating the characteristic spring reverb "chirp"
        for (i, ap) in state.allpasses.iter_mut().enumerate() {
            signal = ap.process(signal, ALLPASS_GAINS[i]);
        }

        // One-pole lowpass in the feedback path for high-frequency damping
        state.damping_filter_state = signal * damp2 + state.damping_filter_state * damp1;
        if state.damping_filter_state.abs() < DENORMAL_THRESHOLD {
            state.damping_filter_state = 0.0;
        }

        // Store feedback for next sample
        state.feedback_sample = state.damping_filter_state * feedback;
        if state.feedback_sample.abs() < DENORMAL_THRESHOLD {
            state.feedback_sample = 0.0;
        }

        // Dry/wet mix
        let result = input * (1.0 - mix) + signal * mix;

        if result.is_finite() {
            result
        } else {
            input
        }
    }
}
