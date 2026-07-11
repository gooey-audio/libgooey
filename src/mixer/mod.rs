//! Multi-channel stereo loop mixer — the core of libgooey's "library owns the
//! audio channels" model.
//!
//! A [`Mixer`] owns a fixed set of [`LoopChannel`]s, each a stereo loop player
//! with its own gain fader, mute/solo, loop window, varispeed, and an arbitrary
//! per-channel [`EffectChain`]. Both the native [`crate::engine::Engine`] and the
//! FFI `GooeyEngine` embed a `Mixer`; the FFI layer simply exposes control over
//! it. The mixer sums its channels into a single stereo frame that the host
//! engine adds to its master bus before the global effects + limiter.

pub mod clip_grid;
pub mod effect_chain;
pub mod graph;
pub mod loop_channel;
pub mod stereo_buffer;
mod wsola;

pub use clip_grid::{
    ClipGrid, LaunchQuantization, CLIP_COLUMN_COUNT, CLIP_QUANTIZE_BAR, CLIP_QUANTIZE_QUARTER,
    CLIP_QUANTIZE_SIXTEENTH, CLIP_ROW_COUNT, CLIP_STATE_LOADED, CLIP_STATE_PLAYING,
    CLIP_STATE_QUEUED,
};
pub use effect_chain::{ChannelEffect, EffectChain};
pub use graph::MixerGraph;
pub use loop_channel::{LoopChannel, PitchMode};
pub use stereo_buffer::StereoSampleBuffer;

use crate::frame::StereoFrame;

/// Number of loop channels in the mixer.
pub const LOOP_CHANNEL_COUNT: usize = 4;

/// Default tempo used to seed note-synced per-channel effects (e.g. delay) until
/// the host engine pushes its own BPM via [`Mixer::set_bpm`].
const DEFAULT_BPM: f32 = 120.0;

pub struct Mixer {
    channels: Vec<LoopChannel>,
    clip_grid: ClipGrid,
    sample_rate: f32,
    bpm: f32,
}

impl Mixer {
    /// Create a mixer with [`LOOP_CHANNEL_COUNT`] empty channels.
    pub fn new(sample_rate: f32) -> Self {
        let channels = (0..LOOP_CHANNEL_COUNT)
            .map(|_| LoopChannel::new(sample_rate))
            .collect();
        Self {
            channels,
            clip_grid: ClipGrid::new(sample_rate, DEFAULT_BPM),
            sample_rate,
            bpm: DEFAULT_BPM,
        }
    }

    /// Sum all channels into one stereo frame, honoring mute/solo: if any channel
    /// is soloed, only soloed channels sound; otherwise all un-muted channels do.
    pub fn tick(&mut self, engine_sample_rate: f32) -> StereoFrame {
        self.clip_grid.before_tick(&mut self.channels);
        let any_solo = self.channels.iter().any(LoopChannel::is_soloed);
        let mut out = StereoFrame::default();
        for channel in &mut self.channels {
            let audible = if any_solo {
                channel.is_soloed()
            } else {
                !channel.is_muted()
            };
            channel.set_active(audible);
            out += channel.tick(engine_sample_rate);
        }
        self.clip_grid.after_tick();
        out
    }

    /// Update the tempo for the mixer: re-tempo every existing note-synced
    /// per-channel effect (so existing delays follow host BPM changes) and seed
    /// the value used when creating new ones.
    pub fn set_bpm(&mut self, bpm: f32) {
        self.bpm = bpm;
        self.clip_grid.set_bpm(bpm);
        for channel in &mut self.channels {
            channel.effects().set_bpm(bpm);
            channel.set_engine_bpm(bpm);
        }
    }

    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    pub fn channel(&self, index: usize) -> Option<&LoopChannel> {
        self.channels.get(index)
    }

    pub fn channel_mut(&mut self, index: usize) -> Option<&mut LoopChannel> {
        self.channels.get_mut(index)
    }

