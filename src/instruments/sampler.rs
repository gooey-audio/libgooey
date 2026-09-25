//! Fixed-size sample-pad rack used by the FFI engine.
//!
//! The rack owns copied PCM slot data and a fixed voice pool, so playback does
//! not allocate on the audio thread.  Every voice is shaped by the rack's
//! shared attack-hold-release amplitude envelope ([`SamplerEnvelopeConfig`]),
//! which bounds unexpectedly long samples while keeping playback polyphonic.

use std::sync::Arc;

use crate::engine::Sequencer;
use crate::frame::StereoFrame;

pub const SAMPLER_SLOT_COUNT: usize = 16;
pub const SAMPLER_VOICE_COUNT: usize = 32;

/// Minimum number of rendered frames any attack or release ramp spans, even
/// when the configured time is zero. A hard 0 ms edge would step the gain from
/// 0 to full scale (or back) in a single sample and click; 32 frames is short
/// enough to feel instantaneous yet long enough to stay inaudible.
const MIN_RAMP_FRAMES: f32 = 32.0;

/// Frames over which a voice fades out when its PCM data runs out before the
/// amplitude envelope has finished. Prevents a click at the raw buffer end.
const BUFFER_TAPER_FRAMES: f64 = 32.0;

/// Shared attack-hold-release amplitude envelope for one sampler rack.
///
/// All times are in seconds and are guaranteed finite and non-negative by the
/// only constructor, [`SamplerEnvelopeConfig::new`]. The envelope ramps a
/// voice's gain from 0 to full over `attack_seconds`, holds full gain for
/// `hold_seconds`, then ramps back to 0 over `release_seconds`. A voice also
/// ends naturally if its PCM buffer runs out first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SamplerEnvelopeConfig {
    attack_seconds: f32,
    hold_seconds: f32,
    release_seconds: f32,
}

impl Default for SamplerEnvelopeConfig {
    /// 1 ms attack, 1 s hold, 50 ms release — about 1.051 s of maximum
    /// envelope duration. Samples shorter than this still end naturally.
    fn default() -> Self {
        Self {
            attack_seconds: 0.001,
            hold_seconds: 1.0,
            release_seconds: 0.05,
        }
    }
}

impl SamplerEnvelopeConfig {
    /// Build a validated envelope. Returns `None` when any value is negative,
    /// NaN, or infinite so callers can leave the previous configuration
    /// unchanged.
    pub fn new(attack_seconds: f32, hold_seconds: f32, release_seconds: f32) -> Option<Self> {
        let finite_non_negative = |value: f32| value.is_finite() && value >= 0.0;
        if finite_non_negative(attack_seconds)
            && finite_non_negative(hold_seconds)
            && finite_non_negative(release_seconds)
        {
            Some(Self {
                attack_seconds,
                hold_seconds,
                release_seconds,
            })
        } else {
            None
        }
    }

    pub fn attack_seconds(&self) -> f32 {
        self.attack_seconds
    }
    pub fn hold_seconds(&self) -> f32 {
        self.hold_seconds
    }
    pub fn release_seconds(&self) -> f32 {
        self.release_seconds
    }

    /// Number of frames the attack ramp spans at `sample_rate`, floored at
    /// [`MIN_RAMP_FRAMES`] so a 0 ms attack still de-clicks.
    #[inline]
    fn attack_frames(&self, sample_rate: f32) -> f32 {
        (self.attack_seconds * sample_rate).max(MIN_RAMP_FRAMES)
    }

    /// Number of frames the release ramp spans at `sample_rate`, floored at
    /// [`MIN_RAMP_FRAMES`] so a 0 ms release still de-clicks.
    #[inline]
    fn release_frames(&self, sample_rate: f32) -> f32 {
        (self.release_seconds * sample_rate).max(MIN_RAMP_FRAMES)
    }

    /// Number of frames the hold phase spans at `sample_rate`. Unlike the
    /// ramps this has no floor: a 0 s hold moves straight from attack to
    /// release.
    #[inline]
    fn hold_frames(&self, sample_rate: f32) -> f64 {
        (self.hold_seconds as f64) * (sample_rate as f64)
    }
}

