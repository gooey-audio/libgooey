//! Performance clip recording and sample-accurate replay for live instruments.
//!
//! Stage 1 focuses on chord pad performances: timed pad-parameter events in a
//! looping clip locked to the engine transport (same beat clock as drum sequencers).

/// Pulses per quarter note. One sixteenth-note step is `TICKS_PER_QUARTER / 4`.
pub const TICKS_PER_QUARTER: u32 = 96;

/// Default clip length in sixteenth-note steps (one 4/4 bar on the current grid).
pub const DEFAULT_LENGTH_STEPS: u32 = 16;

/// Ticks per sixteenth-note step.
pub const TICKS_PER_STEP: u32 = TICKS_PER_QUARTER / 4;

/// Default clip length in ticks (`DEFAULT_LENGTH_STEPS * TICKS_PER_STEP`).
pub const DEFAULT_LENGTH_TICKS: u32 = DEFAULT_LENGTH_STEPS * TICKS_PER_STEP;

/// Continuous overdub: stay armed across loop wraps.
pub const PERF_RECORD_MODE_OVERDUB: u32 = 0;
/// Punch-out: auto-disarm after one full clip length of active recording.
pub const PERF_RECORD_MODE_PUNCH_OUT: u32 = 1;

/// How record-arm behaves across loop boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordMode {
    Overdub,
    PunchOut,
}

impl RecordMode {
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            PERF_RECORD_MODE_OVERDUB => Some(Self::Overdub),
            PERF_RECORD_MODE_PUNCH_OUT => Some(Self::PunchOut),
            _ => None,
        }
    }

    pub fn as_u32(self) -> u32 {
        match self {
            Self::Overdub => PERF_RECORD_MODE_OVERDUB,
            Self::PunchOut => PERF_RECORD_MODE_PUNCH_OUT,
        }
    }
}

/// One recorded chord pad press in the looping clip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChordClipEvent {
    /// Start position within the loop, in ticks `[0, length_ticks)`.
    pub start_tick: u32,
    /// Gate length in ticks. At least 1 when finalized.
    pub duration_ticks: u32,
    pub root: u32,
    pub scale_type: u32,
    pub degree: u32,
    pub voicing: u32,
    pub preset: u32,
    pub octave: i32,
    pub velocity: f32,
}

impl ChordClipEvent {
    pub fn end_tick(&self, length_ticks: u32) -> u32 {
        if length_ticks == 0 {
            return self.start_tick;
        }
        (self.start_tick + self.duration_ticks) % length_ticks
    }

    /// True if `tick` lies in `[start, start+duration)` on the looping timeline.
    pub fn covers(&self, tick: u32, length_ticks: u32) -> bool {
        if length_ticks == 0 || self.duration_ticks == 0 {
            return false;
        }
        let tick = tick % length_ticks;
        let start = self.start_tick % length_ticks;
        let end = start + self.duration_ticks;
        if end <= length_ticks {
            tick >= start && tick < end
        } else {
            // Wraps past loop end.
            tick >= start || tick < (end % length_ticks)
        }
    }
}

/// Action the player wants applied to the poly synth this sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlayerAction {
    Trigger(ChordClipEvent),
    Release,
}

#[derive(Clone, Copy, Debug)]
struct OpenEvent {
    start_tick: u32,
    root: u32,
    scale_type: u32,
    degree: u32,
    voicing: u32,
    preset: u32,
    octave: i32,
    velocity: f32,
}

/// Looping chord clip with record-arm and sample-accurate playback.
pub struct PerformanceRecorder {
    length_ticks: u32,
    events: Vec<ChordClipEvent>,
    mode: RecordMode,
    /// Host wants to record when transport allows.
    armed: bool,
    /// Actively accepting record input (past loop-boundary wait if needed).
    recording_active: bool,
    /// When true, wait until tick 0 before starting active recording.
    wait_for_loop_start: bool,
    /// Samples-of-recording remaining for punch-out, counted in ticks while active.
    punch_ticks_remaining: Option<u32>,
    open: Option<OpenEvent>,
    /// Last transport beat position observed (quarter notes).
    last_beat: f64,
    last_tick: u32,
    transport_running: bool,
    /// Event currently sounding from clip playback (index into `events`), if any.
    playing_index: Option<usize>,
    /// Suppress recording while applying player actions (must stay false for live path).
    applying_playback: bool,
    /// While recording, only events with index `< playback_limit` are played back.
    /// This keeps live monitoring monophonic without immediately replaying the
    /// event just stamped. On each loop wrap during overdub the limit advances
    /// to the current event count so the previous pass becomes audible.
    playback_limit: usize,
}

