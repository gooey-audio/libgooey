//! Nonblocking host-to-render control plane for chord gestures and loop clips.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::{
    ChordClipEdit, ChordCommandAction, ChordLoopSnapshot, PreparedChordEvent,
    CHORD_LOOP_MAX_EVENTS, CHORD_TARGET_PIANO, CHORD_TARGET_POLY,
};

const MAX_ACTIONS: usize = 64;
const MAX_RETIRED_SNAPSHOTS: usize = 32;
const PIANO_COUNT: usize = 2;

struct QueueState {
    actions: VecDeque<ChordCommandAction>,
    edit: Option<ChordClipEdit>,
    retired: Vec<Arc<ChordLoopSnapshot>>,
}

impl Default for QueueState {
    fn default() -> Self {
        Self {
            actions: VecDeque::with_capacity(MAX_ACTIONS),
            edit: None,
            retired: Vec::with_capacity(MAX_RETIRED_SNAPSHOTS),
        }
    }
}

struct SharedControl {
    queue: Mutex<QueueState>,
    has_pending: AtomicBool,
    next_generation: AtomicU64,
    applied_generation: AtomicU64,
    piano_registered: [AtomicBool; PIANO_COUNT],
}

#[derive(Clone)]
pub(crate) struct ChordControl {
    shared: Arc<SharedControl>,
}

pub(crate) struct ChordControlScratch {
    pub(crate) actions: VecDeque<ChordCommandAction>,
    pub(crate) edit: Option<ChordClipEdit>,
}

impl Default for ChordControlScratch {
    fn default() -> Self {
        Self {
            actions: VecDeque::with_capacity(MAX_ACTIONS),
            edit: None,
        }
    }
}

impl ChordControl {
    pub(crate) fn new() -> Self {
        Self {
            shared: Arc::new(SharedControl {
                queue: Mutex::new(QueueState::default()),
                has_pending: AtomicBool::new(false),
                next_generation: AtomicU64::new(1),
                applied_generation: AtomicU64::new(0),
                piano_registered: std::array::from_fn(|_| AtomicBool::new(false)),
            }),
        }
    }

    pub(crate) fn set_piano_registered(&self, piano: usize) {
        if let Some(value) = self.shared.piano_registered.get(piano) {
            value.store(true, Ordering::Release);
        }
    }

    pub(crate) fn target_is_valid(&self, event: &PreparedChordEvent) -> bool {
        match event.target {
            CHORD_TARGET_POLY => event.event.preset < 5,
            CHORD_TARGET_PIANO => self
                .shared
                .piano_registered
                .get(event.target_id as usize)
                .is_some_and(|registered| registered.load(Ordering::Acquire)),
            _ => false,
        }
    }

    fn reap_retired(state: &mut QueueState) -> Vec<Arc<ChordLoopSnapshot>> {
        state.retired.drain(..).collect()
    }

    pub(crate) fn enqueue_trigger(&self, event: PreparedChordEvent) -> bool {
        if !self.target_is_valid(&event) {
            return false;
        }
        let retired = {
            let Ok(mut state) = self.shared.queue.lock() else {
                return false;
            };
            if state.actions.len() >= MAX_ACTIONS {
                return false;
            }
            let retired = Self::reap_retired(&mut state);
            state.actions.push_back(ChordCommandAction::Trigger(event));
            self.shared.has_pending.store(true, Ordering::Release);
            retired
        };
        drop(retired);
        true
    }

    pub(crate) fn enqueue_release_all(&self) -> bool {
        let retired = {
            let Ok(mut state) = self.shared.queue.lock() else {
                return false;
            };
            if state.actions.len() >= MAX_ACTIONS {
                return false;
            }
            let retired = Self::reap_retired(&mut state);
            state.actions.push_back(ChordCommandAction::ReleaseAll);
            self.shared.has_pending.store(true, Ordering::Release);
            retired
        };
        drop(retired);
        true
    }

    pub(crate) fn replace(&self, mut events: Vec<PreparedChordEvent>, length_ticks: u32) -> u64 {
        if length_ticks == 0 || events.len() > CHORD_LOOP_MAX_EVENTS {
            return 0;
        }
        if events.iter().any(|event| {
            event.event.start_tick >= length_ticks
                || event.event.duration_ticks == 0
                || event.event.duration_ticks > length_ticks
                || !self.target_is_valid(event)
        }) {
            return 0;
        }
        events.sort_by_key(|event| event.event.start_tick);
        if !events.is_empty() {
            for index in 0..events.len() {
                let event = &events[index].event;
                let next_start = if index + 1 < events.len() {
                    u64::from(events[index + 1].event.start_tick)
                } else {
                    events
                        .first()
                        .map_or(0, |first| u64::from(first.event.start_tick))
                        + u64::from(length_ticks)
                };
                if u64::from(event.start_tick) + u64::from(event.duration_ticks) > next_start {
                    return 0;
                }
            }
        }

        let Some(generation) = self.next_generation() else {
            return 0;
        };
        let snapshot = Arc::new(ChordLoopSnapshot {
            generation,
            length_ticks,
            events,
        });
        let (retired, superseded) = {
            let Ok(mut state) = self.shared.queue.lock() else {
                return 0;
            };
            let retired = Self::reap_retired(&mut state);
            let superseded = state.edit.replace(ChordClipEdit::Replace(snapshot));
            self.shared.has_pending.store(true, Ordering::Release);
            (retired, superseded)
        };
        drop(retired);
        drop(superseded);
        generation
    }

