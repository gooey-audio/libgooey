//! Looping monophonic note clip for pitched voices (the bass strip).
//!
//! Hosts stage a complete immutable snapshot of timed notes; the render thread
//! installs it at the next buffer boundary and replays it from the mixer's
//! monotonic transport, so the clip stays phase-locked to the chord loop and
//! drum sequencers without the host polling a clock.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::{absolute_beat_tick, TICKS_PER_QUARTER};

/// Maximum events accepted in one note-clip snapshot.
pub const NOTE_CLIP_MAX_EVENTS: usize = 512;
/// Retired snapshots buffered between the render thread and producers.
const MAX_RETIRED_SNAPSHOTS: usize = 32;

/// One timed note. `start_tick` is measured at `TICKS_PER_QUARTER` resolution.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteClipEvent {
    pub start_tick: u32,
    pub duration_ticks: u32,
    pub note: u8,
    pub velocity: f32,
}

#[derive(Debug)]
pub(crate) struct NoteClipSnapshot {
    generation: u64,
    length_ticks: u32,
    events: Vec<NoteClipEvent>,
}

enum NoteClipEdit {
    Replace(Arc<NoteClipSnapshot>),
    Clear { generation: u64 },
}

struct QueueState {
    edit: Option<NoteClipEdit>,
    retired: Vec<Arc<NoteClipSnapshot>>,
}

struct SharedControl {
    queue: Mutex<QueueState>,
    has_pending: AtomicBool,
    next_generation: AtomicU64,
    applied_generation: AtomicU64,
}

/// Producer-side endpoint. Cloning shares the same queue.
#[derive(Clone)]
pub(crate) struct NoteClipControl {
    shared: Arc<SharedControl>,
}

impl NoteClipControl {
    pub(crate) fn new() -> Self {
        Self {
            shared: Arc::new(SharedControl {
                queue: Mutex::new(QueueState {
                    edit: None,
                    retired: Vec::with_capacity(MAX_RETIRED_SNAPSHOTS),
                }),
                has_pending: AtomicBool::new(false),
                next_generation: AtomicU64::new(1),
                applied_generation: AtomicU64::new(0),
            }),
        }
    }

    /// Validate, sort, and stage a snapshot. Returns its generation, or 0 when
    /// the clip is rejected (bad length, out-of-range event, or overlapping
    /// notes on the looping timeline — the voice is monophonic).
    pub(crate) fn replace(&self, mut events: Vec<NoteClipEvent>, length_ticks: u32) -> u64 {
        if length_ticks == 0 || events.len() > NOTE_CLIP_MAX_EVENTS {
            return 0;
        }
        if events.iter().any(|event| {
            event.start_tick >= length_ticks
                || event.duration_ticks == 0
                || event.duration_ticks > length_ticks
                || event.note > 127
                || !event.velocity.is_finite()
        }) {
            return 0;
        }
        events.sort_by_key(|event| event.start_tick);
        for index in 0..events.len() {
            let event = events[index];
            let next_start = match events.get(index + 1) {
                Some(next) => u64::from(next.start_tick),
                None => u64::from(events[0].start_tick) + u64::from(length_ticks),
            };
            if u64::from(event.start_tick) + u64::from(event.duration_ticks) > next_start {
                return 0;
            }
        }
        for event in &mut events {
            event.velocity = event.velocity.clamp(0.0, 1.0);
        }

        let Some(generation) = self.next_generation() else {
            return 0;
        };
        let snapshot = Arc::new(NoteClipSnapshot {
            generation,
            length_ticks,
            events,
        });
        if self.stage(NoteClipEdit::Replace(snapshot)) {
            generation
        } else {
            0
        }
    }

    /// Stage removal of the clip. Returns the generation, or 0 on failure.
    pub(crate) fn clear(&self) -> u64 {
        let Some(generation) = self.next_generation() else {
            return 0;
        };
        if self.stage(NoteClipEdit::Clear { generation }) {
            generation
        } else {
            0
        }
    }

