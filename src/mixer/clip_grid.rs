//! Transport-synchronized session clip grid layered over the loop mixer.

use super::{LoopChannel, PitchMode, StereoSampleBuffer, LOOP_CHANNEL_COUNT};

pub const CLIP_COLUMN_COUNT: usize = LOOP_CHANNEL_COUNT;
pub const CLIP_ROW_COUNT: usize = 8;

pub const CLIP_QUANTIZE_SIXTEENTH: u32 = 0;
pub const CLIP_QUANTIZE_QUARTER: u32 = 1;
pub const CLIP_QUANTIZE_BAR: u32 = 2;

pub const CLIP_STATE_LOADED: u32 = 1 << 0;
pub const CLIP_STATE_PLAYING: u32 = 1 << 1;
pub const CLIP_STATE_QUEUED: u32 = 1 << 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchQuantization {
    Sixteenth,
    Quarter,
    Bar,
}

impl LaunchQuantization {
    pub fn from_id(value: u32) -> Option<Self> {
        match value {
            CLIP_QUANTIZE_SIXTEENTH => Some(Self::Sixteenth),
            CLIP_QUANTIZE_QUARTER => Some(Self::Quarter),
            CLIP_QUANTIZE_BAR => Some(Self::Bar),
            _ => None,
        }
    }

    pub fn id(self) -> u32 {
        match self {
            Self::Sixteenth => CLIP_QUANTIZE_SIXTEENTH,
            Self::Quarter => CLIP_QUANTIZE_QUARTER,
            Self::Bar => CLIP_QUANTIZE_BAR,
        }
    }

    fn beats(self) -> f64 {
        match self {
            Self::Sixteenth => 0.25,
            Self::Quarter => 1.0,
            Self::Bar => 4.0,
        }
    }
}

#[derive(Clone, Debug)]
struct Clip {
    buffer: StereoSampleBuffer,
    length_beats: f64,
}

