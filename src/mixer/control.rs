//! Cross-thread control plane for the loop mixer.
//!
//! The audio thread owns `Mixer`'s DSP state. Hosts use `MixerControl` to
//! submit commands and read snapshots without touching that state directly.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::{
    ChannelEffect, LaunchQuantization, PitchMode, RetrimTiming, StereoSampleBuffer,
    CLIP_COLUMN_COUNT, CLIP_ROW_COUNT, LOOP_CHANNEL_COUNT,
};

/// Maximum number of control operations the audio thread applies in one sample.
/// This bounds command work during a producer burst; remaining operations keep
/// their FIFO order for the following samples.
pub(crate) const MAX_COMMANDS_PER_TICK: usize = 64;
const MAX_QUEUED_COMMANDS: usize = 4096;

/// A control operation which can mutate mixer state. These values are built on
/// host/control threads and consumed only by the audio thread.
pub(crate) enum MixerCommand {
    Load {
        channel: usize,
        buffer: StereoSampleBuffer,
    },
    SetPlaying {
        channel: usize,
        playing: bool,
    },
    SetGain {
        channel: usize,
        gain: f32,
    },
    SetMuted {
        channel: usize,
        muted: bool,
    },
    SetSoloed {
        channel: usize,
        soloed: bool,
    },
    SetLoopStart {
        channel: usize,
        value: f32,
    },
    SetLoopEnd {
        channel: usize,
        value: f32,
    },
    SetSpeed {
        channel: usize,
        speed: f32,
    },
    SetSourceBpm {
        channel: usize,
        bpm: Option<f32>,
    },
    SetPitchMode {
        channel: usize,
        mode: PitchMode,
    },
    Restart {
        channel: usize,
    },
    SetPosition {
        channel: usize,
        value: f32,
    },
    QueueSwap {
        channel: usize,
        buffer: StereoSampleBuffer,
        divisions: u32,
    },
    CancelQueuedSwap {
        channel: usize,
    },
    SetBpm {
        bpm: f32,
    },
    ClipLoad {
        column: usize,
        row: usize,
        buffer: StereoSampleBuffer,
        source_bpm: f32,
    },
    ClipUnload {
        column: usize,
        row: usize,
    },
    ClipClear,
    ClipLaunch {
        column: usize,
        row: usize,
        quantization: LaunchQuantization,
    },
    ClipLaunchAt {
        column: usize,
        row: usize,
        beat: f64,
    },
    ClipLaunchScene {
        row: usize,
        quantization: LaunchQuantization,
    },
    ClipLaunchSceneAt {
        row: usize,
        beat: f64,
    },
    ClipStop {
        column: usize,
        quantization: LaunchQuantization,
    },
    ClipStopAt {
        column: usize,
        beat: f64,
    },
    ClipCancel {
        column: usize,
    },
    ClipCancelAll,
    ClipSetDefaultQuantization {
        quantization: LaunchQuantization,
    },
    ClipSetTrim {
        column: usize,
        row: usize,
        start: f64,
        end: f64,
        timing: RetrimTiming,
    },
    TransportStart,
    TransportStop,
    TransportSeek {
        beat: f64,
    },
    TransportReset,
    EffectAdd {
        channel: usize,
        effect_id: u32,
    },
    EffectRemove {
        channel: usize,
        slot: usize,
    },
    EffectMove {
        channel: usize,
        slot: usize,
        new_position: usize,
    },
    EffectClear {
        channel: usize,
    },
    EffectSetParam {
        channel: usize,
        slot: usize,
        param: u32,
        value: f32,
    },
}

struct QueueState {
    commands: VecDeque<MixerCommand>,
    /// Desired effect layouts, updated under the same lock that establishes
    /// command order. This lets callers queue `add` then `set_param` before a
    /// render boundary without reading the live effect `Vec`.
    effect_layouts: [Vec<u32>; LOOP_CHANNEL_COUNT],
}

impl Default for QueueState {
    fn default() -> Self {
        Self {
            commands: VecDeque::new(),
            effect_layouts: std::array::from_fn(|_| Vec::new()),
        }
    }
}

