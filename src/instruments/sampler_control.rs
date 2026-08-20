//! Cross-thread control plane for sampler-pad buffer replacement.
//!
//! Producers copy/decode PCM before enqueueing it here. The render thread owns
//! sampler racks and applies queued replacements only at a render-buffer
//! boundary, so it never races a host/UI thread over live pad storage.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use super::{SamplerBuffer, SAMPLER_SLOT_COUNT};

pub const SAMPLER_RACK_CONTROL_COUNT: usize = 4;
const MAX_QUEUED_COMMANDS: usize = 256;
const MAX_COMMANDS_PER_BOUNDARY: usize = 16;

pub(crate) enum SamplerCommand {
    Replace {
        rack: usize,
        slot: usize,
        buffer: SamplerBuffer,
    },
}

struct QueueState {
    commands: VecDeque<SamplerCommand>,
    /// Buffers evicted by the render thread. They are dropped only after a
    /// producer acquires this queue lock, never from the audio callback.
    retired: Vec<SamplerBuffer>,
}

impl Default for QueueState {
    fn default() -> Self {
        Self {
            commands: VecDeque::with_capacity(MAX_QUEUED_COMMANDS),
            retired: Vec::with_capacity(MAX_QUEUED_COMMANDS),
        }
    }
}

struct SharedControl {
    queue: Mutex<QueueState>,
    has_commands: AtomicBool,
    committed_generation: [[AtomicU32; SAMPLER_SLOT_COUNT]; SAMPLER_RACK_CONTROL_COUNT],
}

#[derive(Clone)]
pub(crate) struct SamplerControl {
    shared: Arc<SharedControl>,
}

impl SamplerControl {
    pub(crate) fn new() -> Self {
        Self {
            shared: Arc::new(SharedControl {
                queue: Mutex::new(QueueState::default()),
                has_commands: AtomicBool::new(false),
                committed_generation: std::array::from_fn(|_| {
                    std::array::from_fn(|_| AtomicU32::new(0))
                }),
            }),
        }
    }

    pub(crate) fn queue_replace(&self, rack: usize, slot: usize, buffer: SamplerBuffer) -> bool {
        if rack >= SAMPLER_RACK_CONTROL_COUNT || slot >= SAMPLER_SLOT_COUNT {
            return false;
        }
        let retired = {
            let Ok(mut state) = self.shared.queue.lock() else {
                return false;
            };
            if state.commands.len() >= MAX_QUEUED_COMMANDS {
                return false;
            }
            // Last-write-wins for a slot that has not crossed a render boundary.
            if let Some(index) = state.commands.iter().position(|command| {
                matches!(command, SamplerCommand::Replace { rack: pending_rack, slot: pending_slot, .. } if *pending_rack == rack && *pending_slot == slot)
            }) {
                let Some(SamplerCommand::Replace { buffer: old, .. }) = state.commands.remove(index) else {
                    unreachable!("sampler command index must exist")
                };
                state.retired.push(old);
            }
            state
                .commands
                .push_back(SamplerCommand::Replace { rack, slot, buffer });
            self.shared.has_commands.store(true, Ordering::Release);
            (!state.retired.is_empty()).then(|| {
                std::mem::replace(&mut state.retired, Vec::with_capacity(MAX_QUEUED_COMMANDS))
            })
        };
        drop(retired);
        true
    }

    pub(crate) fn drain_into(&self, scratch: &mut VecDeque<SamplerCommand>) {
        if !self.shared.has_commands.load(Ordering::Acquire) {
            return;
        }
        let Ok(mut state) = self.shared.queue.try_lock() else {
            return;
        };
        for _ in 0..MAX_COMMANDS_PER_BOUNDARY {
            let Some(command) = state.commands.pop_front() else {
                break;
            };
            scratch.push_back(command);
        }
        self.shared
            .has_commands
            .store(!state.commands.is_empty(), Ordering::Release);
    }

    pub(crate) fn reclaim_from_audio(&self, retired: &mut Vec<SamplerBuffer>) {
        if retired.is_empty() {
            return;
        }
        let Ok(mut state) = self.shared.queue.try_lock() else {
            return;
        };
        state.retired.append(retired);
    }

    pub(crate) fn mark_committed(&self, rack: usize, slot: usize) {
        if let Some(value) = self
            .shared
            .committed_generation
            .get(rack)
            .and_then(|slots| slots.get(slot))
        {
            value.fetch_add(1, Ordering::Release);
        }
    }

    pub(crate) fn committed_generation(&self, rack: usize, slot: usize) -> u32 {
        self.shared
            .committed_generation
            .get(rack)
            .and_then(|slots| slots.get(slot))
            .map_or(0, |value| value.load(Ordering::Acquire))
    }
}
