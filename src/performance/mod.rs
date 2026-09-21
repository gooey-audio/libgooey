//! Performance clip recording and sample-accurate replay for live instruments.
//!
//! Stage 1 focuses on chord pad performances: timed pad-parameter events in a
//! looping clip locked to the engine transport (same beat clock as drum sequencers).

pub(crate) mod control;

use std::sync::Arc;

use crate::music::{apply_voicing, Chord, ChordSet, Key, NoteName, ScaleType, VoicingType};

/// Pulses per quarter note. One sixteenth-note step is `TICKS_PER_QUARTER / 4`.
pub const TICKS_PER_QUARTER: u32 = 96;

/// Default clip length in sixteenth-note steps (one 4/4 bar on the current grid).
pub const DEFAULT_LENGTH_STEPS: u32 = 16;

/// Ticks per sixteenth-note step.
pub const TICKS_PER_STEP: u32 = TICKS_PER_QUARTER / 4;

/// Default clip length in ticks (`DEFAULT_LENGTH_STEPS * TICKS_PER_STEP`).
pub const DEFAULT_LENGTH_TICKS: u32 = DEFAULT_LENGTH_STEPS * TICKS_PER_STEP;

pub const CHORD_TARGET_POLY: u32 = 0;
pub const CHORD_TARGET_PIANO: u32 = 1;
pub const CHORD_LOOP_MAX_EVENTS: usize = 512;

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
    /// Chord set (harmonic palette) id the pad was played from. Persisted so a
    /// clip replays the palette it was recorded with, not whatever the host UI
    /// happens to show later.
    pub chord_set: u32,
    pub root: u32,
    pub scale_type: u32,
    pub degree: u32,
    pub voicing: u32,
    pub preset: u32,
    pub octave: i32,
    pub velocity: f32,
}

/// Render-ready chord event. Harmony resolution and voicing allocation happen
/// before this value reaches the audio thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreparedChordEvent {
    pub(crate) target: u32,
    pub(crate) target_id: u32,
    pub(crate) event: ChordClipEvent,
    pub(crate) chord: Chord,
    pub(crate) notes: [u8; 6],
    pub(crate) note_count: u8,
}

impl PreparedChordEvent {
    pub(crate) fn notes(&self) -> &[u8] {
        &self.notes[..self.note_count as usize]
    }
}

#[derive(Debug)]
pub(crate) struct ChordLoopSnapshot {
    pub(crate) generation: u64,
    pub(crate) length_ticks: u32,
    pub(crate) events: Vec<PreparedChordEvent>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ChordCommandAction {
    Trigger(PreparedChordEvent),
    ReleaseAll,
}

pub(crate) enum ChordClipEdit {
    Replace(Arc<ChordLoopSnapshot>),
    Clear { generation: u64 },
}

/// Resolve and normalize one chord without retaining any temporary `Vec` in
/// the returned value. This function is called by producer/legacy control
/// paths, never by sample playback.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_chord_event(
    target: u32,
    target_id: u32,
    chord_set: u32,
    root: u32,
    scale_type: u32,
    degree: u32,
    voicing: u32,
    preset: u32,
    octave: i32,
    velocity: f32,
) -> Option<PreparedChordEvent> {
    if !velocity.is_finite() {
        return None;
    }
    let set = ChordSet::from_id(chord_set)?;
    let scale = if scale_type == 1 {
        ScaleType::NaturalMinor
    } else {
        ScaleType::Major
    };
    let key = Key::new(NoteName::from_index((root % 12) as u8), scale);
    let chord = set.chord(&key, degree as usize);
    let voicing = match voicing {
        1 => VoicingType::FirstInversion,
        2 => VoicingType::SecondInversion,
        3 => VoicingType::ThirdInversion,
        4 => VoicingType::OpenVoicing,
        5 => VoicingType::Drop2,
        6 => VoicingType::Drop3,
        7 => VoicingType::Spread,
        8 => VoicingType::Shell,
        9 => VoicingType::Rootless,
        _ => VoicingType::RootPosition,
    };
    let resolved = apply_voicing(&chord, voicing, octave.clamp(0, 8) as i8);
    if resolved.len() > 6 {
        return None;
    }
    let mut notes = [0; 6];
    notes[..resolved.len()].copy_from_slice(&resolved);
    Some(PreparedChordEvent {
        target,
        target_id,
        event: ChordClipEvent {
            start_tick: 0,
            duration_ticks: 1,
            chord_set,
            root,
            scale_type,
            degree,
            voicing: match voicing {
                VoicingType::RootPosition => 0,
                VoicingType::FirstInversion => 1,
                VoicingType::SecondInversion => 2,
                VoicingType::ThirdInversion => 3,
                VoicingType::OpenVoicing => 4,
                VoicingType::Drop2 => 5,
                VoicingType::Drop3 => 6,
                VoicingType::Spread => 7,
                VoicingType::Shell => 8,
                VoicingType::Rootless => 9,
            },
            preset,
            octave,
            velocity: velocity.clamp(0.0, 1.0),
        },
        chord,
        notes,
        note_count: resolved.len() as u8,
    })
}

