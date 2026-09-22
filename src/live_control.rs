//! Lock-free control-to-render handoff for the opt-in live-control C ABI.
//!
//! Both rings are single-producer/single-consumer (SPSC). The host serializes
//! submissions through one [`GooeyLiveControl`](crate::ffi::GooeyLiveControl),
//! while the render callback is the only command consumer and retired-rack
//! producer. Acquire/release publication means a consumer never observes a
//! partially initialized slot. No ring operation allocates, locks, or waits.

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use crate::mixer::EffectChain;

pub(crate) const LIVE_QUEUE_CAPACITY: usize = 64;
pub(crate) const DRUM_LANE_COUNT: usize = 4;
pub(crate) const DRUM_STEP_COUNT: usize = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct DrumCell {
    pub(crate) enabled: bool,
    pub(crate) velocity: f32,
}

// Intentionally inline the complete drum snapshot. Boxing it would make the
// render consumer deallocate when the command is applied.
#[allow(clippy::large_enum_variant)]
pub(crate) enum LiveCommand {
    SetTrackGain {
        generation: u64,
        track: usize,
        gain: f32,
    },
    SetSourceTrim {
        generation: u64,
        source: u32,
        trim: f32,
    },
    ReplaceDrumPattern {
        generation: u64,
        lanes: [[DrumCell; DRUM_STEP_COUNT]; DRUM_LANE_COUNT],
    },
    ReplaceTrackRack {
        generation: u64,
        track: usize,
        rack: EffectChain,
    },
    SetTrackEffectParam {
        generation: u64,
        track: usize,
        slot: usize,
        param: u32,
        value: f32,
    },
}

impl LiveCommand {
    pub(crate) fn generation(&self) -> u64 {
        match self {
            Self::SetTrackGain { generation, .. }
            | Self::SetSourceTrim { generation, .. }
            | Self::ReplaceDrumPattern { generation, .. }
            | Self::ReplaceTrackRack { generation, .. }
            | Self::SetTrackEffectParam { generation, .. } => *generation,
        }
    }
}

struct Slot<T> {
    value: UnsafeCell<MaybeUninit<T>>,
}

impl<T> Slot<T> {
    fn new() -> Self {
        Self {
            value: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }
}

/// Fixed-capacity SPSC ring. Exactly one thread may call `push`, and exactly
/// one other thread may call `pop`.
pub(crate) struct SpscRing<T, const N: usize> {
    slots: Box<[Slot<T>]>,
    head: AtomicUsize,
    tail: AtomicUsize,
}

// SAFETY: SPSC ownership prevents simultaneous access to one initialized slot;
// acquire/release atomics publish initialization before reads and reads before
// slot reuse. T must be movable between threads.
unsafe impl<T: Send, const N: usize> Send for SpscRing<T, N> {}
unsafe impl<T: Send, const N: usize> Sync for SpscRing<T, N> {}

impl<T, const N: usize> SpscRing<T, N> {
    pub(crate) fn new() -> Self {
        assert!(N > 0);
        let slots = (0..N)
            .map(|_| Slot::new())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            slots,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    pub(crate) fn push(&self, value: T) -> Result<(), T> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail.wrapping_sub(head) >= N {
            return Err(value);
        }
        // SAFETY: only the producer writes the unpublished tail slot. Capacity
        // checking proves the consumer has released this slot from any prior lap.
        unsafe { (*self.slots[tail % N].value.get()).write(value) };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Ok(())
    }

    pub(crate) fn pop(&self) -> Option<T> {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head == tail {
            return None;
        }
        // SAFETY: the producer's release store published a fully initialized
        // value, and only the consumer reads/moves this head slot.
        let value = unsafe { (*self.slots[head % N].value.get()).assume_init_read() };
        self.head.store(head.wrapping_add(1), Ordering::Release);
        Some(value)
    }

    pub(crate) fn is_full(&self) -> bool {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        tail.wrapping_sub(head) >= N
    }
}

impl<T, const N: usize> Drop for SpscRing<T, N> {
    fn drop(&mut self) {
        while self.pop().is_some() {}
    }
}

/// Shared engine lifetime state. Render entry is a two-phase handshake: a
/// caller increments the active count and rechecks shutdown, so engine free can
/// set shutdown and then wait until every already-entered callback exits.
pub(crate) struct EngineLifecycle {
    shutting_down: AtomicBool,
    active_renders: AtomicUsize,
    control_attached: AtomicBool,
}

impl EngineLifecycle {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            shutting_down: AtomicBool::new(false),
            active_renders: AtomicUsize::new(0),
            control_attached: AtomicBool::new(false),
        })
    }

    pub(crate) fn try_attach_control(&self) -> bool {
        if self.shutting_down.load(Ordering::Acquire) {
            return false;
        }
        self.control_attached
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub(crate) fn detach_control(&self) {
        self.control_attached.store(false, Ordering::Release);
    }

    pub(crate) fn begin_render(self: &Arc<Self>) -> Option<RenderGuard> {
        if self.shutting_down.load(Ordering::Acquire) {
            return None;
        }
        self.active_renders.fetch_add(1, Ordering::AcqRel);
        if self.shutting_down.load(Ordering::Acquire) {
            self.active_renders.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(RenderGuard {
            lifecycle: Arc::clone(self),
        })
    }

    pub(crate) fn begin_shutdown(&self) {
        self.shutting_down.store(true, Ordering::Release);
    }

    pub(crate) fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::Acquire)
    }

    pub(crate) fn has_users(&self) -> bool {
        self.active_renders.load(Ordering::Acquire) != 0
            || self.control_attached.load(Ordering::Acquire)
    }
}