impl Clip {
    fn new(mut buffer: StereoSampleBuffer, source_bpm: f32) -> Option<Self> {
        if !source_bpm.is_finite() || source_bpm <= 0.0 || buffer.is_empty() {
            return None;
        }
        let length_beats =
            buffer.len() as f64 / buffer.sample_rate() as f64 * source_bpm as f64 / 60.0;
        if !length_beats.is_finite() || length_beats <= 0.0 {
            return None;
        }
        buffer.set_source_bpm(Some(source_bpm));
        Some(Self {
            buffer,
            length_beats,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingKind {
    Launch { row: usize },
    Stop,
    StopAndUnload { row: usize },
}

#[derive(Clone, Copy, Debug)]
struct ScheduledAction {
    kind: PendingKind,
    beat: f64,
}

#[derive(Clone, Debug, Default)]
struct ColumnState {
    active_row: Option<usize>,
    active_clip: Option<Clip>,
    launch_beat: f64,
    pending: Option<ScheduledAction>,
}

/// Engine-owned 4×8 clip grid plus its monotonic musical transport.
pub struct ClipGrid {
    slots: [[Option<Clip>; CLIP_ROW_COUNT]; CLIP_COLUMN_COUNT],
    columns: [ColumnState; CLIP_COLUMN_COUNT],
    default_quantization: LaunchQuantization,
    transport_beat: f64,
    transport_running: bool,
    bpm: f32,
    sample_rate: f32,
}

impl ClipGrid {
    pub fn new(sample_rate: f32, bpm: f32) -> Self {
        Self {
            slots: std::array::from_fn(|_| std::array::from_fn(|_| None)),
            columns: std::array::from_fn(|_| ColumnState::default()),
            default_quantization: LaunchQuantization::Bar,
            transport_beat: 0.0,
            transport_running: false,
            bpm,
            sample_rate,
        }
    }

    fn valid_slot(column: usize, row: usize) -> bool {
        column < CLIP_COLUMN_COUNT && row < CLIP_ROW_COUNT
    }

    fn beats_per_sample(&self) -> f64 {
        self.bpm.max(0.0) as f64 / (60.0 * self.sample_rate.max(1.0) as f64)
    }

    fn quantized_target(&self, quantization: LaunchQuantization) -> f64 {
        let interval = quantization.beats();
        let scaled = self.transport_beat / interval;
        let nearest = scaled.round();
        let aligned = (scaled - nearest).abs() <= 1.0e-9;
        if !self.transport_running && aligned {
            nearest * interval
        } else {
            (scaled.floor() + 1.0) * interval
        }
    }

    fn valid_exact_target(&self, beat: f64) -> bool {
        beat.is_finite() && beat >= 0.0 && beat + 1.0e-9 >= self.transport_beat
    }

    fn schedule(&mut self, column: usize, kind: PendingKind, beat: f64) -> bool {
        let Some(state) = self.columns.get_mut(column) else {
            return false;
        };
        state.pending = Some(ScheduledAction { kind, beat });
        true
    }

    pub fn load(
        &mut self,
        column: usize,
        row: usize,
        buffer: StereoSampleBuffer,
        source_bpm: f32,
    ) -> bool {
        if !Self::valid_slot(column, row) {
            return false;
        }
        let Some(clip) = Clip::new(buffer, source_bpm) else {
            return false;
        };
        self.slots[column][row] = Some(clip);

        if self.columns[column].active_row == Some(row) {
            let target = self.quantized_target(self.default_quantization);
            self.columns[column].pending = Some(ScheduledAction {
                kind: PendingKind::Launch { row },
                beat: target,
            });
        }
        true
    }

    pub fn unload(&mut self, column: usize, row: usize) -> bool {
        if !Self::valid_slot(column, row) || self.slots[column][row].is_none() {
            return false;
        }
        if self.columns[column].active_row == Some(row) {
            let target = self.quantized_target(self.default_quantization);
            self.columns[column].pending = Some(ScheduledAction {
                kind: PendingKind::StopAndUnload { row },
                beat: target,
            });
        } else {
            self.slots[column][row] = None;
            if matches!(
                self.columns[column].pending,
                Some(ScheduledAction {
                    kind: PendingKind::Launch { row: pending_row },
                    ..
                }) if pending_row == row
            ) {
                self.columns[column].pending = None;
            }
        }
        true
    }

    pub fn clear(&mut self, channels: &mut [LoopChannel]) {
        for column in 0..CLIP_COLUMN_COUNT {
            if self.columns[column].active_row.is_some() {
                if let Some(channel) = channels.get_mut(column) {
                    channel.clear_buffer();
                }
            }
            self.columns[column] = ColumnState::default();
            for slot in &mut self.slots[column] {
                *slot = None;
            }
        }
    }

    pub fn launch_quantized(
        &mut self,
        column: usize,
        row: usize,
        quantization: LaunchQuantization,
    ) -> bool {
        if !Self::valid_slot(column, row) || self.slots[column][row].is_none() {
            return false;
        }
        let target = self.quantized_target(quantization);
        self.schedule(column, PendingKind::Launch { row }, target)
    }

    pub fn launch_at(&mut self, column: usize, row: usize, beat: f64) -> bool {
        if !Self::valid_slot(column, row)
            || self.slots[column][row].is_none()
            || !self.valid_exact_target(beat)
        {
            return false;
        }
        self.schedule(column, PendingKind::Launch { row }, beat)
    }

    pub fn launch_scene_quantized(&mut self, row: usize, quantization: LaunchQuantization) -> bool {
        if row >= CLIP_ROW_COUNT {
            return false;
        }
        let target = self.quantized_target(quantization);
        for column in 0..CLIP_COLUMN_COUNT {
            let kind = if self.slots[column][row].is_some() {
                PendingKind::Launch { row }
            } else {
                PendingKind::Stop
            };
            self.columns[column].pending = Some(ScheduledAction { kind, beat: target });
        }
        true
    }

    pub fn launch_scene_at(&mut self, row: usize, beat: f64) -> bool {
        if row >= CLIP_ROW_COUNT || !self.valid_exact_target(beat) {
            return false;
        }
        for column in 0..CLIP_COLUMN_COUNT {
            let kind = if self.slots[column][row].is_some() {
                PendingKind::Launch { row }
            } else {
                PendingKind::Stop
            };
            self.columns[column].pending = Some(ScheduledAction { kind, beat });
        }
        true
    }

    pub fn stop_quantized(&mut self, column: usize, quantization: LaunchQuantization) -> bool {
        let target = self.quantized_target(quantization);
        self.schedule(column, PendingKind::Stop, target)
    }

    pub fn stop_at(&mut self, column: usize, beat: f64) -> bool {
        if !self.valid_exact_target(beat) {
            return false;
        }
        self.schedule(column, PendingKind::Stop, beat)
    }

    pub fn cancel(&mut self, column: usize) {
        if let Some(state) = self.columns.get_mut(column) {
            state.pending = None;
        }
    }

    pub fn cancel_all(&mut self) {
        for state in &mut self.columns {
            state.pending = None;
        }
    }

    pub fn detach_column(&mut self, column: usize) {
        if let Some(state) = self.columns.get_mut(column) {
            *state = ColumnState::default();
        }
    }

    pub fn set_default_quantization(&mut self, quantization: LaunchQuantization) {
        self.default_quantization = quantization;
    }

    pub fn default_quantization(&self) -> LaunchQuantization {
        self.default_quantization
    }

    pub fn slot_state(&self, column: usize, row: usize) -> u32 {
        if !Self::valid_slot(column, row) {
            return 0;
        }
        let mut state = 0;
        if self.slots[column][row].is_some() {
            state |= CLIP_STATE_LOADED;
        }
        if self.columns[column].active_row == Some(row) {
            state |= CLIP_STATE_PLAYING;
        }
        if matches!(
            self.columns[column].pending,
            Some(ScheduledAction {
                kind: PendingKind::Launch { row: pending_row }
                    | PendingKind::StopAndUnload { row: pending_row },
                ..
            }) if pending_row == row
        ) {
            state |= CLIP_STATE_QUEUED;
        }
        state
    }

    pub fn active_row(&self, column: usize) -> Option<usize> {
        self.columns.get(column).and_then(|state| state.active_row)
    }

    pub fn queued_row(&self, column: usize) -> Option<usize> {
        match self.columns.get(column)?.pending?.kind {
            PendingKind::Launch { row } | PendingKind::StopAndUnload { row } => Some(row),
            PendingKind::Stop => None,
        }
    }

    pub fn is_stop_queued(&self, column: usize) -> bool {
        matches!(
            self.columns.get(column).and_then(|state| state.pending),
            Some(ScheduledAction {
                kind: PendingKind::Stop | PendingKind::StopAndUnload { .. },
                ..
            })
        )
    }

    pub fn scheduled_beat(&self, column: usize) -> Option<f64> {
        self.columns
            .get(column)
            .and_then(|state| state.pending.map(|pending| pending.beat))
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        if bpm.is_finite() && bpm > 0.0 {
            self.bpm = bpm;
        }
    }

    pub fn transport_start(&mut self, channels: &mut [LoopChannel]) {
        self.transport_running = true;
        for (column, state) in self.columns.iter().enumerate() {
            if state.active_row.is_some() {
                if let Some(channel) = channels.get_mut(column) {
                    channel.set_playing(true);
                }
            }
        }
    }

    pub fn transport_stop(&mut self, channels: &mut [LoopChannel]) {
        self.transport_running = false;
        self.cancel_all();
        for (column, state) in self.columns.iter().enumerate() {
            if state.active_row.is_some() {
                if let Some(channel) = channels.get_mut(column) {
                    channel.set_playing(false);
                }
            }
        }
    }

    pub fn transport_seek(&mut self, beat: f64, channels: &mut [LoopChannel]) -> bool {
        if !beat.is_finite() || beat < 0.0 {
            return false;
        }
        self.transport_beat = beat;
        for (column, state) in self.columns.iter().enumerate() {
            let Some(clip) = state.active_clip.as_ref() else {
                continue;
            };
            let phase =
                (beat - state.launch_beat).rem_euclid(clip.length_beats) / clip.length_beats;
            if let Some(channel) = channels.get_mut(column) {
                channel.set_position(phase as f32);
            }
        }
        true
    }

    pub fn transport_reset(&mut self, channels: &mut [LoopChannel]) {
        self.cancel_all();
        self.transport_seek(0.0, channels);
    }

    pub fn transport_beat(&self) -> f64 {
        self.transport_beat
    }

    pub fn transport_running(&self) -> bool {
        self.transport_running
    }

    fn activate(&mut self, column: usize, row: usize, channels: &mut [LoopChannel]) {
        let Some(clip) = self.slots[column][row].clone() else {
            self.stop_now(column, channels);
            return;
        };
        let Some(channel) = channels.get_mut(column) else {
            return;
        };
        channel.set_loop_start(0.0);
        channel.set_loop_end(1.0);
        channel.set_speed(1.0);
        channel.set_pitch_mode(PitchMode::PreservePitch);
        channel.cancel_queued_swap();
        channel.set_buffer(clip.buffer.clone());
        channel.set_playing(self.transport_running);
        self.columns[column].active_row = Some(row);
        self.columns[column].active_clip = Some(clip);
        self.columns[column].launch_beat = self.transport_beat;
    }

    fn stop_now(&mut self, column: usize, channels: &mut [LoopChannel]) {
        if let Some(channel) = channels.get_mut(column) {
            channel.set_playing(false);
        }
        self.columns[column].active_row = None;
        self.columns[column].active_clip = None;
        self.columns[column].launch_beat = 0.0;
    }

    /// Apply due actions before this sample is rendered, then advance the
    /// monotonic beat clock after the sample has been produced.
    pub fn before_tick(&mut self, channels: &mut [LoopChannel]) {
        if !self.transport_running {
            return;
        }
        let tolerance = self.beats_per_sample() * 0.5 + 1.0e-12;
        for column in 0..CLIP_COLUMN_COUNT {
            let Some(pending) = self.columns[column].pending else {
                continue;
            };
            if self.transport_beat + tolerance < pending.beat {
                continue;
            }
            self.columns[column].pending = None;
            match pending.kind {
                PendingKind::Launch { row } => self.activate(column, row, channels),
                PendingKind::Stop => self.stop_now(column, channels),
                PendingKind::StopAndUnload { row } => {
                    self.stop_now(column, channels);
                    if let Some(channel) = channels.get_mut(column) {
                        channel.clear_buffer();
                    }
                    self.slots[column][row] = None;
                }
            }
        }
    }

    pub fn after_tick(&mut self) {
        if self.transport_running {
            self.transport_beat += self.beats_per_sample();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 100.0;

    fn clip(value: f32, frames: usize, source_bpm: f32) -> (StereoSampleBuffer, f32) {
        (
            StereoSampleBuffer::from_channels(vec![value; frames], vec![value; frames], SR)
                .unwrap(),
            source_bpm,
        )
    }

    fn channels() -> Vec<LoopChannel> {
        (0..CLIP_COLUMN_COUNT)
            .map(|_| LoopChannel::new(SR))
            .collect()
    }

    #[test]
    fn quantized_running_targets_strictly_future_boundary() {
        let mut grid = ClipGrid::new(SR, 60.0);
        let mut channels = channels();
        grid.transport_start(&mut channels);
        assert_eq!(grid.quantized_target(LaunchQuantization::Bar), 4.0);
        grid.transport_beat = 4.0;
        assert_eq!(grid.quantized_target(LaunchQuantization::Bar), 8.0);
    }

    #[test]
    fn stopped_aligned_launch_fires_on_first_started_tick() {
        let mut grid = ClipGrid::new(SR, 60.0);
        let mut channels = channels();
        let (buffer, bpm) = clip(0.5, 100, 60.0);
        assert!(grid.load(0, 0, buffer, bpm));
        assert!(grid.launch_quantized(0, 0, LaunchQuantization::Bar));
        assert_eq!(grid.scheduled_beat(0), Some(0.0));
        grid.before_tick(&mut channels);
        assert_eq!(grid.active_row(0), None);
        grid.transport_start(&mut channels);
        grid.before_tick(&mut channels);
        assert_eq!(grid.active_row(0), Some(0));
    }

    #[test]
    fn exact_past_target_is_rejected_without_replacing_queue() {
        let mut grid = ClipGrid::new(SR, 60.0);
        let mut channels = channels();
        for row in 0..2 {
            let (buffer, bpm) = clip(row as f32 + 1.0, 100, 60.0);
            assert!(grid.load(0, row, buffer, bpm));
        }
        grid.transport_beat = 3.0;
        grid.transport_start(&mut channels);
        assert!(grid.launch_at(0, 0, 4.0));
        assert!(!grid.launch_at(0, 1, 2.0));
        assert_eq!(grid.queued_row(0), Some(0));
        assert_eq!(grid.scheduled_beat(0), Some(4.0));
    }

    #[test]
    fn scene_launch_starts_loaded_cells_and_stops_empty_cells() {
        let mut grid = ClipGrid::new(SR, 60.0);
        let mut channels = channels();
        for column in 0..CLIP_COLUMN_COUNT {
            let (buffer, bpm) = clip(column as f32 + 1.0, 100, 60.0);
            grid.load(column, 0, buffer, bpm);
        }
        grid.launch_scene_at(0, 0.0);
        grid.transport_start(&mut channels);
        grid.before_tick(&mut channels);
        assert_eq!(
            grid.columns
                .iter()
                .map(|c| c.active_row)
                .collect::<Vec<_>>(),
            vec![Some(0); 4]
        );

        let (buffer, bpm) = clip(9.0, 100, 60.0);
        grid.load(0, 1, buffer, bpm);
        grid.launch_scene_at(1, 0.0);
        grid.before_tick(&mut channels);
        assert_eq!(grid.active_row(0), Some(1));
        for column in 1..CLIP_COLUMN_COUNT {
            assert_eq!(grid.active_row(column), None);
        }
    }

    #[test]
    fn state_bits_support_playing_and_queued_replacement() {
        let mut grid = ClipGrid::new(SR, 60.0);
        let mut channels = channels();
        let (buffer, bpm) = clip(1.0, 100, 60.0);
        grid.load(0, 0, buffer, bpm);
        grid.launch_at(0, 0, 0.0);
        grid.transport_start(&mut channels);
        grid.before_tick(&mut channels);

        let (replacement, bpm) = clip(2.0, 100, 60.0);
        grid.load(0, 0, replacement, bpm);
        assert_eq!(
            grid.slot_state(0, 0),
            CLIP_STATE_LOADED | CLIP_STATE_PLAYING | CLIP_STATE_QUEUED
        );
    }

    #[test]
    fn transport_stop_freezes_and_cancels_queue() {
        let mut grid = ClipGrid::new(SR, 60.0);
        let mut channels = channels();
        let (buffer, bpm) = clip(1.0, 100, 60.0);
        grid.load(0, 0, buffer, bpm);
        grid.launch_at(0, 0, 0.0);
        grid.transport_start(&mut channels);
        grid.before_tick(&mut channels);
        grid.after_tick();
        let beat = grid.transport_beat();
        grid.stop_quantized(0, LaunchQuantization::Quarter);
        grid.transport_stop(&mut channels);
        assert!(grid.scheduled_beat(0).is_none());
        grid.after_tick();
        assert_eq!(grid.transport_beat(), beat);
        assert_eq!(grid.active_row(0), Some(0));
        assert!(!channels[0].is_playing());
    }

    #[test]
    fn seek_realigns_active_clip_phase() {
        let mut grid = ClipGrid::new(SR, 60.0);
        let mut channels = channels();
        let (buffer, bpm) = clip(1.0, 400, 60.0); // four beats
        grid.load(0, 0, buffer, bpm);
        grid.launch_at(0, 0, 0.0);
        grid.transport_start(&mut channels);
        grid.before_tick(&mut channels);
        assert!(grid.transport_seek(2.0, &mut channels));
        assert!((channels[0].position_normalized() - 0.5).abs() < 0.01);
    }
}