/// Which segment of the amplitude envelope a voice is currently in.
#[derive(Clone, Copy, Debug, PartialEq)]
enum EnvPhase {
    Attack,
    Hold,
    Release,
    Finished,
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
    increment: f64,
    velocity: f32,
    age: u64,
    phase: EnvPhase,
    /// Current amplitude-envelope level in `0..=1`, tracked as a running
    /// accumulator so parameter edits only change the ramp *rate* and never
    /// jump the gain.
    level: f32,
    /// Frames elapsed in the current hold phase.
    hold_elapsed: f64,
    /// Envelope level captured when release began; the release ramps from here
    /// down to 0.
    release_from: f32,
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
            phase: EnvPhase::Finished,
            level: 0.0,
            hold_elapsed: 0.0,
            release_from: 1.0,
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
        self.phase = EnvPhase::Attack;
        self.level = 0.0;
        self.hold_elapsed = 0.0;
        self.release_from = 1.0;
        self.buffer = Some(buffer);
    }

    /// Advance the amplitude envelope by one frame using the rack's shared
    /// configuration. Reading the config every frame is what makes live edits
    /// continuous: a changed attack/release alters the slope from the current
    /// level onward, and a shortened hold begins release immediately, without
    /// resurrecting a voice that has already started releasing.
    fn advance_envelope(&mut self, config: &SamplerEnvelopeConfig, sample_rate: f32) {
        match self.phase {
            EnvPhase::Attack => {
                self.level += 1.0 / config.attack_frames(sample_rate);
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.phase = EnvPhase::Hold;
                    self.hold_elapsed = 0.0;
                }
            }
            EnvPhase::Hold => {
                if self.hold_elapsed >= config.hold_frames(sample_rate) {
                    self.phase = EnvPhase::Release;
                    self.release_from = self.level;
                } else {
                    self.hold_elapsed += 1.0;
                }
            }
            EnvPhase::Release => {
                self.level -= self.release_from / config.release_frames(sample_rate);
                if self.level <= 0.0 {
                    self.level = 0.0;
                    self.phase = EnvPhase::Finished;
                }
            }
            EnvPhase::Finished => {}
        }
    }

    fn tick(&mut self, config: &SamplerEnvelopeConfig, sample_rate: f32) -> StereoFrame {
        let Some(buffer) = self.buffer.as_ref() else {
            return StereoFrame::default();
        };
        let frame = buffer.frame(self.position);
        let end = buffer.frames() as f64;
        // Buffer-end taper: fade out over the final frames when the PCM data
        // runs out before the envelope has finished, avoiding a raw-end click.
        // The attack ramp already covers the buffer start.
        let buffer_taper = ((end - self.position) / BUFFER_TAPER_FRAMES).clamp(0.0, 1.0) as f32;
        let gain = self.level * self.velocity * buffer_taper;
        self.advance_envelope(config, sample_rate);
        self.position += self.increment;
        if self.position >= end || self.phase == EnvPhase::Finished {
            self.buffer = None;
        }
        frame.scaled(gain)
    }
}

pub struct SamplerRack {
    sample_rate: f32,
    slots: [Option<SamplerBuffer>; SAMPLER_SLOT_COUNT],
    voices: [SampleVoice; SAMPLER_VOICE_COUNT],
    envelope: SamplerEnvelopeConfig,
    next_age: u64,
    sequencer: Sequencer,
}