    pub(crate) fn clear(&self) -> u64 {
        let Some(generation) = self.next_generation() else {
            return 0;
        };
        let (retired, superseded) = {
            let Ok(mut state) = self.shared.queue.lock() else {
                return 0;
            };
            let retired = Self::reap_retired(&mut state);
            let superseded = state.edit.replace(ChordClipEdit::Clear { generation });
            self.shared.has_pending.store(true, Ordering::Release);
            (retired, superseded)
        };
        drop(retired);
        drop(superseded);
        generation
    }

    fn next_generation(&self) -> Option<u64> {
        self.shared
            .next_generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .ok()
    }

    pub(crate) fn applied_generation(&self) -> u64 {
        self.shared.applied_generation.load(Ordering::Acquire)
    }

    pub(crate) fn mark_applied(&self, generation: u64) {
        self.shared
            .applied_generation
            .store(generation, Ordering::Release);
    }

    pub(crate) fn drain_into(&self, scratch: &mut ChordControlScratch) {
        if !self.shared.has_pending.load(Ordering::Acquire) {
            return;
        }
        let Ok(mut state) = self.shared.queue.try_lock() else {
            return;
        };
        while let Some(action) = state.actions.pop_front() {
            scratch.actions.push_back(action);
        }
        if state.edit.is_some() {
            let displaced_is_snapshot = matches!(scratch.edit, Some(ChordClipEdit::Replace(_)));
            if !displaced_is_snapshot || state.retired.len() < state.retired.capacity() {
                if let Some(newest) = state.edit.take() {
                    if let Some(ChordClipEdit::Replace(snapshot)) = scratch.edit.replace(newest) {
                        // The render thread never releases the final Arc for a
                        // superseded audio-pending snapshot. Producer code reaps it.
                        state.retired.push(snapshot);
                    }
                }
            }
        }
        self.shared
            .has_pending
            .store(state.edit.is_some(), Ordering::Release);
    }

    /// Return audio-owned snapshots to a fixed producer-side buffer. Capacity
    /// is checked before append so this operation cannot allocate in render.
    pub(crate) fn reclaim_from_audio(&self, retired: &mut Vec<Arc<ChordLoopSnapshot>>) -> bool {
        if retired.is_empty() {
            return true;
        }
        let Ok(mut state) = self.shared.queue.try_lock() else {
            return false;
        };
        if state.retired.capacity() - state.retired.len() < retired.len() {
            return false;
        }
        state.retired.append(retired);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::performance::prepare_chord_event;

    fn event(start: u32, duration: u32) -> PreparedChordEvent {
        let mut event = prepare_chord_event(0, 0, 0, 0, 0, 0, 0, 0, 4, 0.8).unwrap();
        event.event.start_tick = start;
        event.event.duration_ticks = duration;
        event
    }

    #[test]
    fn sorts_and_rejects_cyclic_overlap_atomically() {
        let control = ChordControl::new();
        let generation = control.replace(vec![event(96, 48), event(0, 48)], 192);
        assert_ne!(generation, 0);
        let mut scratch = ChordControlScratch::default();
        control.drain_into(&mut scratch);
        let Some(ChordClipEdit::Replace(snapshot)) = scratch.edit.take() else {
            panic!("expected replacement");
        };
        assert_eq!(snapshot.events[0].event.start_tick, 0);
        assert_eq!(snapshot.events[1].event.start_tick, 96);

        assert_eq!(control.replace(vec![event(180, 24), event(0, 20)], 192), 0);
    }

    #[test]
    fn repeated_replacements_keep_only_newest_generation() {
        let control = ChordControl::new();
        let first = control.replace(vec![event(0, 10)], 96);
        let second = control.replace(vec![event(12, 10)], 96);
        assert!(second > first);
        let mut scratch = ChordControlScratch::default();
        control.drain_into(&mut scratch);
        let Some(ChordClipEdit::Replace(snapshot)) = scratch.edit else {
            panic!("expected replacement");
        };
        assert_eq!(snapshot.generation, second);
    }

    #[test]
    fn newer_edit_retires_an_audio_pending_snapshot_without_dropping_it() {
        let control = ChordControl::new();
        let first = control.replace(vec![event(0, 10)], 96);
        let mut scratch = ChordControlScratch::default();
        control.drain_into(&mut scratch);
        assert!(matches!(
            scratch.edit,
            Some(ChordClipEdit::Replace(ref snapshot)) if snapshot.generation == first
        ));

        let newest = control.clear();
        control.drain_into(&mut scratch);
        assert!(matches!(
            scratch.edit,
            Some(ChordClipEdit::Clear { generation }) if generation == newest
        ));
        let state = control.shared.queue.lock().unwrap();
        assert_eq!(state.retired.len(), 1);
        assert_eq!(state.retired[0].generation, first);
    }

    #[test]
    fn contended_drain_defers_immediately() {
        let control = ChordControl::new();
        assert!(control.enqueue_release_all());
        let _guard = control.shared.queue.lock().unwrap();
        let mut scratch = ChordControlScratch::default();
        control.drain_into(&mut scratch);
        assert!(scratch.actions.is_empty());
    }
}