/// One manually played sampler hit captured on the shared performance timeline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SamplerClipEvent {
    pub start_tick: u32,
    pub rack: u32,
    pub slot: u32,
    pub velocity: f32,
}

impl ChordClipEvent {
    pub fn end_tick(&self, length_ticks: u32) -> u32 {
        if length_ticks == 0 {
            return self.start_tick;
        }
        ((u64::from(self.start_tick) + u64::from(self.duration_ticks)) % u64::from(length_ticks))
            as u32
    }

    /// True if `tick` lies in `[start, start+duration)` on the looping timeline.
    pub fn covers(&self, tick: u32, length_ticks: u32) -> bool {
        if length_ticks == 0 || self.duration_ticks == 0 {
            return false;
        }
        let length = u64::from(length_ticks);
        let tick = u64::from(tick % length_ticks);
        let start = u64::from(self.start_tick % length_ticks);
        let end = start + u64::from(self.duration_ticks);
        if end <= length {
            tick >= start && tick < end
        } else {
            // Wraps past loop end.
            tick >= start || tick < (end % length)
        }
    }
}

/// Action the player wants applied to the controlled chord voice this sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlayerAction {
    Trigger(PreparedChordEvent),
    Release,
}

#[derive(Clone, Copy, Debug)]
struct OpenEvent {
    start_tick: u32,
    prepared: PreparedChordEvent,
}

pub(crate) struct ClockUpdate {
    pub(crate) action: Option<PlayerAction>,
    pub(crate) installed_generation: Option<u64>,
}

