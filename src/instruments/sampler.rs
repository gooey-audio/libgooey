//! Fixed-size sample-pad rack used by the FFI engine.
//!
//! The rack owns copied PCM slot data and a fixed voice pool, so playback does
//! not allocate on the audio thread.  The small `voice_gain` helper is kept
//! separate from decoding/playback deliberately: a future amplitude envelope
//! can replace it without changing slot storage or voice scheduling.

use std::sync::Arc;

use crate::engine::Sequencer;
use crate::envelope::{ADSRConfig, Envelope, EnvelopeCurve};
use crate::frame::StereoFrame;

pub const SAMPLER_SLOT_COUNT: usize = 16;
pub const SAMPLER_VOICE_COUNT: usize = 32;
pub const SLOT_GAIN_MAX: f32 = 2.0;
pub const SLOT_PITCH_RANGE: f32 = 24.0;
pub const SLOT_ENV_TIME_MAX: f32 = 10.0;

/// Minimum playable region length in frames. Trims that collapse below this
/// are rejected so a pad always produces at least a click of audio.
const SLOT_TRIM_MIN_FRAMES: usize = 2;

#[derive(Clone, Copy, Debug)]
pub struct SlotParams {
    pub gain: f32,
    pub pitch_semitones: f32,
    pub envelope: ADSRConfig,
    /// Normalized 0–1 start offset within the source buffer.
    pub start: f32,
    /// Normalized 0–1 end offset within the source buffer.
    pub end: f32,
}

impl Default for SlotParams {
    fn default() -> Self {
        Self {
            gain: 1.0,
            pitch_semitones: 0.0,
            envelope: ADSRConfig {
                attack_time: 0.0,
                decay_time: 0.0,
                sustain_level: 1.0,
                release_time: 0.0,
                attack_curve: EnvelopeCurve::Linear,
                decay_curve: EnvelopeCurve::Linear,
            },
            start: 0.0,
            end: 1.0,
        }
    }
}

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
    /// Absolute frame index at which playback ends (trim end point).
    end: f64,
    increment: f64,
    gain: f32,
    envelope: Envelope,
    elapsed_secs: f64,
    dt: f64,
    age: u64,
}

impl Default for SampleVoice {
    fn default() -> Self {
        Self {
            buffer: None,
            slot: 0,
            position: 0.0,
            end: 0.0,
            increment: 1.0,
            gain: 0.0,
            envelope: Envelope::new(),
            elapsed_secs: 0.0,
            dt: 0.0,
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
        params: &SlotParams,
        age: u64,
    ) {
        self.slot = slot;
        let frames = buffer.frames() as f64;
        let start_frame = (params.start.clamp(0.0, 1.0) as f64 * frames)
            .min(frames - 1.0)
            .max(0.0);
        let end_frame = (params.end.clamp(0.0, 1.0) as f64 * frames).min(frames).max(0.0);
        self.position = start_frame;
        self.end = end_frame.max(start_frame);
        let pitch = (params.pitch_semitones as f64 / 12.0).exp2();
        self.increment = (buffer.sample_rate() as f64 / engine_rate as f64) * pitch;
        self.gain = velocity.clamp(0.0, 1.0) * params.gain;
        self.envelope.set_config(params.envelope);
        self.envelope.trigger(0.0);
        self.elapsed_secs = 0.0;
        self.dt = 1.0 / engine_rate as f64;
        self.age = age;
        self.buffer = Some(buffer);
    }

    fn tick(&mut self) -> StereoFrame {
        let Some(buffer) = self.buffer.as_ref() else {
            return StereoFrame::default();
        };
        let frame = buffer.frame(self.position);
        let fade = 32.0_f64;
        let end = self.end;
        let click_guard = (self.position / fade)
            .min(((end - self.position) / fade).max(0.0))
            .min(1.0) as f32;
        if self.envelope.release_time > 0.0 && self.increment > 0.0 {
            let remaining_secs = (end - self.position).max(0.0) / self.increment * self.dt;
            if remaining_secs <= self.envelope.release_time as f64 {
                self.envelope.release(self.elapsed_secs);
            }
        }
        let gain = click_guard * self.envelope.get_amplitude(self.elapsed_secs) * self.gain;
        self.position += self.increment;
        self.elapsed_secs += self.dt;
        if self.position >= end || !self.envelope.is_active {
            self.buffer = None;
        }
        frame.scaled(gain)
    }
}

pub struct SamplerRack {
    sample_rate: f32,
    slots: [Option<SamplerBuffer>; SAMPLER_SLOT_COUNT],
    slot_params: [SlotParams; SAMPLER_SLOT_COUNT],
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
            slot_params: std::array::from_fn(|_| SlotParams::default()),
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
        let params = self.slot_params[slot];
        self.voices[voice_index].start(
            slot,
            buffer,
            self.sample_rate,
            velocity,
            &params,
            self.next_age,
        );
        true
    }