impl Default for PerformanceRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl PerformanceRecorder {
    pub fn new() -> Self {
        Self {
            length_ticks: DEFAULT_LENGTH_TICKS,
            events: Vec::new(),
            mode: RecordMode::PunchOut,
            armed: false,
            recording_active: false,
            wait_for_loop_start: false,
            punch_ticks_remaining: None,
            open: None,
            last_beat: 0.0,
            last_tick: 0,
            transport_running: false,
            playing_index: None,
            applying_playback: false,
            playback_limit: 0,
        }
    }

    pub fn length_ticks(&self) -> u32 {
        self.length_ticks
    }

    pub fn length_steps(&self) -> u32 {
        if TICKS_PER_STEP == 0 {
            0
        } else {
            self.length_ticks / TICKS_PER_STEP
        }
    }

    pub fn set_armed(&mut self, armed: bool) {
        if armed == self.armed {
            return;
        }
        self.armed = armed;
        if !armed {
            self.finalize_open_at(self.last_tick);
            self.recording_active = false;
            self.wait_for_loop_start = false;
            self.punch_ticks_remaining = None;
            return;
        }

        // Arming on: prepare capture window.
        if self.transport_running {
            // Clean takes start at the next loop boundary.
            self.wait_for_loop_start = true;
            self.recording_active = false;
            self.punch_ticks_remaining = None;
        } else {
            // Start active as soon as transport starts.
            self.wait_for_loop_start = false;
            self.recording_active = false;
            self.punch_ticks_remaining = None;
        }
    }

    pub fn is_armed(&self) -> bool {
        self.armed
    }

    /// True when the recorder is actively capturing (armed and inside the capture window).
    pub fn is_recording(&self) -> bool {
        self.armed && self.recording_active && self.transport_running
    }

    pub fn set_mode(&mut self, mode: RecordMode) {
        self.mode = mode;
    }

    pub fn mode(&self) -> RecordMode {
        self.mode
    }

    pub fn clear_clip(&mut self) {
        self.events.clear();
        self.open = None;
        self.playing_index = None;
        self.playback_limit = 0;
    }

    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    pub fn event(&self, index: usize) -> Option<ChordClipEvent> {
        self.events.get(index).copied()
    }

    pub fn events(&self) -> &[ChordClipEvent] {
        &self.events
    }

    /// Advance clock from the engine transport. Call once per audio sample (or
    /// whenever beat position is known). Returns at most one player action.
    pub fn update_clock(&mut self, beat_position: f64, transport_running: bool) -> Option<PlayerAction> {
        let was_running = self.transport_running;
        self.transport_running = transport_running;
        self.last_beat = beat_position;

        if !transport_running {
            if was_running {
                self.finalize_open_at(self.last_tick);
                // Punch remaining freezes while stopped; arm state is preserved.
                self.recording_active = false;
                // Next start: if still armed, wait for loop start again only if
                // we had already been capturing mid-loop; simpler: re-enter via
                // transport start path below.
            }
            self.playing_index = None;
            return None;
        }

        let tick = beat_to_tick(beat_position, self.length_ticks);
        let prev_tick = self.last_tick;

        // Transport just started.
        if !was_running {
            self.last_tick = tick;
            if self.armed {
                if tick == 0 {
                    self.begin_active_recording();
                } else {
                    self.wait_for_loop_start = true;
                    self.recording_active = false;
                }
            }
            return self.playback_action_at(tick, true);
        }

        // Detect loop wrap (tick went backwards within the loop).
        let wrapped = tick < prev_tick;

        if self.armed {
            if self.wait_for_loop_start && (wrapped || tick == 0) {
                self.begin_active_recording();
            } else if self.recording_active {
                if wrapped {
                    // Previous-pass events become audible on the next overdub loop.
                    self.playback_limit = self.events.len();
                }
                if let Some(remaining) = self.punch_ticks_remaining.as_mut() {
                    let advanced = if wrapped {
                        (self.length_ticks - prev_tick) + tick
                    } else {
                        tick.saturating_sub(prev_tick)
                    };
                    if advanced >= *remaining {
                        *remaining = 0;
                        self.finalize_open_at(tick);
                        self.armed = false;
                        self.recording_active = false;
                        self.punch_ticks_remaining = None;
                        self.wait_for_loop_start = false;
                        // Full clip is playable after punch-out completes.
                        self.playback_limit = self.events.len();
                    } else {
                        *remaining -= advanced;
                    }
                }
            }
        } else if wrapped {
            // Keep limit in sync when not recording so all events play.
            self.playback_limit = self.events.len();
        }

        self.last_tick = tick;
        self.playback_action_at(tick, wrapped)
    }