    // --- Channel control (bounds-checked, FFI-friendly) -------------------

    pub fn load(&mut self, channel: usize, buffer: StereoSampleBuffer) -> bool {
        self.clip_grid.detach_column(channel);
        match self.channels.get_mut(channel) {
            Some(ch) => {
                ch.set_buffer(buffer);
                true
            }
            None => false,
        }
    }

    pub fn set_playing(&mut self, channel: usize, playing: bool) {
        self.clip_grid.detach_column(channel);
        if let Some(ch) = self.channels.get_mut(channel) {
            ch.set_playing(playing);
        }
    }

    pub fn set_gain(&mut self, channel: usize, gain: f32) {
        if let Some(ch) = self.channels.get_mut(channel) {
            ch.set_gain(gain);
        }
    }

    pub fn set_muted(&mut self, channel: usize, muted: bool) {
        if let Some(ch) = self.channels.get_mut(channel) {
            ch.set_muted(muted);
        }
    }

    pub fn set_soloed(&mut self, channel: usize, soloed: bool) {
        if let Some(ch) = self.channels.get_mut(channel) {
            ch.set_soloed(soloed);
        }
    }

    pub fn set_loop_start(&mut self, channel: usize, normalized: f32) {
        self.clip_grid.detach_column(channel);
        if let Some(ch) = self.channels.get_mut(channel) {
            ch.set_loop_start(normalized);
        }
    }

    pub fn set_loop_end(&mut self, channel: usize, normalized: f32) {
        self.clip_grid.detach_column(channel);
        if let Some(ch) = self.channels.get_mut(channel) {
            ch.set_loop_end(normalized);
        }
    }

    pub fn set_speed(&mut self, channel: usize, speed: f32) {
        self.clip_grid.detach_column(channel);
        if let Some(ch) = self.channels.get_mut(channel) {
            ch.set_speed(speed);
        }
    }

    pub fn restart(&mut self, channel: usize) {
        self.clip_grid.detach_column(channel);
        if let Some(ch) = self.channels.get_mut(channel) {
            ch.restart();
        }
    }

    pub fn set_position(&mut self, channel: usize, normalized: f32) {
        self.clip_grid.detach_column(channel);
        if let Some(ch) = self.channels.get_mut(channel) {
            ch.set_position(normalized);
        }
    }

    /// Tag a channel's loaded buffer with its source BPM (`None` clears the
    /// tag). No-op for a bad channel index or if no buffer is loaded.
    pub fn set_source_bpm(&mut self, channel: usize, bpm: Option<f32>) {
        self.clip_grid.detach_column(channel);
        if let Some(ch) = self.channels.get_mut(channel) {
            ch.set_source_bpm(bpm);
        }
    }

    /// A channel's tagged source BPM, or `None` for a bad channel index, no
    /// buffer, or no tag.
    pub fn source_bpm(&self, channel: usize) -> Option<f32> {
        self.channels.get(channel).and_then(LoopChannel::source_bpm)
    }

    pub fn set_pitch_mode(&mut self, channel: usize, mode: PitchMode) {
        self.clip_grid.detach_column(channel);
        if let Some(ch) = self.channels.get_mut(channel) {
            ch.set_pitch_mode(mode);
        }
    }

    /// A channel's pitch mode, or `PitchMode::Off` for a bad channel index.
    pub fn pitch_mode(&self, channel: usize) -> PitchMode {
        self.channels
            .get(channel)
            .map(LoopChannel::pitch_mode)
            .unwrap_or_default()
    }

    /// Stage a buffer to swap into `channel` at the next bar-grid boundary.
    /// Returns false for a bad channel index or an empty buffer.
    pub fn queue_swap(
        &mut self,
        channel: usize,
        buffer: StereoSampleBuffer,
        divisions: u32,
    ) -> bool {
        self.clip_grid.detach_column(channel);
        if buffer.is_empty() {
            return false;
        }
        match self.channels.get_mut(channel) {
            Some(ch) => {
                ch.queue_swap(buffer, divisions);
                true
            }
            None => false,
        }
    }

