//! Nonlinear two-pole resonator for struck, self-ringing percussion voices.
//!
//! Unlike the general-purpose state-variable filters in this crate, this core
//! keeps its integrator state in `f64`. That matters for low fundamentals and
//! long decays, where an 8 Hz mode can otherwise disappear into `f32`
//! quantization. Energy can be injected directly into the first integrator to
//! model a trigger pulse charging an analog resonator.

use std::f64::consts::PI;

const MIN_FREQUENCY_HZ: f64 = 2.0;
const MIN_DAMPING: f64 = 1.0e-4;
const MAX_DAMPING: f64 = 1.0;
const MAX_FEEDBACK: f64 = 0.8;
const QUIET_THRESHOLD: f64 = 1.0e-5;

/// A topology-preserving-transform state-variable resonator.
pub struct Resonator {
    sample_rate: f64,
    freq_hz: f64,
    zeta: f64,
    feedback: f64,
    g: f64,
    k: f64,
    s1: f64,
    s2: f64,
    prev_band: f64,
    prev_low: f64,
}

impl Resonator {
    /// Construct a silent resonator at 60 Hz with moderate damping.
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0) as f64;
        let mut resonator = Self {
            sample_rate,
            freq_hz: 60.0,
            zeta: 0.1,
            feedback: 0.0,
            g: 0.0,
            k: 0.2,
            s1: 0.0,
            s2: 0.0,
            prev_band: 0.0,
            prev_low: 0.0,
        };
        resonator.update_frequency_coefficient();
        resonator
    }

    /// Set the resonant frequency in Hz.
    pub fn set_frequency(&mut self, frequency_hz: f32) {
        let frequency_hz = if frequency_hz.is_finite() {
            frequency_hz as f64
        } else {
            60.0
        };
        let clamped = frequency_hz.clamp(MIN_FREQUENCY_HZ, self.sample_rate * 0.45);
        if (clamped - self.freq_hz).abs() > 1.0e-9 {
            self.freq_hz = clamped;
            self.update_frequency_coefficient();
        }
    }

    /// Set decay as the number of seconds needed to fall by 60 dB.
    pub fn set_decay_time(&mut self, t60_seconds: f32) {
        let t60 = if t60_seconds.is_finite() {
            t60_seconds.max(1.0e-4) as f64
        } else {
            0.5
        };
        let zeta = 6.91 / (2.0 * PI * self.effective_frequency_hz() * t60);
        self.set_damping(zeta as f32);
    }

    /// Set the dimensionless damping ratio directly.
    pub fn set_damping(&mut self, zeta: f32) {
        let zeta = if zeta.is_finite() { zeta as f64 } else { 0.1 };
        self.zeta = zeta.clamp(MIN_DAMPING, MAX_DAMPING);
        self.k = 2.0 * self.zeta;
    }

    /// Add positive, saturating feedback around the low-pass state.
    pub fn set_feedback(&mut self, feedback: f32) {
        self.feedback = if feedback.is_finite() {
            feedback.clamp(0.0, MAX_FEEDBACK as f32) as f64
        } else {
            0.0
        };
        self.update_frequency_coefficient();
    }

    /// Inject trigger energy directly into the first integrator.
    pub fn excite(&mut self, energy: f32) {
        if energy.is_finite() {
            self.s1 = Self::bound_state(self.s1 + energy as f64);
        }
    }

    /// Process one external excitation sample and return the resonator output.
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let input = if input.is_finite() { input as f64 } else { 0.0 };
        let x = input + self.feedback * self.prev_low.tanh();
        let h = 1.0 / (1.0 + self.k * self.g + self.g * self.g);
        let v1 = (self.g * (x - self.s2) + self.s1) * h;
        let v2 = self.s2 + self.g * v1;

        self.s1 = Self::bound_state(2.0 * v1 - self.s1);
        self.s2 = Self::bound_state(2.0 * v2 - self.s2);
        self.prev_band = v1;
        self.prev_low = v2;

        if self.s1.is_finite() && self.s2.is_finite() && v2.is_finite() {
            v2 as f32
        } else {
            self.reset();
            0.0
        }
    }

    /// Return the most recently computed band-pass tap.
    pub fn bandpass(&self) -> f32 {
        self.prev_band as f32
    }

    /// Clear all stored energy.
    pub fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
        self.prev_band = 0.0;
        self.prev_low = 0.0;
    }

    /// Return true once both integrator states are below the audible floor.
    pub fn is_quiet(&self) -> bool {
        self.s1.abs() + self.s2.abs() < QUIET_THRESHOLD
    }

    fn update_frequency_coefficient(&mut self) {
        self.g = (PI * self.effective_frequency_hz() / self.sample_rate).tan();
    }

    fn effective_frequency_hz(&self) -> f64 {
        (self.freq_hz / (1.0 - self.feedback).sqrt()).min(self.sample_rate * 0.45)
    }

    #[inline]
    fn bound_state(state: f64) -> f64 {
        if state.abs() <= 1.0 {
            state
        } else {
            state.signum() * (1.0 + (state.abs() - 1.0).tanh())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f32 = 48_000.0;

    fn render(resonator: &mut Resonator, seconds: f32) -> Vec<f32> {
        let samples = (seconds * SAMPLE_RATE) as usize;
        (0..samples).map(|_| resonator.process(0.0)).collect()
    }

    #[test]
    fn excite_rings_at_set_frequency() {
        for feedback in [0.0, 0.6] {
            let mut resonator = Resonator::new(SAMPLE_RATE);
            resonator.set_frequency(60.0);
            resonator.set_feedback(feedback);
            resonator.set_decay_time(1.0);
            resonator.excite(0.1);
            let output = render(&mut resonator, 1.0);
            let crossings = output
                .windows(2)
                .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
                .count();

            assert!(
                (58..=62).contains(&crossings),
                "feedback={feedback}, crossings={crossings}"
            );
        }
    }

    #[test]
    fn decay_does_not_collapse_at_normal_amplitude() {
        let mut resonator = Resonator::new(SAMPLE_RATE);
        resonator.set_frequency(60.0);
        resonator.set_decay_time(1.0);
        resonator.excite(0.6);
        let output = render(&mut resonator, 0.12);
        let peak = output
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        let envelope_100ms = output[4_560..5_040]
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));

        assert!(peak >= 0.5, "peak={peak}");
        assert!(envelope_100ms >= 0.3, "envelope={envelope_100ms}");
    }

    #[test]
    fn output_bounded_with_max_feedback() {
        for frequency in [8.0, 80.0] {
            let mut resonator = Resonator::new(SAMPLE_RATE);
            resonator.set_frequency(frequency);
            resonator.set_feedback(1.0);
            resonator.set_decay_time(6.0);
            resonator.excite(1.0);
            let output = render(&mut resonator, 10.0);
            for sample in &output {
                assert!(sample.is_finite());
                assert!(sample.abs() < 2.0, "frequency={frequency}, sample={sample}");
            }
            let tail = &output[9 * SAMPLE_RATE as usize..];
            let tail_mean =
                tail.iter().map(|&sample| sample as f64).sum::<f64>() / tail.len() as f64;
            assert!(
                tail_mean.abs() < 0.005,
                "frequency={frequency}, tail_mean={tail_mean}"
            );
        }
    }

    #[test]
    fn longer_decay_rings_longer() {
        fn late_energy(decay: f32) -> f64 {
            let mut resonator = Resonator::new(SAMPLE_RATE);
            resonator.set_frequency(60.0);
            resonator.set_decay_time(decay);
            resonator.excite(0.1);
            render(&mut resonator, 1.0)
                .into_iter()
                .skip(36_000)
                .map(|sample| (sample as f64).powi(2))
                .sum::<f64>()
        }

        assert!(late_energy(2.0) > late_energy(0.2) * 100.0);
    }

    #[test]
    fn is_quiet_after_decay() {
        let mut resonator = Resonator::new(SAMPLE_RATE);
        resonator.set_frequency(60.0);
        resonator.set_decay_time(0.05);
        resonator.excite(0.05);
        let _ = render(&mut resonator, 1.0);
        assert!(resonator.is_quiet());
    }
}