/// Atomically published, last-audio-tick state used by FFI getters.
pub(crate) struct MixerSnapshot {
    generation: AtomicU64,
    pub(crate) loop_position: [AtomicU32; LOOP_CHANNEL_COUNT],
    pub(crate) loop_source_bpm: [AtomicU32; LOOP_CHANNEL_COUNT],
    pub(crate) loop_pitch_mode: [AtomicU32; LOOP_CHANNEL_COUNT],
    pub(crate) swaps_completed: [AtomicU32; LOOP_CHANNEL_COUNT],
    pub(crate) default_quantization: AtomicU32,
    pub(crate) slot_state: [[AtomicU32; CLIP_ROW_COUNT]; CLIP_COLUMN_COUNT],
    pub(crate) trim_start: [[AtomicU64; CLIP_ROW_COUNT]; CLIP_COLUMN_COUNT],
    pub(crate) trim_end: [[AtomicU64; CLIP_ROW_COUNT]; CLIP_COLUMN_COUNT],
    pub(crate) active_row: [AtomicI32; CLIP_COLUMN_COUNT],
    pub(crate) queued_row: [AtomicI32; CLIP_COLUMN_COUNT],
    pub(crate) stop_queued: [AtomicBool; CLIP_COLUMN_COUNT],
    pub(crate) scheduled_beat: [AtomicU64; CLIP_COLUMN_COUNT],
    pub(crate) active_playhead: [AtomicU64; CLIP_COLUMN_COUNT],
    pub(crate) transport_beat: AtomicU64,
}

