//! Optional monitor click ("metronome") for the FFI engine.
//!
//! A metronome is a monitoring aid, not part of the mix: it exists so someone
//! auditioning a generated loop can hear where the beats fall. It is therefore
//! summed at the very end of the render path — after the master fader, the
//! global effect chain, and the limiter — so enabling it can never change the
//! sound of the material being auditioned, and it never reaches an export.
//!
//! Timing comes from the engine's musical transport ([`crate::mixer::Mixer`]'s
//! `transport_beat`, the monotonic quarter-note counter owned by the clip
//! grid), not from the 16-step sequencer, so the click stays locked to clip
//! launches and follows host transport seeks.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::envelope::ADSRConfig;
use crate::frame::StereoFrame;
use crate::gen::{Oscillator, Waveform};
use crate::utils::SmoothedParam;

/// Click on every bar line only.
pub const METRONOME_DIVISION_BAR: u32 = 0;
/// Click on every quarter note (the default).
pub const METRONOME_DIVISION_QUARTER: u32 = 1;
/// Click on every eighth note.
pub const METRONOME_DIVISION_EIGHTH: u32 = 2;
/// Click on every sixteenth note.
pub const METRONOME_DIVISION_SIXTEENTH: u32 = 3;

/// Beats per bar assumed by the downbeat accent. The engine has no
/// time-signature concept — `LaunchQuantization::Bar` is 4.0 beats everywhere —
/// so the accent uses the same assumption. Hosts in other meters disable it.
pub const METRONOME_BEATS_PER_BAR: f64 = 4.0;

/// Default monitor level. Sits above the 0.25 default master gain without
/// being able to dominate the mix.
pub const DEFAULT_METRONOME_LEVEL: f32 = 0.35;

/// Pitch of an accented (bar-start) click.
const ACCENT_HZ: f32 = 1_600.0;
/// Pitch of every other click.
const CLICK_HZ: f32 = 800.0;
/// Un-accented clicks sound at this fraction of the configured level.
const OFFBEAT_GAIN: f32 = 0.7;

/// How often the click sounds, as a fraction of the transport's quarter-note
/// grid. Mirrors [`crate::mixer::LaunchQuantization`] so a click always lands
/// on a grid position a clip could have launched on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetronomeDivision {
    Bar,
    Quarter,
    Eighth,
    Sixteenth,
}

impl MetronomeDivision {
    pub fn from_id(value: u32) -> Option<Self> {
        match value {
            METRONOME_DIVISION_BAR => Some(Self::Bar),
            METRONOME_DIVISION_QUARTER => Some(Self::Quarter),
            METRONOME_DIVISION_EIGHTH => Some(Self::Eighth),
            METRONOME_DIVISION_SIXTEENTH => Some(Self::Sixteenth),
            _ => None,
        }
    }

    pub fn id(self) -> u32 {
        match self {
            Self::Bar => METRONOME_DIVISION_BAR,
            Self::Quarter => METRONOME_DIVISION_QUARTER,
            Self::Eighth => METRONOME_DIVISION_EIGHTH,
            Self::Sixteenth => METRONOME_DIVISION_SIXTEENTH,
        }
    }

    /// Length of one click interval in quarter notes.
    fn beats(self) -> f64 {
        match self {
            Self::Bar => METRONOME_BEATS_PER_BAR,
            Self::Quarter => 1.0,
            Self::Eighth => 0.5,
            Self::Sixteenth => 0.25,
        }
    }

    /// How many click intervals fit in one bar. At [`Self::Bar`] this is 1, so
    /// every click is a downbeat.
    fn steps_per_bar(self) -> i64 {
        ((METRONOME_BEATS_PER_BAR / self.beats()).round() as i64).max(1)
    }
}

