//! Cross-thread control plane for swapping a multi-sample instrument's mapping.
//!
//! Building a [`SampleMap`] means decoding hundreds of WAV files, so it must
//! happen on a control thread. This queue hands the finished map to the render
//! thread, which installs it at a render-buffer boundary. The evicted map is
//! handed back for the control thread to drop, because dropping an `Arc` whose
//! refcount reaches zero would free hundreds of megabytes from inside the audio
//! callback.
//!
//! Mirrors [`crate::instruments::sampler_control`], which does the same for
//! per-pad sampler buffers.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use super::multisample::SampleMap;

/// Multi-sample instruments the FFI engine can host.
pub const MULTISAMPLE_INSTRUMENT_COUNT: usize = 2;

/// A map swap is a single pointer handoff, so a shallow queue is plenty; extra
/// depth would only let a caller stack up maps nobody will ever hear.
const MAX_QUEUED_COMMANDS: usize = 8;

pub(crate) enum MultiSampleCommand {
    SetMap {
        instrument: usize,
        map: Arc<SampleMap>,
    },
}

#[derive(Default)]
struct QueueState {
    commands: VecDeque<MultiSampleCommand>,
    /// Maps evicted by the render thread, dropped only once a producer takes
    /// this lock — never from the audio callback.
    retired: Vec<Arc<SampleMap>>,
}

struct SharedControl {
    queue: Mutex<QueueState>,
    has_commands: AtomicBool,
    committed_generation: [AtomicU32; MULTISAMPLE_INSTRUMENT_COUNT],
}

#[derive(Clone)]
pub(crate) struct MultiSampleControl {
    shared: Arc<SharedControl>,
}

impl MultiSampleControl {
    pub(crate) fn new() -> Self {
        Self {
            shared: Arc::new(SharedControl {
                queue: Mutex::new(QueueState::default()),
                has_commands: AtomicBool::new(false),
                committed_generation: std::array::from_fn(|_| AtomicU32::new(0)),
            }),
        }
    }

    /// Stage a map for `instrument`. A map already queued for the same
    /// instrument that has not crossed a render boundary is replaced
    /// (last write wins) and retired here, on the producer thread.
    pub(crate) fn queue_set_map(&self, instrument: usize, map: Arc<SampleMap>) -> bool {
        if instrument >= MULTISAMPLE_INSTRUMENT_COUNT {
            return false;
        }
        let retired = {
            let Ok(mut state) = self.shared.queue.lock() else {
                return false;
            };
            if let Some(index) = state.commands.iter().position(|command| {
                matches!(command, MultiSampleCommand::SetMap { instrument: pending, .. } if *pending == instrument)
            }) {
                let Some(MultiSampleCommand::SetMap { map: old, .. }) = state.commands.remove(index)
                else {
                    unreachable!("multisample command index must exist")
                };
                state.retired.push(old);
            }
            if state.commands.len() >= MAX_QUEUED_COMMANDS {
                return false;
            }
            state
                .commands
                .push_back(MultiSampleCommand::SetMap { instrument, map });
            self.shared.has_commands.store(true, Ordering::Release);
            (!state.retired.is_empty()).then(|| std::mem::take(&mut state.retired))
        };
        // Drop outside the lock so a large map's teardown does not block the
        // render thread's `try_lock`.
        drop(retired);
        true
    }

    /// Move queued commands into render-thread-owned scratch. Uses `try_lock`,
    /// so a contended buffer simply defers to the next one.
    pub(crate) fn drain_into(&self, scratch: &mut VecDeque<MultiSampleCommand>) {
        if !self.shared.has_commands.load(Ordering::Acquire) {
            return;
        }
        let Ok(mut state) = self.shared.queue.try_lock() else {
            return;
        };
        while let Some(command) = state.commands.pop_front() {
            scratch.push_back(command);
        }
        self.shared.has_commands.store(false, Ordering::Release);
    }

    /// Hand maps evicted on the audio thread back for disposal.
    pub(crate) fn reclaim_from_audio(&self, retired: &mut Vec<Arc<SampleMap>>) {
        if retired.is_empty() {
            return;
        }
        let Ok(mut state) = self.shared.queue.try_lock() else {
            return;
        };
        state.retired.append(retired);
    }

    pub(crate) fn mark_committed(&self, instrument: usize) {
        if let Some(value) = self.shared.committed_generation.get(instrument) {
            value.fetch_add(1, Ordering::Release);
        }
    }

    /// Monotonic counter a host can poll to learn when a queued map became
    /// audible. Zero means nothing has been committed yet.
    pub(crate) fn committed_generation(&self, instrument: usize) -> u32 {
        self.shared
            .committed_generation
            .get(instrument)
            .map_or(0, |value| value.load(Ordering::Acquire))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_write_wins_for_a_pending_instrument() {
        let control = MultiSampleControl::new();
        assert!(control.queue_set_map(0, Arc::new(SampleMap::new())));
        assert!(control.queue_set_map(0, Arc::new(SampleMap::new())));
        assert!(control.queue_set_map(1, Arc::new(SampleMap::new())));

        let mut scratch = VecDeque::new();
        control.drain_into(&mut scratch);
        assert_eq!(scratch.len(), 2, "one command per instrument survives");

        // The queue is drained; a second call yields nothing.
        scratch.clear();
        control.drain_into(&mut scratch);
        assert!(scratch.is_empty());
    }

    #[test]
    fn out_of_range_instruments_are_rejected() {
        let control = MultiSampleControl::new();
        assert!(!control.queue_set_map(MULTISAMPLE_INSTRUMENT_COUNT, Arc::new(SampleMap::new())));
    }

    #[test]
    fn commit_generation_advances_only_on_commit() {
        let control = MultiSampleControl::new();
        assert_eq!(control.committed_generation(0), 0);
        control.mark_committed(0);
        assert_eq!(control.committed_generation(0), 1);
        assert_eq!(control.committed_generation(1), 0);
        // Out-of-range reads are zero, not a panic.
        assert_eq!(control.committed_generation(99), 0);
    }
}