impl MixerSnapshot {
    pub(crate) fn new() -> Self {
        Self {
            generation: AtomicU64::new(0),
            loop_position: std::array::from_fn(|_| AtomicU32::new(0)),
            loop_source_bpm: std::array::from_fn(|_| AtomicU32::new(0)),
            loop_pitch_mode: std::array::from_fn(|_| AtomicU32::new(0)),
            swaps_completed: std::array::from_fn(|_| AtomicU32::new(0)),
            default_quantization: AtomicU32::new(2),
            slot_state: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU32::new(0))),
            trim_start: std::array::from_fn(|_| {
                std::array::from_fn(|_| AtomicU64::new((-1.0_f64).to_bits()))
            }),
            trim_end: std::array::from_fn(|_| {
                std::array::from_fn(|_| AtomicU64::new((-1.0_f64).to_bits()))
            }),
            active_row: std::array::from_fn(|_| AtomicI32::new(-1)),
            queued_row: std::array::from_fn(|_| AtomicI32::new(-1)),
            stop_queued: std::array::from_fn(|_| AtomicBool::new(false)),
            scheduled_beat: std::array::from_fn(|_| AtomicU64::new((-1.0_f64).to_bits())),
            active_playhead: std::array::from_fn(|_| AtomicU64::new((-1.0_f64).to_bits())),
            transport_beat: AtomicU64::new(0.0_f64.to_bits()),
        }
    }

    pub(crate) fn begin_write(&self) {
        self.generation.fetch_add(1, Ordering::Release);
    }

    pub(crate) fn finish_write(&self) {
        self.generation.fetch_add(1, Ordering::Release);
    }

    fn read<T: Copy>(&self, value: impl Fn() -> T) -> T {
        loop {
            let before = self.generation.load(Ordering::Acquire);
            if before & 1 != 0 {
                continue;
            }
            let result = value();
            let after = self.generation.load(Ordering::Acquire);
            if before == after {
                return result;
            }
        }
    }

    pub(crate) fn position(&self, channel: usize) -> f32 {
        self.read(|| {
            self.loop_position
                .get(channel)
                .map_or(0.0, |v| f32::from_bits(v.load(Ordering::Relaxed)))
        })
    }
    pub(crate) fn source_bpm(&self, channel: usize) -> f32 {
        self.read(|| {
            self.loop_source_bpm
                .get(channel)
                .map_or(0.0, |v| f32::from_bits(v.load(Ordering::Relaxed)))
        })
    }
    pub(crate) fn pitch_mode(&self, channel: usize) -> PitchMode {
        self.read(|| {
            match self
                .loop_pitch_mode
                .get(channel)
                .map_or(0, |v| v.load(Ordering::Relaxed))
            {
                1 => PitchMode::Resample,
                2 => PitchMode::PreservePitch,
                _ => PitchMode::Off,
            }
        })
    }
    pub(crate) fn swaps_completed(&self, channel: usize) -> u32 {
        self.read(|| {
            self.swaps_completed
                .get(channel)
                .map_or(0, |v| v.load(Ordering::Relaxed))
        })
    }
    pub(crate) fn default_quantization(&self) -> u32 {
        self.read(|| self.default_quantization.load(Ordering::Relaxed))
    }
    pub(crate) fn slot_state(&self, column: usize, row: usize) -> u32 {
        self.read(|| {
            self.slot_state
                .get(column)
                .and_then(|c| c.get(row))
                .map_or(0, |v| v.load(Ordering::Relaxed))
        })
    }
    pub(crate) fn active_row(&self, column: usize) -> i32 {
        self.read(|| {
            self.active_row
                .get(column)
                .map_or(-1, |v| v.load(Ordering::Relaxed))
        })
    }
    pub(crate) fn queued_row(&self, column: usize) -> i32 {
        self.read(|| {
            self.queued_row
                .get(column)
                .map_or(-1, |v| v.load(Ordering::Relaxed))
        })
    }
    pub(crate) fn stop_queued(&self, column: usize) -> bool {
        self.read(|| {
            self.stop_queued
                .get(column)
                .is_some_and(|v| v.load(Ordering::Relaxed))
        })
    }
    pub(crate) fn scheduled_beat(&self, column: usize) -> f64 {
        self.read(|| {
            self.scheduled_beat
                .get(column)
                .map_or(-1.0, |v| f64::from_bits(v.load(Ordering::Relaxed)))
        })
    }
    pub(crate) fn active_playhead(&self, column: usize) -> f64 {
        self.read(|| {
            self.active_playhead
                .get(column)
                .map_or(-1.0, |v| f64::from_bits(v.load(Ordering::Relaxed)))
        })
    }
    pub(crate) fn trim_start(&self, column: usize, row: usize) -> f64 {
        self.read(|| {
            self.trim_start
                .get(column)
                .and_then(|c| c.get(row))
                .map_or(-1.0, |v| f64::from_bits(v.load(Ordering::Relaxed)))
        })
    }
    pub(crate) fn trim_end(&self, column: usize, row: usize) -> f64 {
        self.read(|| {
            self.trim_end
                .get(column)
                .and_then(|c| c.get(row))
                .map_or(-1.0, |v| f64::from_bits(v.load(Ordering::Relaxed)))
        })
    }
    pub(crate) fn transport_beat(&self) -> f64 {
        self.read(|| f64::from_bits(self.transport_beat.load(Ordering::Relaxed)))
    }
}

pub(crate) struct SharedControl {
    queue: Mutex<QueueState>,
    pub(crate) has_commands: AtomicBool,
    pub(crate) snapshot: MixerSnapshot,
}

/// Cloneable, thread-safe control endpoint. It intentionally contains no live
/// channel or grid state.
#[derive(Clone)]
pub struct MixerControl {
    pub(crate) shared: Arc<SharedControl>,
}

impl MixerControl {
    pub(crate) fn new() -> Self {
        Self {
            shared: Arc::new(SharedControl {
                queue: Mutex::new(QueueState::default()),
                has_commands: AtomicBool::new(false),
                snapshot: MixerSnapshot::new(),
            }),
        }
    }

    pub(crate) fn enqueue(&self, command: MixerCommand) -> bool {
        let Ok(mut state) = self.shared.queue.lock() else {
            return false;
        };
        if state.commands.len() >= MAX_QUEUED_COMMANDS {
            return false;
        }
        state.commands.push_back(command);
        self.shared.has_commands.store(true, Ordering::Release);
        true
    }