    pub fn cancel_queued_swap(&mut self, channel: usize) {
        self.clip_grid.detach_column(channel);
        if let Some(ch) = self.channels.get_mut(channel) {
            ch.cancel_queued_swap();
        }
    }

    pub fn swaps_completed(&self, channel: usize) -> u32 {
        self.channels
            .get(channel)
            .map_or(0, |ch| ch.swaps_completed())
    }

    // --- Transport-synchronized clip grid -------------------------------

    pub fn clip_load(
        &mut self,
        column: usize,
        row: usize,
        buffer: StereoSampleBuffer,
        source_bpm: f32,
    ) -> bool {
        self.clip_grid.load(column, row, buffer, source_bpm)
    }

    pub fn clip_unload(&mut self, column: usize, row: usize) -> bool {
        self.clip_grid.unload(column, row)
    }

    pub fn clip_clear(&mut self) {
        self.clip_grid.clear(&mut self.channels);
    }

    pub fn clip_launch(
        &mut self,
        column: usize,
        row: usize,
        quantization: LaunchQuantization,
    ) -> bool {
        self.clip_grid.launch_quantized(column, row, quantization)
    }

    pub fn clip_launch_at(&mut self, column: usize, row: usize, beat: f64) -> bool {
        self.clip_grid.launch_at(column, row, beat)
    }

    pub fn clip_launch_scene(&mut self, row: usize, quantization: LaunchQuantization) -> bool {
        self.clip_grid.launch_scene_quantized(row, quantization)
    }

    pub fn clip_launch_scene_at(&mut self, row: usize, beat: f64) -> bool {
        self.clip_grid.launch_scene_at(row, beat)
    }

    pub fn clip_stop(&mut self, column: usize, quantization: LaunchQuantization) -> bool {
        self.clip_grid.stop_quantized(column, quantization)
    }

    pub fn clip_stop_at(&mut self, column: usize, beat: f64) -> bool {
        self.clip_grid.stop_at(column, beat)
    }

    pub fn clip_cancel(&mut self, column: usize) {
        self.clip_grid.cancel(column);
    }

    pub fn clip_cancel_all(&mut self) {
        self.clip_grid.cancel_all();
    }

    pub fn clip_set_default_quantization(&mut self, quantization: LaunchQuantization) {
        self.clip_grid.set_default_quantization(quantization);
    }

    pub fn clip_default_quantization(&self) -> LaunchQuantization {
        self.clip_grid.default_quantization()
    }

    pub fn clip_slot_state(&self, column: usize, row: usize) -> u32 {
        self.clip_grid.slot_state(column, row)
    }

    pub fn clip_active_row(&self, column: usize) -> Option<usize> {
        self.clip_grid.active_row(column)
    }

    pub fn clip_queued_row(&self, column: usize) -> Option<usize> {
        self.clip_grid.queued_row(column)
    }

    pub fn clip_is_stop_queued(&self, column: usize) -> bool {
        self.clip_grid.is_stop_queued(column)
    }

    pub fn clip_scheduled_beat(&self, column: usize) -> Option<f64> {
        self.clip_grid.scheduled_beat(column)
    }

    pub fn transport_start(&mut self) {
        self.clip_grid.transport_start(&mut self.channels);
    }

    pub fn transport_stop(&mut self) {
        self.clip_grid.transport_stop(&mut self.channels);
    }

    pub fn transport_seek(&mut self, beat: f64) -> bool {
        self.clip_grid.transport_seek(beat, &mut self.channels)
    }

    pub fn transport_reset(&mut self) {
        self.clip_grid.transport_reset(&mut self.channels);
    }

    pub fn transport_beat(&self) -> f64 {
        self.clip_grid.transport_beat()
    }

    pub fn transport_running(&self) -> bool {
        self.clip_grid.transport_running()
    }

    // --- Per-channel effects ----------------------------------------------