    pub(crate) fn applied_generation(&self) -> u64 {
        self.shared.applied_generation.load(Ordering::Acquire)
    }

    fn stage(&self, edit: NoteClipEdit) -> bool {
        // Drop reaped and superseded snapshots outside the lock.
        let (retired, superseded) = {
            let Ok(mut state) = self.shared.queue.lock() else {
                return false;
            };
            let retired: Vec<_> = state.retired.drain(..).collect();
            let superseded = state.edit.replace(edit);
            self.shared.has_pending.store(true, Ordering::Release);
            (retired, superseded)
        };
        drop(retired);
        drop(superseded);
        true
    }

    fn next_generation(&self) -> Option<u64> {
        self.shared
            .next_generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .ok()
    }

    /// Render-thread boundary work: hand retired snapshots back to producers,
    /// then install the newest staged edit into `player`. Never blocks and
    /// never frees a snapshot on the render thread.
    pub(crate) fn apply_to(&self, player: &mut NoteClipPlayer) {
        self.reclaim(&mut player.retired);
        if !self.shared.has_pending.load(Ordering::Acquire)
            || player.retired.len() >= player.retired.capacity()
        {
            return;
        }
        let Ok(mut state) = self.shared.queue.try_lock() else {
            return;
        };
        let Some(edit) = state.edit.take() else {
            self.shared.has_pending.store(false, Ordering::Release);
            return;
        };
        self.shared.has_pending.store(false, Ordering::Release);
        drop(state);

        let generation = match edit {
            NoteClipEdit::Replace(snapshot) => {
                let generation = snapshot.generation;
                player.install(Some(snapshot));
                generation
            }
            NoteClipEdit::Clear { generation } => {
                player.install(None);
                generation
            }
        };
        self.shared
            .applied_generation
            .store(generation, Ordering::Release);
        self.reclaim(&mut player.retired);
    }

    fn reclaim(&self, retired: &mut Vec<Arc<NoteClipSnapshot>>) {
        if retired.is_empty() {
            return;
        }
        let Ok(mut state) = self.shared.queue.try_lock() else {
            return;
        };
        if state.retired.capacity() - state.retired.len() >= retired.len() {
            state.retired.append(retired);
        }
    }
}

/// Render-side clip player. Owns the active snapshot and a fixed-capacity
/// buffer of retired snapshots awaiting producer-side release.
pub(crate) struct NoteClipPlayer {
    active: Option<Arc<NoteClipSnapshot>>,
    retired: Vec<Arc<NoteClipSnapshot>>,
    last_absolute_tick: Option<u64>,
    last_transport_generation: u64,
}

impl NoteClipPlayer {
    pub(crate) fn new() -> Self {
        Self {
            active: None,
            retired: Vec::with_capacity(MAX_RETIRED_SNAPSHOTS),
            last_absolute_tick: None,
            last_transport_generation: 0,
        }
    }

    fn install(&mut self, snapshot: Option<Arc<NoteClipSnapshot>>) {
        let old = std::mem::replace(&mut self.active, snapshot);
        if let Some(old) = old {
            debug_assert!(self.retired.len() < self.retired.capacity());
            self.retired.push(old);
        }
    }

    /// Advance to the transport position of the current sample. Returns the
    /// note that starts on this sample, if any. Only the tick that the clock
    /// lands on is scanned after a seek or restart, so a jump never replays
    /// the notes it skipped over.
    pub(crate) fn tick(
        &mut self,
        beat_position: f64,
        transport_running: bool,
        transport_generation: u64,
    ) -> Option<NoteClipEvent> {
        let discontinuity = transport_generation != self.last_transport_generation;
        self.last_transport_generation = transport_generation;
        if !transport_running {
            self.last_absolute_tick = None;
            return None;
        }
        let absolute_tick = absolute_beat_tick(beat_position);
        let previous = self.last_absolute_tick.replace(absolute_tick);
        let snapshot = self.active.as_ref()?;
        let first_new_tick = match previous {
            Some(previous) if !discontinuity => {
                if absolute_tick <= previous {
                    return None;
                }
                // Normal playback advances at most one tick per sample; cap
                // the scan so a large forward jump behaves like a seek.
                (previous + 1).max(absolute_tick.saturating_sub(u64::from(TICKS_PER_QUARTER)))
            }
            _ => absolute_tick,
        };
        let length = u64::from(snapshot.length_ticks);
        let mut started = None;
        for tick in first_new_tick..=absolute_tick {
            let phase = (tick % length) as u32;
            if let Ok(index) = snapshot
                .events
                .binary_search_by_key(&phase, |event| event.start_tick)
            {
                started = Some(snapshot.events[index]);
            }
        }
        started
    }

