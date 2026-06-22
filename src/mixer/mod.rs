//! Multi-channel stereo loop mixer — the core of libgooey's "library owns the
//! audio channels" model.
//!
//! A [`Mixer`] owns a fixed set of [`LoopChannel`]s, each a stereo loop player
//! with its own gain fader, mute/solo, loop window, varispeed, and an arbitrary
//! per-channel [`EffectChain`]. Both the native [`crate::engine::Engine`] and the
//! FFI `GooeyEngine` embed a `Mixer`; the FFI layer simply exposes control over
//! it. The mixer sums its channels into a single stereo frame that the host
//! engine adds to its master bus before the global effects + limiter.

pub mod effect_chain;
pub mod loop_channel;
pub mod stereo_buffer;

pub use effect_chain::{ChannelEffect, EffectChain};
pub use loop_channel::LoopChannel;
pub use stereo_buffer::StereoSampleBuffer;

use crate::frame::StereoFrame;

/// Number of loop channels in the mixer.
pub const LOOP_CHANNEL_COUNT: usize = 4;

/// Default tempo used to seed note-synced per-channel effects (e.g. delay) until
/// the host engine pushes its own BPM via [`Mixer::set_bpm`].
const DEFAULT_BPM: f32 = 120.0;

pub struct Mixer {
    channels: Vec<LoopChannel>,
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
            sample_rate,
            bpm: DEFAULT_BPM,
        }
    }

    /// Sum all channels into one stereo frame, honoring mute/solo: if any channel
    /// is soloed, only soloed channels sound; otherwise all un-muted channels do.
    pub fn tick(&mut self, engine_sample_rate: f32) -> StereoFrame {
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
        out
    }

    /// Update the tempo used when creating new note-synced per-channel effects.
    pub fn set_bpm(&mut self, bpm: f32) {
        self.bpm = bpm;
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
        match self.channels.get_mut(channel) {
            Some(ch) => {
                ch.set_buffer(buffer);
                true
            }
            None => false,
        }
    }

    pub fn set_playing(&mut self, channel: usize, playing: bool) {
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
        if let Some(ch) = self.channels.get_mut(channel) {
            ch.set_loop_start(normalized);
        }
    }

    pub fn set_loop_end(&mut self, channel: usize, normalized: f32) {
        if let Some(ch) = self.channels.get_mut(channel) {
            ch.set_loop_end(normalized);
        }
    }

    pub fn set_speed(&mut self, channel: usize, speed: f32) {
        if let Some(ch) = self.channels.get_mut(channel) {
            ch.set_speed(speed);
        }
    }

    pub fn restart(&mut self, channel: usize) {
        if let Some(ch) = self.channels.get_mut(channel) {
            ch.restart();
        }
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
