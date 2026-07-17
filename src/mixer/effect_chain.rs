//! Per-channel effect chain for loop mixer channels.
//!
//! Each loop channel owns an ordered chain of [`ChannelEffect`]s so the user
//! can put *any* effect on *any* channel (the mixer's headline requirement).
//! [`ChannelEffect`] is a small enum over the existing effect types — mirroring
//! the `ChannelInstrument` pattern in `ffi.rs` — so parameter dispatch stays
//! typed (no `dyn Any` downcasting) while reusing the exact `EFFECT_*` /
//! `*_PARAM_*` constants and per-effect setters already exported by the FFI.

use crate::effects::{
    DelayEffect, DelayTiming, Effect, FeedbackWaveshaper, LowpassFilterEffect, PlateReverbEffect,
    SpringReverbEffect, TiltFilterEffect, TubeCompressor, TubeSaturation, Waveshaper,
};
use crate::ffi::{
    COMPRESSOR_PARAM_ATTACK, COMPRESSOR_PARAM_MIX, COMPRESSOR_PARAM_RATIO,
    COMPRESSOR_PARAM_RELEASE, COMPRESSOR_PARAM_THRESHOLD, DELAY_PARAM_FEEDBACK,
    DELAY_PARAM_FILTER_CUTOFF, DELAY_PARAM_MIX, DELAY_PARAM_PINGPONG, DELAY_PARAM_TIMING,
    EFFECT_COMPRESSOR, EFFECT_DELAY, EFFECT_FEEDBACK_WAVESHAPER, EFFECT_LOWPASS_FILTER,
    EFFECT_PLATE_REVERB, EFFECT_REVERB, EFFECT_SATURATION, EFFECT_TILT_FILTER, EFFECT_WAVESHAPER,
    FEEDBACK_WAVESHAPER_PARAM_DRIVE, FEEDBACK_WAVESHAPER_PARAM_FEEDBACK,
    FEEDBACK_WAVESHAPER_PARAM_FILTER_CUTOFF, FEEDBACK_WAVESHAPER_PARAM_MIX, FILTER_PARAM_CUTOFF,
    FILTER_PARAM_RESONANCE, PLATE_PARAM_DAMPING, PLATE_PARAM_DECAY, PLATE_PARAM_MIX,
    PLATE_PARAM_PREDELAY, PLATE_PARAM_SIZE, PLATE_PARAM_WIDTH, REVERB_PARAM_DAMPING,
    REVERB_PARAM_DECAY, REVERB_PARAM_MIX, SATURATION_PARAM_DRIVE, SATURATION_PARAM_MIX,
    SATURATION_PARAM_WARMTH, TILT_PARAM_CUTOFF, TILT_PARAM_RESONANCE, WAVESHAPER_PARAM_DRIVE,
    WAVESHAPER_PARAM_MIX,
};
use crate::frame::StereoFrame;
use std::cell::UnsafeCell;

/// A single effect on a loop channel. The variants intentionally match the
/// reorderable `EFFECT_*` ids used everywhere else in the engine.
///
/// The variants are kept inline (not boxed) on purpose: an effect is created
/// only when the user adds one, but its `process_stereo` runs in the per-sample
/// audio loop, so keeping the DSP state contiguous in the channel's `Vec`
/// avoids a pointer chase on the hot path. This mirrors `ChannelInstrument` in
/// `ffi.rs`, which likewise stores its synths inline.
#[allow(clippy::large_enum_variant)]
pub enum ChannelEffect {
    Filter(LowpassFilterEffect),
    Delay(DelayEffect),
    Saturation(TubeSaturation),
    Compressor(TubeCompressor),
    Tilt(TiltFilterEffect),
    Reverb(SpringReverbEffect),
    PlateReverb(PlateReverbEffect),
    Waveshaper(UnsafeCell<[Waveshaper; 2]>),
    FeedbackWaveshaper(UnsafeCell<[FeedbackWaveshaper; 2]>),
}