    /// Append an effect to a channel. Returns the new effect's slot index, or
    /// `None` for a bad channel index or unknown effect id.
    pub fn effect_add(&mut self, channel: usize, effect_id: u32) -> Option<usize> {
        let (sample_rate, bpm) = (self.sample_rate, self.bpm);
        self.channels
            .get_mut(channel)?
            .effects_mut()
            .add(effect_id, sample_rate, bpm)
    }

    pub fn effect_remove(&mut self, channel: usize, slot: usize) -> bool {
        self.channels
            .get_mut(channel)
            .is_some_and(|ch| ch.effects_mut().remove(slot))
    }

    pub fn effect_move(&mut self, channel: usize, slot: usize, new_position: usize) -> bool {
        self.channels
            .get_mut(channel)
            .is_some_and(|ch| ch.effects_mut().move_effect(slot, new_position))
    }

    pub fn effect_clear(&mut self, channel: usize) {
        if let Some(ch) = self.channels.get_mut(channel) {
            ch.effects_mut().clear();
        }
    }

    pub fn effect_set_param(&self, channel: usize, slot: usize, param: u32, value: f32) {
        if let Some(ch) = self.channels.get(channel) {
            ch.effects().set_param(slot, param, value);
        }
    }

    pub fn effect_count(&self, channel: usize) -> usize {
        self.channels
            .get(channel)
            .map_or(0, |ch| ch.effects().len())
    }

    pub fn effect_type_at(&self, channel: usize, slot: usize) -> Option<u32> {
        self.channels
            .get(channel)
            .and_then(|ch| ch.effects().effect_type_at(slot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::EFFECT_DELAY;

    const SR: f32 = 44_100.0;

    fn dc_buffer(value: f32, frames: usize) -> StereoSampleBuffer {
        StereoSampleBuffer::from_channels(vec![value; frames], vec![value; frames], SR).unwrap()
    }

    #[test]
    fn default_channel_count() {
        let mixer = Mixer::new(SR);
        assert_eq!(mixer.channel_count(), LOOP_CHANNEL_COUNT);
    }

    #[test]
    fn muted_channel_drops_out_after_smoothing() {
        let mut mixer = Mixer::new(SR);
        mixer.load(0, dc_buffer(0.5, 64));
        mixer.set_playing(0, true);
        // Let the gain settle, then read a baseline level.
        for _ in 0..4096 {
            mixer.tick(SR);
        }
        let before = mixer.tick(SR).l.abs();
        assert!(before > 0.1, "expected audible channel, got {before}");

        mixer.set_muted(0, true);
        for _ in 0..4096 {
            mixer.tick(SR);
        }
        let after = mixer.tick(SR).l.abs();
        // The exponential gate is ~inaudible here (>50 dB down from the 0.5
        // baseline); it asymptotes toward zero rather than reaching it exactly.
        assert!(after < 5e-3, "muted channel should be silent, got {after}");
    }

    #[test]
    fn solo_silences_other_channels() {
        let mut mixer = Mixer::new(SR);
        mixer.load(0, dc_buffer(0.5, 64));
        mixer.load(1, dc_buffer(0.5, 64));
        mixer.set_playing(0, true);
        mixer.set_playing(1, true);
        mixer.set_soloed(0, true);
        for _ in 0..4096 {
            mixer.tick(SR);
        }
        // Only channel 0 should contribute (~0.5), not both (~1.0).
        let out = mixer.tick(SR).l;
        assert!(
            (out - 0.5).abs() < 0.05,
            "solo leaked other channels: {out}"
        );
    }

    #[test]
    fn effect_add_targets_only_its_channel() {
        let mut mixer = Mixer::new(SR);
        assert_eq!(mixer.effect_add(0, EFFECT_DELAY), Some(0));
        assert_eq!(mixer.effect_count(0), 1);
        assert_eq!(mixer.effect_count(1), 0);
        assert!(mixer.effect_add(99, EFFECT_DELAY).is_none());
    }
}