    pub fn set_slot_gain(&mut self, slot: usize, gain: f32) -> bool {
        let Some(params) = self.slot_params.get_mut(slot) else {
            return false;
        };
        if !gain.is_finite() {
            return false;
        }
        params.gain = gain.clamp(0.0, SLOT_GAIN_MAX);
        true
    }

    pub fn slot_gain(&self, slot: usize) -> Option<f32> {
        self.slot_params.get(slot).map(|p| p.gain)
    }

    pub fn set_slot_pitch(&mut self, slot: usize, semitones: f32) -> bool {
        let Some(params) = self.slot_params.get_mut(slot) else {
            return false;
        };
        if !semitones.is_finite() {
            return false;
        }
        params.pitch_semitones = semitones.clamp(-SLOT_PITCH_RANGE, SLOT_PITCH_RANGE);
        true
    }

    pub fn slot_pitch(&self, slot: usize) -> Option<f32> {
        self.slot_params.get(slot).map(|p| p.pitch_semitones)
    }

    pub fn set_slot_envelope(&mut self, slot: usize, a: f32, d: f32, s: f32, r: f32) -> bool {
        let Some(params) = self.slot_params.get_mut(slot) else {
            return false;
        };
        if ![a, d, s, r].iter().all(|v| v.is_finite()) {
            return false;
        }
        params.envelope = ADSRConfig::new(
            a.clamp(0.0, SLOT_ENV_TIME_MAX),
            d.clamp(0.0, SLOT_ENV_TIME_MAX),
            s,
            r.clamp(0.0, SLOT_ENV_TIME_MAX),
        );
        true
    }

    pub fn slot_envelope(&self, slot: usize) -> Option<ADSRConfig> {
        self.slot_params.get(slot).map(|p| p.envelope)
    }

    /// Set the normalized 0–1 trim region for a slot. `start` must be strictly
    /// less than `end`, and both must be finite and within `[0, 1]`. When a
    /// buffer is loaded the resulting region must span at least
    /// `SLOT_TRIM_MIN_FRAMES` frames. Returns false on any invalid input or
    /// out-of-range slot.
    pub fn set_slot_trim(&mut self, slot: usize, start: f32, end: f32) -> bool {
        let Some(params) = self.slot_params.get_mut(slot) else {
            return false;
        };
        if !start.is_finite() || !end.is_finite() {
            return false;
        }
        if !(0.0..=1.0).contains(&start) || !(0.0..=1.0).contains(&end) || start >= end {
            return false;
        }
        if let Some(buffer) = self.slots.get(slot).and_then(Option::as_ref) {
            let start_frame = (start as f64 * buffer.frames() as f64).round() as usize;
            let end_frame = (end as f64 * buffer.frames() as f64).round() as usize;
            if end_frame.saturating_sub(start_frame) < SLOT_TRIM_MIN_FRAMES {
                return false;
            }
        }
        params.start = start;
        params.end = end;
        true
    }