    /// Record a chord pad press at the current clock. Returns true if stamped.
    pub fn record_chord_on(
        &mut self,
        root: u32,
        scale_type: u32,
        degree: u32,
        voicing: u32,
        preset: u32,
        octave: i32,
        velocity: f32,
    ) -> bool {
        if self.applying_playback || !self.is_recording() {
            return false;
        }
        let tick = beat_to_tick(self.last_beat, self.length_ticks);
        self.finalize_open_at(tick);
        cut_gates_at(&mut self.events, tick, self.length_ticks);
        self.open = Some(OpenEvent {
            start_tick: tick,
            root,
            scale_type,
            degree,
            voicing,
            preset,
            octave,
            velocity: velocity.clamp(0.0, 1.0),
        });
        true
    }

    /// Record a chord pad release. Returns true if an open event was closed.
    pub fn record_chord_off(&mut self) -> bool {
        if self.applying_playback || !self.is_recording() {
            // Still finalize if we somehow have an open event while disarming.
            if self.open.is_some() {
                let tick = beat_to_tick(self.last_beat, self.length_ticks);
                return self.finalize_open_at(tick);
            }
            return false;
        }
        let tick = beat_to_tick(self.last_beat, self.length_ticks);
        self.finalize_open_at(tick)
    }

    /// Mark that subsequent poly actions come from the player (do not record).
    pub fn set_applying_playback(&mut self, applying: bool) {
        self.applying_playback = applying;
    }

    pub fn is_applying_playback(&self) -> bool {
        self.applying_playback
    }

    fn begin_active_recording(&mut self) {
        self.wait_for_loop_start = false;
        self.recording_active = true;
        // Existing events remain playable; newly finalized ones wait for the next wrap.
        self.playback_limit = self.events.len();
        if self.mode == RecordMode::PunchOut {
            self.punch_ticks_remaining = Some(self.length_ticks);
        } else {
            self.punch_ticks_remaining = None;
        }
    }

    fn finalize_open_at(&mut self, end_tick: u32) -> bool {
        let Some(open) = self.open.take() else {
            return false;
        };
        let mut duration = tick_distance(open.start_tick, end_tick, self.length_ticks);
        if duration == 0 {
            duration = 1;
        }
        // Do not allow events longer than one full loop.
        duration = duration.min(self.length_ticks);
        let event = ChordClipEvent {
            start_tick: open.start_tick % self.length_ticks,
            duration_ticks: duration,
            root: open.root,
            scale_type: open.scale_type,
            degree: open.degree,
            voicing: open.voicing,
            preset: open.preset,
            octave: open.octave,
            velocity: open.velocity,
        };
        // Keep insertion order so `playback_limit` (index of first "this pass" event)
        // stays valid during overdub. Hosts that want timeline order can sort on read.
        self.events.push(event);
        true
    }

    fn playback_action_at(&mut self, tick: u32, force_rescan: bool) -> Option<PlayerAction> {
        let playable_end = if self.recording_active {
            self.playback_limit.min(self.events.len())
        } else {
            self.events.len()
        };

        if playable_end == 0 {
            if self.playing_index.take().is_some() {
                return Some(PlayerAction::Release);
            }
            return None;
        }

        // Find the event that should be sounding at `tick`, if any.
        // Prefer the latest-started event that covers this tick (monophonic).
        let mut best: Option<usize> = None;
        for (i, ev) in self.events.iter().enumerate().take(playable_end) {
            if ev.covers(tick, self.length_ticks) {
                match best {
                    None => best = Some(i),
                    Some(bi) => {
                        // Prefer higher start_tick (later in loop), with wrap-aware
                        // comparison relative to tick.
                        if event_start_rank(ev.start_tick, tick, self.length_ticks)
                            >= event_start_rank(self.events[bi].start_tick, tick, self.length_ticks)
                        {
                            best = Some(i);
                        }
                    }
                }
            }
        }

        if best == self.playing_index && !force_rescan {
            return None;
        }

        // On wrap, force re-trigger if the same event still covers (sustains
        // across loop) — monophonic pad policy retriggers only when index changes
        // or we cross a start boundary.
        if best == self.playing_index {
            // Check if we just landed on a start boundary of that event.
            if let Some(i) = best {
                if self.events[i].start_tick == tick {
                    self.playing_index = best;
                    return Some(PlayerAction::Trigger(self.events[i]));
                }
            }
            return None;
        }

        match (self.playing_index, best) {
            (None, Some(i)) => {
                self.playing_index = Some(i);
                Some(PlayerAction::Trigger(self.events[i]))
            }
            (Some(_), None) => {
                self.playing_index = None;
                Some(PlayerAction::Release)
            }
            (Some(_), Some(i)) => {
                self.playing_index = Some(i);
                Some(PlayerAction::Trigger(self.events[i]))
            }
            (None, None) => None,
        }
    }
}

