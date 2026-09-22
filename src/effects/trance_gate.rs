//! Transport-synchronized rhythmic amplitude gating.
//!
//! The processor is deliberately independent of an instrument or mixer. A
//! caller supplies the transport beat, BPM, and running state for each sample,
//! allowing the same gate to be placed at any source boundary without owning a
//! second musical clock.

use crate::frame::StereoFrame;

/// Number of curated patterns, including [`TranceGatePattern::Off`].
pub const TRANCE_GATE_PATTERN_COUNT: u32 = 6;

/// Curated one-bar patterns on a sixteen-step (sixteenth-note) grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum TranceGatePattern {
    Off = 0,
    StraightEighths = 1,
    OffbeatEighths = 2,
    Chopper = 3,
    Trance = 4,
    Syncopated = 5,
}

impl TranceGatePattern {
    pub const fn from_id(id: u32) -> Option<Self> {
        match id {
            0 => Some(Self::Off),
            1 => Some(Self::StraightEighths),
            2 => Some(Self::OffbeatEighths),
            3 => Some(Self::Chopper),
            4 => Some(Self::Trance),
            5 => Some(Self::Syncopated),
            _ => None,
        }
    }

    pub const fn id(self) -> u32 {
        self as u32
    }

    /// A set bit means the corresponding step is open. Step zero is the most
    /// significant bit, matching the left-to-right strings in the documentation.
    pub const fn mask(self) -> u16 {
        match self {
            Self::Off => 0xffff,
            Self::StraightEighths => 0b1010_1010_1010_1010,
            Self::OffbeatEighths => 0b0101_0101_0101_0101,
            Self::Chopper => 0b1100_1100_1100_1100,
            Self::Trance => 0b1110_1110_1110_1110,
            Self::Syncopated => 0b1011_0100_1011_0100,
        }
    }

    pub const fn is_open(self, step: usize) -> bool {
        let bit = 15 - (step % 16);
        self.mask() & (1 << bit) != 0
    }
}

/// Normalized gate controls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TranceGateConfig {
    pub pattern: TranceGatePattern,
    /// `0.0` is transparent and `1.0` fully silences closed steps.
    pub depth: f32,
    /// Edge-ramp length from zero to 45% of one sixteenth-note step.
    pub smoothing: f32,
}

impl TranceGateConfig {
    pub const fn off() -> Self {
        Self {
            pattern: TranceGatePattern::Off,
            depth: 1.0,
            smoothing: 0.1,
        }
    }

    pub fn validated(mut self) -> Option<Self> {
        if !self.depth.is_finite() || !self.smoothing.is_finite() {
            return None;
        }
        self.depth = self.depth.clamp(0.0, 1.0);
        self.smoothing = self.smoothing.clamp(0.0, 1.0);
        Some(self)
    }
}

impl Default for TranceGateConfig {
    fn default() -> Self {
        Self::off()
    }
}

/// Stateful linear-ramp gate driven by an external musical transport.
pub struct TranceGate {
    sample_rate: f32,
    config: TranceGateConfig,
    current_gain: f32,
    target_gain: f32,
    ramp_step: f32,
    ramp_samples_remaining: u32,
}