/// Looping chord clip with record-arm and sample-accurate playback.
pub struct PerformanceRecorder {
    length_ticks: u32,
    events: Vec<PreparedChordEvent>,
    active_snapshot: Option<Arc<ChordLoopSnapshot>>,
    pending_snapshot: Option<Arc<ChordLoopSnapshot>>,
    sampler_events: Vec<SamplerClipEvent>,
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
    last_absolute_tick: Option<u64>,
    last_transport_generation: u64,
    transport_running: bool,
    /// Event currently sounding from clip playback (index into `events`), if any.
    playing_index: Option<usize>,
    /// Next tick at which a sorted host snapshot can change state. Normal
    /// sample playback skips event lookup until this boundary is reached.
    next_event_boundary: Option<u32>,
    /// Suppress recording while applying player actions (must stay false for live path).
    applying_playback: bool,
    /// While recording, only events with index `< playback_limit` are played back.
    /// This keeps live monitoring monophonic without immediately replaying the
    /// event just stamped. On each loop wrap during overdub the limit advances
    /// to the current event count so the previous pass becomes audible.
    playback_limit: usize,
    sampler_playback_limit: usize,
    last_sampler_tick: Option<u32>,
    pending_sampler_hits: Vec<SamplerClipEvent>,
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
            events: Vec::with_capacity(CHORD_LOOP_MAX_EVENTS),
            active_snapshot: None,
            pending_snapshot: None,
            sampler_events: Vec::new(),
            mode: RecordMode::PunchOut,
            armed: false,
            recording_active: false,
            wait_for_loop_start: false,
            punch_ticks_remaining: None,
            open: None,
            last_beat: 0.0,
            last_tick: 0,
            last_absolute_tick: None,
            last_transport_generation: 0,
            transport_running: false,
            playing_index: None,
            next_event_boundary: None,
            applying_playback: false,
            playback_limit: 0,
            sampler_playback_limit: 0,
            last_sampler_tick: None,
            pending_sampler_hits: Vec::with_capacity(CHORD_LOOP_MAX_EVENTS),
        }
    }

    pub fn length_ticks(&self) -> u32 {
        self.length_ticks
    }

    pub fn length_steps(&self) -> u32 {
        self.length_ticks / TICKS_PER_STEP
    }

    pub fn set_armed(&mut self, armed: bool) {
        if armed == self.armed {
            return;
        }
        if armed {
            // Legacy recording is a single-thread API. Convert an installed
            // immutable host clip back into the recorder's preallocated store
            // before capture starts, preserving its variable loop length.
            if let Some(snapshot) = self.active_snapshot.take() {
                self.events.clear();
                self.events.extend_from_slice(&snapshot.events);
            }
            self.pending_snapshot = None;
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
        self.active_snapshot = None;
        self.pending_snapshot = None;
        self.sampler_events.clear();
        self.open = None;
        self.playing_index = None;
        self.next_event_boundary = None;
        self.playback_limit = 0;
        self.sampler_playback_limit = 0;
        self.pending_sampler_hits.clear();
        self.last_absolute_tick = None;
    }

    pub fn event_count(&self) -> usize {
        self.active_events().len()
    }

    pub fn event(&self, index: usize) -> Option<ChordClipEvent> {
        self.active_events().get(index).map(|event| event.event)
    }

    pub fn events(&self) -> Vec<ChordClipEvent> {
        self.active_events()
            .iter()
            .map(|event| event.event)
            .collect()
    }

    fn active_events(&self) -> &[PreparedChordEvent] {
        self.active_snapshot
            .as_ref()
            .map_or(&self.events, |snapshot| &snapshot.events)
    }

    pub fn sampler_event_count(&self) -> usize {
        self.sampler_events.len()
    }
    pub fn sampler_event(&self, index: usize) -> Option<SamplerClipEvent> {
        self.sampler_events.get(index).copied()
    }
    pub fn take_sampler_hits(&mut self) -> &[SamplerClipEvent] {
        &self.pending_sampler_hits
    }
    pub fn clear_pending_sampler_hits(&mut self) {
        self.pending_sampler_hits.clear();
    }

    fn prepare_host_takeover(&mut self) {
        self.finalize_open_at(self.last_tick);
        self.armed = false;
        self.recording_active = false;
        self.wait_for_loop_start = false;
        self.punch_ticks_remaining = None;
        self.sampler_events.clear();
        self.pending_sampler_hits.clear();
        self.sampler_playback_limit = 0;
    }

    fn retire_snapshot(
        snapshot: Option<Arc<ChordLoopSnapshot>>,
        retired: &mut Vec<Arc<ChordLoopSnapshot>>,
    ) {
        if let Some(snapshot) = snapshot {
            debug_assert!(retired.len() < retired.capacity());
            retired.push(snapshot);
        }
    }

    fn install_snapshot(
        &mut self,
        snapshot: Arc<ChordLoopSnapshot>,
        retired: &mut Vec<Arc<ChordLoopSnapshot>>,
    ) -> u64 {
        Self::retire_snapshot(self.active_snapshot.replace(Arc::clone(&snapshot)), retired);
        self.events.clear();
        self.length_ticks = snapshot.length_ticks;
        self.playback_limit = snapshot.events.len();
        self.playing_index = None;
        self.next_event_boundary = None;
        self.last_tick = 0;
        self.last_absolute_tick = None;
        snapshot.generation
    }

    /// Stage a validated host edit at a render boundary. A running non-empty
    /// clip keeps playing until its next wrap; stopped or empty clips install
    /// immediately.
    pub(crate) fn apply_clip_edit(
        &mut self,
        edit: ChordClipEdit,
        transport_running: bool,
        retired: &mut Vec<Arc<ChordLoopSnapshot>>,
    ) -> Option<u64> {
        self.prepare_host_takeover();
        match edit {
            ChordClipEdit::Replace(snapshot) => {
                if transport_running && !self.active_events().is_empty() {
                    Self::retire_snapshot(self.pending_snapshot.replace(snapshot), retired);
                    None
                } else {
                    Self::retire_snapshot(self.pending_snapshot.take(), retired);
                    Some(self.install_snapshot(snapshot, retired))
                }
            }
            ChordClipEdit::Clear { generation } => {
                Self::retire_snapshot(self.active_snapshot.take(), retired);
                Self::retire_snapshot(self.pending_snapshot.take(), retired);
                self.events.clear();
                self.length_ticks = DEFAULT_LENGTH_TICKS;
                self.playing_index = None;
                self.next_event_boundary = None;
                self.playback_limit = 0;
                self.last_absolute_tick = None;
                Some(generation)
            }
        }
    }

    /// Legacy Rust helper retained for callers that do not have the mixer's
    /// discontinuity counter. FFI rendering uses `update_clock_with_transport`
    /// so snapshot retirement remains outside the callback.
    pub fn update_clock(
        &mut self,
        beat_position: f64,
        transport_running: bool,
    ) -> Option<PlayerAction> {
        let generation =
            self.last_transport_generation + u64::from(transport_running != self.transport_running);
        let mut retired = Vec::with_capacity(2);
        self.update_clock_with_transport(beat_position, transport_running, generation, &mut retired)
            .action
    }

    /// Advance from the mixer transport. Called once per rendered sample and
    /// returns at most one chord action plus a snapshot-install generation.
    pub(crate) fn update_clock_with_transport(
        &mut self,
        beat_position: f64,
        transport_running: bool,
        transport_generation: u64,
        retired: &mut Vec<Arc<ChordLoopSnapshot>>,
    ) -> ClockUpdate {
        let was_running = self.transport_running;
        self.transport_running = transport_running;
        self.last_beat = beat_position;
        let discontinuity = transport_generation != self.last_transport_generation;
        self.last_transport_generation = transport_generation;

        if !transport_running {
            if was_running {
                self.finalize_open_at(self.last_tick);
                self.recording_active = false;
            }
            let action = self.playing_index.take().map(|_| PlayerAction::Release);
            self.next_event_boundary = None;
            self.last_sampler_tick = None;
            self.pending_sampler_hits.clear();
            self.last_absolute_tick = None;
            return ClockUpdate {
                action,
                installed_generation: None,
            };
        }

        let absolute_tick = absolute_beat_tick(beat_position);
        let prev_tick = self.last_tick;
        let previous_absolute = self.last_absolute_tick;
        let old_length = u64::from(self.length_ticks);
        let wrapped = previous_absolute.is_some_and(|previous| {
            absolute_tick >= previous
                && old_length > 0
                && absolute_tick / old_length > previous / old_length
        });

        let mut installed_generation = None;
        if wrapped && retired.len() < retired.capacity() {
            if let Some(snapshot) = self.pending_snapshot.take() {
                installed_generation = Some(self.install_snapshot(snapshot, retired));
            }
        }

        let tick = (absolute_tick % u64::from(self.length_ticks)) as u32;

        // Transport just started.
        if !was_running {
            self.last_tick = tick;
            self.last_absolute_tick = Some(absolute_tick);
            if self.armed {
                if tick == 0 {
                    self.begin_active_recording();
                } else {
                    self.wait_for_loop_start = true;
                    self.recording_active = false;
                }
            }
            self.populate_sampler_hits(tick);
            return ClockUpdate {
                action: self.playback_action_at(tick, true, false),
                installed_generation,
            };
        }

        if previous_absolute == Some(absolute_tick)
            && !discontinuity
            && installed_generation.is_none()
        {
            return ClockUpdate {
                action: None,
                installed_generation: None,
            };
        }

        if self.armed {
            if self.wait_for_loop_start && (wrapped || tick == 0) {
                self.begin_active_recording();
            } else if self.recording_active {
                if wrapped {
                    // Previous-pass events become audible on the next overdub loop.
                    self.playback_limit = self.events.len();
                    self.sampler_playback_limit = self.sampler_events.len();
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
                        self.sampler_playback_limit = self.sampler_events.len();
                    } else {
                        *remaining -= advanced;
                    }
                }
            }
        } else if wrapped {
            // Keep limit in sync when not recording so all events play.
            self.playback_limit = self.active_events().len();
            self.sampler_playback_limit = self.sampler_events.len();
        }

        self.last_tick = tick;
        self.last_absolute_tick = Some(absolute_tick);
        self.populate_sampler_hits(tick);
        ClockUpdate {
            action: self.playback_action_at(
                tick,
                discontinuity || wrapped,
                wrapped && !discontinuity,
            ),
            installed_generation,
        }
    }

    /// Record a chord pad press at the current clock. Returns true if stamped.
    ///
    /// `chord_set` is the harmonic palette id the pad came from; it is stored
    /// verbatim so replay reproduces the chord that was actually heard.
    #[allow(clippy::too_many_arguments)]
    pub fn record_chord_on(
        &mut self,
        chord_set: u32,
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
        let Some(prepared) = prepare_chord_event(
            CHORD_TARGET_POLY,
            0,
            chord_set,
            root,
            scale_type,
            degree,
            voicing,
            preset,
            octave,
            velocity,
        ) else {
            return false;
        };
        let tick = beat_to_tick(self.last_beat, self.length_ticks);
        self.finalize_open_at(tick);
        cut_prepared_gates_at(&mut self.events, tick, self.length_ticks);
        self.open = Some(OpenEvent {
            start_tick: tick,
            prepared,
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

    /// Stamp a live sampler hit. Sequencer and clip playback callers set
    /// `applying_playback`, so only explicit host pad hits are captured.
    pub fn record_sampler_hit(&mut self, rack: u32, slot: u32, velocity: f32) -> bool {
        if self.applying_playback || !self.is_recording() {
            return false;
        }
        self.sampler_events.push(SamplerClipEvent {
            start_tick: beat_to_tick(self.last_beat, self.length_ticks),
            rack,
            slot,
            velocity: velocity.clamp(0.0, 1.0),
        });
        true
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
        self.sampler_playback_limit = self.sampler_events.len();
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
        if self.events.len() >= CHORD_LOOP_MAX_EVENTS {
            return false;
        }
        let mut duration = tick_distance(open.start_tick, end_tick, self.length_ticks);
        if duration == 0 {
            duration = 1;
        }
        // Do not allow events longer than one full loop.
        duration = duration.min(self.length_ticks);
        let mut event = open.prepared;
        event.event.start_tick = open.start_tick % self.length_ticks;
        event.event.duration_ticks = duration;
        // Keep insertion order so `playback_limit` (index of first "this pass" event)
        // stays valid during overdub. Hosts that want timeline order can sort on read.
        self.events.push(event);
        true
    }

    fn playback_action_at(
        &mut self,
        tick: u32,
        force_rescan: bool,
        retrigger_same_at_start: bool,
    ) -> Option<PlayerAction> {
        let playable_end = if self.recording_active {
            self.playback_limit.min(self.events.len())
        } else {
            self.active_events().len()
        };

        if playable_end == 0 {
            if self.playing_index.take().is_some() {
                self.next_event_boundary = None;
                return Some(PlayerAction::Release);
            }
            self.next_event_boundary = None;
            return None;
        }

        if self.active_snapshot.is_some() && !force_rescan && self.next_event_boundary != Some(tick)
        {
            return None;
        }

        let best = if self.active_snapshot.is_some() {
            covering_sorted_event(self.active_events(), tick, self.length_ticks)
        } else {
            covering_recorded_event(&self.events[..playable_end], tick, self.length_ticks)
        };

        if best == self.playing_index && !force_rescan {
            self.cache_next_event_boundary(best, tick);
            return None;
        }

        // On wrap, force re-trigger if the same event still covers (sustains
        // across loop) — monophonic pad policy retriggers only when index changes
        // or we cross a start boundary.
        if best == self.playing_index {
            // Check if we just landed on a start boundary of that event.
            if retrigger_same_at_start {
                let Some(i) = best else {
                    self.cache_next_event_boundary(best, tick);
                    return None;
                };
                let event = self.active_events()[i];
                if event.event.start_tick == tick {
                    self.playing_index = best;
                    self.cache_next_event_boundary(best, tick);
                    return Some(PlayerAction::Trigger(event));
                }
            }
            self.cache_next_event_boundary(best, tick);
            return None;
        }

        let action = match (self.playing_index, best) {
            (None, Some(i)) => {
                let event = self.active_events()[i];
                self.playing_index = Some(i);
                Some(PlayerAction::Trigger(event))
            }
            (Some(_), None) => {
                self.playing_index = None;
                Some(PlayerAction::Release)
            }
            (Some(_), Some(i)) => {
                let event = self.active_events()[i];
                self.playing_index = Some(i);
                Some(PlayerAction::Trigger(event))
            }
            (None, None) => None,
        };
        self.cache_next_event_boundary(best, tick);
        action
    }

    fn cache_next_event_boundary(&mut self, active: Option<usize>, tick: u32) {
        let Some(snapshot) = self.active_snapshot.as_ref() else {
            self.next_event_boundary = None;
            return;
        };
        self.next_event_boundary = if let Some(index) = active {
            let event = snapshot.events[index].event;
            (event.duration_ticks < snapshot.length_ticks)
                .then(|| event.end_tick(snapshot.length_ticks))
        } else {
            let insertion = snapshot
                .events
                .partition_point(|event| event.event.start_tick <= tick);
            snapshot
                .events
                .get(insertion)
                .or_else(|| snapshot.events.first())
                .map(|event| event.event.start_tick)
        };
    }

    fn populate_sampler_hits(&mut self, tick: u32) {
        self.pending_sampler_hits.clear();
        if self.last_sampler_tick == Some(tick) {
            return;
        }
        self.last_sampler_tick = Some(tick);
        let playable = if self.recording_active {
            self.sampler_playback_limit.min(self.sampler_events.len())
        } else {
            self.sampler_events.len()
        };
        self.pending_sampler_hits.extend(
            self.sampler_events
                .iter()
                .take(playable)
                .filter(|event| event.start_tick == tick)
                .copied(),
        );
    }
}

fn absolute_beat_tick(beat_position: f64) -> u64 {
    if !beat_position.is_finite() || beat_position <= 0.0 {
        return 0;
    }
    // Repeated f64 beat increments can land a few ulps below an exact grid
    // boundary (for example 383/96 after 500 samples at 48 kHz/60 BPM).
    // This sub-nanotick bias corrects that representation error without being
    // musically large enough to move a real sample across a boundary.
    (beat_position * f64::from(TICKS_PER_QUARTER) + 1.0e-9).floor() as u64
}

fn covering_sorted_event(
    events: &[PreparedChordEvent],
    tick: u32,
    length_ticks: u32,
) -> Option<usize> {
    if events.is_empty() {
        return None;
    }
    let insertion = events.partition_point(|event| event.event.start_tick <= tick);
    if insertion > 0 {
        let index = insertion - 1;
        if events[index].event.covers(tick, length_ticks) {
            return Some(index);
        }
    }
    let wrapped = events.len() - 1;
    events[wrapped]
        .event
        .covers(tick, length_ticks)
        .then_some(wrapped)
}

fn covering_recorded_event(
    events: &[PreparedChordEvent],
    tick: u32,
    length_ticks: u32,
) -> Option<usize> {
    let mut best: Option<usize> = None;
    for (index, event) in events.iter().enumerate() {
        if event.event.covers(tick, length_ticks)
            && best.is_none_or(|previous| {
                event_start_rank(event.event.start_tick, tick, length_ticks)
                    >= event_start_rank(events[previous].event.start_tick, tick, length_ticks)
            })
        {
            best = Some(index);
        }
    }
    best
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

fn cut_prepared_gates_at(events: &mut Vec<PreparedChordEvent>, tick: u32, length_ticks: u32) {
    if length_ticks == 0 {
        return;
    }
    let tick = tick % length_ticks;
    events.retain_mut(|event| {
        let chord = &mut event.event;
        if !chord.covers(tick, length_ticks) {
            return true;
        }
        if chord.start_tick % length_ticks == tick {
            return false;
        }
        let new_duration = tick_distance(chord.start_tick, tick, length_ticks);
        if new_duration == 0 {
            return false;
        }
        chord.duration_ticks = new_duration;
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

    fn prepared(event: ChordClipEvent) -> PreparedChordEvent {
        let mut prepared = prepare_chord_event(
            CHORD_TARGET_POLY,
            0,
            event.chord_set,
            event.root,
            event.scale_type,
            event.degree,
            event.voicing,
            event.preset,
            event.octave,
            event.velocity,
        )
        .unwrap();
        prepared.event = event;
        prepared
    }

    fn host_event(start_tick: u32, duration_ticks: u32, degree: u32) -> PreparedChordEvent {
        prepared(ChordClipEvent {
            start_tick,
            duration_ticks,
            chord_set: 1,
            root: 0,
            scale_type: 0,
            degree,
            voicing: 0,
            preset: 0,
            octave: 4,
            velocity: 0.8,
        })
    }

    fn snapshot(
        generation: u64,
        length_ticks: u32,
        events: Vec<PreparedChordEvent>,
    ) -> ChordClipEdit {
        ChordClipEdit::Replace(Arc::new(ChordLoopSnapshot {
            generation,
            length_ticks,
            events,
        }))
    }

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
            chord_set: 1,
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

        assert!(rec.record_chord_on(1, 0, 0, 0, 0, 1, 4, 0.9));
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
        assert!(rec.record_chord_on(1, 0, 0, 0, 0, 1, 4, 0.9));
        let _ = rec.update_clock(2.0, true); // 192 ticks
        assert!(rec.record_chord_off());
        assert_eq!(rec.event_count(), 1);
        assert_eq!(rec.event(0).unwrap().duration_ticks, 192);

        // Second pass at beat 0.5 (48 ticks): new chord cuts previous.
        let _ = rec.update_clock(4.5, true);
        assert!(rec.record_chord_on(1, 0, 0, 4, 0, 1, 4, 0.8));
        let _ = rec.update_clock(5.0, true);
        assert!(rec.record_chord_off());

        assert_eq!(rec.event_count(), 2);
        let first = rec.events().into_iter().find(|e| e.degree == 0).unwrap();
        assert_eq!(first.duration_ticks, 48);
        let second = rec.events().into_iter().find(|e| e.degree == 4).unwrap();
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
        rec.events.push(prepared(ChordClipEvent {
            start_tick: 0,
            duration_ticks: 48,
            chord_set: 1,
            root: 0,
            scale_type: 0,
            degree: 0,
            voicing: 0,
            preset: 1,
            octave: 4,
            velocity: 0.9,
        }));

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
        rec.events.push(prepared(ChordClipEvent {
            start_tick: 0,
            duration_ticks: 10,
            chord_set: 1,
            root: 0,
            scale_type: 0,
            degree: 0,
            voicing: 0,
            preset: 0,
            octave: 4,
            velocity: 1.0,
        }));
        rec.clear_clip();
        assert_eq!(rec.event_count(), 0);
    }

    #[test]
    fn does_not_record_when_disarmed() {
        let mut rec = PerformanceRecorder::new();
        let _ = rec.update_clock(0.0, true);
        assert!(!rec.record_chord_on(1, 0, 0, 0, 0, 0, 4, 1.0));
        assert_eq!(rec.event_count(), 0);
    }

    #[test]
    fn sorted_cursor_handles_rests_boundaries_and_same_event_phase_corrections() {
        let mut rec = PerformanceRecorder::new();
        let mut retired = Vec::with_capacity(8);
        assert_eq!(
            rec.apply_clip_edit(
                snapshot(1, 96, vec![host_event(24, 24, 0), host_event(72, 12, 4)]),
                false,
                &mut retired,
            ),
            Some(1)
        );

        let update = rec.update_clock_with_transport(0.0, true, 1, &mut retired);
        assert!(update.action.is_none());
        let update = rec.update_clock_with_transport(23.0 / 96.0, true, 1, &mut retired);
        assert!(update.action.is_none());
        let update = rec.update_clock_with_transport(24.0 / 96.0, true, 1, &mut retired);
        assert!(
            matches!(update.action, Some(PlayerAction::Trigger(event)) if event.event.degree == 0)
        );

        // A Link correction that remains inside the same event only moves the cursor.
        let update = rec.update_clock_with_transport(30.0 / 96.0, true, 2, &mut retired);
        assert!(update.action.is_none());
        let update = rec.update_clock_with_transport(48.0 / 96.0, true, 2, &mut retired);
        assert_eq!(update.action, Some(PlayerAction::Release));
        let update = rec.update_clock_with_transport(72.0 / 96.0, true, 2, &mut retired);
        assert!(
            matches!(update.action, Some(PlayerAction::Trigger(event)) if event.event.degree == 4)
        );

        // A backward seek into a different event performs exactly one transition.
        let update = rec.update_clock_with_transport(25.0 / 96.0, true, 3, &mut retired);
        assert!(
            matches!(update.action, Some(PlayerAction::Trigger(event)) if event.event.degree == 0)
        );
        let update = rec.update_clock_with_transport(26.0 / 96.0, true, 4, &mut retired);
        assert!(update.action.is_none());
    }

    #[test]
    fn full_loop_event_survives_wrap_without_overflow_or_retrigger() {
        let mut rec = PerformanceRecorder::new();
        let mut retired = Vec::with_capacity(4);
        let length = u32::MAX;
        assert_eq!(
            rec.apply_clip_edit(
                snapshot(1, length, vec![host_event(length - 10, length, 0)]),
                false,
                &mut retired,
            ),
            Some(1)
        );
        let update = rec.update_clock_with_transport(0.0, true, 1, &mut retired);
        assert!(matches!(update.action, Some(PlayerAction::Trigger(_))));
        let near_wrap = f64::from(length - 1) / f64::from(TICKS_PER_QUARTER);
        assert!(rec
            .update_clock_with_transport(near_wrap, true, 1, &mut retired)
            .action
            .is_none());
        let wrapped = f64::from(length) / f64::from(TICKS_PER_QUARTER);
        assert!(rec
            .update_clock_with_transport(wrapped, true, 1, &mut retired)
            .action
            .is_none());
    }

    #[test]
    fn running_replacement_installs_at_old_wrap_and_uses_global_phase() {
        let mut rec = PerformanceRecorder::new();
        let mut retired = Vec::with_capacity(8);
        assert_eq!(
            rec.apply_clip_edit(
                snapshot(1, 96, vec![host_event(0, 96, 0)]),
                false,
                &mut retired,
            ),
            Some(1)
        );
        let first = rec.update_clock_with_transport(0.0, true, 1, &mut retired);
        assert!(matches!(first.action, Some(PlayerAction::Trigger(_))));

        assert_eq!(
            rec.apply_clip_edit(
                snapshot(2, 192, vec![host_event(90, 20, 5)]),
                true,
                &mut retired,
            ),
            None
        );
        let before = rec.update_clock_with_transport(95.0 / 96.0, true, 1, &mut retired);
        assert_eq!(before.installed_generation, None);
        let wrap = rec.update_clock_with_transport(1.0, true, 1, &mut retired);
        assert_eq!(wrap.installed_generation, Some(2));
        assert!(
            matches!(wrap.action, Some(PlayerAction::Trigger(event)) if event.event.degree == 5)
        );
        assert_eq!(rec.length_ticks(), 192);
        assert_eq!(retired.len(), 1);
    }
}