impl ChannelEffect {
    /// Construct an effect from its `EFFECT_*` id with musically useful initial
    /// values (all adjustable afterwards via [`Self::set_param`]). `bpm` seeds
    /// the delay's note-synced time. Returns `None` for unknown / non-channel
    /// ids (e.g. the master limiter).
    pub fn from_id(effect_id: u32, sample_rate: f32, bpm: f32) -> Option<Self> {
        match effect_id {
            EFFECT_LOWPASS_FILTER => Some(Self::Filter(LowpassFilterEffect::new(
                sample_rate,
                20_000.0,
                0.0,
            ))),
            EFFECT_DELAY => Some(Self::Delay(DelayEffect::new(
                sample_rate,
                DelayTiming::Quarter,
                bpm,
                0.3,
                0.3,
                8_000.0,
            ))),
            EFFECT_SATURATION => Some(Self::Saturation(TubeSaturation::new(
                sample_rate,
                0.3,
                0.4,
                0.5,
            ))),
            EFFECT_COMPRESSOR => Some(Self::Compressor(TubeCompressor::new(
                sample_rate,
                -12.0,
                4.0,
                5.0,
                100.0,
                0.5,
            ))),
            EFFECT_TILT_FILTER => Some(Self::Tilt(TiltFilterEffect::new(sample_rate))),
            EFFECT_REVERB => Some(Self::Reverb(SpringReverbEffect::new(
                sample_rate,
                0.5,
                0.3,
                0.5,
            ))),
            EFFECT_PLATE_REVERB => Some(Self::PlateReverb(PlateReverbEffect::new(
                sample_rate,
                0.5,
                0.3,
                0.5,
            ))),
            EFFECT_WAVESHAPER => Some(Self::Waveshaper(UnsafeCell::new([
                Waveshaper::new(1.0, 0.0),
                Waveshaper::new(1.0, 0.0),
            ]))),
            EFFECT_FEEDBACK_WAVESHAPER => Some(Self::FeedbackWaveshaper(UnsafeCell::new([
                FeedbackWaveshaper::new(sample_rate, 1.0, 0.0, 2000.0, 0.0),
                FeedbackWaveshaper::new(sample_rate, 1.0, 0.0, 2000.0, 0.0),
            ]))),
            _ => None,
        }
    }

    /// The `EFFECT_*` id for this effect.
    pub fn effect_type(&self) -> u32 {
        match self {
            Self::Filter(_) => EFFECT_LOWPASS_FILTER,
            Self::Delay(_) => EFFECT_DELAY,
            Self::Saturation(_) => EFFECT_SATURATION,
            Self::Compressor(_) => EFFECT_COMPRESSOR,
            Self::Tilt(_) => EFFECT_TILT_FILTER,
            Self::Reverb(_) => EFFECT_REVERB,
            Self::PlateReverb(_) => EFFECT_PLATE_REVERB,
            Self::Waveshaper(_) => EFFECT_WAVESHAPER,
            Self::FeedbackWaveshaper(_) => EFFECT_FEEDBACK_WAVESHAPER,
        }
    }

    /// Process one stereo frame through this effect.
    #[inline]
    pub fn process_stereo(&self, input: StereoFrame) -> StereoFrame {
        match self {
            Self::Filter(e) => e.process_stereo(input),
            Self::Delay(e) => e.process_stereo(input),
            Self::Saturation(e) => e.process_stereo(input),
            Self::Compressor(e) => e.process_stereo(input),
            Self::Tilt(e) => e.process_stereo(input),
            Self::Reverb(e) => e.process_stereo(input),
            Self::PlateReverb(e) => e.process_stereo(input),
            Self::Waveshaper(ws) => {
                let ws = unsafe { &mut *ws.get() };
                StereoFrame {
                    l: ws[0].process(input.l),
                    r: ws[1].process(input.r),
                }
            }
            Self::FeedbackWaveshaper(fb) => {
                let fb = unsafe { &mut *fb.get() };
                StereoFrame {
                    l: fb[0].process(input.l),
                    r: fb[1].process(input.r),
                }
            }
        }
    }