impl TranceGate {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = if sample_rate.is_finite() && sample_rate > 0.0 {
            sample_rate
        } else {
            44_100.0
        };
        Self {
            sample_rate,
            config: TranceGateConfig::default(),
            current_gain: 1.0,
            target_gain: 1.0,
            ramp_step: 0.0,
            ramp_samples_remaining: 0,
        }
    }

    pub fn config(&self) -> TranceGateConfig {
        self.config
    }

    /// Install a validated config without resetting the current gain. Any
    /// audible change is ramped the next time a sample is processed.
    pub fn set_config(&mut self, config: TranceGateConfig) -> bool {
        let Some(config) = config.validated() else {
            return false;
        };
        self.config = config;
        true
    }

    pub fn current_gain(&self) -> f32 {
        self.current_gain
    }

    fn step_at_beat(transport_beat: f64) -> usize {
        if !transport_beat.is_finite() {
            return 0;
        }
        (transport_beat * 4.0).floor().rem_euclid(16.0) as usize
    }

    fn desired_gain(&self, transport_beat: f64, transport_running: bool) -> f32 {
        if !transport_running
            || self.config.pattern == TranceGatePattern::Off
            || self.config.depth <= 0.0
        {
            return 1.0;
        }
        if self
            .config
            .pattern
            .is_open(Self::step_at_beat(transport_beat))
        {
            1.0
        } else {
            1.0 - self.config.depth
        }
    }

    fn ramp_length_samples(&self, bpm: f32) -> u32 {
        if self.config.smoothing <= 0.0 || !bpm.is_finite() || bpm <= 0.0 {
            return 0;
        }
        let sixteenth_samples = self.sample_rate * 60.0 / (bpm * 4.0);
        (sixteenth_samples * 0.45 * self.config.smoothing)
            .round()
            .max(1.0) as u32
    }

    fn tick_gain(&mut self, transport_beat: f64, bpm: f32, transport_running: bool) -> f32 {
        let desired = self.desired_gain(transport_beat, transport_running);
        if (desired - self.target_gain).abs() > f32::EPSILON {
            self.target_gain = desired;
            self.ramp_samples_remaining = self.ramp_length_samples(bpm);
            if self.ramp_samples_remaining == 0 {
                self.current_gain = desired;
                self.ramp_step = 0.0;
            } else {
                self.ramp_step = (desired - self.current_gain) / self.ramp_samples_remaining as f32;
            }
        }

        if self.ramp_samples_remaining > 0 {
            self.current_gain += self.ramp_step;
            self.ramp_samples_remaining -= 1;
            if self.ramp_samples_remaining == 0 {
                self.current_gain = self.target_gain;
                self.ramp_step = 0.0;
            }
        }
        self.current_gain
    }

    pub fn process(
        &mut self,
        input: f32,
        transport_beat: f64,
        bpm: f32,
        transport_running: bool,
    ) -> f32 {
        input * self.tick_gain(transport_beat, bpm, transport_running)
    }

    pub fn process_stereo(
        &mut self,
        input: StereoFrame,
        transport_beat: f64,
        bpm: f32,
        transport_running: bool,
    ) -> StereoFrame {
        input.scaled(self.tick_gain(transport_beat, bpm, transport_running))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn config(pattern: TranceGatePattern, depth: f32, smoothing: f32) -> TranceGateConfig {
        TranceGateConfig {
            pattern,
            depth,
            smoothing,
        }
    }

    #[test]
    fn curated_masks_are_stable_and_wrap_after_one_bar() {
        let expected = [
            (TranceGatePattern::Off, 0xffff),
            (TranceGatePattern::StraightEighths, 0xaaaa),
            (TranceGatePattern::OffbeatEighths, 0x5555),
            (TranceGatePattern::Chopper, 0xcccc),
            (TranceGatePattern::Trance, 0xeeee),
            (TranceGatePattern::Syncopated, 0xb4b4),
        ];
        for (pattern, mask) in expected {
            assert_eq!(pattern.mask(), mask);
            assert_eq!(TranceGatePattern::from_id(pattern.id()), Some(pattern));
        }
        assert_eq!(TranceGate::step_at_beat(0.0), 0);
        assert_eq!(TranceGate::step_at_beat(3.99), 15);
        assert_eq!(TranceGate::step_at_beat(4.0), 0);
        assert_eq!(TranceGate::step_at_beat(5.25), 5);
        assert_eq!(TranceGate::step_at_beat(-0.25), 15);
    }

    #[test]
    fn off_zero_depth_and_stopped_transport_are_transparent() {
        let mut gate = TranceGate::new(SR);
        for gate_config in [
            config(TranceGatePattern::Off, 1.0, 0.0),
            config(TranceGatePattern::StraightEighths, 0.0, 0.0),
        ] {
            assert!(gate.set_config(gate_config));
            assert_eq!(gate.process(0.75, 0.25, 120.0, true), 0.75);
        }
        assert!(gate.set_config(config(TranceGatePattern::StraightEighths, 1.0, 0.0)));
        assert_eq!(gate.process(0.75, 0.25, 120.0, false), 0.75);
    }

    #[test]
    fn depth_controls_the_closed_step_floor() {
        let mut gate = TranceGate::new(SR);
        assert!(gate.set_config(config(TranceGatePattern::StraightEighths, 0.75, 0.0)));
        assert_eq!(gate.process(1.0, 0.25, 120.0, true), 0.25);
        assert_eq!(gate.process(1.0, 0.5, 120.0, true), 1.0);
    }

    #[test]
    fn stereo_processing_preserves_channel_ratio() {
        let mut gate = TranceGate::new(SR);
        assert!(gate.set_config(config(TranceGatePattern::StraightEighths, 0.5, 0.0)));
        let result = gate.process_stereo(StereoFrame { l: 0.8, r: -0.2 }, 0.25, 120.0, true);
        assert_eq!(result, StereoFrame { l: 0.4, r: -0.1 });
    }

    #[test]
    fn smoothing_is_the_same_fraction_of_a_step_at_each_tempo() {
        for bpm in [60.0, 120.0, 180.0] {
            let mut gate = TranceGate::new(SR);
            assert!(gate.set_config(config(TranceGatePattern::StraightEighths, 1.0, 0.5)));
            let expected = (SR * 60.0 / (bpm * 4.0) * 0.45 * 0.5).round() as usize;
            for sample in 0..expected {
                let _ = gate.process(1.0, 0.25, bpm, true);
                if sample + 1 < expected {
                    assert!(gate.current_gain() > 0.0);
                }
            }
            assert_eq!(gate.current_gain(), 0.0);
        }
    }

    #[test]
    fn transport_and_pattern_changes_ramp_from_the_current_gain() {
        let mut gate = TranceGate::new(SR);
        assert!(gate.set_config(config(TranceGatePattern::StraightEighths, 1.0, 0.2)));
        let first = gate.process(1.0, 0.25, 120.0, true);
        assert!(first > 0.0 && first < 1.0);
        let before_stop = gate.current_gain();
        let after_stop = gate.process(1.0, 0.25, 120.0, false);
        assert!(after_stop > before_stop && after_stop < 1.0);

        assert!(gate.set_config(config(TranceGatePattern::OffbeatEighths, 1.0, 0.2)));
        let _ = gate.process(1.0, 0.0, 120.0, true);
        assert!(gate.current_gain() < 1.0);
    }

    #[test]
    fn transport_seek_selects_the_destination_step_without_free_running_phase() {
        let mut gate = TranceGate::new(SR);
        assert!(gate.set_config(config(TranceGatePattern::StraightEighths, 1.0, 0.0)));
        assert_eq!(gate.process(1.0, 0.25, 120.0, true), 0.0);
        assert_eq!(gate.process(1.0, 12.5, 120.0, true), 1.0);
        assert_eq!(gate.process(1.0, 20.25, 120.0, true), 0.0);
    }

    #[test]
    fn invalid_config_is_rejected_and_finite_values_are_clamped() {
        let mut gate = TranceGate::new(SR);
        assert!(!gate.set_config(config(TranceGatePattern::Trance, f32::NAN, 0.5)));
        assert_eq!(gate.config(), TranceGateConfig::default());
        assert!(gate.set_config(config(TranceGatePattern::Trance, 2.0, -1.0)));
        assert_eq!(gate.config().depth, 1.0);
        assert_eq!(gate.config().smoothing, 0.0);
    }
}