    pub fn load(&self, channel: usize, buffer: StereoSampleBuffer) -> bool {
        self.enqueue(MixerCommand::Load { channel, buffer })
    }
    pub fn set_playing(&self, channel: usize, playing: bool) -> bool {
        self.enqueue(MixerCommand::SetPlaying { channel, playing })
    }
    pub fn set_gain(&self, channel: usize, gain: f32) -> bool {
        self.enqueue(MixerCommand::SetGain { channel, gain })
    }
    pub fn set_muted(&self, channel: usize, muted: bool) -> bool {
        self.enqueue(MixerCommand::SetMuted { channel, muted })
    }
    pub fn set_soloed(&self, channel: usize, soloed: bool) -> bool {
        self.enqueue(MixerCommand::SetSoloed { channel, soloed })
    }
    pub fn set_loop_start(&self, channel: usize, value: f32) -> bool {
        self.enqueue(MixerCommand::SetLoopStart { channel, value })
    }
    pub fn set_loop_end(&self, channel: usize, value: f32) -> bool {
        self.enqueue(MixerCommand::SetLoopEnd { channel, value })
    }
    pub fn set_speed(&self, channel: usize, speed: f32) -> bool {
        self.enqueue(MixerCommand::SetSpeed { channel, speed })
    }
    pub fn set_source_bpm(&self, channel: usize, bpm: Option<f32>) -> bool {
        self.enqueue(MixerCommand::SetSourceBpm { channel, bpm })
    }
    pub fn set_pitch_mode(&self, channel: usize, mode: PitchMode) -> bool {
        self.enqueue(MixerCommand::SetPitchMode { channel, mode })
    }
    pub fn restart(&self, channel: usize) -> bool {
        self.enqueue(MixerCommand::Restart { channel })
    }
    pub fn set_position(&self, channel: usize, value: f32) -> bool {
        self.enqueue(MixerCommand::SetPosition { channel, value })
    }
    pub fn queue_swap(&self, channel: usize, buffer: StereoSampleBuffer, divisions: u32) -> bool {
        self.enqueue(MixerCommand::QueueSwap {
            channel,
            buffer,
            divisions,
        })
    }
    pub fn cancel_queued_swap(&self, channel: usize) -> bool {
        self.enqueue(MixerCommand::CancelQueuedSwap { channel })
    }
    pub fn set_bpm(&self, bpm: f32) -> bool {
        self.enqueue(MixerCommand::SetBpm { bpm })
    }
    pub fn clip_load(
        &self,
        column: usize,
        row: usize,
        buffer: StereoSampleBuffer,
        source_bpm: f32,
    ) -> bool {
        self.enqueue(MixerCommand::ClipLoad {
            column,
            row,
            buffer,
            source_bpm,
        })
    }
    pub fn clip_unload(&self, column: usize, row: usize) -> bool {
        self.enqueue(MixerCommand::ClipUnload { column, row })
    }
    pub fn clip_clear(&self) -> bool {
        self.enqueue(MixerCommand::ClipClear)
    }
    pub fn clip_launch(&self, column: usize, row: usize, quantization: LaunchQuantization) -> bool {
        self.enqueue(MixerCommand::ClipLaunch {
            column,
            row,
            quantization,
        })
    }
    pub fn clip_launch_at(&self, column: usize, row: usize, beat: f64) -> bool {
        if !beat.is_finite() || beat + 1.0e-9 < self.snapshot().transport_beat() {
            return false;
        }
        self.enqueue(MixerCommand::ClipLaunchAt { column, row, beat })
    }
    pub fn clip_launch_scene(&self, row: usize, quantization: LaunchQuantization) -> bool {
        self.enqueue(MixerCommand::ClipLaunchScene { row, quantization })
    }
    pub fn clip_launch_scene_at(&self, row: usize, beat: f64) -> bool {
        if !beat.is_finite() || beat + 1.0e-9 < self.snapshot().transport_beat() {
            return false;
        }
        self.enqueue(MixerCommand::ClipLaunchSceneAt { row, beat })
    }
    pub fn clip_stop(&self, column: usize, quantization: LaunchQuantization) -> bool {
        self.enqueue(MixerCommand::ClipStop {
            column,
            quantization,
        })
    }
    pub fn clip_stop_at(&self, column: usize, beat: f64) -> bool {
        if !beat.is_finite() || beat + 1.0e-9 < self.snapshot().transport_beat() {
            return false;
        }
        self.enqueue(MixerCommand::ClipStopAt { column, beat })
    }
    pub fn clip_cancel(&self, column: usize) -> bool {
        self.enqueue(MixerCommand::ClipCancel { column })
    }
    pub fn clip_cancel_all(&self) -> bool {
        self.enqueue(MixerCommand::ClipCancelAll)
    }
    pub fn clip_set_default_quantization(&self, quantization: LaunchQuantization) -> bool {
        self.enqueue(MixerCommand::ClipSetDefaultQuantization { quantization })
    }
    pub fn clip_set_trim(
        &self,
        column: usize,
        row: usize,
        start: f64,
        end: f64,
        timing: RetrimTiming,
    ) -> bool {
        self.enqueue(MixerCommand::ClipSetTrim {
            column,
            row,
            start,
            end,
            timing,
        })
    }
    pub fn transport_start(&self) -> bool {
        self.enqueue(MixerCommand::TransportStart)
    }
    pub fn transport_stop(&self) -> bool {
        self.enqueue(MixerCommand::TransportStop)
    }
    pub fn transport_seek(&self, beat: f64) -> bool {
        self.enqueue(MixerCommand::TransportSeek { beat })
    }
    pub fn transport_reset(&self) -> bool {
        self.enqueue(MixerCommand::TransportReset)
    }