    /// Set a parameter by its effect-specific `*_PARAM_*` id. Mirrors the
    /// global-effect dispatch in `gooey_engine_set_global_effect_param`.
    pub fn set_param(&self, param: u32, value: f32) {
        match self {
            Self::Filter(e) => match param {
                FILTER_PARAM_CUTOFF => e.set_cutoff_freq(value),
                FILTER_PARAM_RESONANCE => e.set_resonance(value),
                _ => {}
            },
            Self::Delay(e) => match param {
                DELAY_PARAM_TIMING => {
                    if let Some(timing) = DelayTiming::from_timing_constant(value as u32) {
                        e.set_timing(timing);
                    }
                }
                DELAY_PARAM_FEEDBACK => e.set_feedback(value),
                DELAY_PARAM_MIX => e.set_mix(value),
                DELAY_PARAM_FILTER_CUTOFF => e.set_filter_cutoff(value),
                DELAY_PARAM_PINGPONG => e.set_pingpong(value >= 0.5),
                _ => {}
            },
            Self::Saturation(e) => match param {
                SATURATION_PARAM_DRIVE => e.set_drive(value),
                SATURATION_PARAM_WARMTH => e.set_warmth(value),
                SATURATION_PARAM_MIX => e.set_mix(value),
                _ => {}
            },
            Self::Compressor(e) => match param {
                COMPRESSOR_PARAM_THRESHOLD => e.set_threshold(value),
                COMPRESSOR_PARAM_RATIO => e.set_ratio(value),
                COMPRESSOR_PARAM_ATTACK => e.set_attack(value),
                COMPRESSOR_PARAM_RELEASE => e.set_release(value),
                COMPRESSOR_PARAM_MIX => e.set_mix(value),
                _ => {}
            },
            Self::Tilt(e) => match param {
                TILT_PARAM_CUTOFF => e.set_cutoff(value),
                TILT_PARAM_RESONANCE => e.set_resonance(value),
                _ => {}
            },
            Self::Waveshaper(ws) => {
                let ws = unsafe { &mut *ws.get() };
                // Apply param to both channels
                match param {
                    WAVESHAPER_PARAM_DRIVE => {
                        ws[0].set_drive(value);
                        ws[1].set_drive(value);
                    }
                    WAVESHAPER_PARAM_MIX => {
                        ws[0].set_mix(value);
                        ws[1].set_mix(value);
                    }
                    _ => {}
                }
            }
            Self::FeedbackWaveshaper(fb) => {
                let fb = unsafe { &mut *fb.get() };
                match param {
                    FEEDBACK_WAVESHAPER_PARAM_DRIVE => {
                        fb[0].set_drive(value);
                        fb[1].set_drive(value);
                    }
                    FEEDBACK_WAVESHAPER_PARAM_FEEDBACK => {
                        fb[0].set_feedback(value);
                        fb[1].set_feedback(value);
                    }
                    FEEDBACK_WAVESHAPER_PARAM_FILTER_CUTOFF => {
                        fb[0].set_filter_cutoff(value);
                        fb[1].set_filter_cutoff(value);
                    }
                    FEEDBACK_WAVESHAPER_PARAM_MIX => {
                        fb[0].set_mix(value);
                        fb[1].set_mix(value);
                    }
                    _ => {}
                }
            }
            Self::Reverb(e) => match param {
                REVERB_PARAM_DECAY => e.set_decay(value),
                REVERB_PARAM_MIX => e.set_mix(value),
                REVERB_PARAM_DAMPING => e.set_damping(value),
                _ => {}
            },
            Self::PlateReverb(e) => match param {
                PLATE_PARAM_DECAY => e.set_decay(value),
                PLATE_PARAM_MIX => e.set_mix(value),
                PLATE_PARAM_DAMPING => e.set_damping(value),
                PLATE_PARAM_PREDELAY => e.set_predelay(value),
                PLATE_PARAM_WIDTH => e.set_width(value),
                PLATE_PARAM_SIZE => e.set_size(value),
                _ => {}
            },
        }
    }

    /// Update the tempo for note-synced effects. Only the delay's clocked timing
    /// depends on BPM; the other effects ignore it.
    pub fn set_bpm(&self, bpm: f32) {
        if let Self::Delay(e) = self {
            e.set_bpm(bpm);
        }
    }

    /// Clear internal DSP state (delay lines, filter memory, envelopes).
    pub fn reset(&self) {
        match self {
            Self::Filter(e) => e.reset(),
            Self::Delay(e) => e.reset(),
            Self::Saturation(e) => e.reset(),
            Self::Compressor(e) => e.reset(),
            Self::Tilt(e) => e.reset(),
            Self::Reverb(e) => e.reset(),
            Self::PlateReverb(e) => e.reset(),
            Self::Waveshaper(ws) => {
                let ws = unsafe { &mut *ws.get() };
                ws[0].reset();
                ws[1].reset();
            }
            Self::FeedbackWaveshaper(fb) => {
                let fb = unsafe { &mut *fb.get() };
                fb[0].reset();
                fb[1].reset();
            }
        }
    }
}

/// An ordered, runtime-editable chain of per-channel effects.
#[derive(Default)]
pub struct EffectChain {
    effects: Vec<ChannelEffect>,
}

impl EffectChain {
    pub fn new() -> Self {
        Self::default()
    }

    /// Process a stereo frame through the whole chain, in order.
    #[inline]
    pub fn process(&self, mut frame: StereoFrame) -> StereoFrame {
        for effect in &self.effects {
            frame = effect.process_stereo(frame);
        }
        frame
    }

