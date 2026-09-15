//! Short trigger signals used to strike resonant synthesis voices.

use crate::filters::StateVariableFilterTpt;
use crate::gen::ClickOsc;
use crate::utils::XorShift32;

/// The waveform emitted after [`Exciter::trigger`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExciterKind {
    /// A positive raised-cosine pulse.
    Pulse,
    /// The crate's fixed 64-sample click table.
    ClickTable,
    /// A band-limited burst of deterministic white noise.
    NoiseBurst,
}

/// A small, allocation-free one-shot trigger generator.
pub struct Exciter {
    sample_rate: f32,
    kind: ExciterKind,
    width_ms: f32,
    position: usize,
    length: usize,
    amplitude: f32,
    active: bool,
    click: ClickOsc,
    rng: XorShift32,
    noise_filter: StateVariableFilterTpt,
}

impl Exciter {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);
        let width_ms = 1.0;
        Self {
            sample_rate,
            kind: ExciterKind::Pulse,
            width_ms,
            position: 0,
            length: Self::width_to_samples(sample_rate, width_ms),
            amplitude: 0.0,
            active: false,
            click: ClickOsc::new(),
            rng: XorShift32::default(),
            noise_filter: StateVariableFilterTpt::new(sample_rate, 2_000.0, 2.0),
        }
    }

    pub fn set_kind(&mut self, kind: ExciterKind) {
        self.kind = kind;
    }

    pub fn kind(&self) -> ExciterKind {
        self.kind
    }

    pub fn set_width_ms(&mut self, width_ms: f32) {
        self.width_ms = width_ms.clamp(0.25, 8.0);
        self.length = Self::width_to_samples(self.sample_rate, self.width_ms);
    }

    pub fn width_ms(&self) -> f32 {
        self.width_ms
    }

    /// Start the one-shot. Equal seeds reproduce equal noise bursts.
    pub fn trigger(&mut self, amplitude: f32, seed: u32) {
        self.position = 0;
        self.amplitude = if amplitude.is_finite() {
            amplitude.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.active = self.amplitude > 0.0;
        self.rng = XorShift32::new(seed);
        self.noise_filter.reset();
        self.click.reset();
        if self.kind == ExciterKind::ClickTable && self.active {
            self.click.trigger();
        }
    }

    #[inline]
    pub fn tick(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }

        if self.kind == ExciterKind::ClickTable {
            let sample = self.click.tick() * self.amplitude;
            self.active = self.click.is_active();
            return sample;
        }

        if self.position >= self.length {
            self.active = false;
            return 0.0;
        }

        let phase = if self.length <= 1 {
            0.5
        } else {
            self.position as f32 / (self.length - 1) as f32
        };
        let window = 0.5 - 0.5 * (std::f32::consts::TAU * phase).cos();
        let source = match self.kind {
            ExciterKind::Pulse => 1.0,
            ExciterKind::NoiseBurst => self.noise_filter.process_mode(self.rng.next_bipolar(), 1),
            ExciterKind::ClickTable => unreachable!(),
        };

        self.position += 1;
        if self.position >= self.length {
            self.active = false;
        }
        source * window * self.amplitude
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn reset(&mut self) {
        self.position = 0;
        self.active = false;
        self.click.reset();
        self.noise_filter.reset();
    }

    fn width_to_samples(sample_rate: f32, width_ms: f32) -> usize {
        (sample_rate * width_ms / 1_000.0).round().max(1.0) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulse_has_expected_length_and_area() {
        let mut exciter = Exciter::new(48_000.0);
        exciter.set_width_ms(1.0);
        exciter.trigger(1.0, 1);
        let mut samples = Vec::new();
        while exciter.is_active() {
            samples.push(exciter.tick());
        }
        assert_eq!(samples.len(), 48);
        let area: f32 = samples.iter().sum();
        assert!((area - 23.5).abs() < 0.01, "area={area}");
    }

    #[test]
    fn noise_burst_differs_per_seed() {
        fn render(seed: u32) -> Vec<f32> {
            let mut exciter = Exciter::new(48_000.0);
            exciter.set_kind(ExciterKind::NoiseBurst);
            exciter.trigger(1.0, seed);
            (0..48).map(|_| exciter.tick()).collect()
        }

        assert_ne!(render(1), render(2));
        assert_eq!(render(42), render(42));
    }

    #[test]
    fn exciter_is_silent_when_inactive() {
        let mut exciter = Exciter::new(48_000.0);
        assert_eq!(exciter.tick(), 0.0);
        exciter.trigger(1.0, 1);
        while exciter.is_active() {
            let _ = exciter.tick();
        }
        assert_eq!(exciter.tick(), 0.0);
    }
}
