//! Macro LFOs: continuous, tempo-synced cycling of a macro's value.
//!
//! Each macro has one LFO. A running LFO sweeps its macro through 0-1 along a
//! waveform, so the macro's `from` → `to` mappings cycle without host updates.
//! Phase is free-running: it keeps advancing while the transport is stopped,
//! and a tempo change alters only the speed, never the current phase. Every
//! waveform starts at 0 so a (re)started LFO begins at the mappings' `from`.

use super::macros::{MacroBank, MACRO_COUNT};
use super::motion::MotionClock;
use crate::engine::lfo::MusicalDivision;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MacroLfoShape {
    /// Raised cosine: 0 → 1 → 0.
    #[default]
    Sine,
    /// Linear 0 → 1 → 0.
    Triangle,
    /// Linear ramp 0 → 1, then jump back to 0.
    Saw,
    /// 0 for the first half of the cycle, 1 for the second.
    Square,
}

impl MacroLfoShape {
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Sine),
            1 => Some(Self::Triangle),
            2 => Some(Self::Saw),
            3 => Some(Self::Square),
            _ => None,
        }
    }

    pub fn as_u32(self) -> u32 {
        self as u32
    }

    /// Output (0-1) at a cycle phase (0-1). Every shape is 0 at phase 0.
    pub fn eval(self, phase: f64) -> f32 {
        let phase = phase.rem_euclid(1.0);
        let value = match self {
            Self::Sine => (1.0 - (std::f64::consts::TAU * phase).cos()) * 0.5,
            Self::Triangle => 1.0 - (1.0 - 2.0 * phase).abs(),
            Self::Saw => phase,
            Self::Square => {
                if phase < 0.5 {
                    0.0
                } else {
                    1.0
                }
            }
        };
        (value as f32).clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MacroLfoSettings {
    pub shape: MacroLfoShape,
    /// Length of one cycle at the engine BPM.
    pub division: MusicalDivision,
}

impl Default for MacroLfoSettings {
    fn default() -> Self {
        Self {
            shape: MacroLfoShape::Sine,
            division: MusicalDivision::OneBar,
        }
    }
}

impl MacroLfoSettings {
    /// Length of one cycle in samples (at least one sample).
    pub fn cycle_samples(&self, bpm: f32, sample_rate: f32) -> f64 {
        let seconds = self.division.beats() as f64 * 60.0 / bpm.max(1.0) as f64;
        (seconds * sample_rate as f64).max(1.0)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct MacroLfo {
    settings: MacroLfoSettings,
    running: bool,
    phase: f64,
    value: f32,
}

/// One LFO per macro. Allocation-free.
pub struct MacroLfoRunner {
    lfos: [MacroLfo; MACRO_COUNT],
}

impl Default for MacroLfoRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl MacroLfoRunner {
    pub fn new() -> Self {
        Self {
            lfos: [MacroLfo::default(); MACRO_COUNT],
        }
    }

    /// Start (or restart) the LFO of macro `index` from phase 0, which puts
    /// the macro at 0. The caller stops any motion on the macro.
    pub fn start(&mut self, index: usize, settings: MacroLfoSettings, bank: &mut MacroBank) {
        let Some(lfo) = self.lfos.get_mut(index) else {
            return;
        };
        lfo.settings = settings;
        lfo.running = true;
        Self::restart(index, lfo, bank);
    }

    fn restart(index: usize, lfo: &mut MacroLfo, bank: &mut MacroBank) {
        lfo.phase = 0.0;
        lfo.value = lfo.settings.shape.eval(0.0);
        bank.set_value(index, lfo.value);
        bank.touch(index);
    }

    /// Change shape and rate. A running LFO keeps its phase.
    pub fn configure(&mut self, index: usize, settings: MacroLfoSettings) {
        if let Some(lfo) = self.lfos.get_mut(index) {
            lfo.settings = settings;
        }
    }

    /// Stop a running LFO and hold its macro at `hold` (0-1, clamped).
    pub fn stop(&mut self, index: usize, hold: f32, bank: &mut MacroBank) {
        let Some(lfo) = self.lfos.get_mut(index) else {
            return;
        };
        if !lfo.running {
            return;
        }
        lfo.running = false;
        if hold.is_finite() {
            lfo.value = hold.clamp(0.0, 1.0);
            bank.set_value(index, lfo.value);
        }
    }

    /// Stop the LFO without touching its macro, because something else (a
    /// motion or a manual value) has taken the macro over.
    pub fn stop_macro(&mut self, index: usize) {
        if let Some(lfo) = self.lfos.get_mut(index) {
            lfo.running = false;
        }
    }

    /// Restart a running LFO's cycle at phase 0. Does nothing when stopped.
    pub fn reset_phase(&mut self, index: usize, bank: &mut MacroBank) {
        if let Some(lfo) = self.lfos.get_mut(index) {
            if lfo.running {
                Self::restart(index, lfo, bank);
            }
        }
    }

    pub fn is_running(&self, index: usize) -> bool {
        self.lfos.get(index).is_some_and(|lfo| lfo.running)
    }

    pub fn phase(&self, index: usize) -> f32 {
        self.lfos.get(index).map_or(0.0, |lfo| lfo.phase as f32)
    }

    /// Latest output (0-1). After a stop, the held value.
    pub fn value(&self, index: usize) -> f32 {
        self.lfos.get(index).map_or(0.0, |lfo| lfo.value)
    }

    /// Advance every running LFO by `frames` samples and write macro values.
    /// Running LFOs reassert their macros like running motions do.
    pub fn advance(&mut self, frames: u32, clock: &MotionClock, bank: &mut MacroBank) {
        for (index, lfo) in self.lfos.iter_mut().enumerate() {
            if !lfo.running {
                continue;
            }
            let cycle = lfo.settings.cycle_samples(clock.bpm, clock.sample_rate);
            lfo.phase = (lfo.phase + frames as f64 / cycle).rem_euclid(1.0);
            lfo.value = lfo.settings.shape.eval(lfo.phase);
            bank.set_value(index, lfo.value);
            bank.touch(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;
    const SHAPES: [MacroLfoShape; 4] = [
        MacroLfoShape::Sine,
        MacroLfoShape::Triangle,
        MacroLfoShape::Saw,
        MacroLfoShape::Square,
    ];

    fn clock(bpm: f32) -> MotionClock {
        MotionClock {
            bpm,
            sample_rate: SR,
            transport_running: false,
            transport_beat: 0.0,
            transport_generation: 0,
        }
    }

    fn settings(shape: MacroLfoShape, division: MusicalDivision) -> MacroLfoSettings {
        MacroLfoSettings { shape, division }
    }

    /// Advance in 32-frame control ticks for `seconds`.
    fn run(runner: &mut MacroLfoRunner, bank: &mut MacroBank, clock: &MotionClock, seconds: f64) {
        let ticks = (seconds * SR as f64 / 32.0).round() as usize;
        for _ in 0..ticks {
            runner.advance(32, clock, bank);
        }
    }

    #[test]
    fn waveforms_start_at_zero_and_hit_expected_points() {
        let expected = [
            (MacroLfoShape::Sine, [0.0, 0.5, 1.0, 0.5]),
            (MacroLfoShape::Triangle, [0.0, 0.5, 1.0, 0.5]),
            (MacroLfoShape::Saw, [0.0, 0.25, 0.5, 0.75]),
            (MacroLfoShape::Square, [0.0, 0.0, 1.0, 1.0]),
        ];
        for (shape, values) in expected {
            for (quarter, value) in values.into_iter().enumerate() {
                let actual = shape.eval(quarter as f64 * 0.25);
                assert!(
                    (actual - value).abs() < 1e-6,
                    "{shape:?} {quarter}: {actual}"
                );
            }
        }
        for shape in SHAPES {
            for step in 0..=1000 {
                let value = shape.eval(step as f64 / 1000.0);
                assert!((0.0..=1.0).contains(&value), "{shape:?} out of range");
            }
            assert_eq!(shape.eval(1.0), 0.0, "{shape:?} wraps to its start");
        }
    }

    #[test]
    fn shape_constants_round_trip() {
        for shape in SHAPES {
            assert_eq!(MacroLfoShape::from_u32(shape.as_u32()), Some(shape));
        }
        assert_eq!(MacroLfoShape::from_u32(4), None);
    }

    #[test]
    fn cycle_length_follows_division_at_tempo() {
        let cases = [
            (MusicalDivision::Sixteenth, 0.125),
            (MusicalDivision::Eighth, 0.25),
            (MusicalDivision::Quarter, 0.5),
            (MusicalDivision::Half, 1.0),
            (MusicalDivision::OneBar, 2.0),
            (MusicalDivision::TwoBars, 4.0),
            (MusicalDivision::FourBars, 8.0),
        ];
        for (division, seconds) in cases {
            let lfo = settings(MacroLfoShape::Saw, division);
            assert_eq!(lfo.cycle_samples(120.0, SR), seconds * SR as f64);
        }
    }

    #[test]
    fn start_puts_macro_at_zero_then_cycles() {
        let mut bank = MacroBank::new();
        bank.set_value(0, 0.7);
        let mut runner = MacroLfoRunner::new();
        let saw = settings(MacroLfoShape::Saw, MusicalDivision::Quarter);
        runner.start(0, saw, &mut bank);
        assert_eq!(bank.value(0), 0.0);
        assert!(runner.is_running(0));

        run(&mut runner, &mut bank, &clock(120.0), 0.25);
        assert!((bank.value(0) - 0.5).abs() < 1e-3, "{}", bank.value(0));
        // A whole cycle later it is back at the same point.
        run(&mut runner, &mut bank, &clock(120.0), 0.5);
        assert!((bank.value(0) - 0.5).abs() < 1e-3, "{}", bank.value(0));
    }

    #[test]
    fn tempo_change_keeps_phase_and_changes_speed() {
        let mut bank = MacroBank::new();
        let mut runner = MacroLfoRunner::new();
        let saw = settings(MacroLfoShape::Saw, MusicalDivision::OneBar);
        runner.start(0, saw, &mut bank);
        run(&mut runner, &mut bank, &clock(120.0), 1.0);
        let before = runner.phase(0);
        assert!((before - 0.5).abs() < 1e-3);

        // Doubling the tempo continues from the same phase, twice as fast.
        runner.advance(0, &clock(240.0), &mut bank);
        assert_eq!(runner.phase(0), before);
        run(&mut runner, &mut bank, &clock(240.0), 0.25);
        assert!((runner.phase(0) - 0.75).abs() < 1e-3, "{}", runner.phase(0));
    }

    #[test]
    fn retrigger_and_reset_restart_the_cycle() {
        let mut bank = MacroBank::new();
        let mut runner = MacroLfoRunner::new();
        let tri = settings(MacroLfoShape::Triangle, MusicalDivision::Quarter);
        runner.start(2, tri, &mut bank);
        run(&mut runner, &mut bank, &clock(120.0), 0.2);
        assert!(bank.value(2) > 0.5);

        runner.start(2, tri, &mut bank);
        assert_eq!(runner.phase(2), 0.0);
        assert_eq!(bank.value(2), 0.0);

        run(&mut runner, &mut bank, &clock(120.0), 0.1);
        runner.reset_phase(2, &mut bank);
        assert_eq!(runner.phase(2), 0.0);
        assert_eq!(bank.value(2), 0.0);
        assert!(runner.is_running(2));
    }

    #[test]
    fn reset_does_nothing_while_stopped() {
        let mut bank = MacroBank::new();
        let mut runner = MacroLfoRunner::new();
        runner.start(0, MacroLfoSettings::default(), &mut bank);
        run(&mut runner, &mut bank, &clock(120.0), 0.5);
        runner.stop(0, 0.4, &mut bank);
        runner.reset_phase(0, &mut bank);
        assert!(!runner.is_running(0));
        assert_eq!(bank.value(0), 0.4);
    }

    #[test]
    fn stop_holds_the_given_value() {
        let mut bank = MacroBank::new();
        let mut runner = MacroLfoRunner::new();
        runner.start(1, MacroLfoSettings::default(), &mut bank);
        run(&mut runner, &mut bank, &clock(120.0), 0.3);
        runner.stop(1, 0.35, &mut bank);
        assert_eq!(bank.value(1), 0.35);
        assert_eq!(runner.value(1), 0.35);
        run(&mut runner, &mut bank, &clock(120.0), 1.0);
        assert_eq!(bank.value(1), 0.35);

        // Stopping an LFO that is not running leaves the macro alone.
        bank.set_value(1, 0.9);
        runner.stop(1, 0.1, &mut bank);
        assert_eq!(bank.value(1), 0.9);
    }

    #[test]
    fn override_stop_leaves_the_macro_to_its_new_owner() {
        let mut bank = MacroBank::new();
        let mut runner = MacroLfoRunner::new();
        runner.start(0, MacroLfoSettings::default(), &mut bank);
        run(&mut runner, &mut bank, &clock(120.0), 0.3);
        runner.stop_macro(0);
        bank.set_value(0, 0.8);
        run(&mut runner, &mut bank, &clock(120.0), 0.3);
        assert_eq!(bank.value(0), 0.8);
    }

    #[test]
    fn configure_changes_rate_and_shape_without_losing_phase() {
        let mut bank = MacroBank::new();
        let mut runner = MacroLfoRunner::new();
        runner.start(
            0,
            settings(MacroLfoShape::Saw, MusicalDivision::OneBar),
            &mut bank,
        );
        run(&mut runner, &mut bank, &clock(120.0), 0.5);
        let phase = runner.phase(0);
        runner.configure(0, settings(MacroLfoShape::Square, MusicalDivision::Quarter));
        assert_eq!(runner.phase(0), phase);
        // A quarter note cycle now: 0.25 s moves half a cycle.
        run(&mut runner, &mut bank, &clock(120.0), 0.25);
        assert!((runner.phase(0) - (phase + 0.5)).abs() < 1e-3);
        assert_eq!(bank.value(0), 1.0);
    }

    #[test]
    fn macros_cycle_independently() {
        let mut bank = MacroBank::new();
        let mut runner = MacroLfoRunner::new();
        runner.start(
            0,
            settings(MacroLfoShape::Saw, MusicalDivision::Half),
            &mut bank,
        );
        runner.start(
            5,
            settings(MacroLfoShape::Saw, MusicalDivision::Quarter),
            &mut bank,
        );
        run(&mut runner, &mut bank, &clock(120.0), 0.125);
        assert!((bank.value(0) - 0.125).abs() < 1e-3);
        assert!((bank.value(5) - 0.25).abs() < 1e-3);
        assert_eq!(bank.value(1), 0.0);
    }
}