    pub fn effect_add(&self, channel: usize, effect_id: u32) -> Option<usize> {
        if channel >= LOOP_CHANNEL_COUNT || !ChannelEffect::is_valid_id(effect_id) {
            return None;
        }
        let Ok(mut state) = self.shared.queue.lock() else {
            return None;
        };
        if state.commands.len() >= MAX_QUEUED_COMMANDS {
            return None;
        }
        let slot = state.effect_layouts[channel].len();
        state.effect_layouts[channel].push(effect_id);
        state
            .commands
            .push_back(MixerCommand::EffectAdd { channel, effect_id });
        self.shared.has_commands.store(true, Ordering::Release);
        Some(slot)
    }

    pub fn effect_remove(&self, channel: usize, slot: usize) -> bool {
        let Ok(mut state) = self.shared.queue.lock() else {
            return false;
        };
        if state.commands.len() >= MAX_QUEUED_COMMANDS {
            return false;
        }
        let Some(layout) = state.effect_layouts.get_mut(channel) else {
            return false;
        };
        if slot >= layout.len() {
            return false;
        }
        layout.remove(slot);
        state
            .commands
            .push_back(MixerCommand::EffectRemove { channel, slot });
        self.shared.has_commands.store(true, Ordering::Release);
        true
    }
    pub fn effect_move(&self, channel: usize, slot: usize, new_position: usize) -> bool {
        let Ok(mut state) = self.shared.queue.lock() else {
            return false;
        };
        if state.commands.len() >= MAX_QUEUED_COMMANDS {
            return false;
        }
        let Some(layout) = state.effect_layouts.get_mut(channel) else {
            return false;
        };
        if slot >= layout.len() {
            return false;
        }
        let effect = layout.remove(slot);
        layout.insert(new_position.min(layout.len()), effect);
        state.commands.push_back(MixerCommand::EffectMove {
            channel,
            slot,
            new_position,
        });
        self.shared.has_commands.store(true, Ordering::Release);
        true
    }
    pub fn effect_clear(&self, channel: usize) -> bool {
        let Ok(mut state) = self.shared.queue.lock() else {
            return false;
        };
        if state.commands.len() >= MAX_QUEUED_COMMANDS {
            return false;
        }
        let Some(layout) = state.effect_layouts.get_mut(channel) else {
            return false;
        };
        layout.clear();
        state
            .commands
            .push_back(MixerCommand::EffectClear { channel });
        self.shared.has_commands.store(true, Ordering::Release);
        true
    }
    pub fn effect_set_param(&self, channel: usize, slot: usize, param: u32, value: f32) -> bool {
        let Ok(mut state) = self.shared.queue.lock() else {
            return false;
        };
        if state
            .effect_layouts
            .get(channel)
            .is_none_or(|l| slot >= l.len())
            || state.commands.len() >= MAX_QUEUED_COMMANDS
        {
            return false;
        }
        state.commands.push_back(MixerCommand::EffectSetParam {
            channel,
            slot,
            param,
            value,
        });
        self.shared.has_commands.store(true, Ordering::Release);
        true
    }
    pub fn effect_count(&self, channel: usize) -> usize {
        self.shared
            .queue
            .lock()
            .ok()
            .and_then(|s| s.effect_layouts.get(channel).map(Vec::len))
            .unwrap_or(0)
    }
    pub fn effect_type_at(&self, channel: usize, slot: usize) -> Option<u32> {
        self.shared.queue.lock().ok().and_then(|s| {
            s.effect_layouts
                .get(channel)
                .and_then(|l| l.get(slot))
                .copied()
        })
    }

