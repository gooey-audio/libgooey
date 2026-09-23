//! Motions: one-shot, retriggerable automation of a macro's value.
//!
//! A motion ramps one macro from its start value to a target over a duration
//! (beats or milliseconds) along a curve, then applies its end mode. Many
//! motions can run at once, but only the most recently started motion owns a
//! given macro.

use super::macros::{MacroBank, MACRO_COUNT};

pub const MOTION_SLOT_COUNT: usize = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MotionCurve {
    #[default]
    Linear,
    /// Slow start, fast finish.
    EaseIn,
    /// Fast start, slow finish.
    EaseOut,
    /// Slow start and finish (smoothstep).
    SCurve,
}

impl MotionCurve {
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Linear),
            1 => Some(Self::EaseIn),
            2 => Some(Self::EaseOut),
            3 => Some(Self::SCurve),
            _ => None,
        }
    }

    pub fn as_u32(self) -> u32 {
        self as u32
    }

    /// Map linear progress (0-1) to shaped progress (0-1).
    pub fn eval(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::EaseIn => t * t,
            Self::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Self::SCurve => t * t * (3.0 - 2.0 * t),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MotionEndMode {
    /// Stay at the target.
    #[default]
    Hold,
    /// Retrace the same path back to the start over the same duration.
    Return,
    /// Jump back to the start value as soon as the target is reached.
    SnapBack,
}

impl MotionEndMode {
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Hold),
            1 => Some(Self::Return),
            2 => Some(Self::SnapBack),
            _ => None,
        }
    }

    pub fn as_u32(self) -> u32 {
        self as u32
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MotionDuration {
    /// Tempo-synced length; follows BPM changes mid-motion.
    Beats(f32),
    Millis(f32),
}

impl Default for MotionDuration {
    fn default() -> Self {
        Self::Beats(4.0)
    }
}

impl MotionDuration {
    /// Length of one leg of the motion in samples (at least one sample).
    pub fn samples(self, bpm: f32, sample_rate: f32) -> f64 {
        let seconds = match self {
            Self::Beats(beats) => beats as f64 * 60.0 / bpm.max(1.0) as f64,
            Self::Millis(ms) => ms as f64 / 1000.0,
        };
        (seconds * sample_rate as f64).max(1.0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MotionQuantize {
    #[default]
    None,
    /// Wait for the next beat while the transport runs.
    Beat,
    /// Wait for the next 4-beat bar while the transport runs.
    Bar,
}

impl MotionQuantize {
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::None),
            1 => Some(Self::Beat),
            2 => Some(Self::Bar),
            _ => None,
        }
    }

    pub fn as_u32(self) -> u32 {
        self as u32
    }

    fn grid_beats(self) -> Option<f64> {
        match self {
            Self::None => None,
            Self::Beat => Some(1.0),
            Self::Bar => Some(4.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionDefinition {
    pub macro_index: usize,
    /// Macro value (0-1) the motion ramps to.
    pub target: f32,
    /// Explicit macro value to start from, or `None` to start from wherever
    /// the macro is when the motion begins.
    pub start: Option<f32>,
    pub duration: MotionDuration,
    pub curve: MotionCurve,
    pub end_mode: MotionEndMode,
    pub quantize: MotionQuantize,
}

impl MotionDefinition {
    pub fn new(macro_index: usize, target: f32) -> Self {
        Self {
            macro_index,
            target: target.clamp(0.0, 1.0),
            start: None,
            duration: MotionDuration::default(),
            curve: MotionCurve::default(),
            end_mode: MotionEndMode::default(),
            quantize: MotionQuantize::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MotionPhase {
    #[default]
    Idle,
    /// Waiting for a quantized start.
    Pending,
    /// Ramping toward the target.
    Forward,
    /// Ramping back to the start (end mode `Return`).
    Returning,
}

impl MotionPhase {
    pub fn as_u32(self) -> u32 {
        self as u32
    }
}

/// Clock state sampled by the render loop.
#[derive(Clone, Copy, Debug)]
pub struct MotionClock {
    pub bpm: f32,
    pub sample_rate: f32,
    pub transport_running: bool,
    pub transport_beat: f64,
}

#[derive(Clone, Copy, Debug)]
struct MotionInstance {
    definition: MotionDefinition,
    phase: MotionPhase,
    from: f32,
    /// Progress through the current leg, 0-1.
    progress: f64,
    start_beat: f64,
}

impl Default for MotionInstance {
    fn default() -> Self {
        Self {
            definition: MotionDefinition::new(0, 1.0),
            phase: MotionPhase::Idle,
            from: 0.0,
            progress: 0.0,
            start_beat: 0.0,
        }
    }
}

impl MotionInstance {
    fn value_at(&self, progress: f64) -> f32 {
        let shaped = self.definition.curve.eval(progress as f32);
        self.from + (self.definition.target - self.from) * shaped
    }
}

/// Fixed pool of running motions. Allocation-free.
pub struct MotionRunner {
    instances: [MotionInstance; MOTION_SLOT_COUNT],
}

impl Default for MotionRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl MotionRunner {
    pub fn new() -> Self {
        Self {
            instances: [MotionInstance::default(); MOTION_SLOT_COUNT],
        }
    }

    /// Start (or restart) `slot`. Any other motion driving the same macro is
    /// stopped where it is.
    pub fn start(
        &mut self,
        slot: usize,
        definition: MotionDefinition,
        clock: &MotionClock,
        bank: &mut MacroBank,
    ) {
        if slot >= MOTION_SLOT_COUNT || definition.macro_index >= MACRO_COUNT {
            return;
        }
        self.stop_macro(definition.macro_index);
        let instance = &mut self.instances[slot];
        instance.definition = definition;
        instance.progress = 0.0;

        let grid = definition.quantize.grid_beats();
        match grid {
            Some(grid) if clock.transport_running => {
                let beat = clock.transport_beat;
                let boundary = (beat / grid).ceil() * grid;
                // Already on the grid (within one sample-ish): start now.
                if boundary - beat < 1e-6 {
                    Self::begin(instance, bank);
                } else {
                    instance.start_beat = boundary;
                    instance.phase = MotionPhase::Pending;
                }
            }
            _ => Self::begin(instance, bank),
        }
    }

    fn begin(instance: &mut MotionInstance, bank: &mut MacroBank) {
        let macro_index = instance.definition.macro_index;
        instance.from = instance
            .definition
            .start
            .map(|start| start.clamp(0.0, 1.0))
            .unwrap_or_else(|| bank.value(macro_index));
        instance.progress = 0.0;
        instance.phase = MotionPhase::Forward;
        bank.set_value(macro_index, instance.from);
        bank.touch(macro_index);
    }

    /// Stop `slot`, leaving its macro wherever it currently is.
    pub fn stop(&mut self, slot: usize) {
        if let Some(instance) = self.instances.get_mut(slot) {
            instance.phase = MotionPhase::Idle;
        }
    }

    pub fn stop_all(&mut self) {
        for instance in &mut self.instances {
            instance.phase = MotionPhase::Idle;
        }
    }

    /// Stop every motion driving `macro_index` (for example when the host
    /// grabs the macro by hand).
    pub fn stop_macro(&mut self, macro_index: usize) {
        for instance in &mut self.instances {
            if instance.definition.macro_index == macro_index {
                instance.phase = MotionPhase::Idle;
            }
        }
    }

    pub fn phase(&self, slot: usize) -> MotionPhase {
        self.instances
            .get(slot)
            .map(|instance| instance.phase)
            .unwrap_or_default()
    }

    /// Overall progress 0-1. With `Return`, the outbound leg covers 0-0.5 and
    /// the return leg 0.5-1.
    pub fn progress(&self, slot: usize) -> f32 {
        let Some(instance) = self.instances.get(slot) else {
            return 0.0;
        };
        let progress = instance.progress as f32;
        match (instance.phase, instance.definition.end_mode) {
            (MotionPhase::Idle | MotionPhase::Pending, _) => 0.0,
            (MotionPhase::Forward, MotionEndMode::Return) => progress * 0.5,
            (MotionPhase::Forward, _) => progress,
            (MotionPhase::Returning, _) => 0.5 + progress * 0.5,
        }
    }

    pub fn is_active(&self) -> bool {
        self.instances
            .iter()
            .any(|instance| instance.phase != MotionPhase::Idle)
    }

    /// Advance every motion by `frames` samples and write macro values.
    pub fn advance(&mut self, frames: u32, clock: &MotionClock, bank: &mut MacroBank) {
        for instance in &mut self.instances {
            match instance.phase {
                MotionPhase::Idle => continue,
                MotionPhase::Pending => {
                    if !clock.transport_running || clock.transport_beat >= instance.start_beat {
                        Self::begin(instance, bank);
                    }
                    continue;
                }
                MotionPhase::Forward | MotionPhase::Returning => {}
            }

            let macro_index = instance.definition.macro_index;
            let length = instance
                .definition
                .duration
                .samples(clock.bpm, clock.sample_rate);
            instance.progress = (instance.progress + frames as f64 / length).min(1.0);
            let finished = instance.progress >= 1.0;

            let value = match instance.phase {
                MotionPhase::Forward if finished => match instance.definition.end_mode {
                    MotionEndMode::Hold => {
                        instance.phase = MotionPhase::Idle;
                        instance.definition.target
                    }
                    MotionEndMode::Return => {
                        instance.phase = MotionPhase::Returning;
                        instance.progress = 0.0;
                        instance.definition.target
                    }
                    MotionEndMode::SnapBack => {
                        instance.phase = MotionPhase::Idle;
                        instance.from
                    }
                },
                MotionPhase::Forward => instance.value_at(instance.progress),
                MotionPhase::Returning if finished => {
                    instance.phase = MotionPhase::Idle;
                    instance.from
                }
                // Retrace the outbound path so the return mirrors it in time.
                _ => instance.value_at(1.0 - instance.progress),
            };
            bank.set_value(macro_index, value);
            bank.touch(macro_index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn clock() -> MotionClock {
        MotionClock {
            bpm: 120.0,
            sample_rate: SR,
            transport_running: false,
            transport_beat: 0.0,
        }
    }

    /// Advance in 32-frame control ticks for `seconds`.
    fn run(runner: &mut MotionRunner, bank: &mut MacroBank, clock: &MotionClock, seconds: f32) {
        let ticks = (seconds * SR / 32.0).round() as usize;
        for _ in 0..ticks {
            runner.advance(32, clock, bank);
        }
    }

    #[test]
    fn curves_hit_endpoints_and_are_monotonic() {
        for curve in [
            MotionCurve::Linear,
            MotionCurve::EaseIn,
            MotionCurve::EaseOut,
            MotionCurve::SCurve,
        ] {
            assert_eq!(curve.eval(0.0), 0.0);
            assert!((curve.eval(1.0) - 1.0).abs() < 1e-6);
            let mut previous = 0.0;
            for step in 1..=100 {
                let value = curve.eval(step as f32 / 100.0);
                assert!(value >= previous, "{curve:?} not monotonic");
                previous = value;
            }
        }
        assert!(MotionCurve::EaseIn.eval(0.5) < 0.5);
        assert!(MotionCurve::EaseOut.eval(0.5) > 0.5);
        assert!((MotionCurve::SCurve.eval(0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn duration_converts_beats_and_millis() {
        assert_eq!(MotionDuration::Beats(1.0).samples(120.0, SR), 24_000.0);
        assert_eq!(MotionDuration::Millis(250.0).samples(120.0, SR), 12_000.0);
        assert_eq!(MotionDuration::Millis(0.0).samples(120.0, SR), 1.0);
    }

    #[test]
    fn hold_ramps_linearly_and_stays_at_target() {
        let mut bank = MacroBank::new();
        let mut runner = MotionRunner::new();
        let mut def = MotionDefinition::new(0, 1.0);
        def.duration = MotionDuration::Beats(1.0);
        runner.start(0, def, &clock(), &mut bank);
        assert_eq!(runner.phase(0), MotionPhase::Forward);

        run(&mut runner, &mut bank, &clock(), 0.25);
        assert!((bank.value(0) - 0.5).abs() < 0.01, "{}", bank.value(0));
        assert!((runner.progress(0) - 0.5).abs() < 0.01);

        run(&mut runner, &mut bank, &clock(), 0.3);
        assert_eq!(runner.phase(0), MotionPhase::Idle);
        assert_eq!(bank.value(0), 1.0);
    }

    #[test]
    fn return_retraces_to_start_over_the_same_duration() {
        let mut bank = MacroBank::new();
        bank.set_value(0, 0.2);
        let mut runner = MotionRunner::new();
        let mut def = MotionDefinition::new(0, 1.0);
        def.duration = MotionDuration::Millis(100.0);
        def.end_mode = MotionEndMode::Return;
        runner.start(0, def, &clock(), &mut bank);

        run(&mut runner, &mut bank, &clock(), 0.1);
        assert_eq!(runner.phase(0), MotionPhase::Returning);
        assert_eq!(bank.value(0), 1.0);

        run(&mut runner, &mut bank, &clock(), 0.05);
        assert!((bank.value(0) - 0.6).abs() < 0.02, "{}", bank.value(0));

        run(&mut runner, &mut bank, &clock(), 0.06);
        assert_eq!(runner.phase(0), MotionPhase::Idle);
        assert!((bank.value(0) - 0.2).abs() < 1e-6);
    }

    #[test]
    fn snap_back_returns_immediately_after_reaching_target() {
        let mut bank = MacroBank::new();
        let mut runner = MotionRunner::new();
        let mut def = MotionDefinition::new(3, 0.8);
        def.duration = MotionDuration::Millis(50.0);
        def.end_mode = MotionEndMode::SnapBack;
        runner.start(1, def, &clock(), &mut bank);
        run(&mut runner, &mut bank, &clock(), 0.049);
        assert!(bank.value(3) > 0.7);
        run(&mut runner, &mut bank, &clock(), 0.01);
        assert_eq!(runner.phase(1), MotionPhase::Idle);
        assert_eq!(bank.value(3), 0.0);
    }

    #[test]
    fn explicit_start_makes_retrigger_repeatable() {
        let mut bank = MacroBank::new();
        let mut runner = MotionRunner::new();
        let mut def = MotionDefinition::new(0, 1.0);
        def.start = Some(0.0);
        def.duration = MotionDuration::Millis(10.0);
        runner.start(0, def, &clock(), &mut bank);
        run(&mut runner, &mut bank, &clock(), 0.02);
        assert_eq!(bank.value(0), 1.0);

        runner.start(0, def, &clock(), &mut bank);
        assert_eq!(bank.value(0), 0.0, "explicit start jumps back");
        run(&mut runner, &mut bank, &clock(), 0.005);
        assert!(bank.value(0) > 0.3 && bank.value(0) < 0.7);
    }

    #[test]
    fn current_start_ramps_from_live_value() {
        let mut bank = MacroBank::new();
        bank.set_value(0, 0.4);
        let mut runner = MotionRunner::new();
        let mut def = MotionDefinition::new(0, 0.0);
        def.duration = MotionDuration::Millis(100.0);
        runner.start(0, def, &clock(), &mut bank);
        run(&mut runner, &mut bank, &clock(), 0.05);
        assert!((bank.value(0) - 0.2).abs() < 0.01);
    }

    #[test]
    fn newer_motion_supersedes_older_on_same_macro() {
        let mut bank = MacroBank::new();
        let mut runner = MotionRunner::new();
        runner.start(0, MotionDefinition::new(0, 1.0), &clock(), &mut bank);
        runner.start(1, MotionDefinition::new(1, 1.0), &clock(), &mut bank);
        runner.start(2, MotionDefinition::new(0, 0.0), &clock(), &mut bank);
        assert_eq!(runner.phase(0), MotionPhase::Idle);
        assert_eq!(runner.phase(1), MotionPhase::Forward);
        assert_eq!(runner.phase(2), MotionPhase::Forward);
        runner.stop_macro(1);
        assert_eq!(runner.phase(1), MotionPhase::Idle);
    }

    #[test]
    fn quantized_start_waits_for_bar_boundary() {
        let mut bank = MacroBank::new();
        let mut runner = MotionRunner::new();
        let mut def = MotionDefinition::new(0, 1.0);
        def.quantize = MotionQuantize::Bar;
        let mut clock = clock();
        clock.transport_running = true;
        clock.transport_beat = 1.5;
        runner.start(0, def, &clock, &mut bank);
        assert_eq!(runner.phase(0), MotionPhase::Pending);

        clock.transport_beat = 3.99;
        runner.advance(32, &clock, &mut bank);
        assert_eq!(runner.phase(0), MotionPhase::Pending);

        clock.transport_beat = 4.0;
        runner.advance(32, &clock, &mut bank);
        assert_eq!(runner.phase(0), MotionPhase::Forward);
    }

    #[test]
    fn quantize_is_ignored_when_transport_is_stopped() {
        let mut bank = MacroBank::new();
        let mut runner = MotionRunner::new();
        let mut def = MotionDefinition::new(0, 1.0);
        def.quantize = MotionQuantize::Beat;
        runner.start(0, def, &clock(), &mut bank);
        assert_eq!(runner.phase(0), MotionPhase::Forward);
    }

    #[test]
    fn beat_duration_follows_tempo_changes() {
        let mut bank = MacroBank::new();
        let mut runner = MotionRunner::new();
        let mut def = MotionDefinition::new(0, 1.0);
        def.duration = MotionDuration::Beats(2.0);
        let mut clock = clock();
        runner.start(0, def, &clock, &mut bank);
        // Half the motion at 120 BPM (0.5 s), then the rest at 240 BPM (0.25 s).
        run(&mut runner, &mut bank, &clock, 0.5);
        assert!((bank.value(0) - 0.5).abs() < 0.01);
        clock.bpm = 240.0;
        run(&mut runner, &mut bank, &clock, 0.26);
        assert_eq!(runner.phase(0), MotionPhase::Idle);
    }

    #[test]
    fn stop_freezes_in_place() {
        let mut bank = MacroBank::new();
        let mut runner = MotionRunner::new();
        let mut def = MotionDefinition::new(0, 1.0);
        def.duration = MotionDuration::Millis(100.0);
        runner.start(0, def, &clock(), &mut bank);
        run(&mut runner, &mut bank, &clock(), 0.05);
        runner.stop(0);
        let frozen = bank.value(0);
        run(&mut runner, &mut bank, &clock(), 0.1);
        assert_eq!(bank.value(0), frozen);
        assert!(!runner.is_active());
    }
}
