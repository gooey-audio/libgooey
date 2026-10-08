//! Transport-synchronized session clip grid layered over the loop mixer.

use super::{LoopChannel, PitchMode, StereoSampleBuffer, LOOP_CHANNEL_COUNT};

pub const CLIP_COLUMN_COUNT: usize = LOOP_CHANNEL_COUNT;
pub const CLIP_ROW_COUNT: usize = 8;
/// Largest clip transpose, in semitones either way (a 0.25x..4x pitch ratio).
pub const CLIP_TRANSPOSE_MAX_SEMITONES: f32 = 24.0;

pub(crate) fn valid_transpose(semitones: f32) -> bool {
    semitones.is_finite() && semitones.abs() <= CLIP_TRANSPOSE_MAX_SEMITONES
}

pub const CLIP_QUANTIZE_SIXTEENTH: u32 = 0;
pub const CLIP_QUANTIZE_QUARTER: u32 = 1;
pub const CLIP_QUANTIZE_BAR: u32 = 2;
/// Retrim timing sentinel: apply immediately, ignoring the transport grid.
/// Valid only for trim edits — launches still reject it.
pub const CLIP_QUANTIZE_IMMEDIATE: u32 = 3;

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

/// When a trim edit to the *active* clip takes effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetrimTiming {
    /// Re-window the loop right now (live marker scrubbing); works with the
    /// transport stopped too.
    Immediate,
    /// Land the new window on the transport grid, like a launch, so the loop
    /// never jumps mid-bar to a dissonant position.
    Quantized(LaunchQuantization),
}

impl RetrimTiming {
    /// Decode a `CLIP_QUANTIZE_*` id, where `CLIP_QUANTIZE_IMMEDIATE` selects
    /// [`RetrimTiming::Immediate`] and the rest reuse [`LaunchQuantization`].
    pub fn from_id(value: u32) -> Option<Self> {
        if value == CLIP_QUANTIZE_IMMEDIATE {
            Some(Self::Immediate)
        } else {
            LaunchQuantization::from_id(value).map(Self::Quantized)
        }
    }
}

