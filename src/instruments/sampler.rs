//! Fixed-size sample-pad rack used by the FFI engine.
//!
//! The rack owns copied PCM slot data and a fixed voice pool, so playback does
//! not allocate on the audio thread.  The small `voice_gain` helper is kept
//! separate from decoding/playback deliberately: a future amplitude envelope
//! can replace it without changing slot storage or voice scheduling.

use std::sync::Arc;

use crate::engine::Sequencer;
use crate::frame::StereoFrame;

pub const SAMPLER_SLOT_COUNT: usize = 16;
pub const SAMPLER_VOICE_COUNT: usize = 32;

#[derive(Clone, Debug)]
pub struct SamplerBuffer {
    samples: Arc<[f32]>,
    frames: usize,
    channels: usize,
    sample_rate: f32,
}

impl SamplerBuffer {
    pub fn from_interleaved(
        samples: &[f32],
        frames: usize,
        channels: usize,
        sample_rate: f32,
    ) -> Result<Self, &'static str> {
        if !(channels == 1 || channels == 2)
            || frames == 0
            || !sample_rate.is_finite()
            || sample_rate <= 0.0
        {
            return Err("invalid sampler buffer format");
        }
        let expected = frames
            .checked_mul(channels)
            .ok_or("sampler buffer is too large")?;
        if samples.len() != expected || samples.iter().any(|sample| !sample.is_finite()) {
            return Err("invalid sampler buffer samples");
        }
        Ok(Self {
            samples: Arc::from(samples),
            frames,
            channels,
            sample_rate,
        })
    }

    pub fn frames(&self) -> usize {
        self.frames
    }
    pub fn channels(&self) -> usize {
        self.channels
    }
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    #[inline]
    fn frame(&self, position: f64) -> StereoFrame {
        let position = position.clamp(0.0, (self.frames - 1) as f64);
        let i0 = position.floor() as usize;
        let i1 = (i0 + 1).min(self.frames - 1);
        let frac = (position - i0 as f64) as f32;
        let sample = |frame: usize, channel: usize| self.samples[frame * self.channels + channel];
        let lerp = |a: f32, b: f32| a + (b - a) * frac;
        if self.channels == 1 {
            StereoFrame::mono(lerp(sample(i0, 0), sample(i1, 0)))
        } else {
            StereoFrame {
                l: lerp(sample(i0, 0), sample(i1, 0)),
                r: lerp(sample(i0, 1), sample(i1, 1)),
            }
        }
    }
}

#[derive(Clone)]
struct SampleVoice {
    buffer: Option<SamplerBuffer>,
    slot: usize,
    position: f64,
    increment: f64,
    velocity: f32,
    age: u64,
}

impl Default for SampleVoice {
    fn default() -> Self {
        Self {
            buffer: None,
            slot: 0,
            position: 0.0,
            increment: 1.0,
            velocity: 0.0,
            age: 0,
        }
    }
}

impl SampleVoice {
    fn active(&self) -> bool {
        self.buffer.is_some()
    }

    fn start(
        &mut self,
        slot: usize,
        buffer: SamplerBuffer,
        engine_rate: f32,
        velocity: f32,
        age: u64,
    ) {
        self.slot = slot;
        self.position = 0.0;
        self.increment = buffer.sample_rate() as f64 / engine_rate as f64;
        self.velocity = velocity.clamp(0.0, 1.0);
        self.age = age;
        self.buffer = Some(buffer);
    }

    fn tick(&mut self) -> StereoFrame {
        let Some(buffer) = self.buffer.as_ref() else {
            return StereoFrame::default();
        };
        let frame = buffer.frame(self.position);
        // Fixed click guard only. This is intentionally not an exposed envelope.
        let fade = 32.0_f64;
        let end = buffer.frames() as f64;
        let gain = (self.position / fade)
            .min(((end - self.position) / fade).max(0.0))
            .min(1.0) as f32
            * self.velocity;
        self.position += self.increment;
        if self.position >= end {
            self.buffer = None;
        }
        frame.scaled(gain)
    }
}

pub struct SamplerRack {
    sample_rate: f32,
    slots: [Option<SamplerBuffer>; SAMPLER_SLOT_COUNT],
    voices: [SampleVoice; SAMPLER_VOICE_COUNT],
    next_age: u64,
    sequencer: Sequencer,
    /// Pattern dispatch is opt-in. A registered rack must remain silent until
    /// the host explicitly starts it on the shared transport.
    pattern_running: bool,
    /// Absolute shared-transport beat at which a requested start lands.
    pending_start_beat: Option<f64>,
}

impl SamplerRack {
    pub fn new(sample_rate: f32, bpm: f32, name: impl Into<String>) -> Self {
        Self {
            sample_rate,
            slots: std::array::from_fn(|_| None),
            voices: std::array::from_fn(|_| SampleVoice::default()),
            next_age: 0,
            sequencer: Sequencer::with_pattern(
                bpm,
                sample_rate,
                vec![false; SAMPLER_SLOT_COUNT],
                name,
            ),
            pattern_running: false,
            pending_start_beat: None,
        }
    }