impl SamplerRack {
    pub fn new(sample_rate: f32, bpm: f32, name: impl Into<String>) -> Self {
        Self {
            sample_rate,
            slots: std::array::from_fn(|_| None),
            voices: std::array::from_fn(|_| SampleVoice::default()),
            envelope: SamplerEnvelopeConfig::default(),
            next_age: 0,
            sequencer: Sequencer::with_pattern(
                bpm,
                sample_rate,
                vec![false; SAMPLER_SLOT_COUNT],
                name,
            ),
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
        let config = self.envelope;
        let sample_rate = self.sample_rate;
        self.voices
            .iter_mut()
            .fold(StereoFrame::default(), |out, voice| {
                out + voice.tick(&config, sample_rate)
            })
    }

    /// Set the shared amplitude envelope from raw seconds. Rejects negative,
    /// NaN, or infinite values, leaving the previous configuration unchanged,
    /// and returns whether the update was applied. Active voices keep playing
    /// and adopt the new attack/hold/release rates continuously.
    pub fn set_amp_envelope(
        &mut self,
        attack_seconds: f32,
        hold_seconds: f32,
        release_seconds: f32,
    ) -> bool {
        match SamplerEnvelopeConfig::new(attack_seconds, hold_seconds, release_seconds) {
            Some(config) => {
                self.envelope = config;
                true
            }
            None => false,
        }
    }

    /// Current shared amplitude envelope.
    pub fn amp_envelope(&self) -> SamplerEnvelopeConfig {
        self.envelope
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

    /// A rack whose slot holds a long DC (constant 1.0) buffer at the engine
    /// sample rate, so each rendered frame equals `envelope_level * velocity`
    /// (buffer interpolation is flat and the increment is exactly 1.0).
    fn dc_rack(sample_rate: f32, frames: usize) -> SamplerRack {
        let mut rack = SamplerRack::new(sample_rate, 120.0, "test");
        rack.set_buffer(
            0,
            SamplerBuffer::from_interleaved(&vec![1.0; frames], frames, 1, sample_rate).unwrap(),
        );
        rack
    }

    fn render(rack: &mut SamplerRack, frames: usize) -> Vec<f32> {
        (0..frames).map(|_| rack.tick().l).collect()
    }

    #[test]
    fn default_envelope_matches_documented_times() {
        let rack = SamplerRack::new(44_100.0, 120.0, "test");
        let env = rack.amp_envelope();
        assert_eq!(env.attack_seconds(), 0.001);
        assert_eq!(env.hold_seconds(), 1.0);
        assert_eq!(env.release_seconds(), 0.05);
    }

    #[test]
    fn envelope_follows_attack_hold_release_progression() {
        // sr = 1000 → 100-frame attack, 200-frame hold, 50-frame release.
        let mut rack = dc_rack(1_000.0, 4_000);
        assert!(rack.set_amp_envelope(0.1, 0.2, 0.05));
        assert!(rack.trigger(0, 1.0));
        let out = render(&mut rack, 400);

        // Attack ramps 0 -> 1 over 100 frames.
        assert!(out[0].abs() < 1e-6, "attack starts at zero");
        assert!((out[50] - 0.5).abs() < 0.02, "attack is mid-ramp at 50%");
        // Hold sits at full scale.
        assert!((out[150] - 1.0).abs() < 1e-3, "hold holds full gain");
        assert!((out[250] - 1.0).abs() < 1e-3, "hold still full");
        // Release ramps 1 -> 0 over 50 frames beginning at frame 300.
        assert!((out[300] - 1.0).abs() < 1e-3, "hold ends at full gain");
        assert!((out[325] - 0.5).abs() < 0.1, "release is mid-ramp near 50%");
        assert!(out[360].abs() < 1e-6, "voice is silent after release");
    }

    #[test]
    fn short_buffer_ends_naturally_before_envelope() {
        // 64-frame buffer with a 1 s hold: PCM runs out long before release.
        let mut rack = dc_rack(1_000.0, 64);
        assert!(rack.set_amp_envelope(0.0, 1.0, 0.05));
        assert!(rack.trigger(0, 1.0));
        let out = render(&mut rack, 200);
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(out[70].abs() < 1e-6, "voice stops when PCM ends");
        assert!(out[199].abs() < 1e-6, "and stays silent");
    }

    #[test]
    fn voice_deactivates_after_release_even_with_pcm_remaining() {
        let mut rack = dc_rack(1_000.0, 4_000);
        assert!(rack.set_amp_envelope(0.05, 0.1, 0.05)); // ends near frame 200
        assert!(rack.trigger(0, 1.0));
        let out = render(&mut rack, 400);
        assert!(out[399].abs() < 1e-6, "silent past release");
        // A fully released voice frees its buffer even though PCM remains.
        assert!(!rack.voices.iter().any(SampleVoice::active));
    }

    #[test]
    fn zero_attack_and_release_still_de_click() {
        let mut rack = dc_rack(1_000.0, 4_000);
        assert!(rack.set_amp_envelope(0.0, 0.5, 0.0));
        assert!(rack.trigger(0, 1.0));
        let out = render(&mut rack, 64);
        assert!(
            out[0].abs() < 1e-6,
            "no full-scale jump on the first sample"
        );
        // 0 ms attack is floored to the 32-frame minimum ramp.
        assert!((out[16] - 0.5).abs() < 0.05, "ramps across ~32 frames");
        let max_step = out
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(max_step < 0.1, "no click-sized per-sample jump: {max_step}");
    }

    #[test]
    fn active_voice_edits_do_not_discontinue_gain() {
        let mut rack = dc_rack(1_000.0, 8_000);
        assert!(rack.set_amp_envelope(0.05, 4.0, 0.05));
        assert!(rack.trigger(0, 1.0));
        let mut out = render(&mut rack, 200); // settle into the hold at full gain
                                              // Re-shape while sounding: shorten hold (begins release) and lengthen it.
        assert!(rack.set_amp_envelope(0.05, 0.05, 1.0));
        out.extend(render(&mut rack, 200));
        assert!(rack.set_amp_envelope(0.05, 0.05, 0.2));
        out.extend(render(&mut rack, 200));
        let max_step = out
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(max_step < 0.05, "edits stay continuous: {max_step}");
    }
}