#[derive(Clone, Debug)]
struct Clip {
    revision: u64,
    speed: f32,
    preserve_pitch: bool,
    /// Pitch shift in semitones, independent of speed and transport timing.
    transpose: f32,
    playback_configured: bool,
    buffer: StereoSampleBuffer,
    length_beats: f64,
    /// Normalized loop trim markers in `[0, 1]`. `trim_end < trim_start` selects
    /// a wrap-around loop region (see [`LoopChannel::set_loop_window`]). Applied
    /// by [`ClipGrid::activate`] and round-tripped by the trim getters.
    trim_start: f64,
    trim_end: f64,
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
            revision: 0,
            speed: 1.0,
            preserve_pitch: true,
            transpose: 0.0,
            playback_configured: false,
            buffer,
            length_beats,
            trim_start: 0.0,
            trim_end: 1.0,
        })
    }

    fn transpose_ratio(&self) -> f32 {
        2f32.powf(self.transpose / 12.0)
    }

    /// Musical duration of the playable window, including wrap-around trims.
    fn window_length_beats(&self) -> f64 {
        let fraction = if self.trim_end > self.trim_start {
            self.trim_end - self.trim_start
        } else {
            1.0 - self.trim_start + self.trim_end
        };
        self.length_beats * fraction
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

/// A trim edit queued to land on the transport grid. Kept separate from
/// `pending` (single latest-wins launch/stop slot) so a queued retrim never
/// cancels a queued launch and vice versa.
#[derive(Clone, Copy, Debug)]
struct PendingRetrim {
    beat: f64,
    start: f64,
    end: f64,
}

#[derive(Clone, Debug, Default)]
struct ColumnState {
    active_row: Option<usize>,
    active_clip: Option<Clip>,
    launch_beat: f64,
    pending: Option<ScheduledAction>,
    pending_retrim: Option<PendingRetrim>,
}

/// Engine-owned 4×8 clip grid plus its monotonic musical transport.
pub struct ClipGrid {
    slots: [[Option<Clip>; CLIP_ROW_COUNT]; CLIP_COLUMN_COUNT],
    columns: [ColumnState; CLIP_COLUMN_COUNT],
    default_quantization: LaunchQuantization,
    phase_aligned_launches: bool,
    next_revision: u64,
    transport_beat: f64,
    transport_running: bool,
    /// Bumped on every transport *discontinuity* (seek, reset, start) but never
    /// by the per-sample advance in [`ClipGrid::after_tick`]. Consumers that
    /// derive their own phase from `transport_beat` — the metronome, for
    /// instance — cannot always infer a discontinuity from the beat value or
    /// from `transport_running`, because a stop/reset/start issued between two
    /// audio callbacks is drained as one batch and never observed as a stopped
    /// tick. Watching this counter for change is exact.
    transport_generation: u64,
    bpm: f32,
    sample_rate: f32,
}

impl ClipGrid {
    pub fn new(sample_rate: f32, bpm: f32) -> Self {
        Self {
            slots: std::array::from_fn(|_| std::array::from_fn(|_| None)),
            columns: std::array::from_fn(|_| ColumnState::default()),
            default_quantization: LaunchQuantization::Bar,
            phase_aligned_launches: false,
            next_revision: 0,
            transport_beat: 0.0,
            transport_running: false,
            transport_generation: 0,
            bpm,
            sample_rate,
        }
    }

    /// Opt-in shared phrase origin for session hosts. Other apps retain
    /// launch-from-start behavior. Configure before launching any clips.
    pub fn set_phase_aligned_launches(&mut self, enabled: bool) {
        self.phase_aligned_launches = enabled;
    }

    fn valid_slot(column: usize, row: usize) -> bool {
        column < CLIP_COLUMN_COUNT && row < CLIP_ROW_COUNT
    }

    fn beats_per_sample(&self) -> f64 {
        self.bpm.max(0.0) as f64 / (60.0 * self.sample_rate.max(1.0) as f64)
    }

    /// Return the sole scheduling target used by clip-grid actions. A stopped
    /// transport always accepts the responsive beat-zero launch; once running,
    /// even a request made exactly on a boundary waits for the *next* boundary.
    pub fn quantized_target(&self, quantization: LaunchQuantization) -> f64 {
        if !self.transport_running {
            return 0.0;
        }
        let interval = quantization.beats();
        let scaled = self.transport_beat / interval;
        // The render clock accumulates f64 sample increments, so an exact musical
        // boundary can be represented as 0.9999999999999999. Treat that small
        // numerical residue as aligned; "strictly future" then means one whole
        // additional interval rather than accidentally scheduling the same bar.
        let nearest = scaled.round();
        let base = if (scaled - nearest).abs() <= 1.0e-9 {
            nearest
        } else {
            scaled.floor()
        };
        (base + 1.0) * interval
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
        let Some(mut clip) = Clip::new(buffer, source_bpm) else {
            return false;
        };
        self.next_revision = self.next_revision.wrapping_add(1);
        clip.revision = self.next_revision;
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

    pub fn load_with_playback(
        &mut self,
        column: usize,
        row: usize,
        buffer: StereoSampleBuffer,
        source_bpm: f32,
        speed: f32,
        preserve_pitch: bool,
    ) -> bool {
        if !speed.is_finite()
            || !(0.25..=4.0).contains(&speed)
            || !self.load(column, row, buffer, source_bpm)
        {
            return false;
        }
        let clip = self.slots[column][row].as_mut().unwrap();
        clip.speed = speed;
        clip.preserve_pitch = preserve_pitch;
        clip.playback_configured = true;
        true
    }

    pub fn set_playback(
        &mut self,
        column: usize,
        row: usize,
        speed: f32,
        preserve_pitch: bool,
        channels: &mut [LoopChannel],
    ) -> bool {
        if !Self::valid_slot(column, row) || !speed.is_finite() || !(0.25..=4.0).contains(&speed) {
            return false;
        }
        let Some(clip) = self.slots[column][row].as_mut() else {
            return false;
        };
        clip.speed = speed;
        clip.preserve_pitch = preserve_pitch;
        clip.playback_configured = true;
        if let Some(active) = self.columns[column].active_clip.as_mut() {
            if active.revision == clip.revision {
                // A pending replacement has a different revision: only edit
                // its stored settings, never the outgoing channel.
                let was_configured = active.playback_configured;
                active.speed = speed;
                active.preserve_pitch = preserve_pitch;
                active.playback_configured = true;
                if let Some(channel) = channels.get_mut(column) {
                    if !self.phase_aligned_launches && !was_configured {
                        channel.seed_clip_phase_from_cursor(self.rendered_clip_beat(column));
                    }
                    channel.set_clip_playback(speed, preserve_pitch, false);
                }
            }
        }
        true
    }

    /// Transpose a loaded clip live without changing its timing. Like
    /// [`Self::set_playback`], a queued replacement only has its stored
    /// setting edited.
    pub fn set_transpose(
        &mut self,
        column: usize,
        row: usize,
        semitones: f32,
        channels: &mut [LoopChannel],
    ) -> bool {
        if !Self::valid_slot(column, row) || !valid_transpose(semitones) {
            return false;
        }
        let Some(clip) = self.slots[column][row].as_mut() else {
            return false;
        };
        clip.transpose = semitones;
        clip.playback_configured = true;
        let ratio = clip.transpose_ratio();
        if let Some(active) = self.columns[column].active_clip.as_mut() {
            if active.revision == clip.revision {
                let was_configured = active.playback_configured;
                active.transpose = semitones;
                active.playback_configured = true;
                if let Some(channel) = channels.get_mut(column) {
                    if !self.phase_aligned_launches && !was_configured {
                        channel.seed_clip_phase_from_cursor(self.rendered_clip_beat(column));
                    }
                    channel.set_clip_transpose(ratio);
                }
            }
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
            state.pending_retrim = None;
        }
    }

    pub fn cancel_all(&mut self) {
        for state in &mut self.columns {
            state.pending = None;
            state.pending_retrim = None;
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

    /// The actual source-buffer cursor of an active clip. This deliberately
    /// reads the loop channel instead of deriving a phase from transport beat:
    /// a launch at beat 4 and a cropped/wrapped window still begin at their
    /// physical first audible frame.
    pub fn active_playhead(&self, column: usize, channels: &[LoopChannel]) -> Option<f64> {
        self.active_row(column)?;
        channels
            .get(column)
            .filter(|channel| channel.has_buffer())
            .map(|channel| channel.position_normalized() as f64)
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

    /// Set a slot's loop trim as normalized `[0, 1]` start/end markers. The
    /// value is always stored into the slot `Clip` (so the next launch applies
    /// it, and the getters round-trip). When the slot is the currently active
    /// one, the active clone is updated too and the change is applied to the
    /// sounding channel per `timing`: [`RetrimTiming::Immediate`] re-windows the
    /// loop now (also works with the transport stopped); a quantized timing
    /// schedules the retrim for the next matching grid boundary, fired
    /// sample-accurately in [`Self::before_tick`].
    ///
    /// `end < start` selects a wrap-around loop region. Rejects non-finite
    /// markers, markers outside `[0, 1]`, `start == end`, invalid slots, and
    /// sub-sample windows (`span * buffer_len < 1`).
    pub fn set_trim(
        &mut self,
        column: usize,
        row: usize,
        start: f64,
        end: f64,
        timing: RetrimTiming,
        channels: &mut [LoopChannel],
    ) -> bool {
        if !Self::valid_slot(column, row)
            || !start.is_finite()
            || !end.is_finite()
            || !(0.0..=1.0).contains(&start)
            || !(0.0..=1.0).contains(&end)
            || start == end
        {
            return false;
        }
        let Some(clip) = self.slots[column][row].as_mut() else {
            return false;
        };
        // Reject a window smaller than one frame (the grid owns the buffer, so
        // it can check the physical span the channel can't).
        let buffer_len = clip.buffer.len() as f64;
        let span_frac = if end < start {
            1.0 - start + end
        } else {
            end - start
        };
        if span_frac * buffer_len < 1.0 {
            return false;
        }
        clip.trim_start = start;
        clip.trim_end = end;

        if self.columns[column].active_row != Some(row) {
            return true;
        }
        // Keep the active clone in sync so a transport seek / relaunch uses the
        // new trim, then apply to the sounding channel.
        if let Some(active) = self.columns[column].active_clip.as_mut() {
            active.trim_start = start;
            active.trim_end = end;
        }
        match timing {
            RetrimTiming::Immediate => {
                if let Some(channel) = channels.get_mut(column) {
                    channel.set_loop_window(start as f32, end as f32);
                    self.reseed_after_retrim(column, channel);
                }
                // An immediate retrim supersedes any queued one.
                self.columns[column].pending_retrim = None;
            }
            RetrimTiming::Quantized(quantization) => {
                let beat = self.quantized_target(quantization);
                self.columns[column].pending_retrim = Some(PendingRetrim { beat, start, end });
            }
        }
        true
    }

    /// A transport-driven clip derives its cursor from the clip phase, which a
    /// new window would reinterpret. Without phase-aligned launches, keep the
    /// playhead where it is (the no-restart retrim contract). Phase-aligned
    /// columns intentionally stay on the shared phrase position, measured from
    /// the new loop start, like a relaunch.
    fn reseed_after_retrim(&self, column: usize, channel: &mut LoopChannel) {
        if !self.phase_aligned_launches {
            channel.seed_clip_phase_from_cursor(self.rendered_clip_beat(column));
        }
    }

    /// Clip-relative beat of the channel's current cursor. While running, the
    /// cursor belongs to the last rendered sample, one sample behind the
    /// transport beat, so seeding from it must not repeat that sample.
    fn rendered_clip_beat(&self, column: usize) -> f64 {
        let lag = if self.transport_running {
            self.beats_per_sample()
        } else {
            0.0
        };
        self.transport_beat - self.columns[column].launch_beat - lag
    }

    /// The stored normalized loop-start marker for a slot, or `None` for an
    /// invalid or empty slot.
    pub fn trim_start(&self, column: usize, row: usize) -> Option<f64> {
        if !Self::valid_slot(column, row) {
            return None;
        }
        self.slots[column][row].as_ref().map(|clip| clip.trim_start)
    }

    /// The stored normalized loop-end marker for a slot, or `None` for an
    /// invalid or empty slot.
    pub fn trim_end(&self, column: usize, row: usize) -> Option<f64> {
        if !Self::valid_slot(column, row) {
            return None;
        }
        self.slots[column][row].as_ref().map(|clip| clip.trim_end)
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        if bpm.is_finite() && bpm > 0.0 {
            self.bpm = bpm;
        }
    }

    pub fn transport_start(&mut self, channels: &mut [LoopChannel]) {
        self.transport_running = true;
        // Starting is a discontinuity for phase-deriving consumers even though
        // the beat does not move: a stop/start round trip should restate the
        // grid position rather than be swallowed as "no change".
        self.transport_generation = self.transport_generation.wrapping_add(1);
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
        // After the validity guard, so a rejected seek is not a discontinuity.
        // `transport_reset` routes through here, so it bumps exactly once.
        self.transport_generation = self.transport_generation.wrapping_add(1);
        for (column, state) in self.columns.iter().enumerate() {
            let Some(clip) = state.active_clip.as_ref() else {
                continue;
            };
            let length = if self.phase_aligned_launches || clip.playback_configured {
                clip.window_length_beats()
            } else {
                clip.length_beats
            };
            let source_beat = (beat - state.launch_beat) * clip.speed as f64;
            let phase = source_beat.rem_euclid(length) / length;
            if let Some(channel) = channels.get_mut(column) {
                // Map the musical phrase phase through the loop window so it
                // lands correctly inside a trimmed (or wrapped) region.
                channel.set_window_phase(phase as f32);
                channel.seed_clip_phase(beat - state.launch_beat, clip.speed);
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

    /// Monotonic counter of transport discontinuities. Only the *change* is
    /// meaningful; the magnitude is not (an armed host-time start seeks and
    /// then starts, so one logical discontinuity can bump it twice).
    pub fn transport_generation(&self) -> u64 {
        self.transport_generation
    }

    fn activate(&mut self, column: usize, row: usize, channels: &mut [LoopChannel]) {
        let Some(clip) = self.slots[column][row].clone() else {
            self.stop_now(column, channels);
            return;
        };
        let Some(channel) = channels.get_mut(column) else {
            return;
        };
        // Apply the slot's stored trim before `set_buffer` so the cursor lands
        // at the trimmed loop start.
        channel.set_loop_window(clip.trim_start as f32, clip.trim_end as f32);
        channel.set_speed(1.0);
        channel.set_pitch_mode(PitchMode::PreservePitch);
        channel.cancel_queued_swap();
        channel.set_buffer(clip.buffer.clone());
        channel.set_clip_playback(clip.speed, clip.preserve_pitch, true);
        channel.set_clip_transpose(clip.transpose_ratio());
        let launch_beat = if self.phase_aligned_launches {
            0.0
        } else {
            self.transport_beat
        };
        channel.seed_clip_phase(self.transport_beat - launch_beat, clip.speed);
        if self.phase_aligned_launches {
            // Compute at the actual render sample that activates this clip,
            // never at generation completion or from a host timer.
            let length = clip.window_length_beats();
            let phase = (self.transport_beat * clip.speed as f64).rem_euclid(length) / length;
            channel.set_window_phase(phase as f32);
        }
        channel.set_playing(self.transport_running);
        self.columns[column].active_row = Some(row);
        self.columns[column].active_clip = Some(clip);
        self.columns[column].launch_beat = if self.phase_aligned_launches {
            0.0
        } else {
            self.transport_beat
        };
    }

    fn stop_now(&mut self, column: usize, channels: &mut [LoopChannel]) {
        if let Some(channel) = channels.get_mut(column) {
            // Drop the grid buffer, not just playback: a stopped column keeps no
            // sounding material, so a later `clear` or legacy detach cannot
            // replay a slot the grid already reported as inactive. Transport
            // freeze keeps its buffer via `transport_stop`, which never routes
            // through here.
            channel.clear_buffer();
        }
        self.columns[column].active_row = None;
        self.columns[column].active_clip = None;
        self.columns[column].launch_beat = 0.0;
    }

    /// Apply due actions before this sample is rendered, then advance the
    /// monotonic beat clock after the sample has been produced.
    pub fn before_tick(&mut self, channels: &mut [LoopChannel]) {
        for channel in channels.iter_mut() {
            channel.set_transport_beat(None);
        }
        if !self.transport_running {
            return;
        }
        let tolerance = self.beats_per_sample() * 0.5 + 1.0e-12;
        for column in 0..CLIP_COLUMN_COUNT {
            if let Some(pending) = self.columns[column].pending {
                if self.transport_beat + tolerance >= pending.beat {
                    self.columns[column].pending = None;
                    // A launch/stop re-applies the slot's stored trim on
                    // activate, so any queued retrim is now stale.
                    self.columns[column].pending_retrim = None;
                    match pending.kind {
                        PendingKind::Launch { row } => self.activate(column, row, channels),
                        PendingKind::Stop => self.stop_now(column, channels),
                        PendingKind::StopAndUnload { row } => {
                            // `stop_now` already drops the channel buffer.
                            self.stop_now(column, channels);
                            self.slots[column][row] = None;
                        }
                    }
                }
            }

            // Fire a due retrim on the active clip. Independent of `pending`
            // so a scheduled trim and a scheduled launch coexist.
            if let Some(retrim) = self.columns[column].pending_retrim {
                if self.transport_beat + tolerance >= retrim.beat {
                    self.columns[column].pending_retrim = None;
                    if let Some(channel) = channels.get_mut(column) {
                        channel.set_loop_window(retrim.start as f32, retrim.end as f32);
                        self.reseed_after_retrim(column, channel);
                    }
                }
            }
            let state = &self.columns[column];
            if state
                .active_clip
                .as_ref()
                .is_some_and(|clip| self.phase_aligned_launches || clip.playback_configured)
            {
                if let Some(channel) = channels.get_mut(column) {
                    channel.set_transport_beat(Some(self.transport_beat - state.launch_beat));
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

    #[test]
    fn stopped_column_drops_channel_buffer_so_clear_leaves_nothing() {
        let mut grid = ClipGrid::new(SR, 60.0);
        let mut channels = channels();
        let (buffer, bpm) = clip(0.5, 100, 60.0);
        grid.load(0, 0, buffer, bpm);
        grid.launch_at(0, 0, 0.0);
        grid.transport_start(&mut channels);
        grid.before_tick(&mut channels);
        assert!(channels[0].has_buffer());

        // A plain column stop must leave no sounding material behind.
        grid.stop_at(0, grid.transport_beat());
        grid.before_tick(&mut channels);
        assert_eq!(grid.active_row(0), None);
        assert!(!channels[0].has_buffer());

        // `clear` after the stop cannot resurrect the buffer via a later
        // legacy detach + set_playing.
        grid.clear(&mut channels);
        assert_eq!(grid.slot_state(0, 0), 0);
        assert!(!channels[0].has_buffer());
    }

    /// Drive the grid + channels sample-by-sample, like the real render loop.
    fn step(grid: &mut ClipGrid, channels: &mut [LoopChannel], samples: usize) {
        for _ in 0..samples {
            grid.before_tick(channels);
            for ch in channels.iter_mut() {
                ch.tick(SR);
            }
            grid.after_tick();
        }
    }

    #[test]
    fn set_trim_stores_and_activate_applies() {
        let mut grid = ClipGrid::new(SR, 60.0);
        let mut channels = channels();
        let (buffer, bpm) = clip(0.5, 100, 60.0);
        grid.load(0, 0, buffer, bpm);
        // Store trim on an inactive slot (timing is irrelevant here).
        assert!(grid.set_trim(0, 0, 0.25, 0.75, RetrimTiming::Immediate, &mut channels));
        assert_eq!(grid.trim_start(0, 0), Some(0.25));
        assert_eq!(grid.trim_end(0, 0), Some(0.75));
        // Launch -> activate applies the stored trim to the channel window.
        grid.launch_at(0, 0, 0.0);
        grid.transport_start(&mut channels);
        grid.before_tick(&mut channels);
        assert_eq!(grid.active_row(0), Some(0));
        assert!((channels[0].loop_start() - 0.25).abs() < 1e-6);
        assert!((channels[0].loop_end() - 0.75).abs() < 1e-6);
        // Cursor landed at the trimmed loop start.
        assert!((channels[0].position_normalized() - 0.25).abs() < 1e-3);
    }

    #[test]
    fn immediate_retrim_rewrites_active_window_without_restart() {
        let mut grid = ClipGrid::new(SR, 60.0);
        let mut channels = channels();
        let (buffer, bpm) = clip(0.5, 400, 60.0);
        grid.load(0, 0, buffer, bpm);
        grid.launch_at(0, 0, 0.0);
        grid.transport_start(&mut channels);
        grid.before_tick(&mut channels);
        // Advance the playhead into the middle of the full loop (~0.2).
        step(&mut grid, &mut channels, 80);
        let before = channels[0].position_normalized();
        assert!(before > 0.1, "cursor did not advance: {before}");
        // Retrim to a window that still contains the cursor -> no jump/restart.
        assert!(grid.set_trim(0, 0, 0.1, 0.9, RetrimTiming::Immediate, &mut channels));
        let after = channels[0].position_normalized();
        assert!(
            (after - before).abs() < 1e-6,
            "cursor moved: {before} -> {after}"
        );
        assert!((channels[0].loop_start() - 0.1).abs() < 1e-6);
        assert!((channels[0].loop_end() - 0.9).abs() < 1e-6);
    }

    #[test]
    fn quantized_retrim_fires_at_boundary_and_coexists_with_pending_launch() {
        let mut grid = ClipGrid::new(SR, 60.0);
        let mut channels = channels();
        let (a, bpm) = clip(0.3, 400, 60.0);
        let (b, _) = clip(0.9, 400, 60.0);
        grid.load(0, 0, a, bpm);
        grid.load(0, 1, b, bpm);
        grid.launch_at(0, 0, 0.0);
        grid.transport_start(&mut channels);
        grid.before_tick(&mut channels); // activate row 0 at beat 0

        // Queue a retrim (sixteenth = 0.25 beat) AND a launch of row 1 (beat 1).
        assert!(grid.set_trim(
            0,
            0,
            0.8,
            0.95,
            RetrimTiming::Quantized(LaunchQuantization::Sixteenth),
            &mut channels,
        ));
        assert!(grid.launch_at(0, 1, 1.0));
        // Both coexist: launch pending on row 1, retrim not yet applied.
        assert_eq!(grid.queued_row(0), Some(1));
        assert!((channels[0].loop_start() - 0.0).abs() < 1e-6);

        // Cross the sixteenth boundary (0.25 beat = 25 samples) but not beat 1.
        step(&mut grid, &mut channels, 30);
        assert!(
            (channels[0].loop_start() - 0.8).abs() < 1e-6,
            "retrim not applied: {}",
            channels[0].loop_start()
        );
        assert_eq!(grid.active_row(0), Some(0)); // no detach
        assert_eq!(grid.queued_row(0), Some(1)); // launch survived

        // Reach the launch boundary (beat 1 = 100 samples from start).
        step(&mut grid, &mut channels, 100);
        assert_eq!(grid.active_row(0), Some(1));
    }

    #[test]
    fn set_trim_rejects_degenerate() {
        let mut grid = ClipGrid::new(SR, 60.0);
        let mut channels = channels();
        let (buffer, bpm) = clip(0.5, 100, 60.0);
        grid.load(0, 0, buffer, bpm);
        // start == end
        assert!(!grid.set_trim(0, 0, 0.5, 0.5, RetrimTiming::Immediate, &mut channels));
        // out of range
        assert!(!grid.set_trim(0, 0, -0.1, 0.5, RetrimTiming::Immediate, &mut channels));
        assert!(!grid.set_trim(0, 0, 0.5, 1.5, RetrimTiming::Immediate, &mut channels));
        // non-finite
        assert!(!grid.set_trim(0, 0, f64::NAN, 0.5, RetrimTiming::Immediate, &mut channels));
        // sub-sample window: 0.005 * 100 frames = 0.5 frame < 1
        assert!(!grid.set_trim(0, 0, 0.0, 0.005, RetrimTiming::Immediate, &mut channels));
        // invalid slot
        assert!(!grid.set_trim(
            CLIP_COLUMN_COUNT,
            0,
            0.2,
            0.8,
            RetrimTiming::Immediate,
            &mut channels
        ));
        // valid
        assert!(grid.set_trim(0, 0, 0.2, 0.8, RetrimTiming::Immediate, &mut channels));
    }

    #[test]
    fn transport_seek_maps_phase_into_trimmed_window() {
        // Cropped (non-wrap) window: middle phase lands in the crop's middle.
        {
            let mut grid = ClipGrid::new(SR, 60.0);
            let mut channels = channels();
            let (buffer, bpm) = clip(0.5, 400, 60.0); // 4 beats
            grid.load(0, 0, buffer, bpm);
            grid.set_trim(0, 0, 0.25, 0.75, RetrimTiming::Immediate, &mut channels);
            grid.launch_at(0, 0, 0.0);
            grid.transport_start(&mut channels);
            grid.before_tick(&mut channels);
            assert!(grid.transport_seek(2.0, &mut channels)); // phase 0.5 of 4 beats
            assert!((channels[0].position_normalized() - 0.5).abs() < 0.01);
        }
        // Wrapped window: half the span crosses the seam to frame 0.
        {
            let mut grid = ClipGrid::new(SR, 60.0);
            let mut channels = channels();
            let (buffer, bpm) = clip(0.5, 400, 60.0);
            grid.load(0, 0, buffer, bpm);
            grid.set_trim(0, 0, 0.75, 0.25, RetrimTiming::Immediate, &mut channels);
            grid.launch_at(0, 0, 0.0);
            grid.transport_start(&mut channels);
            grid.before_tick(&mut channels);
            assert!(grid.transport_seek(2.0, &mut channels)); // phase 0.5 -> seam
            assert!(channels[0].position_normalized() < 0.01);
        }
    }
    #[test]
    fn phase_aligned_join_and_cycle_match_existing_audio_sample_for_sample() {
        let mut grid = ClipGrid::new(SR, 60.0);
        grid.set_phase_aligned_launches(true);
        let mut channels = channels();
        for channel in &mut channels {
            channel.set_engine_bpm(60.0);
        }
        let samples: Vec<f32> = (0..1600).map(|i| i as f32 / 1600.0).collect();
        let buffer = StereoSampleBuffer::from_channels(samples.clone(), samples, SR).unwrap();
        grid.load(0, 0, buffer.clone(), 60.0);
        grid.launch_at(0, 0, 0.0);
        grid.transport_start(&mut channels);
        step(&mut grid, &mut channels, 700);
        grid.load(1, 0, buffer, 60.0);
        grid.launch_quantized(1, 0, LaunchQuantization::Bar);
        step(&mut grid, &mut channels, 100);
        assert_eq!(grid.active_row(1), None);
        for sample in 800..3201 {
            grid.before_tick(&mut channels);
            let a = channels[0].tick(SR);
            let b = channels[1].tick(SR);
            assert_eq!(a.l, b.l, "mismatched source audio at sample {sample}");
            if sample % 1600 == 0 {
                assert_eq!(a.l, 0.0);
            }
            grid.after_tick();
        }
    }

    #[test]
    fn phase_aligned_stretch_does_not_drift_after_tempo_changes() {
        let mut grid = ClipGrid::new(SR, 60.0);
        grid.set_phase_aligned_launches(true);
        let mut channels = channels();
        for channel in &mut channels {
            channel.set_engine_bpm(60.0);
        }
        let (buffer, bpm) = clip(0.25, 1600, 60.0);
        grid.load(0, 0, buffer.clone(), bpm);
        grid.launch_at(0, 0, 0.0);
        grid.transport_start(&mut channels);
        step(&mut grid, &mut channels, 700);
        grid.load(1, 0, buffer, bpm);
        grid.launch_quantized(1, 0, LaunchQuantization::Bar);
        grid.set_bpm(90.0);
        for channel in &mut channels {
            channel.set_engine_bpm(90.0);
        }
        step(&mut grid, &mut channels, 68);
        assert_eq!(grid.active_row(1), Some(0));
        for _ in 0..5000 {
            grid.before_tick(&mut channels);
            for channel in channels.iter_mut().take(2) {
                let out = channel.tick(SR);
                assert!(out.l.is_finite());
            }
            assert!(
                (channels[0].position_normalized() - channels[1].position_normalized()).abs()
                    < 1e-7
            );
            grid.after_tick();
        }
    }

    #[test]
    fn phase_aligned_trimmed_and_wrapped_windows_use_their_musical_duration() {
        for (start, end, expected) in [(0.25, 0.75, 0.5), (0.75, 0.25, 0.0)] {
            let mut grid = ClipGrid::new(SR, 60.0);
            grid.set_phase_aligned_launches(true);
            let mut channels = channels();
            for channel in &mut channels {
                channel.set_engine_bpm(60.0);
            }
            let (buffer, bpm) = clip(0.25, 1600, 60.0);
            grid.load(0, 0, buffer, bpm);
            grid.set_trim(0, 0, start, end, RetrimTiming::Immediate, &mut channels);
            grid.transport_start(&mut channels);
            grid.transport_seek(3.0, &mut channels);
            grid.launch_quantized(0, 0, LaunchQuantization::Bar);
            step(&mut grid, &mut channels, 101);
            assert!(
                (channels[0].position_normalized() - expected).abs() < 1e-6,
                "{} != {expected}, beat {}",
                channels[0].position_normalized(),
                grid.transport_beat()
            );
        }
    }
}

#[cfg(test)]
mod playback_tests {
    use super::*;
    const SR: f32 = 8_000.0;

    fn tone() -> StereoSampleBuffer {
        let data: Vec<f32> = (0..32_000)
            .map(|i| (i as f32 * std::f32::consts::TAU * 220.0 / SR).sin() * 0.3)
            .collect();
        StereoSampleBuffer::from_interleaved(&data, 1, SR).unwrap()
    }
    fn setup(speed: f32, preserve: bool, bpm: f32) -> (ClipGrid, Vec<LoopChannel>) {
        let mut grid = ClipGrid::new(SR, bpm);
        grid.set_phase_aligned_launches(true);
        let mut channels: Vec<_> = (0..CLIP_COLUMN_COUNT)
            .map(|_| LoopChannel::new(SR))
            .collect();
        for ch in &mut channels {
            ch.set_engine_bpm(bpm);
        }
        assert!(grid.load_with_playback(0, 0, tone(), 120.0, speed, preserve));
        assert!(grid.launch_quantized(0, 0, LaunchQuantization::Bar));
        grid.transport_start(&mut channels);
        (grid, channels)
    }
    fn render(grid: &mut ClipGrid, channels: &mut [LoopChannel], count: usize) -> Vec<f32> {
        (0..count)
            .map(|_| {
                grid.before_tick(channels);
                let frame = channels[0].tick(SR).l;
                grid.after_tick();
                frame
            })
            .collect()
    }
    fn frequency(samples: &[f32]) -> f32 {
        let crossings = samples
            .windows(2)
            .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
            .count();
        crossings as f32 * SR / samples.len() as f32
    }

    #[test]
    fn speed_changes_duration_and_preservation_compensates_both_speed_and_bpm() {
        for speed in [0.25, 0.5, 1.0, 2.0, 4.0] {
            for bpm in [90.0, 120.0, 180.0] {
                for preserve in [false, true] {
                    let (mut grid, mut channels) = setup(speed, preserve, bpm);
                    let samples = render(&mut grid, &mut channels, 16_000);
                    assert!(samples.iter().all(|x| x.is_finite() && x.abs() <= 0.31));
                    let hz = frequency(&samples[4_000..]);
                    let expected = if preserve {
                        220.0
                    } else {
                        220.0 * speed * bpm / 120.0
                    };
                    assert!(
                        (hz - expected).abs() < expected * 0.08 + 2.0,
                        "speed={speed}, bpm={bpm}, preserve={preserve}: {hz} vs {expected}"
                    );
                    let phase =
                        ((15_999.0 * bpm as f64 / (60.0 * SR as f64) * speed as f64) % 8.0) / 8.0;
                    assert!((channels[0].position_normalized() as f64 - phase).abs() < 1e-5);
                }
            }
        }
    }

    #[test]
    fn rendered_pulses_repeat_at_the_adjusted_loop_duration() {
        let data: Vec<f32> = (0..4_000)
            .map(|i| {
                if (1_000..1_400).contains(&i) {
                    0.2
                } else {
                    0.0
                }
            })
            .collect();
        let pulse = StereoSampleBuffer::from_interleaved(&data, 1, SR).unwrap();
        for speed in [0.25, 0.5, 1.0, 2.0, 4.0] {
            for preserve in [false, true] {
                let (mut grid, mut channels) = setup(speed, preserve, 120.0);
                grid.transport_stop(&mut channels);
                assert!(grid.load_with_playback(0, 0, pulse.clone(), 120.0, speed, preserve));
                grid.transport_reset(&mut channels);
                assert!(grid.launch_quantized(0, 0, LaunchQuantization::Bar));
                grid.transport_start(&mut channels);
                let period = 4_000.0 / speed as f64;
                let samples = render(&mut grid, &mut channels, (period * 4.0) as usize);
                let mut starts = Vec::new();
                let mut previous = None;
                for (i, x) in samples.iter().enumerate() {
                    if *x > 0.05 {
                        if previous.is_none_or(|p| i - p > 400) {
                            starts.push(i);
                        }
                        previous = Some(i);
                    }
                }
                assert_eq!(
                    starts.len(),
                    4,
                    "speed={speed}, preserve={preserve}: {starts:?}"
                );
                for pair in starts.windows(2) {
                    assert!(
                        ((pair[1] - pair[0]) as f64 - period).abs() < 250.0,
                        "speed={speed}, preserve={preserve}: {starts:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn live_bpm_changes_at_non_neutral_speed_keep_pitch_and_position() {
        let (mut grid, mut channels) = setup(2.0, true, 120.0);
        render(&mut grid, &mut channels, 5_000);
        let before = channels[0].position_normalized();
        grid.set_bpm(180.0);
        channels[0].set_engine_bpm(180.0);
        render(&mut grid, &mut channels, 1);
        assert!((channels[0].position_normalized() - before).abs() < 0.001);
        let samples = render(&mut grid, &mut channels, 8_000);
        assert!((frequency(&samples[2_000..]) - 220.0).abs() < 20.0);
    }

    #[test]
    fn live_rate_and_mode_changes_keep_position_and_other_channels_untouched() {
        let (mut grid, mut channels) = setup(1.0, true, 120.0);
        assert!(grid.load_with_playback(1, 0, tone(), 120.0, 1.0, true));
        assert!(grid.launch_quantized(1, 0, LaunchQuantization::Bar));
        render(&mut grid, &mut channels, 5_000);
        let before = channels[0].position_normalized();
        assert!(grid.set_playback(0, 0, 4.0, false, &mut channels));
        render(&mut grid, &mut channels, 1);
        assert!((channels[0].position_normalized() - before).abs() < 0.001);
        assert_eq!(grid.slots[1][0].as_ref().unwrap().speed, 1.0);
        let samples = render(&mut grid, &mut channels, 4_000);
        assert!(samples.iter().all(|x| x.is_finite() && x.abs() <= 0.31));
        assert!(grid.set_playback(0, 0, 0.25, true, &mut channels));
        let samples = render(&mut grid, &mut channels, 8_000);
        assert!(samples.iter().all(|x| x.is_finite() && x.abs() <= 0.31));
        assert!((frequency(&samples[4_000..]) - 220.0).abs() < 20.0);
    }

    #[test]
    fn replacement_settings_wait_for_swap_and_relaunch_applies_stored_rate() {
        let (mut grid, mut channels) = setup(1.0, true, 120.0);
        render(&mut grid, &mut channels, 4_000);
        assert!(grid.load_with_playback(0, 0, tone(), 120.0, 4.0, false));
        assert!(grid.set_playback(0, 0, 2.0, false, &mut channels));
        assert_eq!(grid.columns[0].active_clip.as_ref().unwrap().speed, 1.0);
        render(&mut grid, &mut channels, 12_001);
        assert_eq!(grid.columns[0].active_clip.as_ref().unwrap().speed, 2.0);
        assert!((channels[0].position_normalized() - 0.0).abs() < 1e-5);
        assert!(grid.launch_quantized(0, 0, LaunchQuantization::Bar));
        render(&mut grid, &mut channels, 16_000);
        assert_eq!(grid.columns[0].active_clip.as_ref().unwrap().speed, 2.0);
    }

    #[test]
    fn seek_pause_and_wrapped_trim_respect_playback_speed() {
        let (mut grid, mut channels) = setup(2.0, true, 120.0);
        render(&mut grid, &mut channels, 1);
        assert!(grid.set_trim(0, 0, 0.75, 0.25, RetrimTiming::Immediate, &mut channels));
        assert!(grid.transport_seek(1.0, &mut channels));
        render(&mut grid, &mut channels, 1);
        assert!(channels[0].position_normalized().abs() < 1e-5);
        grid.transport_stop(&mut channels);
        let before = channels[0].position_normalized();
        render(&mut grid, &mut channels, 1_000);
        assert_eq!(channels[0].position_normalized(), before);
        grid.transport_start(&mut channels);
        let samples = render(&mut grid, &mut channels, 5_000);
        assert!(samples.iter().all(|x| x.is_finite() && x.abs() <= 0.31));
        grid.transport_reset(&mut channels);
        render(&mut grid, &mut channels, 1);
        assert!((channels[0].position_normalized() - 0.75).abs() < 1e-5);
    }

    #[test]
    fn invalid_playback_edits_do_not_replace_clip_or_change_settings() {
        let (mut grid, mut channels) = setup(1.0, true, 120.0);
        for speed in [f32::NAN, f32::INFINITY, 0.0, -1.0, 0.249, 4.001] {
            assert!(!grid.load_with_playback(0, 0, tone(), 120.0, speed, false));
            assert!(!grid.set_playback(0, 0, speed, false, &mut channels));
        }
        assert!(!grid.set_playback(0, 1, 2.0, false, &mut channels));
        assert!(!grid.set_playback(CLIP_COLUMN_COUNT, 0, 2.0, false, &mut channels));
        assert_eq!(grid.slots[0][0].as_ref().unwrap().speed, 1.0);
        render(&mut grid, &mut channels, 1);
        assert_eq!(grid.columns[0].active_clip.as_ref().unwrap().speed, 1.0);
    }

    fn transposed(
        speed: f32,
        preserve: bool,
        bpm: f32,
        semitones: f32,
    ) -> (ClipGrid, Vec<LoopChannel>) {
        let (mut grid, mut channels) = setup(speed, preserve, bpm);
        // Stored before the first render sample, so activation applies it.
        assert!(grid.set_transpose(0, 0, semitones, &mut channels));
        (grid, channels)
    }

    #[test]
    fn transpose_shifts_pitch_and_stacks_on_tape_without_moving_position() {
        for semitones in [-12.0, -5.0, 7.0, 12.0] {
            for speed in [0.5, 1.0, 2.0] {
                for bpm in [120.0, 180.0] {
                    for preserve in [true, false] {
                        let (mut grid, mut channels) = transposed(speed, preserve, bpm, semitones);
                        let (mut ref_grid, mut ref_channels) = setup(speed, preserve, bpm);
                        let samples = render(&mut grid, &mut channels, 16_000);
                        render(&mut ref_grid, &mut ref_channels, 16_000);
                        assert!(samples.iter().all(|x| x.is_finite() && x.abs() <= 0.31));
                        let tape = if preserve { 1.0 } else { speed * bpm / 120.0 };
                        let expected = 220.0 * 2f32.powf(semitones / 12.0) * tape;
                        let hz = frequency(&samples[4_000..]);
                        assert!(
                            (hz - expected).abs() < expected * 0.08 + 2.0,
                            "st={semitones}, speed={speed}, bpm={bpm}, preserve={preserve}: \
                             {hz} vs {expected}"
                        );
                        assert_eq!(
                            channels[0].position_normalized(),
                            ref_channels[0].position_normalized(),
                            "st={semitones}, speed={speed}, bpm={bpm}, preserve={preserve}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn transposed_pulses_keep_the_loop_period() {
        let data: Vec<f32> = (0..4_000)
            .map(|i| {
                if (1_000..1_400).contains(&i) {
                    0.2
                } else {
                    0.0
                }
            })
            .collect();
        let pulse = StereoSampleBuffer::from_interleaved(&data, 1, SR).unwrap();
        for semitones in [-12.0, 12.0] {
            for speed in [0.5, 1.0, 2.0] {
                let (mut grid, mut channels) = setup(speed, true, 120.0);
                grid.transport_stop(&mut channels);
                assert!(grid.load_with_playback(0, 0, pulse.clone(), 120.0, speed, true));
                assert!(grid.set_transpose(0, 0, semitones, &mut channels));
                grid.transport_reset(&mut channels);
                assert!(grid.launch_quantized(0, 0, LaunchQuantization::Bar));
                grid.transport_start(&mut channels);
                let period = 4_000.0 / speed as f64;
                let samples = render(&mut grid, &mut channels, (period * 4.0) as usize);
                let mut starts = Vec::new();
                let mut previous = None;
                for (i, x) in samples.iter().enumerate() {
                    if x.abs() > 0.05 {
                        if previous.is_none_or(|p| i - p > 400) {
                            starts.push(i);
                        }
                        previous = Some(i);
                    }
                }
                assert_eq!(starts.len(), 4, "st={semitones}, speed={speed}: {starts:?}");
                for pair in starts.windows(2) {
                    assert!(
                        ((pair[1] - pair[0]) as f64 - period).abs() < 250.0,
                        "st={semitones}, speed={speed}: {starts:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn live_transpose_keeps_position_and_returning_to_zero_restores_direct_audio() {
        let (mut grid, mut channels) = setup(1.0, false, 120.0);
        let (mut ref_grid, mut ref_channels) = setup(1.0, false, 120.0);
        render(&mut grid, &mut channels, 5_000);
        render(&mut ref_grid, &mut ref_channels, 5_000);
        assert!(grid.set_transpose(0, 0, 7.0, &mut channels));
        let samples = render(&mut grid, &mut channels, 8_000);
        render(&mut ref_grid, &mut ref_channels, 8_000);
        assert_eq!(
            channels[0].position_normalized(),
            ref_channels[0].position_normalized()
        );
        let expected = 220.0 * 2f32.powf(7.0 / 12.0);
        assert!((frequency(&samples[4_000..]) - expected).abs() < expected * 0.08);
        assert!(grid.set_transpose(0, 0, 0.0, &mut channels));
        render(&mut grid, &mut channels, 4_000);
        let back = render(&mut grid, &mut channels, 1_000);
        render(&mut ref_grid, &mut ref_channels, 4_000);
        assert_eq!(back, render(&mut ref_grid, &mut ref_channels, 1_000));
    }

    #[test]
    fn replacement_transpose_waits_for_swap_and_other_columns_are_untouched() {
        let (mut grid, mut channels) = setup(1.0, true, 120.0);
        assert!(grid.load_with_playback(1, 0, tone(), 120.0, 1.0, true));
        render(&mut grid, &mut channels, 4_000);
        assert!(grid.load_with_playback(0, 0, tone(), 120.0, 1.0, true));
        assert!(grid.set_transpose(0, 0, 5.0, &mut channels));
        assert_eq!(grid.columns[0].active_clip.as_ref().unwrap().transpose, 0.0);
        let before = render(&mut grid, &mut channels, 2_000);
        assert!((frequency(&before) - 220.0).abs() < 20.0);
        render(&mut grid, &mut channels, 10_001);
        assert_eq!(grid.columns[0].active_clip.as_ref().unwrap().transpose, 5.0);
        assert_eq!(grid.slots[1][0].as_ref().unwrap().transpose, 0.0);
    }

    /// Smallest RMS over sliding windows of roughly one 220 Hz period.
    fn min_window_rms(samples: &[f32]) -> f32 {
        samples
            .windows(36)
            .step_by(4)
            .map(|w| (w.iter().map(|x| x * x).sum::<f32>() / w.len() as f32).sqrt())
            .fold(f32::INFINITY, f32::min)
    }

    /// Largest sample-to-sample step: a 0.3 sine at 330 Hz moves < 0.08.
    fn max_step(samples: &[f32]) -> f32 {
        samples
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max)
    }

    #[test]
    fn engaging_and_bypassing_the_stretcher_is_click_free() {
        // Steady 0.3 sine: RMS ~0.21. A freshly reset WSOLA hop used to start
        // at the zero edge of its window, dropping the level toward silence,
        // and bypass used to switch to the direct read at a different phase.
        let edits: [(f32, bool, f32); 3] = [(1.0, false, 7.0), (1.0, true, -5.0), (2.0, true, 0.0)];
        for (speed, preserve, semitones) in edits {
            let (mut grid, mut channels) = setup(1.0, preserve, 120.0);
            let mut before = render(&mut grid, &mut channels, 2_000);
            assert!(grid.set_playback(0, 0, speed, preserve, &mut channels));
            assert!(grid.set_transpose(0, 0, semitones, &mut channels));
            let engage = render(&mut grid, &mut channels, 2_000);
            let label = format!("speed={speed} st={semitones}");
            assert!(
                min_window_rms(&engage) > 0.15,
                "engage {label}: {}",
                min_window_rms(&engage)
            );
            before.extend_from_slice(&engage);
            assert!(
                max_step(&before) < 0.1,
                "engage {label}: {}",
                max_step(&before)
            );
            assert!(grid.set_playback(0, 0, 1.0, preserve, &mut channels));
            assert!(grid.set_transpose(0, 0, 0.0, &mut channels));
            let mut bypass = engage[1_999..].to_vec();
            bypass.extend(render(&mut grid, &mut channels, 2_000));
            assert!(
                max_step(&bypass) < 0.1,
                "bypass {label}: {}",
                max_step(&bypass)
            );
            // The tail returns to the untouched direct level.
            assert!(min_window_rms(&bypass[1_000..]) > 0.19, "settled {label}");
        }
    }

    #[test]
    fn retrim_keeps_the_playhead_without_phase_aligned_launches() {
        for timing in [
            RetrimTiming::Immediate,
            RetrimTiming::Quantized(LaunchQuantization::Sixteenth),
        ] {
            let mut grid = ClipGrid::new(SR, 120.0);
            let mut channels: Vec<_> = (0..CLIP_COLUMN_COUNT)
                .map(|_| LoopChannel::new(SR))
                .collect();
            for ch in &mut channels {
                ch.set_engine_bpm(120.0);
            }
            assert!(grid.load_with_playback(0, 0, tone(), 120.0, 1.0, true));
            assert!(grid.set_transpose(0, 0, 3.0, &mut channels));
            assert!(grid.launch_quantized(0, 0, LaunchQuantization::Bar));
            grid.transport_start(&mut channels);
            // Full loop is 32_000 frames; stop just short of a sixteenth boundary.
            render(&mut grid, &mut channels, 6_399);
            let before = channels[0].position_normalized();
            assert!(grid.set_trim(0, 0, 0.1, 0.9, timing, &mut channels));
            render(&mut grid, &mut channels, 2);
            let after = channels[0].position_normalized();
            let step = 2.0 / 32_000.0;
            assert!(
                (after - before - step).abs() < 1e-5,
                "{timing:?}: {before} -> {after}"
            );
        }
    }

    #[test]
    fn phase_aligned_retrim_follows_the_shared_phrase_position() {
        let (mut grid, mut channels) = setup(1.0, true, 120.0);
        render(&mut grid, &mut channels, 6_400);
        assert!(grid.set_trim(0, 0, 0.1, 0.9, RetrimTiming::Immediate, &mut channels));
        render(&mut grid, &mut channels, 1);
        // The full 32_000-frame tone is 8 beats; the trimmed window is 6.4.
        let beat = 6_400.0 * 120.0 / (60.0 * SR as f64);
        let expected = 0.1 + (beat % 6.4) / 8.0;
        assert!((channels[0].position_normalized() as f64 - expected).abs() < 1e-4);
    }

    #[test]
    fn invalid_transpose_edits_are_rejected() {
        let (mut grid, mut channels) = setup(1.0, true, 120.0);
        for semitones in [f32::NAN, f32::INFINITY, -24.001, 24.001] {
            assert!(!grid.set_transpose(0, 0, semitones, &mut channels));
        }
        assert!(!grid.set_transpose(0, 1, 2.0, &mut channels));
        assert!(!grid.set_transpose(CLIP_COLUMN_COUNT, 0, 2.0, &mut channels));
        assert!(grid.set_transpose(0, 0, -24.0, &mut channels));
        assert!(grid.set_transpose(0, 0, 24.0, &mut channels));
        assert_eq!(grid.slots[0][0].as_ref().unwrap().transpose, 24.0);
    }

    #[test]
    fn tiny_wrapped_windows_are_bounded_in_both_modes() {
        for preserve in [true, false] {
            let (mut grid, mut channels) = setup(4.0, preserve, 180.0);
            render(&mut grid, &mut channels, 1);
            assert!(grid.set_trim(0, 0, 0.9999, 0.0001, RetrimTiming::Immediate, &mut channels));
            let samples = render(&mut grid, &mut channels, 4_000);
            assert!(samples.iter().all(|x| x.is_finite() && x.abs() <= 0.31));
        }
    }
}