/// A transport-locked click track.
///
/// Disabled by default. Call [`Metronome::tick`] once per rendered sample with
/// the transport state read *before* the mixer advances the beat.
pub struct Metronome {
    /// Audio thread reads, host thread writes — the same pattern as
    /// `GooeyEngine::sequencer_triggers_enabled`.
    enabled: AtomicBool,
    accent_enabled: AtomicBool,
    /// A [`MetronomeDivision`] id. Stored as an atomic so the host can change
    /// it without racing the render thread.
    division: AtomicU32,
    /// 0.0..=1.0, smoothed so a fader move never zips.
    level: SmoothedParam,
    voice: Oscillator,
    /// Grid index of the most recent click, in units of the current division,
    /// or `None` when the tracker must resync (transport parked, metronome
    /// disabled, division changed, offline bounce).
    last_index: Option<i64>,
    sample_rate: f32,
    bpm: f32,
}

impl Metronome {
    pub fn new(sample_rate: f32, bpm: f32) -> Self {
        let mut voice = Oscillator::new(sample_rate, CLICK_HZ);
        voice.waveform = Waveform::Sine;
        // 1 ms attack, 30 ms decay to zero sustain — which makes `Envelope`
        // auto-release — then a 5 ms release: a short, dry, unambiguous tick
        // that terminates itself in ~36 ms without any note-off bookkeeping.
        voice.set_adsr(ADSRConfig::new(0.001, 0.030, 0.0, 0.005));
        Self {
            enabled: AtomicBool::new(false),
            accent_enabled: AtomicBool::new(true),
            division: AtomicU32::new(METRONOME_DIVISION_QUARTER),
            level: SmoothedParam::new(DEFAULT_METRONOME_LEVEL, 0.0, 1.0, sample_rate, 10.0),
            voice,
            last_index: None,
            sample_rate,
            bpm,
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    pub fn set_accent_enabled(&self, enabled: bool) {
        self.accent_enabled.store(enabled, Ordering::Release);
    }

    pub fn accent_enabled(&self) -> bool {
        self.accent_enabled.load(Ordering::Acquire)
    }

    /// Change the click interval. Resyncs the grid tracker so the new division
    /// takes effect from the next boundary rather than inheriting a stale
    /// index measured in the old units.
    pub fn set_division(&mut self, division: MetronomeDivision) {
        self.division.store(division.id(), Ordering::Release);
        self.last_index = None;
    }

    pub fn division(&self) -> MetronomeDivision {
        MetronomeDivision::from_id(self.division.load(Ordering::Acquire))
            .unwrap_or(MetronomeDivision::Quarter)
    }

    /// Set the click level. [`SmoothedParam`] clamps to `0.0..=1.0`.
    pub fn set_level(&mut self, level: f32) {
        self.level.set_target(level);
    }

    /// The most recently set level target, not the in-flight smoothed value,
    /// so host set→get round-trips exactly.
    pub fn level(&self) -> f32 {
        self.level.target()
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        self.bpm = bpm;
    }

    /// Silence the click and clear the grid tracker.
    pub fn reset(&mut self) {
        self.voice.envelope.is_active = false;
        self.last_index = None;
        self.level.snap();
    }

    /// Render one sample of click.
    ///
    /// `running` and `transport_beat` must be read from the mixer *before* it
    /// ticks, so the click lands on the same sample as a clip launch scheduled
    /// at the same grid position. `current_time` is the engine's absolute
    /// sample clock, which drives the voice's envelope and phase.
    pub fn tick(&mut self, running: bool, transport_beat: f64, current_time: f64) -> StereoFrame {
        // Always advance the smoother so a level change made while the click is
        // disabled or the transport is parked has settled before the next click.
        let level = self.level.tick();

        if !self.is_enabled() {
            self.voice.envelope.is_active = false;
            self.last_index = None;
            return StereoFrame::default();
        }

        if !running {
            // A parked transport schedules nothing, but a click already
            // sounding finishes its decay so stopping never pops.
            self.last_index = None;
        } else if let Some(index) = self.due_index(transport_beat) {
            self.fire(index, current_time);
        }

        if !self.voice.envelope.is_active {
            return StereoFrame::default();
        }
        StereoFrame::mono(self.voice.tick(current_time) * level)
    }

    fn beats_per_sample(&self) -> f64 {
        self.bpm.max(0.0) as f64 / (60.0 * self.sample_rate.max(1.0) as f64)
    }

    /// The grid index whose boundary this sample lands on, if any.
    ///
    /// The render clock accumulates f64 increments and lands on e.g.
    /// 3.9999999999999996 rather than 4.0, so a boundary counts as reached once
    /// the transport is within half a sample of it — the same half-sample
    /// tolerance the clip grid uses to fire launches. `last_index` makes the
    /// fire edge-triggered so the tolerance window can never double-click.
    ///
    /// Seek behavior falls out of this for free: seeking onto a boundary clicks
    /// immediately, seeking mid-interval waits for the next boundary, and
    /// seeking backwards re-clicks. There is no phase accumulator to drift or
    /// resync. At `bpm == 0` the transport is frozen and nothing fires.
    fn due_index(&mut self, transport_beat: f64) -> Option<i64> {
        let interval = self.division().beats();
        let steps_per_sample = self.beats_per_sample() / interval;
        let tolerance = steps_per_sample * 0.5 + 1.0e-12;
        let shifted = transport_beat / interval + tolerance;
        let boundary = shifted.floor();
        if shifted - boundary > steps_per_sample {
            return None; // mid-interval
        }
        let index = boundary as i64;
        if self.last_index == Some(index) {
            return None; // already clicked this boundary
        }
        self.last_index = Some(index);
        Some(index)
    }

    fn fire(&mut self, index: i64, current_time: f64) {
        let accent =
            self.accent_enabled() && index.rem_euclid(self.division().steps_per_bar()) == 0;
        self.voice.frequency_hz = if accent { ACCENT_HZ } else { CLICK_HZ };
        self.voice
            .set_volume(if accent { 1.0 } else { OFFBEAT_GAIN });
        self.voice.trigger(current_time);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 0.02 beats/sample at 120 BPM: exactly 50 samples per quarter note, which
    // keeps the expected click positions exact integers.
    const SR: f32 = 100.0;
    const BPM: f32 = 120.0;
    const SAMPLES_PER_BEAT: usize = 50;

    fn enabled_metronome() -> Metronome {
        let metronome = Metronome::new(SR, BPM);
        metronome.set_enabled(true);
        metronome
    }

    fn beats_per_sample() -> f64 {
        BPM as f64 / (60.0 * SR as f64)
    }

    /// Run `samples` frames of a free-running transport starting at
    /// `start_beat`, calling `observe(frame_index, metronome)` on each frame a
    /// click begins.
    ///
    /// A click is detected as the envelope's inactive→active edge rather than a
    /// change in `trigger_time`, because the very first click of a run fires at
    /// `current_time == 0.0`, which is also the envelope's initial
    /// `trigger_time`. The ~36 ms voice is always finished well before the next
    /// boundary at these tempos, so the edge is never missed.
    fn drive(
        metronome: &mut Metronome,
        start_beat: f64,
        samples: usize,
        mut observe: impl FnMut(usize, &Metronome),
    ) {
        for index in 0..samples {
            let was_active = metronome.voice.envelope.is_active;
            let beat = start_beat + index as f64 * beats_per_sample();
            metronome.tick(true, beat, index as f64 / SR as f64);
            if !was_active && metronome.voice.envelope.is_active {
                observe(index, metronome);
            }
        }
    }

    /// The frame indices at which a click began.
    fn onsets(metronome: &mut Metronome, start_beat: f64, samples: usize) -> Vec<usize> {
        let mut fired = Vec::new();
        drive(metronome, start_beat, samples, |index, _| fired.push(index));
        fired
    }

    /// The voice volume used by each click, which is how the accent is applied.
    fn click_volumes(metronome: &mut Metronome, start_beat: f64, samples: usize) -> Vec<f32> {
        let mut volumes = Vec::new();
        drive(metronome, start_beat, samples, |_, m| {
            volumes.push(m.voice.volume)
        });
        volumes
    }

    #[test]
    fn disabled_by_default_and_silent() {
        let mut metronome = Metronome::new(SR, BPM);
        assert!(!metronome.is_enabled());
        for index in 0..200 {
            let frame = metronome.tick(
                true,
                index as f64 * beats_per_sample(),
                index as f64 / SR as f64,
            );
            assert_eq!(frame, StereoFrame::default());
        }
    }

    #[test]
    fn fires_once_per_beat() {
        let mut metronome = enabled_metronome();
        assert_eq!(
            onsets(&mut metronome, 0.0, 200),
            vec![
                0,
                SAMPLES_PER_BEAT,
                SAMPLES_PER_BEAT * 2,
                SAMPLES_PER_BEAT * 3
            ]
        );
    }

    #[test]
    fn division_controls_click_rate() {
        let mut metronome = enabled_metronome();

        metronome.set_division(MetronomeDivision::Eighth);
        assert_eq!(onsets(&mut metronome, 0.0, 100).len(), 4);

        metronome.set_division(MetronomeDivision::Sixteenth);
        assert_eq!(onsets(&mut metronome, 0.0, 100).len(), 8);

        // One bar is 4 beats = 200 samples here, so 400 samples is 2 bar lines.
        metronome.set_division(MetronomeDivision::Bar);
        assert_eq!(onsets(&mut metronome, 0.0, 400), vec![0, 200]);
    }

    #[test]
    fn tolerance_window_never_double_fires() {
        let mut metronome = enabled_metronome();
        // Straddle a boundary with the f64 residue the render clock produces.
        metronome.tick(true, 3.999_999_999_999_999_6, 0.0);
        let after_first = metronome.voice.envelope.trigger_time;
        metronome.tick(true, 4.000_000_000_000_000_4, 0.01);
        assert_eq!(metronome.voice.envelope.trigger_time, after_first);
        assert_eq!(metronome.last_index, Some(4));
    }

    #[test]
    fn stopped_transport_schedules_nothing() {
        let mut metronome = enabled_metronome();
        for index in 0..200 {
            metronome.tick(false, 0.0, index as f64 / SR as f64);
        }
        assert!(!metronome.voice.envelope.is_active);
        // Starting again from a boundary clicks immediately.
        assert_eq!(onsets(&mut metronome, 0.0, 1), vec![0]);
    }

    #[test]
    fn mid_beat_seek_waits_for_the_next_boundary() {
        let mut metronome = enabled_metronome();
        assert!(onsets(&mut metronome, 2.6, 1).is_empty());
        assert_eq!(onsets(&mut metronome, 3.0, 1), vec![0]);
    }

    #[test]
    fn accent_only_on_bar_starts() {
        let mut metronome = enabled_metronome();
        assert_eq!(
            click_volumes(&mut metronome, 0.0, 250),
            vec![1.0, OFFBEAT_GAIN, OFFBEAT_GAIN, OFFBEAT_GAIN, 1.0]
        );

        metronome.set_accent_enabled(false);
        let unaccented = click_volumes(&mut metronome, 8.0, 250);
        assert_eq!(unaccented.len(), 5);
        assert!(unaccented.iter().all(|v| *v == OFFBEAT_GAIN));
    }

    #[test]
    fn reset_clears_the_tracker_and_the_voice() {
        let mut metronome = enabled_metronome();
        onsets(&mut metronome, 0.0, 1);
        assert!(metronome.voice.envelope.is_active);
        metronome.reset();
        assert!(!metronome.voice.envelope.is_active);
        assert_eq!(metronome.last_index, None);
    }

    #[test]
    fn level_round_trips_and_clamps() {
        let mut metronome = Metronome::new(SR, BPM);
        assert_eq!(metronome.level(), DEFAULT_METRONOME_LEVEL);
        metronome.set_level(0.5);
        assert_eq!(metronome.level(), 0.5);
        metronome.set_level(5.0);
        assert_eq!(metronome.level(), 1.0);
    }
}