pub(crate) struct RenderGuard {
    lifecycle: Arc<EngineLifecycle>,
}

impl Drop for RenderGuard {
    fn drop(&mut self) {
        self.lifecycle.active_renders.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) struct LiveControlShared {
    pub(crate) lifecycle: Arc<EngineLifecycle>,
    commands: SpscRing<LiveCommand, LIVE_QUEUE_CAPACITY>,
    retired: SpscRing<EffectChain, LIVE_QUEUE_CAPACITY>,
    next_generation: AtomicU64,
    last_applied_generation: AtomicU64,
    rack_busy: Box<[AtomicBool]>,
}

impl LiveControlShared {
    pub(crate) fn new(lifecycle: Arc<EngineLifecycle>, track_count: usize) -> Arc<Self> {
        Arc::new(Self {
            lifecycle,
            commands: SpscRing::new(),
            retired: SpscRing::new(),
            next_generation: AtomicU64::new(1),
            last_applied_generation: AtomicU64::new(0),
            rack_busy: (0..track_count)
                .map(|_| AtomicBool::new(false))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        })
    }

    pub(crate) fn next_generation(&self) -> Option<u64> {
        self.next_generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |generation| {
                generation.checked_add(1).filter(|next| *next != 0)
            })
            .ok()
    }

    #[allow(clippy::result_large_err)]
    pub(crate) fn submit(&self, command: LiveCommand) -> Result<(), LiveCommand> {
        if self.lifecycle.is_shutting_down() {
            return Err(command);
        }
        self.commands.push(command)
    }

    pub(crate) fn queue_is_full(&self) -> bool {
        self.commands.is_full()
    }

    pub(crate) fn pop_command(&self) -> Option<LiveCommand> {
        self.commands.pop()
    }

    pub(crate) fn mark_applied(&self, generation: u64) {
        self.last_applied_generation
            .store(generation, Ordering::Release);
    }

    pub(crate) fn last_applied(&self) -> u64 {
        self.last_applied_generation.load(Ordering::Acquire)
    }

    pub(crate) fn begin_rack_transition(&self, track: usize) -> bool {
        self.rack_busy.get(track).is_some_and(|busy| {
            busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        })
    }

    pub(crate) fn cancel_rack_transition(&self, track: usize) {
        if let Some(busy) = self.rack_busy.get(track) {
            busy.store(false, Ordering::Release);
        }
    }

    pub(crate) fn retire_rack(&self, track: usize, rack: EffectChain) -> Result<(), EffectChain> {
        match self.retired.push(rack) {
            Ok(()) => {
                self.cancel_rack_transition(track);
                Ok(())
            }
            Err(rack) => Err(rack),
        }
    }

    /// Drain on the control thread so effect graph destructors never run in the
    /// render callback.
    pub(crate) fn reap_retired(&self) {
        while let Some(rack) = self.retired.pop() {
            drop(rack);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_is_fifo_and_rejects_when_full() {
        let ring = SpscRing::<u32, 3>::new();
        assert_eq!(ring.push(1), Ok(()));
        assert_eq!(ring.push(2), Ok(()));
        assert_eq!(ring.push(3), Ok(()));
        assert_eq!(ring.push(4), Err(4));
        assert_eq!(ring.pop(), Some(1));
        assert_eq!(ring.push(4), Ok(()));
        assert_eq!(ring.pop(), Some(2));
        assert_eq!(ring.pop(), Some(3));
        assert_eq!(ring.pop(), Some(4));
        assert_eq!(ring.pop(), None);
    }

    #[test]
    fn lifecycle_rejects_new_render_after_shutdown() {
        let lifecycle = EngineLifecycle::new();
        let guard = lifecycle.begin_render().unwrap();
        lifecycle.begin_shutdown();
        assert!(lifecycle.begin_render().is_none());
        assert!(lifecycle.has_users());
        drop(guard);
        assert!(!lifecycle.has_users());
    }
}