    /// Append an effect by id. Returns the slot index of the new effect, or
    /// `None` if the id is not a valid channel effect.
    pub fn add(&mut self, effect_id: u32, sample_rate: f32, bpm: f32) -> Option<usize> {
        let effect = ChannelEffect::from_id(effect_id, sample_rate, bpm)?;
        self.effects.push(effect);
        Some(self.effects.len() - 1)
    }

    /// Remove the effect at `slot`. Returns `false` if out of range.
    pub fn remove(&mut self, slot: usize) -> bool {
        if slot < self.effects.len() {
            self.effects.remove(slot);
            true
        } else {
            false
        }
    }

    /// Move the effect at `slot` to `new_position` (clamped to the chain end),
    /// preserving the relative order of the others. Returns `false` if `slot`
    /// is out of range.
    pub fn move_effect(&mut self, slot: usize, new_position: usize) -> bool {
        if slot >= self.effects.len() {
            return false;
        }
        let effect = self.effects.remove(slot);
        let dest = new_position.min(self.effects.len());
        self.effects.insert(dest, effect);
        true
    }

    pub fn clear(&mut self) {
        self.effects.clear();
    }

    /// Clear every effect's internal DSP state (delay lines, reverb tails,
    /// filter memory) without removing the effects from the chain. Used to
    /// start an offline render from a clean, un-warmed state.
    pub fn reset(&self) {
        for effect in &self.effects {
            effect.reset();
        }
    }

    /// Set a parameter on the effect at `slot`.
    pub fn set_param(&self, slot: usize, param: u32, value: f32) {
        if let Some(effect) = self.effects.get(slot) {
            effect.set_param(param, value);
        }
    }

    /// Re-tempo every note-synced effect in the chain (currently the delay).
    pub fn set_bpm(&self, bpm: f32) {
        for effect in &self.effects {
            effect.set_bpm(bpm);
        }
    }

    pub fn len(&self) -> usize {
        self.effects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    /// The `EFFECT_*` id of the effect at `slot`, for UI / FFI getters.
    pub fn effect_type_at(&self, slot: usize) -> Option<u32> {
        self.effects.get(slot).map(ChannelEffect::effect_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44_100.0;

    #[test]
    fn add_reports_slot_and_type() {
        let mut chain = EffectChain::new();
        assert_eq!(chain.add(EFFECT_DELAY, SR, 120.0), Some(0));
        assert_eq!(chain.add(EFFECT_REVERB, SR, 120.0), Some(1));
        assert_eq!(chain.add(EFFECT_PLATE_REVERB, SR, 120.0), Some(2));
        assert_eq!(chain.len(), 3);
        assert_eq!(chain.effect_type_at(0), Some(EFFECT_DELAY));
        assert_eq!(chain.effect_type_at(1), Some(EFFECT_REVERB));
        assert_eq!(chain.effect_type_at(2), Some(EFFECT_PLATE_REVERB));
    }

    #[test]
    fn unknown_effect_id_is_rejected() {
        let mut chain = EffectChain::new();
        assert_eq!(chain.add(9999, SR, 120.0), None);
        assert!(chain.is_empty());
    }

    #[test]
    fn move_effect_reorders() {
        let mut chain = EffectChain::new();
        chain.add(EFFECT_LOWPASS_FILTER, SR, 120.0);
        chain.add(EFFECT_DELAY, SR, 120.0);
        chain.add(EFFECT_REVERB, SR, 120.0);
        // Move reverb (slot 2) to the front.
        assert!(chain.move_effect(2, 0));
        assert_eq!(chain.effect_type_at(0), Some(EFFECT_REVERB));
        assert_eq!(chain.effect_type_at(1), Some(EFFECT_LOWPASS_FILTER));
        assert_eq!(chain.effect_type_at(2), Some(EFFECT_DELAY));
    }

    #[test]
    fn remove_and_clear() {
        let mut chain = EffectChain::new();
        chain.add(EFFECT_DELAY, SR, 120.0);
        chain.add(EFFECT_REVERB, SR, 120.0);
        assert!(chain.remove(0));
        assert_eq!(chain.effect_type_at(0), Some(EFFECT_REVERB));
        assert!(!chain.remove(5));
        chain.clear();
        assert!(chain.is_empty());
    }

    #[test]
    fn empty_chain_is_passthrough() {
        let chain = EffectChain::new();
        let f = StereoFrame { l: 0.5, r: -0.25 };
        assert_eq!(chain.process(f), f);
    }
}