    #[cfg(test)]
    fn active_len(&self) -> Option<usize> {
        self.active.as_ref().map(|snapshot| snapshot.events.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start_tick: u32, duration_ticks: u32, note: u8) -> NoteClipEvent {
        NoteClipEvent {
            start_tick,
            duration_ticks,
            note,
            velocity: 0.8,
        }
    }

    fn beat(tick: u32) -> f64 {
        f64::from(tick) / f64::from(TICKS_PER_QUARTER)
    }

    #[test]
    fn rejects_invalid_and_overlapping_clips() {
        let control = NoteClipControl::new();
        assert_eq!(control.replace(vec![note(0, 24, 36)], 0), 0);
        assert_eq!(control.replace(vec![note(384, 24, 36)], 384), 0);
        assert_eq!(control.replace(vec![note(0, 0, 36)], 384), 0);
        assert_eq!(control.replace(vec![note(0, 24, 128)], 384), 0);
        assert_eq!(
            control.replace(vec![note(0, 100, 36), note(96, 24, 38)], 384),
            0
        );
        // The last note may not wrap into the first.
        assert_eq!(
            control.replace(vec![note(0, 24, 36), note(360, 48, 38)], 384),
            0
        );
        assert!(control.replace(vec![note(96, 96, 36), note(0, 96, 40)], 384) > 0);
    }

    #[test]
    fn newest_edit_installs_and_retired_snapshots_return_to_producers() {
        let control = NoteClipControl::new();
        let mut player = NoteClipPlayer::new();
        control.replace(vec![note(0, 24, 36)], 384);
        let newest = control.replace(vec![note(0, 24, 36), note(96, 24, 43)], 384);
        control.apply_to(&mut player);
        assert_eq!(control.applied_generation(), newest);
        assert_eq!(player.active_len(), Some(2));

        let cleared = control.clear();
        control.apply_to(&mut player);
        assert_eq!(control.applied_generation(), cleared);
        assert_eq!(player.active_len(), None);
        assert!(player.retired.is_empty());
    }

    #[test]
    fn fires_each_note_once_per_pass_across_the_wrap() {
        let control = NoteClipControl::new();
        let mut player = NoteClipPlayer::new();
        control.replace(vec![note(0, 48, 36), note(192, 48, 43)], 384);
        control.apply_to(&mut player);

        let mut fired = Vec::new();
        for tick in 0..(384 * 2) {
            if let Some(event) = player.tick(beat(tick), true, 1) {
                fired.push((tick, event.note));
            }
        }
        assert_eq!(fired, [(0, 36), (192, 43), (384, 36), (576, 43)]);
    }

    #[test]
    fn seeking_does_not_replay_skipped_notes() {
        let control = NoteClipControl::new();
        let mut player = NoteClipPlayer::new();
        control.replace(vec![note(0, 24, 36), note(96, 24, 38)], 384);
        control.apply_to(&mut player);

        assert!(player.tick(beat(10), true, 1).is_none());
        // A transport discontinuity to tick 200 skips the note at 96.
        assert!(player.tick(beat(200), true, 2).is_none());
        assert!(player.tick(beat(201), true, 2).is_none());
        // Stopping forgets position; restarting on a note fires it.
        assert!(player.tick(beat(201), false, 3).is_none());
        assert_eq!(player.tick(beat(96), true, 4).map(|e| e.note), Some(38));
    }
}