/// Convert a beat position (quarter notes) into a tick within `[0, length_ticks)`.
pub fn beat_to_tick(beat_position: f64, length_ticks: u32) -> u32 {
    if length_ticks == 0 {
        return 0;
    }
    let raw = beat_position * f64::from(TICKS_PER_QUARTER);
    // Floor toward -inf then wrap into the loop.
    let floored = raw.floor();
    let mut tick = floored as i64 % i64::from(length_ticks);
    if tick < 0 {
        tick += i64::from(length_ticks);
    }
    tick as u32
}

/// Forward distance from `start` to `end` on a looping timeline of `length` ticks.
pub fn tick_distance(start: u32, end: u32, length: u32) -> u32 {
    if length == 0 {
        return 0;
    }
    let start = start % length;
    let end = end % length;
    if end >= start {
        end - start
    } else {
        length - start + end
    }
}

/// Truncate any event whose gate covers `tick` so it ends at `tick` (cut-gate).
pub fn cut_gates_at(events: &mut Vec<ChordClipEvent>, tick: u32, length_ticks: u32) {
    if length_ticks == 0 {
        return;
    }
    let tick = tick % length_ticks;
    events.retain_mut(|ev| {
        if !ev.covers(tick, length_ticks) {
            return true;
        }
        // If the event starts at tick, remove it entirely (replaced by new note-on).
        if ev.start_tick % length_ticks == tick {
            return false;
        }
        let new_duration = tick_distance(ev.start_tick, tick, length_ticks);
        if new_duration == 0 {
            return false;
        }
        ev.duration_ticks = new_duration;
        true
    });
}