    pub fn set_buffer(&mut self, slot: usize, buffer: SamplerBuffer) -> bool {
        let Some(target) = self.slots.get_mut(slot) else {
            return false;
        };
        *target = Some(buffer);
        self.stop_slot(slot);
        true
    }

    pub fn clear_slot(&mut self, slot: usize) -> bool {
        let Some(target) = self.slots.get_mut(slot) else {
            return false;
        };
        *target = None;
        self.stop_slot(slot);
        true
    }

    pub fn slot(&self, slot: usize) -> Option<&SamplerBuffer> {
        self.slots.get(slot)?.as_ref()
    }

    pub fn trigger(&mut self, slot: usize, velocity: f32) -> bool {
        let Some(buffer) = self.slot(slot).cloned() else {
            return false;
        };
        let voice_index = self
            .voices
            .iter()
            .position(|voice| !voice.active())
            .unwrap_or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, voice)| voice.age)
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            });
        self.next_age = self.next_age.wrapping_add(1);
        self.voices[voice_index].start(slot, buffer, self.sample_rate, velocity, self.next_age);
        true
    }

    pub fn tick(&mut self) -> StereoFrame {
        self.voices
            .iter_mut()
            .fold(StereoFrame::default(), |out, voice| out + voice.tick())
    }

    pub fn set_step(&mut self, step: usize, enabled: bool, slot: usize, velocity: f32) -> bool {
        if step >= SAMPLER_SLOT_COUNT || slot >= SAMPLER_SLOT_COUNT {
            return false;
        }
        self.sequencer
            .set_step_with_velocity(step, enabled, velocity);
        self.sequencer.set_step_note(step, slot as u8);
        true
    }

    pub fn step(&self, step: usize) -> Option<(bool, usize, f32)> {
        (step < SAMPLER_SLOT_COUNT).then(|| {
            (
                self.sequencer.get_step_enabled(step),
                self.sequencer.get_step_note(step).unwrap_or(0) as usize,
                self.sequencer.get_step_velocity(step),
            )
        })
    }

    pub fn tick_sequencer(&mut self) -> Option<(usize, f32)> {
        if !self.pattern_running {
            return None;
        }
        self.sequencer
            .tick_with_settings()
            .map(|trigger| (trigger.note.unwrap_or(0) as usize, trigger.velocity))
    }
    pub fn sequencer_mut(&mut self) -> &mut Sequencer {
        &mut self.sequencer
    }
    pub fn sequencer(&self) -> &Sequencer {
        &self.sequencer
    }

    pub fn schedule_start(&mut self, beat: f64) -> bool {
        if !beat.is_finite() || beat < 0.0 {
            return false;
        }
        self.pattern_running = false;
        self.sequencer.stop();
        self.pending_start_beat = Some(beat);
        true
    }

    /// Called from the render thread before the sequencer is ticked.
    pub fn activate_start_if_due(&mut self, transport_beat: f64) {
        let Some(target) = self.pending_start_beat else { return };
        if transport_beat + 1.0e-8 < target {
            return;
        }
        self.pending_start_beat = None;
        self.sequencer.set_beat_position(target);
        self.sequencer.start();
        self.pattern_running = true;
    }

    pub fn stop_pattern(&mut self) {
        self.pending_start_beat = None;
        self.pattern_running = false;
        self.sequencer.stop();
        self.stop_all();
    }

    pub fn cancel_pending_start(&mut self) {
        self.pending_start_beat = None;
    }

    pub fn pending_start_beat(&self) -> Option<f64> {
        self.pending_start_beat
    }

    pub fn pattern_running(&self) -> bool {
        self.pattern_running
    }

    pub fn transport_stop(&mut self) {
        self.pending_start_beat = None;
        self.pattern_running = false;
        self.sequencer.stop();
        self.stop_all();
    }

    pub fn transport_reset(&mut self) {
        self.pending_start_beat = None;
        self.pattern_running = false;
        self.sequencer.reset();
        self.stop_all();
    }

    fn stop_all(&mut self) {
        for voice in &mut self.voices {
            voice.buffer = None;
        }
    }

    fn stop_slot(&mut self, slot: usize) {
        for voice in &mut self.voices {
            if voice.active() && voice.slot == slot {
                voice.buffer = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_buffer_is_interpolated_and_preserved() {
        let buffer =
            SamplerBuffer::from_interleaved(&[0.0, 1.0, 1.0, 0.0], 2, 2, 44_100.0).unwrap();
        let frame = buffer.frame(0.5);
        assert!((frame.l - 0.5).abs() < 1e-6 && (frame.r - 0.5).abs() < 1e-6);
    }

    #[test]
    fn rack_layers_and_steals_without_non_finite_audio() {
        let mut rack = SamplerRack::new(44_100.0, 120.0, "test");
        rack.set_buffer(
            0,
            SamplerBuffer::from_interleaved(&vec![0.5; 256], 256, 1, 22_050.0).unwrap(),
        );
        for _ in 0..(SAMPLER_VOICE_COUNT + 4) {
            assert!(rack.trigger(0, 1.0));
        }
        for _ in 0..32 {
            assert!(rack.tick().l.is_finite());
        }
    }
}
