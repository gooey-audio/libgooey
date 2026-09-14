use crate::params::{ParamId, Parameters};
use gooey::instruments::KickDrum;

pub struct KickAdapter {
    kick: KickDrum,
    sample_rate: f64,
    sample_counter: u64,
    parameters: Parameters,
}

impl KickAdapter {
    pub fn new(sample_rate: f64, parameters: Parameters) -> Self {
        let sample_rate = valid_sample_rate(sample_rate);
        let mut adapter = Self {
            kick: KickDrum::new(sample_rate as f32),
            sample_rate,
            sample_counter: 0,
            parameters,
        };
        adapter.apply_all_parameters();
        adapter
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    pub fn sample_counter(&self) -> u64 {
        self.sample_counter
    }

    pub fn parameters(&self) -> Parameters {
        self.parameters
    }

    pub fn set_sample_rate(&mut self, sample_rate: f64) -> bool {
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            return false;
        }
        if self.sample_rate == sample_rate {
            return true;
        }
        self.sample_rate = sample_rate;
        self.sample_counter = 0;
        self.kick = KickDrum::new(sample_rate as f32);
        self.apply_all_parameters();
        true
    }

    pub fn set_parameter(&mut self, id: ParamId, value: f32) {
        self.parameters.set(id, value);
        let value = self.parameters.get(id);
        match id {
            ParamId::Frequency => self.kick.set_frequency(value),
            ParamId::Decay => {
                self.kick.set_oscillator_decay(value);
                self.kick.set_amp_decay(value);
            }
            ParamId::Punch => self.kick.set_punch(value),
            ParamId::Click => self.kick.set_click(value),
            ParamId::PitchSweep => self.kick.set_pitch_envelope_amount(value),
            ParamId::Drive => self.kick.set_overdrive(value),
            ParamId::Output => self.kick.set_volume(value),
        }
    }

    pub fn replace_parameters(&mut self, parameters: Parameters) {
        self.parameters = parameters;
        self.apply_all_parameters();
    }

    pub fn trigger(&mut self, velocity: f32) {
        self.kick
            .trigger_with_velocity(self.current_time(), finite_unit(velocity));
    }

    #[inline]
    pub fn next_sample(&mut self) -> f32 {
        let sample = self.kick.tick(self.current_time());
        self.sample_counter = self.sample_counter.wrapping_add(1);
        if sample.is_finite() {
            sample
        } else {
            0.0
        }
    }

    #[inline]
    pub fn next_stereo(&mut self) -> [f32; 2] {
        let sample = self.next_sample();
        [sample, sample]
    }

    fn current_time(&self) -> f64 {
        self.sample_counter as f64 / self.sample_rate
    }

    fn apply_all_parameters(&mut self) {
        for id in ParamId::ALL {
            self.set_parameter(id, self.parameters.get(id));
        }
    }
}

fn valid_sample_rate(sample_rate: f64) -> f64 {
    if sample_rate.is_finite() && sample_rate > 0.0 {
        sample_rate
    } else {
        44_100.0
    }
}

fn finite_unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn energy(adapter: &mut KickAdapter, frames: usize) -> f32 {
        (0..frames).map(|_| adapter.next_sample().abs()).sum()
    }

    #[test]
    fn silent_before_trigger_and_finite_after_at_common_rates() {
        for rate in [44_100.0, 48_000.0, 96_000.0] {
            let mut adapter = KickAdapter::new(rate, Parameters::default());
            assert_eq!(energy(&mut adapter, 128), 0.0);
            adapter.trigger(1.0);
            let samples: Vec<_> = (0..2048).map(|_| adapter.next_sample()).collect();
            assert!(samples.iter().all(|sample| sample.is_finite()));
            assert!(samples.iter().any(|sample| sample.abs() > 1e-5));
        }
    }

    #[test]
    fn stereo_channels_are_identical() {
        let mut adapter = KickAdapter::new(48_000.0, Parameters::default());
        adapter.trigger(1.0);
        for _ in 0..1024 {
            let [left, right] = adapter.next_stereo();
            assert_eq!(left, right);
        }
    }

    #[test]
    fn velocity_changes_output_energy() {
        let mut quiet = KickAdapter::new(48_000.0, Parameters::default());
        let mut loud = KickAdapter::new(48_000.0, Parameters::default());
        quiet.trigger(0.2);
        loud.trigger(1.0);
        assert!(energy(&mut quiet, 4096) < energy(&mut loud, 4096));
    }

    #[test]
    fn parameters_influence_subsequent_hits() {
        let mut dry = KickAdapter::new(48_000.0, Parameters::default());
        let mut driven = KickAdapter::new(48_000.0, Parameters::default());
        dry.set_parameter(ParamId::Drive, 0.0);
        driven.set_parameter(ParamId::Drive, 1.0);
        dry.trigger(1.0);
        driven.trigger(1.0);
        let delta: f32 = (0..4096)
            .map(|_| (dry.next_sample() - driven.next_sample()).abs())
            .sum();
        assert!(delta > 0.01);
    }

    #[test]
    fn retrigger_starts_a_new_transient() {
        let mut adapter = KickAdapter::new(48_000.0, Parameters::default());
        adapter.trigger(1.0);
        let _ = energy(&mut adapter, 20_000);
        let tail = energy(&mut adapter, 64);
        adapter.trigger(1.0);
        let retrigger = energy(&mut adapter, 64);
        assert!(retrigger > tail);
    }

    #[test]
    fn sample_rate_change_rebuilds_and_preserves_parameters() {
        let mut adapter = KickAdapter::new(44_100.0, Parameters::default());
        adapter.set_parameter(ParamId::Frequency, 0.73);
        assert!(adapter.set_sample_rate(96_000.0));
        assert_eq!(adapter.sample_counter(), 0);
        assert_eq!(adapter.parameters().get(ParamId::Frequency), 0.73);
        assert!(!adapter.set_sample_rate(f64::NAN));
    }
}