fn event_start_rank(start: u32, tick: u32, length: u32) -> u32 {
    // Distance backward from tick to start (how long ago it started).
    // Smaller distance = more recent = higher priority → invert.
    let dist = tick_distance(start, tick, length);
    length.saturating_sub(dist)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beat_to_tick_basic() {
        assert_eq!(beat_to_tick(0.0, DEFAULT_LENGTH_TICKS), 0);
        // One quarter note = 96 ticks.
        assert_eq!(beat_to_tick(1.0, DEFAULT_LENGTH_TICKS), 96);
        // One 16th = 0.25 beats = 24 ticks.
        assert_eq!(beat_to_tick(0.25, DEFAULT_LENGTH_TICKS), 24);
        // Full bar (4 beats) wraps to 0.
        assert_eq!(beat_to_tick(4.0, DEFAULT_LENGTH_TICKS), 0);
        // 4.5 beats → half bar into next loop = 48 ticks? 0.5 * 96 = 48.
        assert_eq!(beat_to_tick(4.5, DEFAULT_LENGTH_TICKS), 48);
    }

    #[test]
    fn tick_distance_wraps() {
        assert_eq!(tick_distance(10, 20, 100), 10);
        assert_eq!(tick_distance(90, 10, 100), 20);
        assert_eq!(tick_distance(0, 0, 100), 0);
    }

    #[test]
    fn cut_gates_shortens_overlapping() {
        let mut events = vec![ChordClipEvent {
            start_tick: 0,
            duration_ticks: 100,
            root: 0,
            scale_type: 0,
            degree: 0,
            voicing: 0,
            preset: 0,
            octave: 4,
            velocity: 0.9,
        }];
        cut_gates_at(&mut events, 40, DEFAULT_LENGTH_TICKS);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].duration_ticks, 40);
    }

    #[test]
    fn record_chord_on_off_stores_event() {
        let mut rec = PerformanceRecorder::new();
        rec.set_mode(RecordMode::Overdub);
        rec.set_armed(true);
        // Simulate transport running at beat 0.
        let _ = rec.update_clock(0.0, true);
        assert!(rec.is_recording());

        assert!(rec.record_chord_on(0, 0, 0, 0, 1, 4, 0.9));
        // Advance to beat 1 (96 ticks).
        let _ = rec.update_clock(1.0, true);
        assert!(rec.record_chord_off());
        assert_eq!(rec.event_count(), 1);
        let e = rec.event(0).unwrap();
        assert_eq!(e.start_tick, 0);
        assert_eq!(e.duration_ticks, 96);
        assert_eq!(e.degree, 0);
    }

    #[test]
    fn punch_out_disarms_after_one_loop() {
        let mut rec = PerformanceRecorder::new();
        rec.set_mode(RecordMode::PunchOut);
        rec.set_armed(true);
        let _ = rec.update_clock(0.0, true);
        assert!(rec.is_armed());
        assert!(rec.is_recording());

        // Advance almost one full loop (383 ticks ≈ 3.9896 beats).
        let almost = (DEFAULT_LENGTH_TICKS - 1) as f64 / f64::from(TICKS_PER_QUARTER);
        let _ = rec.update_clock(almost, true);
        assert!(rec.is_armed());

        // Cross the loop boundary — punch should complete.
        let _ = rec.update_clock(4.0, true);
        assert!(!rec.is_armed());
        assert!(!rec.is_recording());
    }

    #[test]
    fn overdub_stays_armed_across_loop() {
        let mut rec = PerformanceRecorder::new();
        rec.set_mode(RecordMode::Overdub);
        rec.set_armed(true);
        let _ = rec.update_clock(0.0, true);
        let _ = rec.update_clock(4.0, true);
        let _ = rec.update_clock(4.5, true);
        assert!(rec.is_armed());
        assert!(rec.is_recording());
    }

    #[test]
    fn overdub_cut_gate_on_second_pass() {
        let mut rec = PerformanceRecorder::new();
        rec.set_mode(RecordMode::Overdub);
        rec.set_armed(true);
        let _ = rec.update_clock(0.0, true);

        // First pass: chord from 0 for a long gate.
        assert!(rec.record_chord_on(0, 0, 0, 0, 1, 4, 0.9));
        let _ = rec.update_clock(2.0, true); // 192 ticks
        assert!(rec.record_chord_off());
        assert_eq!(rec.event_count(), 1);
        assert_eq!(rec.event(0).unwrap().duration_ticks, 192);

        // Second pass at beat 0.5 (48 ticks): new chord cuts previous.
        let _ = rec.update_clock(4.5, true);
        assert!(rec.record_chord_on(0, 0, 4, 0, 1, 4, 0.8));
        let _ = rec.update_clock(5.0, true);
        assert!(rec.record_chord_off());

        assert_eq!(rec.event_count(), 2);
        let first = rec.events().iter().find(|e| e.degree == 0).unwrap();
        assert_eq!(first.duration_ticks, 48);
        let second = rec.events().iter().find(|e| e.degree == 4).unwrap();
        assert_eq!(second.start_tick, 48);
    }

    #[test]
    fn arm_while_running_waits_for_loop_start() {
        let mut rec = PerformanceRecorder::new();
        rec.set_mode(RecordMode::PunchOut);
        // Transport already mid-bar.
        let _ = rec.update_clock(1.0, true);
        rec.set_armed(true);
        assert!(!rec.is_recording());
        // Still mid-bar.
        let _ = rec.update_clock(2.0, true);
        assert!(!rec.is_recording());
        // Loop wrap.
        let _ = rec.update_clock(4.0, true);
        assert!(rec.is_recording());
    }

    #[test]
    fn playback_triggers_and_releases() {
        let mut rec = PerformanceRecorder::new();
        rec.events.push(ChordClipEvent {
            start_tick: 0,
            duration_ticks: 48,
            root: 0,
            scale_type: 0,
            degree: 0,
            voicing: 0,
            preset: 1,
            octave: 4,
            velocity: 0.9,
        });

        let a = rec.update_clock(0.0, true);
        assert!(matches!(a, Some(PlayerAction::Trigger(_))));

        // Still inside gate (beat 0.25 = 24 ticks).
        let a = rec.update_clock(0.25, true);
        assert!(a.is_none());

        // Past gate (beat 0.6 = 57.6 → 57 ticks).
        let a = rec.update_clock(0.6, true);
        assert!(matches!(a, Some(PlayerAction::Release)));
    }

    #[test]
    fn clear_clip_empties_events() {
        let mut rec = PerformanceRecorder::new();
        rec.events.push(ChordClipEvent {
            start_tick: 0,
            duration_ticks: 10,
            root: 0,
            scale_type: 0,
            degree: 0,
            voicing: 0,
            preset: 0,
            octave: 4,
            velocity: 1.0,
        });
        rec.clear_clip();
        assert_eq!(rec.event_count(), 0);
    }

    #[test]
    fn does_not_record_when_disarmed() {
        let mut rec = PerformanceRecorder::new();
        let _ = rec.update_clock(0.0, true);
        assert!(!rec.record_chord_on(0, 0, 0, 0, 0, 4, 1.0));
        assert_eq!(rec.event_count(), 0);
    }
}
