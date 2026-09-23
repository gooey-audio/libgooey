//! Host-to-render handoff for macros and motions.
//!
//! The host locks a mutex to edit projected definitions and enqueue commands;
//! the render thread only `try_lock`s and moves commands into pre-reserved
//! scratch, so it never waits or allocates. Live macro values and motion
//! status are published back through atomics for host UI polling.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use super::macros::{MacroDefinition, MACRO_COUNT};
use super::motion::{MotionDefinition, MotionPhase, MOTION_SLOT_COUNT};

pub(crate) const AUTOMATION_QUEUE_CAPACITY: usize = 128;

// Definitions travel inline so applying a command never frees memory on the
// render thread.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum AutomationCommand {
    ReplaceMacro {
        index: usize,
        definition: MacroDefinition,
    },
    /// Manual macro gesture. Stops any motion driving the macro.
    SetMacroValue {
        index: usize,
        value: f32,
    },
    StartMotion {
        slot: usize,
        definition: MotionDefinition,
    },
    StopMotion {
        slot: usize,
    },
    StopAll,
}

pub(crate) struct AutomationState {
    pub(crate) macros: [MacroDefinition; MACRO_COUNT],
    pub(crate) motions: [Option<MotionDefinition>; MOTION_SLOT_COUNT],
    commands: VecDeque<AutomationCommand>,
}

impl AutomationState {
    pub(crate) fn has_room(&self, commands: usize) -> bool {
        self.commands.len() + commands <= AUTOMATION_QUEUE_CAPACITY
    }

    /// Enqueue a command. Consecutive manual values for the same macro
    /// coalesce so a fast knob drag cannot fill the queue.
    pub(crate) fn push(&mut self, command: AutomationCommand) -> bool {
        if let (
            AutomationCommand::SetMacroValue { index, value },
            Some(AutomationCommand::SetMacroValue {
                index: last_index,
                value: last_value,
            }),
        ) = (command, self.commands.back_mut())
        {
            if *last_index == index {
                *last_value = value;
                return true;
            }
        }
        if self.commands.len() >= AUTOMATION_QUEUE_CAPACITY {
            return false;
        }
        self.commands.push_back(command);
        true
    }
}

struct Shared {
    state: Mutex<AutomationState>,
    has_pending: AtomicBool,
    macro_values: [AtomicU32; MACRO_COUNT],
    motion_phases: [AtomicU32; MOTION_SLOT_COUNT],
    motion_progress: [AtomicU32; MOTION_SLOT_COUNT],
}

#[derive(Clone)]
pub(crate) struct AutomationControl {
    shared: Arc<Shared>,
}

impl Default for AutomationControl {
    fn default() -> Self {
        Self::new()
    }
}

impl AutomationControl {
    pub(crate) fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(AutomationState {
                    macros: [MacroDefinition::default(); MACRO_COUNT],
                    motions: [None; MOTION_SLOT_COUNT],
                    commands: VecDeque::with_capacity(AUTOMATION_QUEUE_CAPACITY),
                }),
                has_pending: AtomicBool::new(false),
                macro_values: std::array::from_fn(|_| AtomicU32::new(0.0_f32.to_bits())),
                motion_phases: std::array::from_fn(|_| AtomicU32::new(0)),
                motion_progress: std::array::from_fn(|_| AtomicU32::new(0.0_f32.to_bits())),
            }),
        }
    }

    /// Run a host-side edit under the producer lock. The closure returns
    /// whether it enqueued anything the render thread must see.
    pub(crate) fn edit<R>(&self, edit: impl FnOnce(&mut AutomationState) -> R) -> Option<R> {
        let mut state = self.lock()?;
        let result = edit(&mut state);
        if !state.commands.is_empty() {
            self.shared.has_pending.store(true, Ordering::Release);
        }
        Some(result)
    }

    pub(crate) fn read<R>(&self, read: impl FnOnce(&AutomationState) -> R) -> Option<R> {
        self.lock().map(|state| read(&state))
    }

    fn lock(&self) -> Option<MutexGuard<'_, AutomationState>> {
        self.shared.state.lock().ok()
    }

    /// Move queued commands into render-owned scratch. Never waits: a
    /// contended lock leaves the batch queued for the next buffer.
    pub(crate) fn drain_into(&self, destination: &mut VecDeque<AutomationCommand>) {
        if !self.shared.has_pending.load(Ordering::Acquire) {
            return;
        }
        let Ok(mut state) = self.shared.state.try_lock() else {
            return;
        };
        while destination.len() < destination.capacity() {
            let Some(command) = state.commands.pop_front() else {
                break;
            };
            destination.push_back(command);
        }
        self.shared
            .has_pending
            .store(!state.commands.is_empty(), Ordering::Release);
    }

    pub(crate) fn publish_macro_value(&self, index: usize, value: f32) {
        if let Some(slot) = self.shared.macro_values.get(index) {
            slot.store(value.to_bits(), Ordering::Release);
        }
    }

    pub(crate) fn macro_value(&self, index: usize) -> Option<f32> {
        self.shared
            .macro_values
            .get(index)
            .map(|slot| f32::from_bits(slot.load(Ordering::Acquire)))
    }

    pub(crate) fn publish_motion(&self, slot: usize, phase: MotionPhase, progress: f32) {
        if slot < MOTION_SLOT_COUNT {
            self.shared.motion_phases[slot].store(phase.as_u32(), Ordering::Release);
            self.shared.motion_progress[slot].store(progress.to_bits(), Ordering::Release);
        }
    }

    pub(crate) fn motion_phase(&self, slot: usize) -> Option<u32> {
        self.shared
            .motion_phases
            .get(slot)
            .map(|phase| phase.load(Ordering::Acquire))
    }

    pub(crate) fn motion_progress(&self, slot: usize) -> Option<f32> {
        self.shared
            .motion_progress
            .get(slot)
            .map(|progress| f32::from_bits(progress.load(Ordering::Acquire)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_values_coalesce_per_macro() {
        let control = AutomationControl::new();
        control.edit(|state| {
            for step in 0..1000 {
                assert!(state.push(AutomationCommand::SetMacroValue {
                    index: 2,
                    value: step as f32 / 1000.0,
                }));
            }
        });
        let mut scratch = VecDeque::with_capacity(AUTOMATION_QUEUE_CAPACITY);
        control.drain_into(&mut scratch);
        assert_eq!(scratch.len(), 1);
        assert!(matches!(
            scratch[0],
            AutomationCommand::SetMacroValue { index: 2, value } if (value - 0.999).abs() < 1e-6
        ));
    }

    #[test]
    fn queue_rejects_when_full() {
        let control = AutomationControl::new();
        let accepted = control
            .edit(|state| {
                (0..AUTOMATION_QUEUE_CAPACITY + 5)
                    .filter(|&slot| {
                        state.push(AutomationCommand::StopMotion {
                            slot: slot % MOTION_SLOT_COUNT,
                        })
                    })
                    .count()
            })
            .unwrap();
        assert_eq!(accepted, AUTOMATION_QUEUE_CAPACITY);
    }

    #[test]
    fn contended_drain_defers_without_waiting() {
        let control = AutomationControl::new();
        control.edit(|state| state.push(AutomationCommand::StopAll));
        let mut scratch = VecDeque::with_capacity(AUTOMATION_QUEUE_CAPACITY);
        {
            let _guard = control.shared.state.lock().unwrap();
            control.drain_into(&mut scratch);
            assert!(scratch.is_empty());
        }
        control.drain_into(&mut scratch);
        assert_eq!(scratch.len(), 1);
    }
}