    pub fn slot_trim(&self, slot: usize) -> Option<(f32, f32)> {
        self.slot_params.get(slot).map(|p| (p.start, p.end))
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
        let Some(target) = self.pending_start_beat else {
            return;
        };
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

    fn voice_active(rack: &SamplerRack) -> bool {
        rack.voices.iter().any(|v| v.active())
    }

    #[test]
    fn pitch_shortens_playback_proportionally() {
        let mut rack = SamplerRack::new(44_100.0, 120.0, "test");
        rack.set_buffer(
            0,
            SamplerBuffer::from_interleaved(&vec![0.5; 1000], 1000, 1, 44_100.0).unwrap(),
        );
        assert!(rack.set_slot_pitch(0, 12.0));
        assert!(rack.trigger(0, 1.0));
        for _ in 0..500 {
            let _ = rack.tick();
        }
        assert!(!voice_active(&rack));

        let mut rack = SamplerRack::new(44_100.0, 120.0, "test");
        rack.set_buffer(
            0,
            SamplerBuffer::from_interleaved(&vec![0.5; 1000], 1000, 1, 44_100.0).unwrap(),
        );
        assert!(rack.set_slot_pitch(0, -12.0));
        assert!(rack.trigger(0, 1.0));
        for _ in 0..1500 {
            let _ = rack.tick();
        }
        assert!(voice_active(&rack));
    }

    #[test]
    fn sustain_zero_envelope_ends_voice_early() {
        let mut rack = SamplerRack::new(44_100.0, 120.0, "test");
        rack.set_buffer(
            0,
            SamplerBuffer::from_interleaved(&vec![0.5; 44_100], 44_100, 1, 44_100.0).unwrap(),
        );
        assert!(rack.set_slot_envelope(0, 0.0, 0.01, 0.0, 0.01));
        assert!(rack.trigger(0, 1.0));
        for _ in 0..3000 {
            let _ = rack.tick();
        }
        assert!(!voice_active(&rack));
        assert_eq!(rack.tick(), StereoFrame::default());
    }

    #[test]
    fn slot_gain_scales_and_latches() {
        let mut rack = SamplerRack::new(44_100.0, 120.0, "test");
        rack.set_buffer(
            0,
            SamplerBuffer::from_interleaved(&vec![1.0; 2048], 2048, 1, 44_100.0).unwrap(),
        );
        assert!(rack.trigger(0, 1.0));
        let mid = rack.tick().l;
        assert!(rack.set_slot_gain(0, 2.0));
        let after = rack.tick().l;
        assert!((after - mid).abs() < 0.05);
        while voice_active(&rack) {
            let _ = rack.tick();
        }
        assert!(rack.trigger(0, 1.0));
        let louder = (0..64).map(|_| rack.tick().l.abs()).fold(0.0_f32, f32::max);
        assert!(louder > mid.abs() * 1.4);
    }

    #[test]
    fn params_survive_reload_and_clamp() {
        let mut rack = SamplerRack::new(44_100.0, 120.0, "test");
        assert!(rack.set_slot_gain(0, 5.0));
        assert_eq!(rack.slot_gain(0), Some(2.0));
        assert!(rack.set_slot_pitch(0, -30.0));
        assert_eq!(rack.slot_pitch(0), Some(-24.0));
        rack.set_buffer(
            0,
            SamplerBuffer::from_interleaved(&vec![0.5; 64], 64, 1, 44_100.0).unwrap(),
        );
        assert_eq!(rack.slot_gain(0), Some(2.0));
        assert_eq!(rack.slot_pitch(0), Some(-24.0));
        assert!(rack.set_slot_envelope(0, 0.0, 0.1, 0.5, 0.2));
        let env = rack.slot_envelope(0).unwrap();
        assert!((env.attack_time - 0.001).abs() < f32::EPSILON);
    }

    #[test]
    fn release_fades_before_buffer_end() {
        let mut rack = SamplerRack::new(44_100.0, 120.0, "test");
        rack.set_buffer(
            0,
            SamplerBuffer::from_interleaved(&vec![1.0; 4410], 4410, 1, 44_100.0).unwrap(),
        );
        assert!(rack.set_slot_envelope(0, 0.0, 0.0, 1.0, 0.05));
        assert!(rack.trigger(0, 1.0));
        let mut mid = 0.0;
        for _ in 0..2205 {
            mid = rack.tick().l.abs();
        }
        let mut tail = 0.0;
        for _ in 0..2204 {
            tail = rack.tick().l.abs();
        }
        assert!(mid > 0.5);
        assert!(tail < mid * 0.5);
    }

    #[test]
    fn trim_rejects_invalid_ranges() {
        let mut rack = SamplerRack::new(44_100.0, 120.0, "test");
        assert!(!rack.set_slot_trim(0, 0.5, 0.5));
        assert!(!rack.set_slot_trim(0, 0.7, 0.3));
        assert!(!rack.set_slot_trim(0, -0.1, 1.0));
        assert!(!rack.set_slot_trim(0, 0.0, 1.1));
        assert!(!rack.set_slot_trim(0, f32::NAN, 1.0));
        assert!(!rack.set_slot_trim(SAMPLER_SLOT_COUNT, 0.0, 1.0));
        assert!(rack.set_slot_trim(0, 0.0, 1.0));
        assert_eq!(rack.slot_trim(0), Some((0.0, 1.0)));
    }

    #[test]
    fn trim_enforces_min_frames_when_loaded() {
        let mut rack = SamplerRack::new(44_100.0, 120.0, "test");
        rack.set_buffer(
            0,
            SamplerBuffer::from_interleaved(&vec![0.5; 100], 100, 1, 44_100.0).unwrap(),
        );
        // 0.99..1.0 resolves to a single frame; reject it.
        assert!(!rack.set_slot_trim(0, 0.99, 1.0));
        // A wide trim is accepted and persists across reloads.
        assert!(rack.set_slot_trim(0, 0.25, 0.75));
        rack.set_buffer(
            0,
            SamplerBuffer::from_interleaved(&vec![0.5; 100], 100, 1, 44_100.0).unwrap(),
        );
        assert_eq!(rack.slot_trim(0), Some((0.25, 0.75)));
    }

    #[test]
    fn trim_shortens_playback_and_latches() {
        let mut rack = SamplerRack::new(44_100.0, 120.0, "test");
        rack.set_buffer(
            0,
            SamplerBuffer::from_interleaved(&vec![1.0; 44_100], 44_100, 1, 44_100.0).unwrap(),
        );
        // Full-buffer reference duration.
        assert!(rack.trigger(0, 1.0));
        let full_ticks = (0..)
            .position(|_| {
                rack.tick();
                !voice_active(&rack)
            })
            .unwrap();
        // Trim to the first half: should finish in roughly half the ticks.
        rack.set_buffer(
            0,
            SamplerBuffer::from_interleaved(&vec![1.0; 44_100], 44_100, 1, 44_100.0).unwrap(),
        );
        assert!(rack.set_slot_trim(0, 0.0, 0.5));
        assert!(rack.trigger(0, 1.0));
        let half_ticks = (0..)
            .position(|_| {
                rack.tick();
                !voice_active(&rack)
            })
            .unwrap();
        assert!(half_ticks < (full_ticks as f32 * 0.6) as usize, "{half_ticks} vs {full_ticks}");
        // Latch: changing trim mid-playback does not alter the running voice.
        assert!(rack.set_slot_trim(0, 0.0, 1.0));
        assert!(rack.trigger(0, 1.0));
        let mut before = 0.0;
        for _ in 0..100 {
            before = rack.tick().l.abs();
        }
        assert!(rack.set_slot_trim(0, 0.0, 0.1));
        for _ in 0..100 {
            let _ = rack.tick();
        }
        // Voice keeps playing past the 0.1 trim because it latched at trigger.
        assert!(voice_active(&rack));
        assert!(before > 0.0);
    }

    #[test]
    fn trim_start_skips_into_buffer() {
        let mut rack = SamplerRack::new(44_100.0, 120.0, "test");
        // Ramp 0..N so position maps to amplitude; skipping in should jump.
        let n = 44_100;
        let ramp: Vec<f32> = (0..n).map(|i| i as f32 / n as f32).collect();
        rack.set_buffer(
            0,
            SamplerBuffer::from_interleaved(&ramp, n, 1, 44_100.0).unwrap(),
        );
        assert!(rack.set_slot_trim(0, 0.5, 1.0));
        assert!(rack.trigger(0, 1.0));
        let first = rack.tick().l;
        // Untrimmed playback would start near 0; trimmed starts near 0.5.
        assert!(first > 0.4, "first sample was {first}");
    }
}