    pub fn position(&self, channel: usize) -> f32 {
        self.shared.snapshot.position(channel)
    }
    pub fn source_bpm(&self, channel: usize) -> f32 {
        self.shared.snapshot.source_bpm(channel)
    }
    pub fn pitch_mode(&self, channel: usize) -> PitchMode {
        self.shared.snapshot.pitch_mode(channel)
    }
    pub fn swaps_completed(&self, channel: usize) -> u32 {
        self.shared.snapshot.swaps_completed(channel)
    }
    pub fn clip_default_quantization(&self) -> u32 {
        self.shared.snapshot.default_quantization()
    }
    pub fn clip_slot_state(&self, column: usize, row: usize) -> u32 {
        self.shared.snapshot.slot_state(column, row)
    }
    pub fn clip_active_row(&self, column: usize) -> i32 {
        self.shared.snapshot.active_row(column)
    }
    pub fn clip_queued_row(&self, column: usize) -> i32 {
        self.shared.snapshot.queued_row(column)
    }
    pub fn clip_stop_queued(&self, column: usize) -> bool {
        self.shared.snapshot.stop_queued(column)
    }
    pub fn clip_scheduled_beat(&self, column: usize) -> f64 {
        self.shared.snapshot.scheduled_beat(column)
    }
    pub fn clip_active_playhead(&self, column: usize) -> f64 {
        self.shared.snapshot.active_playhead(column)
    }
    pub fn clip_trim_start(&self, column: usize, row: usize) -> f64 {
        self.shared.snapshot.trim_start(column, row)
    }
    pub fn clip_trim_end(&self, column: usize, row: usize) -> f64 {
        self.shared.snapshot.trim_end(column, row)
    }
    pub fn transport_beat(&self) -> f64 {
        self.shared.snapshot.transport_beat()
    }

    pub(crate) fn drain_into(&self, scratch: &mut VecDeque<MixerCommand>) {
        if !self.shared.has_commands.load(Ordering::Acquire) {
            return;
        }
        let Ok(mut state) = self.shared.queue.try_lock() else {
            return;
        };
        for _ in 0..MAX_COMMANDS_PER_TICK {
            let Some(command) = state.commands.pop_front() else {
                break;
            };
            scratch.push_back(command);
        }
        self.shared
            .has_commands
            .store(!state.commands.is_empty(), Ordering::Release);
    }

    pub(crate) fn has_pending(&self) -> bool {
        self.shared.has_commands.load(Ordering::Acquire)
    }

    pub(crate) fn snapshot(&self) -> &MixerSnapshot {
        &self.shared.snapshot
    }
}
