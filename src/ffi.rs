//! C FFI bindings for the gooey audio engine
//!
//! This module exposes the audio engine to C/Swift via C-compatible functions.
//! Designed for integration with iOS (and other platforms in the future).

use crate::effects::{
    DelayEffect, DelayTiming, Effect, FeedbackWaveshaper, LowpassFilterEffect, PlateReverbEffect,
    SoftLimiter, SpringReverbEffect, TiltFilterEffect, TubeCompressor, TubeSaturation, Waveshaper,
};
use crate::engine::lfo::{Lfo, MusicalDivision};
use crate::engine::{Instrument, Sequencer, SequencerBlendSetting, SequencerStepSettings};
use crate::frame::StereoFrame;
use crate::instruments::{
    BassConfig, BassSynth, Granulator, HiHat2, HiHat2Config, KickConfig, KickDrum, PolySynth,
    PolySynthConfig, SampleBuffer, SnareConfig, SnareDrum, Tom2, Tom2Config,
};
use crate::mixer::{Mixer, MixerGraph, PitchMode, StereoSampleBuffer};
use crate::music::{apply_voicing, available_voicings, Key, NoteName, ScaleType, VoicingType};
use crate::utils::{PresetBlender, SmoothedParam};
use std::ffi::{c_char, c_void, CStr, CString};
use std::slice;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

// =============================================================================
// LFO constants
// =============================================================================

/// Number of LFOs in the pool
pub const LFO_COUNT: usize = 8;
/// Maximum number of routes per LFO
pub const LFO_MAX_ROUTES: usize = 16;

/// LFO timing: 4 bars (16 beats)
pub const LFO_TIMING_FOUR_BARS: u32 = 0;
/// LFO timing: 2 bars (8 beats)
pub const LFO_TIMING_TWO_BARS: u32 = 1;
/// LFO timing: 1 bar (4 beats)
pub const LFO_TIMING_ONE_BAR: u32 = 2;
/// LFO timing: Half note (2 beats)
pub const LFO_TIMING_HALF: u32 = 3;
/// LFO timing: Quarter note (1 beat)
pub const LFO_TIMING_QUARTER: u32 = 4;
/// LFO timing: Eighth note (1/2 beat)
pub const LFO_TIMING_EIGHTH: u32 = 5;
/// LFO timing: Sixteenth note (1/4 beat)
pub const LFO_TIMING_SIXTEENTH: u32 = 6;
/// LFO timing: Thirty-second note (1/8 beat)
pub const LFO_TIMING_THIRTY_SECOND: u32 = 7;
/// Invalid LFO value (returned on error or when LFO is in Hz mode)
pub const LFO_INVALID: u32 = 0xFFFFFFFF;

/// LFO route configuration
#[derive(Clone)]
struct LfoRoute {
    /// Unique ID for this route (used for removal)
    id: u32,
    /// Target instrument (INSTRUMENT_KICK, etc.)
    instrument: u32,
    /// Target parameter index (KICK_PARAM_FREQUENCY, etc.)
    param: u32,
    /// Modulation depth for this route (0.0 to 1.0)
    depth: f32,
}

/// Maximum number of MIDI events buffered per render pass.
/// Events beyond this limit are silently dropped to avoid audio-thread allocation.
const MIDI_EVENT_CAPACITY: usize = 64;

/// A MIDI event produced by the sequencer during render.
///
/// Used to export note-on events to the host (e.g., AUv3 MIDI output).
/// `sample_offset` is the frame position within the render buffer (0 to frameCount-1)
/// where the note fired, enabling sample-accurate MIDI timing.
#[repr(C)]
pub struct GooeyMidiEvent {
    pub instrument_index: u32,
    pub velocity: f32,
    pub sample_offset: u32,
}

// =============================================================================
// Channel instrument and blender enums
// =============================================================================

/// A polymorphic instrument that can be any drum synth type.
/// Each channel holds one of these, enabling runtime instrument reassignment.
enum ChannelInstrument {
    Kick(KickDrum),
    Snare(SnareDrum),
    HiHat(HiHat2),
    Tom(Tom2),
    Bass(BassSynth),
}

impl ChannelInstrument {
    /// Returns the instrument type constant for this variant.
    fn instrument_type(&self) -> u32 {
        match self {
            Self::Kick(_) => INSTRUMENT_KICK,
            Self::Snare(_) => INSTRUMENT_SNARE,
            Self::HiHat(_) => INSTRUMENT_HIHAT,
            Self::Tom(_) => INSTRUMENT_TOM,
            Self::Bass(_) => INSTRUMENT_BASS,
        }
    }

    /// Trigger the instrument with a given velocity.
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        match self {
            Self::Kick(k) => k.trigger_with_velocity(time, velocity),
            Self::Snare(s) => s.trigger_with_velocity(time, velocity),
            Self::HiHat(h) => h.trigger_with_velocity(time, velocity),
            Self::Tom(t) => t.trigger_with_velocity(time, velocity),
            Self::Bass(b) => b.trigger_with_velocity(time, velocity),
        }
    }

    /// Snap all smoothed parameters to their targets instantly.
    /// Used for per-step sequencer blend overrides to avoid off-by-one latency.
    fn snap_params(&mut self) {
        match self {
            Self::Kick(k) => k.snap_params(),
            Self::Snare(s) => s.snap_params(),
            Self::HiHat(h) => h.snap_params(),
            Self::Tom(_) => {} // Tom2 uses plain f32, already immediate
            Self::Bass(b) => b.snap_params(),
        }
    }

    /// Generate the next audio sample.
    fn tick(&mut self, current_time: f64) -> f32 {
        match self {
            Self::Kick(k) => k.tick(current_time),
            Self::Snare(s) => s.tick(current_time),
            Self::HiHat(h) => h.tick(current_time),
            Self::Tom(t) => t.tick(current_time),
            Self::Bass(b) => b.tick(current_time),
        }
    }

    /// Get the current normalized frequency parameter (0-1) for pitched instruments.
    fn get_freq_param(&self) -> Option<f32> {
        match self {
            Self::Kick(k) => Some(k.params.frequency.get()),
            Self::Tom(t) => Some(t.tune()),
            Self::Bass(b) => Some(b.params.frequency.get()),
            _ => None,
        }
    }

    /// Get the current tuning value (0-1, 0.5 = neutral).
    fn get_tuning(&self) -> f32 {
        match self {
            Self::Kick(k) => k.params.tuning.get(),
            Self::Snare(s) => s.params.tuning.get(),
            Self::HiHat(h) => h.params.tuning.get(),
            Self::Tom(t) => t.tuning(),
            Self::Bass(b) => b.params.tuning.get(),
        }
    }

    /// Set a parameter by index. Dispatches to the correct setter for the current instrument type.
    /// All parameters use normalized 0-1 range from the FFI.
    fn set_param(&mut self, param: u32, value: f32) {
        match self {
            Self::Kick(k) => match param {
                KICK_PARAM_FREQUENCY => k.set_frequency(value),
                KICK_PARAM_PUNCH => k.set_punch(value),
                KICK_PARAM_SUB => k.set_sub(value),
                KICK_PARAM_CLICK => k.set_click(value),
                KICK_PARAM_DECAY => k.set_oscillator_decay(value),
                KICK_PARAM_PITCH_ENVELOPE => k.set_pitch_envelope_amount(value),
                KICK_PARAM_VOLUME => k.set_volume(value),
                KICK_PARAM_TUNING => k.set_tuning(value),
                _ => {}
            },
            Self::Snare(s) => match param {
                SNARE_PARAM_FREQUENCY => s.set_frequency(value),
                SNARE_PARAM_DECAY => s.set_decay(value),
                SNARE_PARAM_BRIGHTNESS => s.set_brightness(value),
                SNARE_PARAM_VOLUME => s.set_volume(value),
                SNARE_PARAM_TONAL => s.set_tonal(value),
                SNARE_PARAM_NOISE => s.set_noise(value),
                SNARE_PARAM_PITCH_DROP => s.set_pitch_drop(value),
                SNARE_PARAM_TONAL_DECAY => s.set_tonal_decay(value),
                SNARE_PARAM_NOISE_DECAY => s.set_noise_decay(value),
                SNARE_PARAM_NOISE_TAIL_DECAY => s.set_noise_tail_decay(value),
                SNARE_PARAM_FILTER_CUTOFF => s.set_filter_cutoff(value),
                SNARE_PARAM_FILTER_RESONANCE => s.set_filter_resonance(value),
                SNARE_PARAM_FILTER_TYPE => s.set_filter_type(value as u8),
                SNARE_PARAM_XFADE => s.set_xfade(value),
                SNARE_PARAM_PHASE_MOD_AMOUNT => s.set_phase_mod_amount(value),
                SNARE_PARAM_OVERDRIVE => s.set_overdrive(value),
                SNARE_PARAM_AMP_DECAY => s.set_amp_decay(value),
                SNARE_PARAM_AMP_DECAY_CURVE => s.set_amp_decay_curve(value),
                SNARE_PARAM_TONAL_DECAY_CURVE => s.set_tonal_decay_curve(value),
                SNARE_PARAM_TUNING => s.set_tuning(value),
                _ => {}
            },
            Self::HiHat(h) => match param {
                HIHAT_PARAM_PITCH => h.set_pitch(value),
                HIHAT_PARAM_DECAY => h.set_decay(value),
                HIHAT_PARAM_ATTACK => h.set_attack(value),
                HIHAT_PARAM_VOLUME => h.set_volume(value),
                HIHAT_PARAM_TONE => h.set_tone(value),
                HIHAT_PARAM_TUNING => h.set_tuning(value),
                _ => {}
            },
            Self::Tom(t) => {
                // Tom2 uses 0-100 range internally, FFI uses 0-1 normalized
                let scaled = value.clamp(0.0, 1.0) * 100.0;
                match param {
                    TOM_PARAM_TUNE => t.set_tune(scaled),
                    TOM_PARAM_BEND => t.set_bend(scaled),
                    TOM_PARAM_TONE => t.set_tone(scaled),
                    TOM_PARAM_COLOR => t.set_color(scaled),
                    TOM_PARAM_DECAY => t.set_decay(scaled),
                    TOM_PARAM_MEMBRANE => t.set_membrane(scaled),
                    TOM_PARAM_MEMBRANE_Q => t.set_membrane_q(scaled),
                    TOM_PARAM_VOLUME => t.set_volume(scaled),
                    // Tuning uses 0-1 directly (not 0-100)
                    TOM_PARAM_TUNING => t.set_tuning(value.clamp(0.0, 1.0)),
                    _ => {}
                }
            }
            Self::Bass(b) => match param {
                BASS_PARAM_FREQUENCY => b.set_frequency(value),
                BASS_PARAM_SUB_LEVEL => b.set_sub_level(value),
                BASS_PARAM_OSC_LEVEL => b.set_osc_level(value),
                BASS_PARAM_DETUNE_LEVEL => b.set_detune_level(value),
                BASS_PARAM_DETUNE_AMOUNT => b.set_detune_amount(value),
                BASS_PARAM_OSC_SHAPE => b.set_osc_shape(value),
                BASS_PARAM_FILTER_CUTOFF => b.set_filter_cutoff(value),
                BASS_PARAM_FILTER_RESONANCE => b.set_filter_resonance(value),
                BASS_PARAM_FILTER_ENV_AMOUNT => b.set_filter_env_amount(value),
                BASS_PARAM_FILTER_ENV_DECAY => b.set_filter_env_decay(value),
                BASS_PARAM_FILTER_ENV_CURVE => b.set_filter_env_curve(value),
                BASS_PARAM_AMP_DECAY => b.set_amp_decay(value),
                BASS_PARAM_AMP_DECAY_CURVE => b.set_amp_decay_curve(value),
                BASS_PARAM_OVERDRIVE => b.set_overdrive(value),
                BASS_PARAM_VOLUME => b.set_volume(value),
                BASS_PARAM_TUNING => b.set_tuning(value),
                _ => {}
            },
        }
    }

    /// Read the most-recently-set value of a parameter, in the same normalized 0-1
    /// range used by `set_param`. Returns `f32::NAN` if `param` is not recognized
    /// for the current variant.
    ///
    /// Uses `SmoothedParam::target()` (not `.get()`) so callers see the value they
    /// set, independent of any in-flight smoothing — appropriate for state recovery.
    fn get_param(&self, param: u32) -> f32 {
        match self {
            Self::Kick(k) => match param {
                KICK_PARAM_FREQUENCY => k.params.frequency.target(),
                KICK_PARAM_PUNCH => k.params.punch.target(),
                KICK_PARAM_SUB => k.params.sub.target(),
                KICK_PARAM_CLICK => k.params.click.target(),
                KICK_PARAM_DECAY => k.params.oscillator_decay.target(),
                KICK_PARAM_PITCH_ENVELOPE => k.params.pitch_envelope_amount.target(),
                KICK_PARAM_VOLUME => k.params.volume.target(),
                KICK_PARAM_TUNING => k.params.tuning.target(),
                _ => f32::NAN,
            },
            Self::Snare(s) => match param {
                SNARE_PARAM_FREQUENCY => s.params.frequency.target(),
                SNARE_PARAM_DECAY => s.params.decay.target(),
                SNARE_PARAM_BRIGHTNESS => s.params.brightness.target(),
                SNARE_PARAM_VOLUME => s.params.volume.target(),
                SNARE_PARAM_TONAL => s.params.tonal.target(),
                SNARE_PARAM_NOISE => s.params.noise.target(),
                SNARE_PARAM_PITCH_DROP => s.params.pitch_drop.target(),
                SNARE_PARAM_TONAL_DECAY => s.params.tonal_decay.target(),
                SNARE_PARAM_NOISE_DECAY => s.params.noise_decay.target(),
                SNARE_PARAM_NOISE_TAIL_DECAY => s.params.noise_tail_decay.target(),
                SNARE_PARAM_FILTER_CUTOFF => s.params.filter_cutoff.target(),
                SNARE_PARAM_FILTER_RESONANCE => s.params.filter_resonance.target(),
                SNARE_PARAM_FILTER_TYPE => s.params.filter_type as f32,
                SNARE_PARAM_XFADE => s.params.xfade.target(),
                SNARE_PARAM_PHASE_MOD_AMOUNT => s.params.phase_mod_amount.target(),
                SNARE_PARAM_OVERDRIVE => s.params.overdrive.target(),
                SNARE_PARAM_AMP_DECAY => s.params.amp_decay.target(),
                SNARE_PARAM_AMP_DECAY_CURVE => s.params.amp_decay_curve.target(),
                SNARE_PARAM_TONAL_DECAY_CURVE => s.params.tonal_decay_curve.target(),
                SNARE_PARAM_TUNING => s.params.tuning.target(),
                _ => f32::NAN,
            },
            Self::HiHat(h) => match param {
                HIHAT_PARAM_PITCH => h.params.pitch.target(),
                HIHAT_PARAM_DECAY => h.params.decay.target(),
                HIHAT_PARAM_ATTACK => h.params.attack.target(),
                HIHAT_PARAM_VOLUME => h.params.volume.target(),
                HIHAT_PARAM_TONE => h.params.tone.target(),
                HIHAT_PARAM_TUNING => h.params.tuning.target(),
                _ => f32::NAN,
            },
            Self::Tom(t) => match param {
                // Tom2 stores 0-100 internally; renormalize to 0-1 for the FFI surface.
                TOM_PARAM_TUNE => t.tune() / 100.0,
                TOM_PARAM_BEND => t.bend() / 100.0,
                TOM_PARAM_TONE => t.tone() / 100.0,
                TOM_PARAM_COLOR => t.color() / 100.0,
                TOM_PARAM_DECAY => t.decay() / 100.0,
                TOM_PARAM_MEMBRANE => t.membrane() / 100.0,
                TOM_PARAM_MEMBRANE_Q => t.membrane_q() / 100.0,
                TOM_PARAM_VOLUME => t.volume() / 100.0,
                // Tuning is stored 0-1 directly, mirroring the setter exception.
                TOM_PARAM_TUNING => t.tuning(),
                _ => f32::NAN,
            },
            Self::Bass(_) => f32::NAN,
        }
    }

    /// Apply LFO modulation to a parameter. Uses bipolar modulation for smoothed params.
    fn apply_modulation(&mut self, param: u32, value: f32) {
        match self {
            Self::Kick(k) => match param {
                KICK_PARAM_FREQUENCY => k.params.frequency.set_bipolar(value),
                KICK_PARAM_PUNCH => k.params.punch.set_bipolar(value),
                KICK_PARAM_SUB => k.params.sub.set_bipolar(value),
                KICK_PARAM_CLICK => k.params.click.set_bipolar(value),
                KICK_PARAM_DECAY => k.params.oscillator_decay.set_bipolar(value),
                // KICK_PARAM_PITCH_ENVELOPE is no longer modulatable: it was
                // baked at trigger time and never re-read. Use KICK_PARAM_TUNING
                // for live pitch modulation instead.
                KICK_PARAM_VOLUME => k.params.volume.set_bipolar(value),
                KICK_PARAM_TUNING => k.params.tuning.set_bipolar(value),
                _ => {}
            },
            Self::Snare(s) => match param {
                SNARE_PARAM_FREQUENCY => s.params.frequency.set_bipolar(value),
                SNARE_PARAM_DECAY => s.params.decay.set_bipolar(value),
                SNARE_PARAM_BRIGHTNESS => s.params.brightness.set_bipolar(value),
                SNARE_PARAM_VOLUME => s.params.volume.set_bipolar(value),
                SNARE_PARAM_TONAL => s.params.tonal.set_bipolar(value),
                SNARE_PARAM_NOISE => s.params.noise.set_bipolar(value),
                SNARE_PARAM_PITCH_DROP => s.params.pitch_drop.set_bipolar(value),
                SNARE_PARAM_TONAL_DECAY => s.params.tonal_decay.set_bipolar(value),
                SNARE_PARAM_NOISE_DECAY => s.params.noise_decay.set_bipolar(value),
                SNARE_PARAM_NOISE_TAIL_DECAY => s.params.noise_tail_decay.set_bipolar(value),
                SNARE_PARAM_FILTER_CUTOFF => s.params.filter_cutoff.set_bipolar(value),
                SNARE_PARAM_FILTER_RESONANCE => s.params.filter_resonance.set_bipolar(value),
                SNARE_PARAM_XFADE => s.params.xfade.set_bipolar(value),
                SNARE_PARAM_PHASE_MOD_AMOUNT => s.params.phase_mod_amount.set_bipolar(value),
                SNARE_PARAM_OVERDRIVE => s.params.overdrive.set_bipolar(value),
                SNARE_PARAM_AMP_DECAY => s.params.amp_decay.set_bipolar(value),
                SNARE_PARAM_AMP_DECAY_CURVE => s.params.amp_decay_curve.set_bipolar(value),
                SNARE_PARAM_TONAL_DECAY_CURVE => s.params.tonal_decay_curve.set_bipolar(value),
                SNARE_PARAM_TUNING => s.params.tuning.set_bipolar(value),
                _ => {}
            },
            Self::HiHat(h) => match param {
                HIHAT_PARAM_PITCH => h.params.pitch.set_bipolar(value),
                HIHAT_PARAM_DECAY => h.params.decay.set_bipolar(value),
                HIHAT_PARAM_ATTACK => h.params.attack.set_bipolar(value),
                HIHAT_PARAM_TONE => h.params.tone.set_bipolar(value),
                HIHAT_PARAM_VOLUME => h.params.volume.set_bipolar(value),
                HIHAT_PARAM_TUNING => h.params.tuning.set_bipolar(value),
                _ => {}
            },
            Self::Tom(t) => {
                // Tom2 uses 0-100 range, scale modulation accordingly
                let scaled = value * 100.0;
                match param {
                    TOM_PARAM_TUNE => t.set_tune(scaled),
                    TOM_PARAM_BEND => t.set_bend(scaled),
                    TOM_PARAM_TONE => t.set_tone(scaled),
                    TOM_PARAM_COLOR => t.set_color(scaled),
                    TOM_PARAM_DECAY => t.set_decay(scaled),
                    TOM_PARAM_MEMBRANE => t.set_membrane(scaled),
                    TOM_PARAM_MEMBRANE_Q => t.set_membrane_q(scaled),
                    TOM_PARAM_VOLUME => t.set_volume(scaled),
                    // Tuning uses 0-1 directly (not 0-100)
                    TOM_PARAM_TUNING => t.set_tuning(value.clamp(0.0, 1.0)),
                    _ => {}
                }
            }
            Self::Bass(b) => match param {
                BASS_PARAM_FREQUENCY => b.params.frequency.set_bipolar(value),
                BASS_PARAM_SUB_LEVEL => b.params.sub_level.set_bipolar(value),
                BASS_PARAM_OSC_LEVEL => b.params.osc_level.set_bipolar(value),
                BASS_PARAM_DETUNE_LEVEL => b.params.detune_level.set_bipolar(value),
                BASS_PARAM_DETUNE_AMOUNT => b.params.detune_amount.set_bipolar(value),
                BASS_PARAM_OSC_SHAPE => b.params.osc_shape.set_bipolar(value),
                BASS_PARAM_FILTER_CUTOFF => b.params.filter_cutoff.set_bipolar(value),
                BASS_PARAM_FILTER_RESONANCE => b.params.filter_resonance.set_bipolar(value),
                BASS_PARAM_FILTER_ENV_AMOUNT => b.params.filter_env_amount.set_bipolar(value),
                BASS_PARAM_FILTER_ENV_DECAY => b.params.filter_env_decay.set_bipolar(value),
                BASS_PARAM_FILTER_ENV_CURVE => b.params.filter_env_curve.set_bipolar(value),
                BASS_PARAM_AMP_DECAY => b.params.amp_decay.set_bipolar(value),
                BASS_PARAM_AMP_DECAY_CURVE => b.params.amp_decay_curve.set_bipolar(value),
                BASS_PARAM_OVERDRIVE => b.params.overdrive.set_bipolar(value),
                BASS_PARAM_VOLUME => b.params.volume.set_bipolar(value),
                BASS_PARAM_TUNING => b.params.tuning.set_bipolar(value),
                _ => {}
            },
        }
    }
}

/// A polymorphic preset blender matching the instrument type on a channel.
enum ChannelBlender {
    Kick(PresetBlender<KickConfig>),
    Snare(PresetBlender<SnareConfig>),
    HiHat(PresetBlender<HiHat2Config>),
    Tom(PresetBlender<Tom2Config>),
    Bass(PresetBlender<BassConfig>),
}

impl ChannelBlender {
    /// Blend at position (x,y) and apply the result to the instrument.
    fn blend_and_apply(&self, instrument: &mut ChannelInstrument, x: f32, y: f32) {
        match (self, instrument) {
            (Self::Kick(b), ChannelInstrument::Kick(k)) => k.set_config(b.blend(x, y)),
            (Self::Snare(b), ChannelInstrument::Snare(s)) => s.set_config(b.blend(x, y)),
            (Self::HiHat(b), ChannelInstrument::HiHat(h)) => h.set_config(b.blend(x, y)),
            (Self::Tom(b), ChannelInstrument::Tom(t)) => t.set_config(b.blend(x, y)),
            (Self::Bass(b), ChannelInstrument::Bass(bs)) => bs.set_config(b.blend(x, y)),
            _ => {} // type mismatch — should not happen if blender/instrument are kept in sync
        }
    }

    /// Set a corner preset by corner index and preset ID.
    fn set_corner_preset(&mut self, corner: u32, preset_id: u32) {
        match self {
            Self::Kick(b) => {
                if let Some(config) = GooeyEngine::kick_preset_by_id(preset_id) {
                    match corner {
                        BLEND_CORNER_BOTTOM_LEFT => b.set_bottom_left(config),
                        BLEND_CORNER_BOTTOM_RIGHT => b.set_bottom_right(config),
                        BLEND_CORNER_TOP_LEFT => b.set_top_left(config),
                        BLEND_CORNER_TOP_RIGHT => b.set_top_right(config),
                        _ => {}
                    }
                }
            }
            Self::Snare(b) => {
                if let Some(config) = GooeyEngine::snare_preset_by_id(preset_id) {
                    match corner {
                        BLEND_CORNER_BOTTOM_LEFT => b.set_bottom_left(config),
                        BLEND_CORNER_BOTTOM_RIGHT => b.set_bottom_right(config),
                        BLEND_CORNER_TOP_LEFT => b.set_top_left(config),
                        BLEND_CORNER_TOP_RIGHT => b.set_top_right(config),
                        _ => {}
                    }
                }
            }
            Self::HiHat(b) => {
                if let Some(config) = GooeyEngine::hihat_preset_by_id(preset_id) {
                    match corner {
                        BLEND_CORNER_BOTTOM_LEFT => b.set_bottom_left(config),
                        BLEND_CORNER_BOTTOM_RIGHT => b.set_bottom_right(config),
                        BLEND_CORNER_TOP_LEFT => b.set_top_left(config),
                        BLEND_CORNER_TOP_RIGHT => b.set_top_right(config),
                        _ => {}
                    }
                }
            }
            Self::Tom(b) => {
                if let Some(config) = GooeyEngine::tom_preset_by_id(preset_id) {
                    match corner {
                        BLEND_CORNER_BOTTOM_LEFT => b.set_bottom_left(config),
                        BLEND_CORNER_BOTTOM_RIGHT => b.set_bottom_right(config),
                        BLEND_CORNER_TOP_LEFT => b.set_top_left(config),
                        BLEND_CORNER_TOP_RIGHT => b.set_top_right(config),
                        _ => {}
                    }
                }
            }
            Self::Bass(b) => {
                if let Some(config) = GooeyEngine::bass_preset_by_id(preset_id) {
                    match corner {
                        BLEND_CORNER_BOTTOM_LEFT => b.set_bottom_left(config),
                        BLEND_CORNER_BOTTOM_RIGHT => b.set_bottom_right(config),
                        BLEND_CORNER_TOP_LEFT => b.set_top_left(config),
                        BLEND_CORNER_TOP_RIGHT => b.set_top_right(config),
                        _ => {}
                    }
                }
            }
        }
    }

    /// Create a default blender with standard corner presets for the given instrument type.
    fn default_for_type(instrument_type: u32) -> Self {
        match instrument_type {
            INSTRUMENT_KICK => Self::Kick(PresetBlender::new(
                KickConfig::tight(),
                KickConfig::punch(),
                KickConfig::loose(),
                KickConfig::dirt(),
            )),
            INSTRUMENT_SNARE => Self::Snare(PresetBlender::new(
                SnareConfig::tight(),
                SnareConfig::loose(),
                SnareConfig::hiss(),
                SnareConfig::smack(),
            )),
            INSTRUMENT_HIHAT => Self::HiHat(PresetBlender::new(
                HiHat2Config::short(),
                HiHat2Config::loose(),
                HiHat2Config::dark(),
                HiHat2Config::soft(),
            )),
            INSTRUMENT_TOM => Self::Tom(PresetBlender::new(
                Tom2Config::derp(),
                Tom2Config::ring(),
                Tom2Config::brush(),
                Tom2Config::void_preset(),
            )),
            INSTRUMENT_BASS => Self::Bass(PresetBlender::new(
                BassConfig::acid(),
                BassConfig::sub(),
                BassConfig::reese(),
                BassConfig::stab(),
            )),
            _ => Self::Kick(PresetBlender::new(
                KickConfig::tight(),
                KickConfig::punch(),
                KickConfig::loose(),
                KickConfig::dirt(),
            )),
        }
    }

    /// Returns the default corner preset IDs for a given instrument type.
    fn default_corner_preset_ids(instrument_type: u32) -> [u32; 4] {
        match instrument_type {
            INSTRUMENT_KICK => [
                KICK_PRESET_TIGHT,
                KICK_PRESET_PUNCH,
                KICK_PRESET_LOOSE,
                KICK_PRESET_DIRT,
            ],
            INSTRUMENT_SNARE => [
                SNARE_PRESET_TIGHT,
                SNARE_PRESET_LOOSE,
                SNARE_PRESET_HISS,
                SNARE_PRESET_SMACK,
            ],
            INSTRUMENT_HIHAT => [
                HIHAT_PRESET_SHORT,
                HIHAT_PRESET_LOOSE,
                HIHAT_PRESET_DARK,
                HIHAT_PRESET_SOFT,
            ],
            INSTRUMENT_TOM => [
                TOM_PRESET_DERP,
                TOM_PRESET_RING,
                TOM_PRESET_BRUSH,
                TOM_PRESET_VOID,
            ],
            INSTRUMENT_BASS => [
                BASS_PRESET_ACID,
                BASS_PRESET_SUB,
                BASS_PRESET_REESE,
                BASS_PRESET_STAB,
            ],
            _ => [0, 1, 2, 3],
        }
    }
}

/// Opaque wrapper around the audio engine for FFI
///
/// This struct provides a simplified C-compatible interface for iOS integration.
/// It manages 5 channels, each with an instrument and its own
/// 16-step sequencer with sample-accurate timing. Channels can be reassigned
/// to any instrument type at runtime.
///
/// Parameter smoothing is handled internally by each instrument,
/// so all parameter changes are automatically smoothed to prevent clicks/pops.
/// Number of drum voices in the kit (kick, snare, hihat, tom). Bass is a
/// separate top-level voice, so the addressable voice space
/// (`NUM_INSTRUMENTS` = 5) is the kit voices plus bass at index 4.
const KIT_VOICE_COUNT: usize = 4;

/// One voice's complete per-channel state: the instrument plus its sequencer,
/// preset blender, mixer strip (fader / mute-solo / pan / peak), manual-trigger
/// latch, and per-step MIDI-note frequency save slot. This bundles what were
/// previously parallel `[_; NUM_INSTRUMENTS]` arrays into a single owned column
/// so voices can be grouped into a `DrumKit` collection and routed as sources.
struct VoiceStrip {
    instrument: ChannelInstrument,
    sequencer: Sequencer,
    blender: ChannelBlender,
    blend_enabled: bool,
    blend_x: f32,
    blend_y: f32,
    blend_corner_presets: [u32; 4],
    /// Mixer fader (0.0–1.0), applied after synthesis/blend; the blend system
    /// cannot override it. Was `instrument_channel_gains[i]`.
    channel_gain: SmoothedParam,
    /// Smoothed mute/solo multiplier for click-free transitions. Was
    /// `instrument_gains[i]`.
    mute_gain: SmoothedParam,
    /// Stereo pan (0.0 = left, 0.5 = center, 1.0 = right), equal-power. Was
    /// `instrument_pans[i]`.
    pan: SmoothedParam,
    muted: AtomicBool,
    soloed: AtomicBool,
    /// Peak amplitude since last read (f32 bits, read-and-reset by UI). Was
    /// `channel_peaks[i]`.
    peak: AtomicU32,
    trigger_pending: AtomicBool,
    trigger_velocity: AtomicU32, // f32 bits stored atomically
    /// Saved global frequency for restoring after per-step MIDI note overrides.
    saved_global_freq: Option<f32>,
}

impl VoiceStrip {
    /// Build a voice from its instrument and a fresh sequencer. `instrument_type`
    /// selects the default preset blender and corner presets.
    fn new(
        instrument: ChannelInstrument,
        sequencer: Sequencer,
        instrument_type: u32,
        sample_rate: f32,
    ) -> Self {
        Self {
            instrument,
            sequencer,
            blender: ChannelBlender::default_for_type(instrument_type),
            blend_enabled: false,
            blend_x: 0.5,
            blend_y: 0.5,
            blend_corner_presets: ChannelBlender::default_corner_preset_ids(instrument_type),
            channel_gain: SmoothedParam::new(1.0, 0.0, 1.0, sample_rate, 10.0),
            mute_gain: SmoothedParam::new(1.0, 0.0, 1.0, sample_rate, 10.0),
            pan: SmoothedParam::new(0.5, 0.0, 1.0, sample_rate, 10.0),
            muted: AtomicBool::new(false),
            soloed: AtomicBool::new(false),
            peak: AtomicU32::new(0.0_f32.to_bits()),
            trigger_pending: AtomicBool::new(false),
            trigger_velocity: AtomicU32::new(1.0_f32.to_bits()),
            saved_global_freq: None,
        }
    }

    /// Record a new peak (read-and-reset by the UI). `level` is a pre-pan mono
    /// magnitude. Uses the same compare-and-store pattern as the old
    /// `channel_peaks` array.
    fn record_peak(&self, level: f32) {
        let prev = f32::from_bits(self.peak.load(Ordering::Relaxed));
        if level > prev {
            self.peak.store(level.to_bits(), Ordering::Relaxed);
        }
    }
}

/// A submixable collection of drum voices (kick, snare, hihat, tom). Each voice
/// keeps its own sequencer, blender, and mixer strip; the whole kit is routed as
/// one source (`SourceId::DrumKit`) in the mixer graph. Bass is intentionally not
/// a member — it is a melodic voice routed as its own source.
struct DrumKit {
    voices: [VoiceStrip; KIT_VOICE_COUNT],
}

pub struct GooeyEngine {
    // Drum voices (kick, snare, hihat, tom) grouped as one submixable kit.
    kit: DrumKit,

    // Bass voice — a melodic instrument routed as its own source, sibling to the
    // kit. Addressed as legacy instrument index 4.
    bass: VoiceStrip,

    /// Global effects. Applied in the order described by `effect_order`,
    /// followed by the optional limiter which is always last.
    /// Processing order when enabled: saturation -> lowpass filter -> tilt filter -> delay -> compressor -> reverb -> limiter.
    delay: DelayEffect,
    delay_enabled: bool,
    lowpass_filter: LowpassFilterEffect,
    lowpass_filter_enabled: bool,
    tilt_filter: TiltFilterEffect,
    tilt_filter_enabled: bool,
    saturation: TubeSaturation,
    saturation_enabled: bool,
    compressor: TubeCompressor,
    compressor_enabled: bool,
    compressor_sidechain: u32,
    reverb: SpringReverbEffect,
    reverb_enabled: bool,
    plate_reverb: PlateReverbEffect,
    plate_reverb_enabled: bool,
    waveshaper: Waveshaper,
    waveshaper_enabled: bool,
    feedback_waveshaper: FeedbackWaveshaper,
    feedback_waveshaper_enabled: bool,
    limiter: SoftLimiter,
    limiter_enabled: bool,

    /// Order in which the reorderable effects are applied. Stores `EFFECT_*`
    /// IDs (excluding `EFFECT_LIMITER`, which is pinned at the end of the chain).
    effect_order: [u32; REORDERABLE_EFFECT_COUNT as usize],

    // Engine state
    sample_rate: f32,
    bpm: f32,
    swing: f32,
    current_time: f64,
    /// Smoothed gain applied to the complete instrument sum before global effects.
    master_gain: SmoothedParam,

    // LFO pool (8 LFOs with multi-target routing)
    lfos: [Lfo; LFO_COUNT],
    lfo_enabled: [bool; LFO_COUNT],
    lfo_routes: [Vec<LfoRoute>; LFO_COUNT],
    lfo_next_route_id: [u32; LFO_COUNT],

    // Pending MIDI events from the most recent render pass (pre-allocated, no audio-thread alloc)
    pending_midi_events: Vec<GooeyMidiEvent>,

    // When false, sequencers still advance position but don't trigger instruments or emit MIDI events.
    // Used to let host MIDI input drive instruments instead of the internal sequencer.
    // Atomic so the host thread (FFI setter) and audio thread (render read) don't race;
    // mirrors the `instrument_muted` thread-safety pattern.
    sequencer_triggers_enabled: AtomicBool,

    // Polyphonic synthesizer for chord playback
    poly_synth: PolySynth,

    // Mono granular instrument (sample buffer loaded by the host)
    granulator: Granulator,

    // Multi-channel stereo loop mixer (4 channels, per-channel effects). Summed
    // into the master bus before the global effects chain. The `gooey_engine_loop_*`
    // FFI functions expose control over this.
    mixer: Mixer,

    // Host-defined mixer graph: named submix tracks, each with a strip
    // (gain / balance / mute-solo / peak) and its own effect rack. Sources
    // (drum kit, bass, poly, granulator, loop mixer) route into tracks, which
    // sum to the master bus. Controlled via `gooey_engine_mixer_*` and
    // `gooey_engine_track_effect_*`.
    graph: MixerGraph,

    // When true, an external source (e.g. Ableton Link) owns the tempo.
    // The host should check this flag to decide whether local BPM changes
    // should be applied directly or routed through the external sync source.
    link_enabled: AtomicBool,

    // Error state (set after a panic in render, checked on every render call)
    error_occurred: AtomicBool,
    error_message: Option<CString>,
    error_callback: Option<extern "C" fn(*mut c_void, *const c_char)>,
    error_callback_context: *mut c_void,

    // Host-clock state for scheduled (Link-synced) sequencer start.
    // `host_clock_anchor` is set by `gooey_engine_set_render_host_time` once
    // per buffer in the audio callback. `pending_arm_host_time` is staged by
    // `gooey_engine_sequencer_start_at_host_time` and resolved at the top of
    // each render call against `host_clock_anchor`.
    host_clock_anchor: Option<HostClockAnchor>,
    pending_arm_host_time: Option<PendingArm>,
}

/// Host-clock reference for the next render buffer. The audio callback sets
/// this just before calling `gooey_engine_render` so the engine can convert
/// absolute host times into per-buffer sample offsets.
#[derive(Clone, Copy, Debug)]
struct HostClockAnchor {
    /// Host time (e.g. mach_absolute_time) of sample 0 of the buffer.
    host_time_first_sample: u64,
    /// Host-clock ticks per audio sample (host_ticks_per_second / sample_rate).
    host_ticks_per_sample: f64,
}

/// A pending scheduled start. Resolved at render time using the current
/// `HostClockAnchor`; if the buffer ends before the start time, the arm
/// stays staged for the next render.
#[derive(Clone, Copy, Debug)]
struct PendingArm {
    /// Absolute host time at which the sequencer should start.
    start_host_time: u64,
    /// Cursor position (in quarter notes) at the moment the arm fires.
    beat_position: f64,
}

/// Outcome of resolving `pending_arm_host_time` against `host_clock_anchor`
/// at the start of a render call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArmResolution {
    /// No arm pending — render normally.
    NotPending,
    /// Arm fires at this sample offset within the buffer (0-based).
    FiresAt(u32),
    /// Arm fires later than this buffer — emit silence for the whole buffer
    /// and leave the arm staged for the next render.
    SilentBuffer,
}

impl GooeyEngine {
    fn new(sample_rate: f32) -> Self {
        let bpm = 120.0;

        // Drum kit: four voices (kick, snare, hihat, tom), each with its own
        // 16-step sequencer, blender, and mixer strip.
        let kit = DrumKit {
            voices: [
                VoiceStrip::new(
                    ChannelInstrument::Kick(KickDrum::new(sample_rate)),
                    Sequencer::with_pattern(bpm, sample_rate, vec![false; 16], "kick"),
                    INSTRUMENT_KICK,
                    sample_rate,
                ),
                VoiceStrip::new(
                    ChannelInstrument::Snare(SnareDrum::new(sample_rate)),
                    Sequencer::with_pattern(bpm, sample_rate, vec![false; 16], "snare"),
                    INSTRUMENT_SNARE,
                    sample_rate,
                ),
                VoiceStrip::new(
                    ChannelInstrument::HiHat(HiHat2::new(sample_rate)),
                    Sequencer::with_pattern(bpm, sample_rate, vec![false; 16], "hihat"),
                    INSTRUMENT_HIHAT,
                    sample_rate,
                ),
                VoiceStrip::new(
                    ChannelInstrument::Tom(Tom2::new(sample_rate)),
                    Sequencer::with_pattern(bpm, sample_rate, vec![false; 16], "tom"),
                    INSTRUMENT_TOM,
                    sample_rate,
                ),
            ],
        };

        // Bass voice — routed as its own source (legacy instrument index 4).
        let bass = VoiceStrip::new(
            ChannelInstrument::Bass(BassSynth::new(sample_rate)),
            Sequencer::with_pattern(bpm, sample_rate, vec![false; 16], "bass"),
            INSTRUMENT_BASS,
            sample_rate,
        );

        // Create delay with default settings (quarter note timing, no feedback, no mix, filter open)
        let delay = DelayEffect::new(sample_rate, DelayTiming::Quarter, bpm, 0.0, 0.0, 20000.0);

        // Create lowpass filter with default settings (fully open, no resonance)
        let lowpass_filter = LowpassFilterEffect::new(sample_rate, 20000.0, 0.0);

        // Create tilt filter with default settings (center = passthrough)
        let tilt_filter = TiltFilterEffect::new(sample_rate);

        // Create saturation with default light warmth settings
        let saturation = TubeSaturation::new(sample_rate, 0.3, 0.4, 0.5);

        // Create compressor with drum-friendly defaults
        // threshold: -12 dB, ratio: 4:1, attack: 5ms, release: 100ms, mix: 0.5
        let compressor = TubeCompressor::new(sample_rate, -12.0, 4.0, 5.0, 100.0, 0.5);

        // Create spring reverb with default settings (decay: 0.5, mix: 0.0 = dry, damping: 0.5)
        let reverb = SpringReverbEffect::new(sample_rate, 0.5, 0.0, 0.5);

        // Create plate reverb with default settings (decay: 0.5, mix: 0.0 = dry, damping: 0.5)
        let plate_reverb = PlateReverbEffect::new(sample_rate, 0.5, 0.0, 0.5);

        // Create waveshaper with default bypass settings (drive: 1.0, mix: 0.0)
        let waveshaper = Waveshaper::new(1.0, 0.0);

        // Create feedback waveshaper with default bypass settings
        let feedback_waveshaper = FeedbackWaveshaper::new(sample_rate, 1.0, 0.0, 2000.0, 0.0);

        // Create LFO pool (8 LFOs, all disabled by default with quarter note timing)
        let lfos = std::array::from_fn(|_| Lfo::with_sample_rate(sample_rate));
        let lfo_routes: [Vec<LfoRoute>; LFO_COUNT] = std::array::from_fn(|_| Vec::new());

        Self {
            kit,
            bass,
            delay,
            delay_enabled: false,
            lowpass_filter,
            lowpass_filter_enabled: false,
            tilt_filter,
            tilt_filter_enabled: false,
            saturation,
            saturation_enabled: false,
            compressor,
            compressor_enabled: false,
            compressor_sidechain: COMPRESSOR_SIDECHAIN_NONE,
            reverb,
            reverb_enabled: false,
            plate_reverb,
            plate_reverb_enabled: false,
            waveshaper,
            waveshaper_enabled: false,
            feedback_waveshaper,
            feedback_waveshaper_enabled: false,
            limiter: SoftLimiter::new(1.0),
            limiter_enabled: false,
            effect_order: DEFAULT_EFFECT_ORDER,
            sample_rate,
            bpm,
            swing: 0.5,
            current_time: 0.0,
            // Match the native Engine's default summing headroom.
            master_gain: SmoothedParam::new(DEFAULT_MASTER_GAIN, 0.0, 2.0, sample_rate, 30.0),
            // LFO pool
            lfos,
            lfo_enabled: [false; LFO_COUNT],
            lfo_routes,
            lfo_next_route_id: [0; LFO_COUNT],
            // MIDI event buffer (pre-allocated for audio thread safety)
            pending_midi_events: Vec::with_capacity(MIDI_EVENT_CAPACITY),
            // Sequencer triggers enabled by default (internal sequencer drives instruments)
            sequencer_triggers_enabled: AtomicBool::new(true),
            // Polyphonic synthesizer for chord playback
            poly_synth: PolySynth::new(sample_rate),
            // Granulator with a silent placeholder buffer until the host loads samples.
            // The placeholder uses a hardcoded sample rate so this constructor cannot
            // fail on a non-finite or non-positive `sample_rate` argument — `gooey_engine_new`
            // is an `extern "C"` entry point and must never panic on user input.
            granulator: Granulator::new(
                sample_rate,
                SampleBuffer::from_mono(vec![0.0_f32], 44_100.0)
                    .unwrap_or_else(|_| unreachable!("constant placeholder buffer is valid")),
            ),
            // Multi-channel stereo loop mixer (empty until the host loads loops).
            mixer: Mixer::new(sample_rate),
            // Host-defined mixer graph, seeded with the default 4-track layout
            // (Drums / Bass / Synth / Loops). Bit-identical to the flat mix until
            // the host adjusts a track strip, rack, or routing.
            graph: MixerGraph::with_default_layout(sample_rate, bpm),
            // External sync (e.g. Ableton Link)
            link_enabled: AtomicBool::new(false),
            // Error state
            error_occurred: AtomicBool::new(false),
            error_message: None,
            error_callback: None,
            error_callback_context: std::ptr::null_mut(),
            // Scheduled-start state (used for Ableton Link Sync Start/Stop)
            host_clock_anchor: None,
            pending_arm_host_time: None,
        }
    }

    /// Resolve any `pending_arm_host_time` against the current
    /// `host_clock_anchor` for a buffer of `frames` samples. Called once at
    /// the top of `render`; the result drives whether (and at which sample
    /// offset within this buffer) the arm fires.
    fn resolve_pending_arm(&self, frames: usize) -> ArmResolution {
        let pending = match self.pending_arm_host_time {
            Some(p) => p,
            None => return ArmResolution::NotPending,
        };

        let anchor = match self.host_clock_anchor {
            Some(a) if a.host_ticks_per_sample > 0.0 => a,
            // No host-clock reference — fail-safe to immediate fire so
            // callers that arm but forget to set the host clock get a
            // working sequencer rather than silence forever.
            _ => return ArmResolution::FiresAt(0),
        };

        // Use i128 to safely express the (signed) delta between two u64
        // host times when the start time may be earlier than the anchor.
        let delta_ticks = pending.start_host_time as i128 - anchor.host_time_first_sample as i128;
        if delta_ticks <= 0 {
            return ArmResolution::FiresAt(0);
        }

        let samples_until = (delta_ticks as f64 / anchor.host_ticks_per_sample).ceil();
        if !samples_until.is_finite() || samples_until <= 0.0 {
            return ArmResolution::FiresAt(0);
        }

        let samples_until = samples_until as u64;
        if (samples_until as usize) < frames {
            ArmResolution::FiresAt(samples_until as u32)
        } else {
            ArmResolution::SilentBuffer
        }
    }

    /// Push a MIDI event without growing the buffer. Drops the event if at capacity.
    #[inline]
    fn push_midi_event(&mut self, instrument_index: u32, velocity: f32, sample_offset: u32) {
        if self.pending_midi_events.len() < MIDI_EVENT_CAPACITY {
            self.pending_midi_events.push(GooeyMidiEvent {
                instrument_index,
                velocity,
                sample_offset,
            });
        }
    }

    /// Render audio into an interleaved stereo `buffer` of `buffer.len() / 2`
    /// frames. Each frame occupies two consecutive slots: `[left, right]`. The
    /// signal path is mono, so left and right are currently identical (see the
    /// "stereo seam" near the end of the per-frame loop), but the engine writes
    /// two-channel output so hosts (and future stereo features) consume stereo.
    /// Borrow a voice by legacy instrument index: 0..=3 are the kit drum voices
    /// (kick, snare, hihat, tom), 4 is bass. Returns `None` for out-of-range.
    fn voice(&self, idx: usize) -> Option<&VoiceStrip> {
        match idx {
            i if i < KIT_VOICE_COUNT => self.kit.voices.get(i),
            i if i == KIT_VOICE_COUNT => Some(&self.bass),
            _ => None,
        }
    }

    /// Mutable counterpart to [`voice`](Self::voice).
    fn voice_mut(&mut self, idx: usize) -> Option<&mut VoiceStrip> {
        match idx {
            i if i < KIT_VOICE_COUNT => self.kit.voices.get_mut(i),
            i if i == KIT_VOICE_COUNT => Some(&mut self.bass),
            _ => None,
        }
    }

    /// Iterate all addressable voices in index order (kit drums then bass).
    fn voices_iter(&self) -> impl Iterator<Item = &VoiceStrip> {
        self.kit.voices.iter().chain(std::iter::once(&self.bass))
    }

    /// Mutable counterpart to [`voices_iter`](Self::voices_iter).
    fn voices_iter_mut(&mut self) -> impl Iterator<Item = &mut VoiceStrip> {
        self.kit
            .voices
            .iter_mut()
            .chain(std::iter::once(&mut self.bass))
    }

    fn render(&mut self, buffer: &mut [f32]) {
        // Clear pending MIDI events from previous render pass
        self.pending_midi_events.clear();

        // Number of stereo frames this buffer holds (two slots per frame).
        let frame_count = buffer.len() / 2;

        // Resolve any host-time-armed start against this buffer's host clock.
        // Possible outcomes:
        //   `Some(0)`  → arm fires on sample 0 of this buffer.
        //   `Some(N)`  → samples 0..N produce silence; arm fires on sample N.
        //   `None`     → no arm pending OR arm fires later than this buffer.
        // When the arm fires later than this buffer, we zero the whole
        // buffer below and leave `pending_arm_host_time` staged so the next
        // render re-evaluates against its own host time.
        let arm_state = self.resolve_pending_arm(frame_count);
        let mut arm_fires_at: Option<u32> = match arm_state {
            ArmResolution::FiresAt(n) => Some(n),
            ArmResolution::SilentBuffer => {
                // Whole buffer is silent — pre-fire countdown spans this entire buffer.
                // Drain any pending manual triggers so they don't latch and fire
                // late once the arm resolves; the trigger API contract is "fires
                // on the next render call", and this render produced silence.
                for voice in self.voices_iter() {
                    voice.trigger_pending.store(false, Ordering::Release);
                }
                for sample in buffer.iter_mut() {
                    *sample = 0.0;
                }
                return;
            }
            ArmResolution::NotPending => None,
        };

        // Check for pending manual triggers with velocity (all channels)
        // Manual triggers fire at sample_offset 0 (start of buffer)
        for ch in 0..NUM_INSTRUMENTS {
            let fired = self.voice(ch).and_then(|v| {
                if v.trigger_pending.swap(false, Ordering::Acquire) {
                    Some(f32::from_bits(v.trigger_velocity.load(Ordering::Acquire)))
                } else {
                    None
                }
            });
            if let Some(velocity) = fired {
                self.push_midi_event(ch as u32, velocity, 0);
                let time = self.current_time;
                if let Some(voice) = self.voice_mut(ch) {
                    voice.instrument.trigger_with_velocity(time, velocity);
                }
            }
        }

        let sample_period = 1.0 / self.sample_rate as f64;

        // Update mute/solo gain targets (check once per buffer for efficiency)
        let any_soloed = self.voices_iter().any(|v| v.soloed.load(Ordering::Relaxed));
        for voice in self.voices_iter_mut() {
            let target = Self::calculate_instrument_gain(
                voice.muted.load(Ordering::Relaxed),
                voice.soloed.load(Ordering::Relaxed),
                any_soloed,
            );
            voice.mute_gain.set_target(target);
        }
        // Recompute per-track mute/solo targets (scoped across tracks) once per buffer.
        self.graph.update_mute_solo_targets();

        let mut sample_offset: u32 = 0;
        for frame in buffer.chunks_mut(2) {
            // Pre-fire portion of an armed start: emit silence and skip all
            // per-sample work (no sequencer ticks, no instrument ticks, no
            // LFO ticks, no time advance) so the engine stays exactly where
            // it was at arm time.
            if let Some(fire_at) = arm_fires_at {
                if sample_offset < fire_at {
                    frame.fill(0.0);
                    sample_offset += 1;
                    continue;
                }
                // sample_offset == fire_at — arm fires this sample. Apply
                // the staged seek+start to all sequencers so they begin
                // running at the requested beat position. Clearing
                // arm_fires_at locally and pending_arm_host_time on the
                // engine ensures we only fire once per render.
                if let Some(pending) = self.pending_arm_host_time.take() {
                    for voice in self.voices_iter_mut() {
                        voice.sequencer.set_beat_position(pending.beat_position);
                        voice.sequencer.start();
                    }
                }
                arm_fires_at = None;
            }

            // Tick ALL sequencers first to ensure sample-accurate synchronization
            let mut seq_triggers: [Option<(f32, Option<SequencerBlendSetting>, Option<u8>)>;
                NUM_INSTRUMENTS] = [None; NUM_INSTRUMENTS];
            for ch in 0..NUM_INSTRUMENTS {
                if let Some(voice) = self.voice_mut(ch) {
                    seq_triggers[ch] = voice
                        .sequencer
                        .tick_with_settings()
                        .map(|trigger| (trigger.velocity, trigger.blend, trigger.note));
                }
            }

            // Apply triggers with velocity after all sequencers have been ticked.
            if self.sequencer_triggers_enabled.load(Ordering::Relaxed) {
                let time = self.current_time;
                for ch in 0..NUM_INSTRUMENTS {
                    if let Some((velocity, blend, note)) = seq_triggers[ch] {
                        self.apply_sequencer_blend_setting(ch as u32, blend);
                        if let Some(voice) = self.voice_mut(ch) {
                            // Snap params only when a blend was actually applied,
                            // so we don't clobber in-flight UI/LFO smoothing.
                            if blend.is_some() || voice.blend_enabled {
                                voice.instrument.snap_params();
                            }
                            // Apply per-step MIDI note frequency override (sample-accurate).
                            // When a step has a note, save the global freq and override.
                            // When a step has no note, restore the saved global freq.
                            if let Some(midi_note) = note {
                                let instr_type = voice.instrument.instrument_type();
                                if let Some((freq_min, freq_max)) =
                                    Self::freq_range_for_instrument(instr_type)
                                {
                                    if voice.saved_global_freq.is_none() {
                                        voice.saved_global_freq = voice.instrument.get_freq_param();
                                    }
                                    let normalized = Self::midi_note_to_normalized_freq(
                                        midi_note, freq_min, freq_max,
                                    );
                                    voice.instrument.set_param(0, normalized);
                                    voice.instrument.snap_params();
                                }
                            } else if let Some(saved) = voice.saved_global_freq.take() {
                                voice.instrument.set_param(0, saved);
                                voice.instrument.snap_params();
                            }
                            voice.instrument.trigger_with_velocity(time, velocity);
                        }
                        self.push_midi_event(ch as u32, velocity, sample_offset);
                    }
                }
            }

            // Process LFOs and apply modulation to routed parameters
            for lfo_idx in 0..LFO_COUNT {
                if self.lfo_enabled[lfo_idx] {
                    let lfo_value = self.lfos[lfo_idx].tick();
                    let route_count = self.lfo_routes[lfo_idx].len();

                    for route_idx in 0..route_count {
                        let channel = self.lfo_routes[lfo_idx][route_idx].instrument;
                        let param = self.lfo_routes[lfo_idx][route_idx].param;
                        let depth = self.lfo_routes[lfo_idx][route_idx].depth;
                        let modulation = lfo_value * depth;
                        self.apply_modulation_by_index(channel, param, modulation);
                    }
                }
            }

            // Generate audio from each channel with gains and mute/solo, then
            // spread it across the stereo field via the per-channel pan.
            // Stereo seam: each instrument's mono output is panned (equal-power,
            // default center) and summed here — BEFORE the effects chain — so
            // each effect can process true left/right. Effects are stereo-aware
            // (per-channel state); with every channel centered and no stereo
            // effect engaged the two channels stay identical.
            // Sum each voice into its source frame: kit voices (0..KIT_VOICE_COUNT)
            // form the DrumKit source, bass forms the Bass source. Per-voice gain,
            // mute/solo, pan, and peak metering are unchanged; only the routing
            // target differs. `channel_outs` still feeds the compressor sidechain.
            let mut channel_outs = [0.0_f32; NUM_INSTRUMENTS];
            let mut kit_frame = StereoFrame::default();
            let mut bass_frame = StereoFrame::default();
            let time = self.current_time;
            for (ch, voice) in self.voices_iter_mut().enumerate() {
                let ch_out = voice.instrument.tick(time)
                    * voice.channel_gain.tick()
                    * voice.mute_gain.tick();
                channel_outs[ch] = ch_out;

                let panned = StereoFrame::panned(ch_out, voice.pan.tick());
                if ch < KIT_VOICE_COUNT {
                    kit_frame += panned;
                } else {
                    bass_frame += panned;
                }

                // Track per-voice peak for UI metering (pre-pan mono level)
                voice.record_peak(ch_out.abs());
            }

            // Poly synth and granulator have no pan control yet — center them
            // for consistency with the equal-power law used above.
            let poly_frame = StereoFrame::panned(self.poly_synth.tick(time), 0.5);
            let gran_frame = StereoFrame::panned(self.granulator.tick(time), 0.5);
            // Loop mixer is already stereo, with its own per-channel effects.
            let loop_frame = self.mixer.tick(self.sample_rate);

            // Scatter each source into its routed track, then apply per-track
            // strip (gain × mute/solo, balance) + effect rack and sum to master.
            self.graph.clear_scratch();
            self.graph.scatter(SOURCE_DRUMKIT, kit_frame);
            self.graph.scatter(SOURCE_BASS, bass_frame);
            self.graph.scatter(SOURCE_POLYSYNTH, poly_frame);
            self.graph.scatter(SOURCE_GRANULATOR, gran_frame);
            self.graph.scatter(SOURCE_LOOPMIXER, loop_frame);
            let mut stereo = self.graph.mix_down();

            // Apply master headroom to the full mix (instruments + loops) before
            // the optional global effects + limiter, so the master fader scales
            // loops too.
            stereo = stereo.scaled(self.master_gain.tick());

            // Apply global effects chain (order is user-configurable; limiter is always last)
            for &effect_id in &self.effect_order {
                match effect_id {
                    EFFECT_SATURATION if self.saturation_enabled => {
                        stereo = self.saturation.process_stereo(stereo);
                    }
                    EFFECT_LOWPASS_FILTER if self.lowpass_filter_enabled => {
                        stereo = self.lowpass_filter.process_stereo(stereo);
                    }
                    EFFECT_TILT_FILTER if self.tilt_filter_enabled => {
                        stereo = self.tilt_filter.process_stereo(stereo);
                    }
                    EFFECT_DELAY if self.delay_enabled => {
                        stereo = self.delay.process_stereo(stereo);
                    }
                    EFFECT_COMPRESSOR if self.compressor_enabled => {
                        let sc = self.compressor_sidechain as usize;
                        stereo = if sc < NUM_INSTRUMENTS {
                            // The sidechain source is a mono per-instrument
                            // sample; feed it to both detectors equally.
                            self.compressor.process_stereo_with_sidechain(
                                stereo,
                                StereoFrame::mono(channel_outs[sc]),
                            )
                        } else {
                            self.compressor.process_stereo(stereo)
                        };
                    }
                    EFFECT_WAVESHAPER if self.waveshaper_enabled => {
                        stereo = StereoFrame {
                            l: self.waveshaper.process(stereo.l),
                            r: self.waveshaper.process(stereo.r),
                        };
                    }
                    EFFECT_FEEDBACK_WAVESHAPER if self.feedback_waveshaper_enabled => {
                        stereo = StereoFrame {
                            l: self.feedback_waveshaper.process(stereo.l),
                            r: self.feedback_waveshaper.process(stereo.r),
                        };
                    }
                    EFFECT_REVERB if self.reverb_enabled => {
                        stereo = self.reverb.process_stereo(stereo);
                    }
                    EFFECT_PLATE_REVERB if self.plate_reverb_enabled => {
                        stereo = self.plate_reverb.process_stereo(stereo);
                    }
                    _ => {}
                }
            }

            // Optional limiter (always last when enabled)
            let stereo = if self.limiter_enabled {
                self.limiter.process_stereo(stereo)
            } else {
                stereo
            };

            // Write the frame interleaved as [left, right].
            frame[0] = stereo.l;
            if let Some(right) = frame.get_mut(1) {
                *right = stereo.r;
            }

            self.current_time += sample_period;
            sample_offset += 1;
        }
    }

    fn apply_sequencer_blend_setting(
        &mut self,
        channel: u32,
        blend: Option<SequencerBlendSetting>,
    ) {
        if let Some(blend_setting) = blend {
            self.apply_blend_position(channel, blend_setting.x, blend_setting.y);
            return;
        }

        let idx = channel as usize;
        if let Some((x, y)) = self
            .voice(idx)
            .filter(|v| v.blend_enabled)
            .map(|v| (v.blend_x, v.blend_y))
        {
            self.apply_blend_position(channel, x, y);
        }
    }

    fn apply_blend_position(&mut self, channel: u32, x: f32, y: f32) {
        let x = x.clamp(0.0, 1.0);
        let y = y.clamp(0.0, 1.0);
        let idx = channel as usize;
        if let Some(voice) = self.voice_mut(idx) {
            voice.blender.blend_and_apply(&mut voice.instrument, x, y);
        }
    }

    /// Clear internal state of all reorderable effects so a new chain order
    /// does not inherit stale buffers/envelopes from the previous routing.
    /// Limiter is intentionally skipped — keeping its gain-reduction state
    /// stable avoids transients if the optional final stage is enabled.
    fn reset_effect_states(&self) {
        self.saturation.reset();
        self.lowpass_filter.reset();
        self.tilt_filter.reset();
        self.delay.reset();
        self.compressor.reset();
        self.reverb.reset();
        self.plate_reverb.reset();
    }

    /// Apply LFO modulation to a channel's instrument parameter by index
    fn apply_modulation_by_index(&mut self, channel: u32, param: u32, value: f32) {
        if let Some(voice) = self.voice_mut(channel as usize) {
            voice.instrument.apply_modulation(param, value);
        }
    }

    /// Calculate the target gain for an instrument based on mute/solo state
    /// Returns 1.0 (full volume) or 0.0 (silent)
    #[inline]
    fn calculate_instrument_gain(muted: bool, soloed: bool, any_soloed: bool) -> f32 {
        // Solo takes precedence: if this instrument is soloed, it plays
        if soloed {
            return 1.0;
        }

        // If any instrument is soloed but this one isn't, silence it
        if any_soloed {
            return 0.0;
        }

        // No solo active: check mute state
        if muted {
            return 0.0;
        }

        // Not muted, not affected by solo: full volume
        1.0
    }

    /// Borrow the first voice's instrument matching the given type.
    fn instrument_by_type(&self, instrument_type: u32) -> Option<&ChannelInstrument> {
        self.voices_iter()
            .map(|v| &v.instrument)
            .find(|i| i.instrument_type() == instrument_type)
    }

    /// Mutable counterpart to [`instrument_by_type`](Self::instrument_by_type).
    fn instrument_by_type_mut(&mut self, instrument_type: u32) -> Option<&mut ChannelInstrument> {
        self.voices_iter_mut()
            .map(|v| &mut v.instrument)
            .find(|i| i.instrument_type() == instrument_type)
    }

    /// Get a KickConfig preset by ID
    fn kick_preset_by_id(id: u32) -> Option<KickConfig> {
        match id {
            KICK_PRESET_TIGHT => Some(KickConfig::tight()),
            KICK_PRESET_PUNCH => Some(KickConfig::punch()),
            KICK_PRESET_LOOSE => Some(KickConfig::loose()),
            KICK_PRESET_DIRT => Some(KickConfig::dirt()),
            _ => None,
        }
    }

    /// Get a Tom2Config preset by ID
    fn tom_preset_by_id(id: u32) -> Option<Tom2Config> {
        match id {
            TOM_PRESET_DERP => Some(Tom2Config::derp()),
            TOM_PRESET_RING => Some(Tom2Config::ring()),
            TOM_PRESET_BRUSH => Some(Tom2Config::brush()),
            TOM_PRESET_VOID => Some(Tom2Config::void_preset()),
            _ => None,
        }
    }

    /// Get a BassConfig preset by ID
    fn bass_preset_by_id(id: u32) -> Option<BassConfig> {
        match id {
            BASS_PRESET_ACID => Some(BassConfig::acid()),
            BASS_PRESET_SUB => Some(BassConfig::sub()),
            BASS_PRESET_REESE => Some(BassConfig::reese()),
            BASS_PRESET_STAB => Some(BassConfig::stab()),
            _ => None,
        }
    }

    /// Convert a MIDI note number to a normalized frequency value for an instrument's range.
    fn midi_note_to_normalized_freq(note: u8, freq_min: f32, freq_max: f32) -> f32 {
        let hz = 440.0 * 2f32.powf((note as f32 - 69.0) / 12.0);
        ((hz - freq_min) / (freq_max - freq_min)).clamp(0.0, 1.0)
    }

    /// Get the frequency range (min, max) for a pitched instrument type.
    fn freq_range_for_instrument(instrument_type: u32) -> Option<(f32, f32)> {
        match instrument_type {
            INSTRUMENT_BASS => Some((30.0, 200.0)),
            INSTRUMENT_KICK => Some((30.0, 120.0)),
            INSTRUMENT_TOM => Some((40.0, 600.0)),
            _ => None,
        }
    }

    /// Get a SnareConfig preset by ID
    fn snare_preset_by_id(id: u32) -> Option<SnareConfig> {
        match id {
            SNARE_PRESET_TIGHT => Some(SnareConfig::tight()),
            SNARE_PRESET_LOOSE => Some(SnareConfig::loose()),
            SNARE_PRESET_HISS => Some(SnareConfig::hiss()),
            SNARE_PRESET_SMACK => Some(SnareConfig::smack()),
            _ => None,
        }
    }

    /// Get a HiHat2Config preset by ID
    fn hihat_preset_by_id(id: u32) -> Option<HiHat2Config> {
        match id {
            HIHAT_PRESET_SHORT => Some(HiHat2Config::short()),
            HIHAT_PRESET_LOOSE => Some(HiHat2Config::loose()),
            HIHAT_PRESET_DARK => Some(HiHat2Config::dark()),
            HIHAT_PRESET_SOFT => Some(HiHat2Config::soft()),
            _ => None,
        }
    }
}

// =============================================================================
// Global effect IDs (must match Swift GlobalEffect enum)
// =============================================================================

/// Global effect: Lowpass filter
pub const EFFECT_LOWPASS_FILTER: u32 = 0;
/// Global effect: Delay
pub const EFFECT_DELAY: u32 = 1;
/// Global effect: Saturation
pub const EFFECT_SATURATION: u32 = 2;
/// Global effect: Compressor
pub const EFFECT_COMPRESSOR: u32 = 3;
/// Global effect: Tilt filter (unified lowpass/highpass)
pub const EFFECT_TILT_FILTER: u32 = 4;
/// Global effect: Optional soft limiter
pub const EFFECT_LIMITER: u32 = 5;

// =============================================================================
// Limiter parameter indices (must match Swift LimiterParam enum)
// =============================================================================

/// Limiter parameter: threshold (0.001-1.0)
pub const LIMITER_PARAM_THRESHOLD: u32 = 0;
/// Global effect: Spring reverb
pub const EFFECT_REVERB: u32 = 6;
/// Global effect: Waveshaper (tanh soft-clip distortion)
pub const EFFECT_WAVESHAPER: u32 = 7;
/// Global effect: Feedback waveshaper (self-exciting distortion)
pub const EFFECT_FEEDBACK_WAVESHAPER: u32 = 8;
/// Global effect: Plate reverb (Dattorro figure-eight tank)
pub const EFFECT_PLATE_REVERB: u32 = 9;
/// Total number of global effects
pub const EFFECT_COUNT: u32 = 10;

/// Number of reorderable effects in the chain. Excludes the optional limiter,
/// which is pinned at the end of the chain when enabled.
pub const REORDERABLE_EFFECT_COUNT: u32 = 9;

/// Default order for the reorderable effects, matching the historical
/// hardcoded chain (saturation -> lowpass -> tilt -> delay -> compressor -> reverb).
const DEFAULT_EFFECT_ORDER: [u32; REORDERABLE_EFFECT_COUNT as usize] = [
    EFFECT_WAVESHAPER,
    EFFECT_SATURATION,
    EFFECT_LOWPASS_FILTER,
    EFFECT_TILT_FILTER,
    EFFECT_DELAY,
    EFFECT_COMPRESSOR,
    EFFECT_FEEDBACK_WAVESHAPER,
    EFFECT_REVERB,
    EFFECT_PLATE_REVERB,
];

// =============================================================================
// Lowpass filter parameter indices (must match Swift FilterParam enum)
// =============================================================================

/// Filter parameter: cutoff frequency (20-20000 Hz)
pub const FILTER_PARAM_CUTOFF: u32 = 0;
/// Filter parameter: resonance (0.0-0.95)
pub const FILTER_PARAM_RESONANCE: u32 = 1;

// =============================================================================
// Delay parameter indices (must match Swift DelayParam enum)
// =============================================================================

/// Delay parameter: timing division (see DELAY_TIMING_* constants)
pub const DELAY_PARAM_TIMING: u32 = 0;
/// Delay parameter: feedback amount (0.0-0.95)
pub const DELAY_PARAM_FEEDBACK: u32 = 1;
/// Delay parameter: wet/dry mix (0.0-1.0)
pub const DELAY_PARAM_MIX: u32 = 2;
/// Delay parameter: filter cutoff in Hz (20-20000)
pub const DELAY_PARAM_FILTER_CUTOFF: u32 = 3;
/// Delay parameter: ping-pong mode (0.0 = off, >= 0.5 = on). When on, the delay
/// feedback crosses channels so echoes bounce between left and right.
pub const DELAY_PARAM_PINGPONG: u32 = 4;

// =============================================================================
// Delay timing constants (must match Swift DelayTiming enum)
// =============================================================================

/// Delay timing: whole note (4 beats)
pub const DELAY_TIMING_WHOLE: u32 = 0;
/// Delay timing: half note (2 beats)
pub const DELAY_TIMING_HALF: u32 = 1;
/// Delay timing: quarter note (1 beat)
pub const DELAY_TIMING_QUARTER: u32 = 2;
/// Delay timing: eighth note (1/2 beat)
pub const DELAY_TIMING_EIGHTH: u32 = 3;
/// Delay timing: sixteenth note (1/4 beat)
pub const DELAY_TIMING_SIXTEENTH: u32 = 4;
/// Delay timing: half note triplet (4/3 beats)
pub const DELAY_TIMING_HALF_TRIPLET: u32 = 5;
/// Delay timing: quarter note triplet (2/3 beat)
pub const DELAY_TIMING_QUARTER_TRIPLET: u32 = 6;
/// Delay timing: eighth note triplet (1/3 beat)
pub const DELAY_TIMING_EIGHTH_TRIPLET: u32 = 7;
/// Delay timing: sixteenth note triplet (1/6 beat)
pub const DELAY_TIMING_SIXTEENTH_TRIPLET: u32 = 8;

// =============================================================================
// Saturation parameter indices (must match Swift SaturationParam enum)
// =============================================================================

/// Saturation parameter: drive amount (0.0-1.0)
pub const SATURATION_PARAM_DRIVE: u32 = 0;
/// Saturation parameter: warmth/even harmonics (0.0-1.0)
pub const SATURATION_PARAM_WARMTH: u32 = 1;
/// Saturation parameter: wet/dry mix (0.0-1.0)
pub const SATURATION_PARAM_MIX: u32 = 2;

// =============================================================================
// Compressor parameter indices (must match Swift CompressorParam enum)
// =============================================================================

/// Compressor parameter: threshold in dB (-60.0 to 0.0)
pub const COMPRESSOR_PARAM_THRESHOLD: u32 = 0;
/// Compressor parameter: ratio (1.0 to 20.0)
pub const COMPRESSOR_PARAM_RATIO: u32 = 1;
/// Compressor parameter: attack time in ms (0.1 to 100.0)
pub const COMPRESSOR_PARAM_ATTACK: u32 = 2;
/// Compressor parameter: release time in ms (5.0 to 1000.0)
pub const COMPRESSOR_PARAM_RELEASE: u32 = 3;
/// Compressor parameter: wet/dry mix (0.0 to 1.0)
pub const COMPRESSOR_PARAM_MIX: u32 = 4;

/// No sidechain — compressor uses main mix as input (default)
pub const COMPRESSOR_SIDECHAIN_NONE: u32 = 0xFFFFFFFF;

// =============================================================================
// Tilt filter parameter indices
// =============================================================================

/// Tilt filter parameter: cutoff position (0.0-1.0, 0.5 = center/passthrough)
pub const TILT_PARAM_CUTOFF: u32 = 0;
/// Tilt filter parameter: resonance (0.0-1.0)
pub const TILT_PARAM_RESONANCE: u32 = 1;

// =============================================================================
// Reverb parameter indices
// =============================================================================

/// Reverb parameter: decay amount (0.0-1.0)
pub const REVERB_PARAM_DECAY: u32 = 0;
/// Reverb parameter: wet/dry mix (0.0-1.0)
pub const REVERB_PARAM_MIX: u32 = 1;
/// Reverb parameter: high-frequency damping (0.0-1.0)
pub const REVERB_PARAM_DAMPING: u32 = 2;

// =============================================================================
// Plate reverb parameter indices (must match Swift PlateParam enum)
// =============================================================================

/// Plate reverb parameter: decay amount (0.0-1.0)
pub const PLATE_PARAM_DECAY: u32 = 0;
/// Plate reverb parameter: wet/dry mix (0.0-1.0)
pub const PLATE_PARAM_MIX: u32 = 1;
/// Plate reverb parameter: high-frequency damping (0.0-1.0)
pub const PLATE_PARAM_DAMPING: u32 = 2;
/// Plate reverb parameter: predelay (0.0-1.0, maps linearly to 0-200 ms)
pub const PLATE_PARAM_PREDELAY: u32 = 3;
/// Plate reverb parameter: stereo width of the wet signal (0.0 = mono, 1.0 = full)
pub const PLATE_PARAM_WIDTH: u32 = 4;
/// Plate reverb parameter: tank size (0.0-1.0; 0.5 = the published Dattorro
/// plate, endpoints scale the tank from 0.25x to 2.0x)
pub const PLATE_PARAM_SIZE: u32 = 5;

// =============================================================================
// Waveshaper parameter indices
// =============================================================================

/// Waveshaper parameter: drive amount (1.0-10.0)
pub const WAVESHAPER_PARAM_DRIVE: u32 = 0;
/// Waveshaper parameter: dry/wet mix (0.0-1.0)
pub const WAVESHAPER_PARAM_MIX: u32 = 1;

// =============================================================================
// Feedback waveshaper parameter indices
// =============================================================================

/// Feedback waveshaper parameter: drive amount (1.0-100.0)
pub const FEEDBACK_WAVESHAPER_PARAM_DRIVE: u32 = 0;
/// Feedback waveshaper parameter: feedback gain (0.0-0.98)
pub const FEEDBACK_WAVESHAPER_PARAM_FEEDBACK: u32 = 1;
/// Feedback waveshaper parameter: feedback lowpass cutoff (200-20000 Hz)
pub const FEEDBACK_WAVESHAPER_PARAM_FILTER_CUTOFF: u32 = 2;
/// Feedback waveshaper parameter: dry/wet mix (0.0-1.0)
pub const FEEDBACK_WAVESHAPER_PARAM_MIX: u32 = 3;

// =============================================================================
// Kick drum parameter indices (must match Swift KickParam enum)
// =============================================================================

/// Kick parameter: base frequency (30-80 Hz)
pub const KICK_PARAM_FREQUENCY: u32 = 0;
/// Kick parameter: punch/mid presence (0-1)
pub const KICK_PARAM_PUNCH: u32 = 1;
/// Kick parameter: sub bass presence (0-1)
pub const KICK_PARAM_SUB: u32 = 2;
/// Kick parameter: click/transient amount (0-1)
pub const KICK_PARAM_CLICK: u32 = 3;
/// Kick parameter: decay time (0.01-5.0 seconds)
pub const KICK_PARAM_DECAY: u32 = 4;
/// Kick parameter: pitch envelope amount (0-1)
pub const KICK_PARAM_PITCH_ENVELOPE: u32 = 5;
/// Kick parameter: overall volume (0-1)
pub const KICK_PARAM_VOLUME: u32 = 6;
/// Kick parameter: tuning offset (0=−12 semitones, 0.5=neutral, 1=+12 semitones)
pub const KICK_PARAM_TUNING: u32 = 7;

// =============================================================================
// Hi-hat parameter indices (must match Swift HiHatParam enum)
// =============================================================================

/// Hi-hat parameter: pitch (0-1 normalized)
pub const HIHAT_PARAM_PITCH: u32 = 0;
/// Hi-hat parameter: decay (0-1 normalized)
pub const HIHAT_PARAM_DECAY: u32 = 1;
/// Hi-hat parameter: attack (0-1 normalized)
pub const HIHAT_PARAM_ATTACK: u32 = 2;
/// Hi-hat parameter: tone (0-1 normalized)
pub const HIHAT_PARAM_TONE: u32 = 3;
/// Hi-hat parameter: volume (0-1 normalized)
pub const HIHAT_PARAM_VOLUME: u32 = 4;
/// Hi-hat parameter: tuning offset (0=−12 semitones, 0.5=neutral, 1=+12 semitones)
pub const HIHAT_PARAM_TUNING: u32 = 5;

// =============================================================================
// Snare drum parameter indices (must match Swift SnareParam enum)
// =============================================================================

/// Snare parameter: base frequency (0-1 → 100-600 Hz)
pub const SNARE_PARAM_FREQUENCY: u32 = 0;
/// Snare parameter: decay time (0-1 → 0.05-3.5 seconds)
pub const SNARE_PARAM_DECAY: u32 = 1;
/// Snare parameter: brightness/snap amount (0-1)
pub const SNARE_PARAM_BRIGHTNESS: u32 = 2;
/// Snare parameter: overall volume (0-1)
pub const SNARE_PARAM_VOLUME: u32 = 3;
/// Snare parameter: tonal body amount (0-1)
pub const SNARE_PARAM_TONAL: u32 = 4;
/// Snare parameter: noise amount (0-1)
pub const SNARE_PARAM_NOISE: u32 = 5;
/// Snare parameter: pitch drop amount (0-1)
pub const SNARE_PARAM_PITCH_DROP: u32 = 6;
/// Snare parameter: tonal decay (0-1 → 0-3.5s)
pub const SNARE_PARAM_TONAL_DECAY: u32 = 7;
/// Snare parameter: noise decay (0-1 → 0-3.5s)
pub const SNARE_PARAM_NOISE_DECAY: u32 = 8;
/// Snare parameter: noise tail decay (0-1 → 0-3.5s)
pub const SNARE_PARAM_NOISE_TAIL_DECAY: u32 = 9;
/// Snare parameter: filter cutoff (0-1 → 100-10000 Hz)
pub const SNARE_PARAM_FILTER_CUTOFF: u32 = 10;
/// Snare parameter: filter resonance (0-1 → 0.5-10.0)
pub const SNARE_PARAM_FILTER_RESONANCE: u32 = 11;
/// Snare parameter: filter type (0=LP, 1=BP, 2=HP, 3=notch)
pub const SNARE_PARAM_FILTER_TYPE: u32 = 12;
/// Snare parameter: tonal/noise crossfade (0-1)
pub const SNARE_PARAM_XFADE: u32 = 13;
/// Snare parameter: phase modulation amount (0-1, 0 = disabled)
pub const SNARE_PARAM_PHASE_MOD_AMOUNT: u32 = 14;
/// Snare parameter: overdrive/saturation (0-1, 0 = bypass)
pub const SNARE_PARAM_OVERDRIVE: u32 = 15;
/// Snare parameter: master amplitude decay (0-1 → 0-4.0s)
pub const SNARE_PARAM_AMP_DECAY: u32 = 16;
/// Snare parameter: amplitude decay curve (0-1 → 0.1-10.0)
pub const SNARE_PARAM_AMP_DECAY_CURVE: u32 = 17;
/// Snare parameter: tonal decay curve (0-1 → 0.1-10.0)
pub const SNARE_PARAM_TONAL_DECAY_CURVE: u32 = 18;
/// Snare parameter: tuning offset (0=−12 semitones, 0.5=neutral, 1=+12 semitones)
pub const SNARE_PARAM_TUNING: u32 = 19;

// =============================================================================
// Tom drum parameter indices (Tom2 - must match Swift TomParam enum)
// =============================================================================

/// Tom parameter: tune (0-1 → 0-100, maps to 40-600 Hz)
pub const TOM_PARAM_TUNE: u32 = 0;
/// Tom parameter: bend (0-1 → 0-100, pitch envelope depth)
pub const TOM_PARAM_BEND: u32 = 1;
/// Tom parameter: tone (0-1 → 0-100, mix control)
pub const TOM_PARAM_TONE: u32 = 2;
/// Tom parameter: color (0-1 → 0-100, noise rate / filter cutoff)
pub const TOM_PARAM_COLOR: u32 = 3;
/// Tom parameter: decay (0-1 → 0-100, maps to 0.5-4000ms)
pub const TOM_PARAM_DECAY: u32 = 4;
/// Tom parameter: membrane mix (0-1 → 0-100, resonator mix amount)
pub const TOM_PARAM_MEMBRANE: u32 = 5;
/// Tom parameter: membrane Q (0-1 → 0-100, resonator Q scale)
pub const TOM_PARAM_MEMBRANE_Q: u32 = 6;
/// Tom parameter: volume (0-1 → 0-100, overall volume)
pub const TOM_PARAM_VOLUME: u32 = 7;
/// Tom parameter: tuning offset (0=−12 semitones, 0.5=neutral, 1=+12 semitones)
pub const TOM_PARAM_TUNING: u32 = 8;

// =============================================================================
// Instrument IDs (must match Swift/C enum if used)
// =============================================================================

/// Instrument ID: kick drum
pub const INSTRUMENT_KICK: u32 = 0;
/// Instrument ID: snare drum
pub const INSTRUMENT_SNARE: u32 = 1;
/// Instrument ID: hi-hat
pub const INSTRUMENT_HIHAT: u32 = 2;
/// Instrument ID: tom drum
pub const INSTRUMENT_TOM: u32 = 3;
/// Instrument ID: bass synth
pub const INSTRUMENT_BASS: u32 = 4;
/// Total number of instruments
pub const INSTRUMENT_COUNT: u32 = 5;
/// Internal usize version for array indexing
const NUM_INSTRUMENTS: usize = INSTRUMENT_COUNT as usize;
const DEFAULT_MASTER_GAIN: f32 = 0.25;

/// Number of stereo loop-mixer channels (see `gooey_engine_loop_*`).
pub const LOOP_CHANNEL_COUNT: u32 = crate::mixer::LOOP_CHANNEL_COUNT as u32;

/// Mixer graph source: the drum kit (kick/snare/hihat/tom summed).
pub const SOURCE_DRUMKIT: u32 = crate::mixer::graph::SOURCE_DRUMKIT;
/// Mixer graph source: the bass voice.
pub const SOURCE_BASS: u32 = crate::mixer::graph::SOURCE_BASS;
/// Mixer graph source: the polyphonic synth.
pub const SOURCE_POLYSYNTH: u32 = crate::mixer::graph::SOURCE_POLYSYNTH;
/// Mixer graph source: the granulator.
pub const SOURCE_GRANULATOR: u32 = crate::mixer::graph::SOURCE_GRANULATOR;
/// Mixer graph source: the stereo loop mixer.
pub const SOURCE_LOOPMIXER: u32 = crate::mixer::graph::SOURCE_LOOPMIXER;
/// Number of routable mixer graph sources.
pub const SOURCE_COUNT: u32 = crate::mixer::graph::SOURCE_COUNT as u32;

// =============================================================================
// Preset blend constants
// =============================================================================

/// Kick preset: Tight - short, punchy kick with strong pitch envelope
pub const KICK_PRESET_TIGHT: u32 = 0;
/// Kick preset: Punch - mid-focused with click and resonant noise
pub const KICK_PRESET_PUNCH: u32 = 1;
/// Kick preset: Loose - longer decay, more punch, subtle pitch envelope
pub const KICK_PRESET_LOOSE: u32 = 2;
/// Kick preset: Dirt - higher frequency, more noise with high resonance
pub const KICK_PRESET_DIRT: u32 = 3;

/// Tom preset: Derp - punchy mid tom
pub const TOM_PRESET_DERP: u32 = 0;
/// Tom preset: Ring - high, long decay
pub const TOM_PRESET_RING: u32 = 1;
/// Tom preset: Brush - low, textured
pub const TOM_PRESET_BRUSH: u32 = 2;
/// Tom preset: Void - atmospheric, long
pub const TOM_PRESET_VOID: u32 = 3;

// =============================================================================
// Bass synth parameter constants
// =============================================================================

/// Bass parameter: base frequency (0-1 -> 30-200 Hz)
pub const BASS_PARAM_FREQUENCY: u32 = 0;
/// Bass parameter: sub sine level (0-1)
pub const BASS_PARAM_SUB_LEVEL: u32 = 1;
/// Bass parameter: main saw/square level (0-1)
pub const BASS_PARAM_OSC_LEVEL: u32 = 2;
/// Bass parameter: detuned layer level (0-1)
pub const BASS_PARAM_DETUNE_LEVEL: u32 = 3;
/// Bass parameter: detune spread (0-1 -> 0-30 cents)
pub const BASS_PARAM_DETUNE_AMOUNT: u32 = 4;
/// Bass parameter: oscillator shape - saw(0) to square(1)
pub const BASS_PARAM_OSC_SHAPE: u32 = 5;
/// Bass parameter: filter cutoff (0-1 -> 20-18000 Hz exp)
pub const BASS_PARAM_FILTER_CUTOFF: u32 = 6;
/// Bass parameter: filter resonance (0-1 -> 0.5-15.0 Q)
pub const BASS_PARAM_FILTER_RESONANCE: u32 = 7;
/// Bass parameter: filter envelope depth (0-1)
pub const BASS_PARAM_FILTER_ENV_AMOUNT: u32 = 8;
/// Bass parameter: filter envelope decay (0-1 -> 0.01-2.0s)
pub const BASS_PARAM_FILTER_ENV_DECAY: u32 = 9;
/// Bass parameter: filter envelope curve (0-1 -> 0.1-8.0)
pub const BASS_PARAM_FILTER_ENV_CURVE: u32 = 10;
/// Bass parameter: amplitude decay (0-1 -> 0.05-4.0s)
pub const BASS_PARAM_AMP_DECAY: u32 = 11;
/// Bass parameter: amplitude decay curve (0-1 -> 0.1-10.0)
pub const BASS_PARAM_AMP_DECAY_CURVE: u32 = 12;
/// Bass parameter: pre-filter overdrive/saturation (0-1)
pub const BASS_PARAM_OVERDRIVE: u32 = 13;
/// Bass parameter: master volume (0-1)
pub const BASS_PARAM_VOLUME: u32 = 14;
/// Bass parameter: tuning offset (0=−12 semitones, 0.5=neutral, 1=+12 semitones)
pub const BASS_PARAM_TUNING: u32 = 15;

// =============================================================================
// Granulator parameter constants
// =============================================================================
//
// All values are normalized 0.0-1.0. Mapping to internal units (ms, Hz, ratio)
// happens inside `Granulator`; see `src/instruments/granulator.rs`.

/// Granulator parameter: scan position into the loaded buffer (0=start, 1=end)
pub const GRANULATOR_PARAM_SCAN_POSITION: u32 = 0;
/// Granulator parameter: grain length (0-1 -> 5-3000 ms, quadratic curve)
pub const GRANULATOR_PARAM_GRAIN_LENGTH: u32 = 1;
/// Granulator parameter: spray / randomization of grain start (0-1 -> 0-10 s, cubic)
pub const GRANULATOR_PARAM_SPRAY: u32 = 2;
/// Granulator parameter: playback pitch (0-1 -> 0.25x-4x, exponential)
pub const GRANULATOR_PARAM_PITCH: u32 = 3;
/// Granulator parameter: density of grains per second (0-1 -> 0-80 g/s)
pub const GRANULATOR_PARAM_DENSITY: u32 = 4;
/// Granulator parameter: window shape / texture (0=soft, 1=hard)
pub const GRANULATOR_PARAM_TEXTURE: u32 = 5;
/// Granulator parameter: probability a grain plays in reverse (0=forward only, 1=reverse only)
pub const GRANULATOR_PARAM_DIRECTION: u32 = 6;
/// Granulator parameter: cloud duration after a trigger (0-1 -> 50-8000 ms, quadratic)
pub const GRANULATOR_PARAM_CLOUD_DURATION: u32 = 7;
/// Granulator parameter: output volume (0-1)
pub const GRANULATOR_PARAM_VOLUME: u32 = 8;
/// Granulator parameter: per-grain spawn-time jitter (0=periodic, 1=±100% of interval)
pub const GRANULATOR_PARAM_RANDOM_TIMING: u32 = 9;
/// Granulator parameter: per-grain random amplitude (0=uniform, 1=full random duck to 0)
pub const GRANULATOR_PARAM_RANDOM_AMP: u32 = 10;
/// Granulator parameter: output soft-saturation drive (0=clean, 1=heavy tanh)
pub const GRANULATOR_PARAM_DRIVE: u32 = 11;
/// Total number of granulator parameters
pub const GRANULATOR_PARAM_COUNT: u32 = 12;

/// Bass preset: Acid - TB-303-style, high resonance, short filter sweep
pub const BASS_PRESET_ACID: u32 = 0;
/// Bass preset: Sub - clean sub-bass, sine dominant
pub const BASS_PRESET_SUB: u32 = 1;
/// Bass preset: Reese - detuned saws, heavy overdrive
pub const BASS_PRESET_REESE: u32 = 2;
/// Bass preset: Stab - square wave, sharp filter, short decay
pub const BASS_PRESET_STAB: u32 = 3;

/// Sentinel value indicating no MIDI note is set for a step (use instrument's global frequency)
pub const STEP_NOTE_NONE: u8 = 255;

/// Snare preset: Tight - short, punchy snare
pub const SNARE_PRESET_TIGHT: u32 = 0;
/// Snare preset: Loose - longer decay, more body
pub const SNARE_PRESET_LOOSE: u32 = 1;
/// Snare preset: Hiss - noise-focused with phase modulation
pub const SNARE_PRESET_HISS: u32 = 2;
/// Snare preset: Smack - DS-style transient with SVF noise
pub const SNARE_PRESET_SMACK: u32 = 3;

/// Hi-hat preset: Short
pub const HIHAT_PRESET_SHORT: u32 = 0;
/// Hi-hat preset: Loose
pub const HIHAT_PRESET_LOOSE: u32 = 1;
/// Hi-hat preset: Dark
pub const HIHAT_PRESET_DARK: u32 = 2;
/// Hi-hat preset: Soft
pub const HIHAT_PRESET_SOFT: u32 = 3;

/// Blend corner: bottom-left (x=0, y=0)
pub const BLEND_CORNER_BOTTOM_LEFT: u32 = 0;
/// Blend corner: bottom-right (x=1, y=0)
pub const BLEND_CORNER_BOTTOM_RIGHT: u32 = 1;
/// Blend corner: top-left (x=0, y=1)
pub const BLEND_CORNER_TOP_LEFT: u32 = 2;
/// Blend corner: top-right (x=1, y=1)
pub const BLEND_CORNER_TOP_RIGHT: u32 = 3;

// =============================================================================
// Engine lifecycle
// =============================================================================

/// Create a new gooey engine
///
/// # Arguments
/// * `sample_rate` - Audio sample rate (e.g., 44100.0 or 48000.0)
///
/// # Returns
/// Pointer to a new GooeyEngine instance. Must be freed with `gooey_engine_free`.
///
/// # Safety
/// The returned pointer must be freed with `gooey_engine_free` to avoid memory leaks.
#[no_mangle]
pub extern "C" fn gooey_engine_new(sample_rate: f32) -> *mut GooeyEngine {
    let engine = Box::new(GooeyEngine::new(sample_rate));
    Box::into_raw(engine)
}

/// Free a gooey engine
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`, or null.
/// After calling this function, the pointer is invalid and must not be used.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_free(engine: *mut GooeyEngine) {
    if !engine.is_null() {
        drop(Box::from_raw(engine));
    }
}

// =============================================================================
// Audio rendering
// =============================================================================

/// Number of interleaved output channels produced by `gooey_engine_render`.
/// The render buffer must hold `frames * GOOEY_OUTPUT_CHANNELS` floats.
pub const GOOEY_OUTPUT_CHANNELS: u32 = 2;

/// Render interleaved stereo audio into the provided buffer
///
/// This is the main audio callback function. Call this from your audio thread
/// to generate audio samples. Output is two-channel: the buffer holds `frames`
/// stereo frames laid out as interleaved `[left, right]` pairs, so the caller
/// must provide `frames * GOOEY_OUTPUT_CHANNELS` (`frames * 2`) floats. The
/// signal path is currently mono, so left and right are identical, but callers
/// should always treat the buffer as stereo.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `buffer` - Pointer to a buffer of floats to fill with interleaved L/R audio
/// * `frames` - Number of stereo frames to render
///
/// # Safety
/// - `engine` must be a valid pointer returned by `gooey_engine_new`
/// - `buffer` must point to at least `frames * 2` floats of allocated memory
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_render(
    engine: *mut GooeyEngine,
    buffer: *mut f32,
    frames: u32,
) {
    if engine.is_null() || buffer.is_null() {
        return;
    }

    let engine_ref = &mut *engine;
    let buffer_slice = slice::from_raw_parts_mut(buffer, frames as usize * 2);

    // If engine is already in error state, output silence
    if engine_ref.error_occurred.load(Ordering::Relaxed) {
        for sample in buffer_slice.iter_mut() {
            *sample = 0.0;
        }
        return;
    }

    // Wrap render in catch_unwind to prevent panics from crossing the FFI boundary.
    // AssertUnwindSafe is sound here: after a panic we mark the engine as permanently
    // errored and never call render() on it again.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        engine_ref.render(buffer_slice);
    }));

    if let Err(panic_payload) = result {
        let msg = if let Some(s) = panic_payload.downcast_ref::<&str>() {
            format!("gooey: render panic: {}", s)
        } else if let Some(s) = panic_payload.downcast_ref::<String>() {
            format!("gooey: render panic: {}", s)
        } else {
            "gooey: render panic (unknown cause)".to_string()
        };

        let c_msg = CString::new(msg).unwrap_or_else(|_| {
            CString::new("gooey: render panic (message contained null byte)").unwrap()
        });
        let msg_ptr = c_msg.as_ptr();

        engine_ref.error_message = Some(c_msg);
        engine_ref.error_occurred.store(true, Ordering::Release);

        // Zero the output buffer (interleaved stereo: frames * 2 floats)
        let buffer_slice = slice::from_raw_parts_mut(buffer, frames as usize * 2);
        for sample in buffer_slice.iter_mut() {
            *sample = 0.0;
        }

        // Invoke error callback if registered
        if let Some(callback) = engine_ref.error_callback {
            callback(engine_ref.error_callback_context, msg_ptr);
        }
    }
}

// =============================================================================
// MIDI event output
// =============================================================================

/// Copies pending MIDI events into `out_events` and returns how many were written.
///
/// Events are cleared after draining — the caller gets each event exactly once.
/// Must be called from the audio thread, immediately after `gooey_engine_render()`.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `out_events` - Pointer to a caller-allocated array of GooeyMidiEvent
/// * `max_events` - Capacity of the `out_events` array
///
/// # Returns
/// Number of events written (0 if none pending or on null input)
///
/// # Safety
/// - `engine` must be a valid pointer returned by `gooey_engine_new`
/// - `out_events` must point to at least `max_events` elements of allocated memory
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_drain_midi_events(
    engine: *mut GooeyEngine,
    out_events: *mut GooeyMidiEvent,
    max_events: u32,
) -> u32 {
    if engine.is_null() || out_events.is_null() || max_events == 0 {
        return 0;
    }

    let engine_ref = &mut *engine;
    let count = engine_ref
        .pending_midi_events
        .len()
        .min(max_events as usize);

    if count > 0 {
        std::ptr::copy_nonoverlapping(engine_ref.pending_midi_events.as_ptr(), out_events, count);
        // Remove only the events that were copied; retain any overflow
        engine_ref.pending_midi_events.drain(..count);
    }

    count as u32
}

// =============================================================================
// Sequencer trigger control
// =============================================================================

/// Enable or disable note triggering from the internal sequencer.
///
/// When disabled (`enabled = false`):
/// - The sequencer still advances its position each render cycle (step tracking works)
/// - But the sequencer does NOT call instrument trigger functions or emit MIDI events
///
/// When enabled (`enabled = true`, the default):
/// - Normal behavior — sequencer triggers instruments on active steps
///
/// Use this to let the host's MIDI input drive the instruments instead
/// of the internal sequencer, while keeping position tracking intact.
///
/// # Safety
/// - `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_sequencer_triggers_enabled(
    engine: *mut GooeyEngine,
    enabled: bool,
) {
    if engine.is_null() {
        return;
    }
    (*engine)
        .sequencer_triggers_enabled
        .store(enabled, Ordering::Release);
}

/// Query whether sequencer triggers are currently enabled.
///
/// Returns `true` if the engine pointer is null (safe default).
///
/// # Safety
/// - `engine` must be a valid pointer returned by `gooey_engine_new`, or null
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_sequencer_triggers_enabled(
    engine: *const GooeyEngine,
) -> bool {
    if engine.is_null() {
        return true;
    }
    (*engine).sequencer_triggers_enabled.load(Ordering::Acquire)
}

// =============================================================================
// Error handling
// =============================================================================

/// Register an error callback
///
/// The callback will be invoked if a fatal error (e.g., panic) occurs during rendering.
/// It is called at most once, from the audio thread, immediately after the error.
/// After the callback fires, the engine is in a terminal error state and must be freed
/// with `gooey_engine_free` and recreated with `gooey_engine_new`.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `context` - Opaque user pointer passed back to the callback (can be null)
/// * `callback` - C function pointer, or null to unregister
///
/// # Safety
/// - `engine` must be a valid pointer returned by `gooey_engine_new`
/// - `context` must remain valid for the lifetime of the engine
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_error_callback(
    engine: *mut GooeyEngine,
    context: *mut c_void,
    callback: Option<extern "C" fn(*mut c_void, *const c_char)>,
) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    engine.error_callback = callback;
    engine.error_callback_context = context;
}

/// Check if the engine is in an error state
///
/// Returns true if a fatal error has occurred during rendering.
/// Once in an error state, the engine outputs silence and must be freed and recreated.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_has_error(engine: *const GooeyEngine) -> bool {
    if engine.is_null() {
        return false;
    }
    (*engine).error_occurred.load(Ordering::Relaxed)
}

/// Get the error message if the engine is in an error state
///
/// Returns a pointer to a null-terminated C string describing the error,
/// or null if no error has occurred. The string is owned by the engine
/// and remains valid until `gooey_engine_free` is called.
///
/// # Safety
/// - `engine` must be a valid pointer returned by `gooey_engine_new`
/// - The returned string must not be freed by the caller
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_error_message(
    engine: *const GooeyEngine,
) -> *const c_char {
    if engine.is_null() {
        return std::ptr::null();
    }
    match &(*engine).error_message {
        Some(msg) => msg.as_ptr(),
        None => std::ptr::null(),
    }
}

// =============================================================================
// Channel instrument type swap
// =============================================================================

/// Reassign which synthesizer type runs on a given channel.
///
/// After calling this, the channel produces the new instrument's sound.
/// Resets synth DSP state for that channel (new instrument starts fresh).
/// Preserves channel-level state: gain, mute, solo, sequencer pattern, blend position.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `channel` - Channel index (0-3)
/// * `instrument_type` - Instrument type (INSTRUMENT_KICK=0, INSTRUMENT_SNARE=1, INSTRUMENT_HIHAT=2, INSTRUMENT_TOM=3)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_channel_instrument_type(
    engine: *mut GooeyEngine,
    channel: u32,
    instrument_type: u32,
) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    let sample_rate = engine.sample_rate;
    let Some(voice) = engine.voice_mut(channel as usize) else {
        return;
    };

    // No-op if already the requested type
    if voice.instrument.instrument_type() == instrument_type {
        return;
    }

    let new_instrument = match instrument_type {
        INSTRUMENT_KICK => ChannelInstrument::Kick(KickDrum::new(sample_rate)),
        INSTRUMENT_SNARE => ChannelInstrument::Snare(SnareDrum::new(sample_rate)),
        INSTRUMENT_HIHAT => ChannelInstrument::HiHat(HiHat2::new(sample_rate)),
        INSTRUMENT_TOM => ChannelInstrument::Tom(Tom2::new(sample_rate)),
        INSTRUMENT_BASS => ChannelInstrument::Bass(BassSynth::new(sample_rate)),
        _ => return,
    };

    voice.instrument = new_instrument;
    voice.blender = ChannelBlender::default_for_type(instrument_type);
    voice.blend_corner_presets = ChannelBlender::default_corner_preset_ids(instrument_type);

    // If blend is enabled, re-apply position with the new blender
    if voice.blend_enabled {
        let x = voice.blend_x;
        let y = voice.blend_y;
        voice.blender.blend_and_apply(&mut voice.instrument, x, y);
    }
    // channel gain, mute, solo, sequencer pattern all preserved on the voice
}

/// Returns the current instrument type for a channel.
///
/// Default mapping: channel N has instrument_type N.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `channel` - Channel index (0-3)
///
/// # Returns
/// Instrument type (INSTRUMENT_KICK=0, INSTRUMENT_SNARE=1, INSTRUMENT_HIHAT=2, INSTRUMENT_TOM=3),
/// or 0xFFFFFFFF if invalid.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_channel_instrument_type(
    engine: *const GooeyEngine,
    channel: u32,
) -> u32 {
    if engine.is_null() {
        return 0xFFFFFFFF;
    }
    let engine = &*engine;
    match engine.voice(channel as usize) {
        Some(voice) => voice.instrument.instrument_type(),
        None => 0xFFFFFFFF,
    }
}

/// Set a parameter on a channel's instrument, regardless of what synth type it holds.
///
/// Parameter index meaning depends on the channel's current instrument type.
/// For kick: param 0=frequency, 1=punch, etc. (same as `gooey_engine_set_kick_param`)
/// For snare: param 0=frequency, 1=decay, etc. (same as `gooey_engine_set_snare_param`)
/// For hihat: param 0=pitch, 1=decay, etc. (same as `gooey_engine_set_hihat_param`)
/// For tom: param 0=tune, 1=bend, etc. (same as `gooey_engine_set_tom_param`)
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `channel` - Channel index (0-3)
/// * `param` - Parameter index (meaning depends on instrument type)
/// * `value` - Parameter value (0-1 normalized)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_channel_param(
    engine: *mut GooeyEngine,
    channel: u32,
    param: u32,
    value: f32,
) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    if let Some(voice) = engine.voice_mut(channel as usize) {
        voice.instrument.set_param(param, value);
    }
}

/// Set the tuning offset for a channel (0.0 = −12 semitones, 0.5 = neutral, 1.0 = +12 semitones).
///
/// This is a convenience function that dispatches to the correct tuning parameter
/// for whatever instrument type is currently loaded on the channel.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_channel_tuning(
    engine: *mut GooeyEngine,
    channel: u32,
    value: f32,
) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    let Some(voice) = engine.voice_mut(channel as usize) else {
        return;
    };
    let tuning_param = match voice.instrument.instrument_type() {
        INSTRUMENT_KICK => KICK_PARAM_TUNING,
        INSTRUMENT_SNARE => SNARE_PARAM_TUNING,
        INSTRUMENT_HIHAT => HIHAT_PARAM_TUNING,
        INSTRUMENT_TOM => TOM_PARAM_TUNING,
        INSTRUMENT_BASS => BASS_PARAM_TUNING,
        _ => return,
    };
    voice.instrument.set_param(tuning_param, value);
}

/// Get the current tuning value for a channel (0.0–1.0).
///
/// Returns 0.5 (neutral) if the channel index is out of range or engine is null.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_channel_tuning(
    engine: *const GooeyEngine,
    channel: u32,
) -> f32 {
    if engine.is_null() {
        return 0.5;
    }
    let engine = &*engine;
    match engine.voice(channel as usize) {
        Some(voice) => voice.instrument.get_tuning(),
        None => 0.5,
    }
}

/// Trigger a specific channel (regardless of what instrument type it holds).
///
/// The trigger will be processed on the next call to `gooey_engine_render`.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `channel` - Channel index (0-3)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_trigger_channel(engine: *mut GooeyEngine, channel: u32) {
    gooey_engine_trigger_channel_with_velocity(engine, channel, 1.0);
}

/// Trigger a specific channel with velocity.
///
/// The trigger will be processed on the next call to `gooey_engine_render`.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `channel` - Channel index (0-3)
/// * `velocity` - Velocity from 0.0 (softest) to 1.0 (hardest)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_trigger_channel_with_velocity(
    engine: *mut GooeyEngine,
    channel: u32,
    velocity: f32,
) {
    if let Some(engine) = engine.as_ref() {
        if let Some(voice) = engine.voice(channel as usize) {
            let vel_clamped = velocity.clamp(0.0, 1.0);
            voice
                .trigger_velocity
                .store(vel_clamped.to_bits(), Ordering::Release);
            voice.trigger_pending.store(true, Ordering::Release);
        }
    }
}

// =============================================================================
// Instrument triggering
// =============================================================================

/// Trigger any instrument manually by ID with velocity
///
/// Use this for manual triggering with velocity sensitivity (e.g., velocity-sensitive pads, MIDI input).
/// The trigger will be processed on the next call to `gooey_engine_render`.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
/// * `velocity` - Velocity from 0.0 (softest) to 1.0 (hardest)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_trigger_instrument_with_velocity(
    engine: *mut GooeyEngine,
    instrument: u32,
    velocity: f32,
) {
    if let Some(engine) = engine.as_ref() {
        if let Some(voice) = engine.voice(instrument as usize) {
            let vel_clamped = velocity.clamp(0.0, 1.0);
            voice
                .trigger_velocity
                .store(vel_clamped.to_bits(), Ordering::Release);
            voice.trigger_pending.store(true, Ordering::Release);
        }
    }
}

/// Trigger any instrument manually by ID at full velocity
///
/// Use this for manual triggering outside of the sequencer (e.g., user tap).
/// The trigger will be processed on the next call to `gooey_engine_render`.
/// For velocity-sensitive triggering, use `gooey_engine_trigger_instrument_with_velocity`.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_trigger_instrument(
    engine: *mut GooeyEngine,
    instrument: u32,
) {
    gooey_engine_trigger_instrument_with_velocity(engine, instrument, 1.0);
}

// =============================================================================
// Per-channel peak metering
// =============================================================================

/// Read per-channel peak amplitudes since the last call and reset them to zero.
///
/// Each value represents the maximum absolute amplitude seen on that channel
/// since the previous call. Useful for driving UI level meters.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `out_peaks` - Pointer to a float buffer to receive peak values (0.0–1.0+)
/// * `count` - Number of channels to read (clamped to NUM_INSTRUMENTS)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
/// `out_peaks` must point to a buffer of at least `count` floats.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_channel_peaks(
    engine: *mut GooeyEngine,
    out_peaks: *mut f32,
    count: u32,
) {
    if let Some(engine) = engine.as_ref() {
        let n = (count as usize).min(NUM_INSTRUMENTS);
        for (i, voice) in engine.voices_iter().take(n).enumerate() {
            let bits = voice.peak.swap(0.0_f32.to_bits(), Ordering::Relaxed);
            *out_peaks.add(i) = f32::from_bits(bits);
        }
    }
}

/// Trigger the kick drum manually (legacy function, prefer `gooey_engine_trigger_instrument`)
///
/// Use this for manual triggering outside of the sequencer (e.g., user tap).
/// The trigger will be processed on the next call to `gooey_engine_render`.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_trigger_kick(engine: *mut GooeyEngine) {
    if let Some(engine) = engine.as_ref() {
        if let Some(voice) = engine.voice(INSTRUMENT_KICK as usize) {
            voice.trigger_pending.store(true, Ordering::Release);
        }
    }
}

/// Set a kick drum parameter
///
/// All parameters are automatically smoothed to prevent clicks/pops.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `param` - Parameter index (see KICK_PARAM_* constants)
/// * `value` - Parameter value (range depends on parameter)
///
/// # Parameter indices and ranges
/// - 0 (FREQUENCY): 30-80 Hz
/// - 1 (PUNCH): 0-1
/// - 2 (SUB): 0-1
/// - 3 (CLICK): 0-1
/// - 4 (DECAY): 0.01-5.0 seconds
/// - 5 (PITCH_ENVELOPE): 0-1
/// - 6 (VOLUME): 0-1
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_kick_param(
    engine: *mut GooeyEngine,
    param: u32,
    value: f32,
) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    if let Some(instr) = engine.instrument_by_type_mut(INSTRUMENT_KICK) {
        instr.set_param(param, value);
    }
}

/// Read a kick drum parameter in the same normalized form used by
/// `gooey_engine_set_kick_param`.
///
/// Returns the most-recently-set target value (not the in-flight smoothed sample),
/// so set→get round-trips exactly. Use this to capture mutations applied by
/// randomize/mutate flows for snapshot/undo.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `param` - Parameter index (see KICK_PARAM_* constants)
///
/// # Returns
/// The current parameter value, or `f32::NAN` if `engine` is null, the kick
/// channel is missing, or `param` is unrecognized. Note that
/// `KICK_PARAM_PITCH_ENVELOPE` reports the value set via the setter; it only
/// takes audible effect at the next trigger.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_kick_param(
    engine: *const GooeyEngine,
    param: u32,
) -> f32 {
    if engine.is_null() {
        return f32::NAN;
    }
    let engine = &*engine;
    match engine.instrument_by_type(INSTRUMENT_KICK) {
        Some(instr) => instr.get_param(param),
        None => f32::NAN,
    }
}

/// Set a hi-hat parameter
///
/// All parameters are automatically smoothed to prevent clicks/pops.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `param` - Parameter index (see HIHAT_PARAM_* constants)
/// * `value` - Parameter value (range depends on parameter)
///
/// # Parameter indices and ranges
/// - 0 (PITCH): 0-1 normalized
/// - 1 (DECAY): 0-1 normalized
/// - 2 (ATTACK): 0-1 normalized
/// - 3 (TONE): 0-1 normalized
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_hihat_param(
    engine: *mut GooeyEngine,
    param: u32,
    value: f32,
) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    if let Some(instr) = engine.instrument_by_type_mut(INSTRUMENT_HIHAT) {
        instr.set_param(param, value);
    }
}

/// Read a hi-hat parameter in the same normalized form used by
/// `gooey_engine_set_hihat_param`.
///
/// Returns the most-recently-set target value (not the in-flight smoothed sample),
/// so set→get round-trips exactly.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `param` - Parameter index (see HIHAT_PARAM_* constants)
///
/// # Returns
/// The current parameter value, or `f32::NAN` if `engine` is null, the hi-hat
/// channel is missing, or `param` is unrecognized.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_hihat_param(
    engine: *const GooeyEngine,
    param: u32,
) -> f32 {
    if engine.is_null() {
        return f32::NAN;
    }
    let engine = &*engine;
    match engine.instrument_by_type(INSTRUMENT_HIHAT) {
        Some(instr) => instr.get_param(param),
        None => f32::NAN,
    }
}

/// Set a snare drum parameter
///
/// All parameters are automatically smoothed to prevent clicks/pops.
/// All parameters use normalized 0-1 range.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `param` - Parameter index (see SNARE_PARAM_* constants)
/// * `value` - Parameter value (0-1 normalized)
///
/// # Parameter indices and ranges (all 0-1 normalized)
/// - 0 (FREQUENCY): 0-1 → 100-600 Hz
/// - 1 (DECAY): 0-1 → 0.05-3.5 seconds
/// - 2 (BRIGHTNESS): 0-1
/// - 3 (VOLUME): 0-1
/// - 4 (TONAL): 0-1
/// - 5 (NOISE): 0-1
/// - 6 (PITCH_DROP): 0-1
/// - 7 (TONAL_DECAY): 0-1 → 0-3.5s
/// - 8 (NOISE_DECAY): 0-1 → 0-3.5s
/// - 9 (NOISE_TAIL_DECAY): 0-1 → 0-3.5s
/// - 10 (FILTER_CUTOFF): 0-1 → 100-10000 Hz
/// - 11 (FILTER_RESONANCE): 0-1 → 0.5-10.0
/// - 12 (FILTER_TYPE): 0-3 (LP/BP/HP/notch)
/// - 13 (XFADE): 0-1
/// - 14 (PHASE_MOD_AMOUNT): 0-1
/// - 15 (OVERDRIVE): 0-1
/// - 16 (AMP_DECAY): 0-1 → 0-4.0s
/// - 17 (AMP_DECAY_CURVE): 0-1 → 0.1-10.0
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_snare_param(
    engine: *mut GooeyEngine,
    param: u32,
    value: f32,
) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    if let Some(instr) = engine.instrument_by_type_mut(INSTRUMENT_SNARE) {
        instr.set_param(param, value);
    }
}

/// Read a snare drum parameter in the same normalized form used by
/// `gooey_engine_set_snare_param`.
///
/// Returns the most-recently-set target value (not the in-flight smoothed sample),
/// so set→get round-trips exactly. `SNARE_PARAM_FILTER_TYPE` is reported as the
/// raw 0-3 enum value cast to `f32`.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `param` - Parameter index (see SNARE_PARAM_* constants)
///
/// # Returns
/// The current parameter value, or `f32::NAN` if `engine` is null, the snare
/// channel is missing, or `param` is unrecognized.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_snare_param(
    engine: *const GooeyEngine,
    param: u32,
) -> f32 {
    if engine.is_null() {
        return f32::NAN;
    }
    let engine = &*engine;
    match engine.instrument_by_type(INSTRUMENT_SNARE) {
        Some(instr) => instr.get_param(param),
        None => f32::NAN,
    }
}

/// Set a tom drum parameter
///
/// All parameters use normalized 0-1 range. Values are internally scaled
/// to Tom2's 0-100 range.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `param` - Parameter index (see TOM_PARAM_* constants)
/// * `value` - Parameter value (0-1 normalized)
///
/// # Parameter indices and ranges (all 0-1 normalized → 0-100 internal)
/// - 0 (TUNE): 0-1 → 0-100 (maps to 40-600 Hz)
/// - 1 (BEND): 0-1 → 0-100 (pitch envelope depth)
/// - 2 (TONE): 0-1 → 0-100 (mix control)
/// - 3 (COLOR): 0-1 → 0-100 (noise rate / filter cutoff)
/// - 4 (DECAY): 0-1 → 0-100 (maps to 0.5-4000ms)
/// - 5 (MEMBRANE): 0-1 → 0-100 (resonator mix)
/// - 6 (MEMBRANE_Q): 0-1 → 0-100 (resonator Q scale)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_tom_param(
    engine: *mut GooeyEngine,
    param: u32,
    value: f32,
) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    if let Some(instr) = engine.instrument_by_type_mut(INSTRUMENT_TOM) {
        instr.set_param(param, value);
    }
}

/// Read a tom drum parameter in the same normalized form used by
/// `gooey_engine_set_tom_param`.
///
/// Tom2 stores parameters 0-7 internally on a 0-100 scale; the getter
/// renormalizes back to 0-1 to match the setter contract. `TOM_PARAM_TUNING`
/// (param 8) is the exception: it's already 0-1 in both directions.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `param` - Parameter index (see TOM_PARAM_* constants)
///
/// # Returns
/// The current parameter value, or `f32::NAN` if `engine` is null, the tom
/// channel is missing, or `param` is unrecognized.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_tom_param(engine: *const GooeyEngine, param: u32) -> f32 {
    if engine.is_null() {
        return f32::NAN;
    }
    let engine = &*engine;
    match engine.instrument_by_type(INSTRUMENT_TOM) {
        Some(instr) => instr.get_param(param),
        None => f32::NAN,
    }
}

/// Set a bass synth parameter
///
/// All parameters use normalized 0-1 range. Values are internally scaled.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `param` - Parameter index (see BASS_PARAM_* constants)
/// * `value` - Parameter value (0.0-1.0 normalized)
///
/// # Parameter indices and ranges
/// - 0 (FREQUENCY): 0-1 → 30-200 Hz
/// - 1 (SUB_LEVEL): 0-1
/// - 2 (OSC_LEVEL): 0-1
/// - 3 (DETUNE_LEVEL): 0-1
/// - 4 (DETUNE_AMOUNT): 0-1 → 0-30 cents
/// - 5 (OSC_SHAPE): 0-1 (saw to square)
/// - 6 (FILTER_CUTOFF): 0-1 → 20-18000 Hz exp
/// - 7 (FILTER_RESONANCE): 0-1 → 0.5-15.0 Q
/// - 8 (FILTER_ENV_AMOUNT): 0-1
/// - 9 (FILTER_ENV_DECAY): 0-1 → 0.01-2.0s
/// - 10 (FILTER_ENV_CURVE): 0-1 → 0.1-8.0
/// - 11 (AMP_DECAY): 0-1 → 0.05-4.0s
/// - 12 (AMP_DECAY_CURVE): 0-1 → 0.1-10.0
/// - 13 (OVERDRIVE): 0-1
/// - 14 (VOLUME): 0-1
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_bass_param(
    engine: *mut GooeyEngine,
    param: u32,
    value: f32,
) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    if let Some(instr) = engine.instrument_by_type_mut(INSTRUMENT_BASS) {
        instr.set_param(param, value);
    }
}

/// Load a bass preset, setting all bass parameters to the preset's values.
///
/// This directly applies the preset's parameter values to the bass instrument
/// without requiring the blend system.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `preset_id` - Preset ID (BASS_PRESET_ACID, BASS_PRESET_SUB, etc.)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_load_bass_preset(engine: *mut GooeyEngine, preset_id: u32) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    if let Some(config) = GooeyEngine::bass_preset_by_id(preset_id) {
        if let Some(ChannelInstrument::Bass(bass)) = engine.instrument_by_type_mut(INSTRUMENT_BASS)
        {
            bass.set_config(config);
        }
    }
}

// =============================================================================
// Global effects control
// =============================================================================

/// Set a parameter on a global effect
///
/// This provides a generic interface for controlling any global effect's parameters.
/// Use effect ID constants (EFFECT_LOWPASS_FILTER, etc.) and corresponding parameter
/// constants (FILTER_PARAM_CUTOFF, FILTER_PARAM_RESONANCE, etc.).
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `effect` - Effect ID (see EFFECT_* constants)
/// * `param` - Parameter ID (depends on effect type, see *_PARAM_* constants)
/// * `value` - Parameter value (range depends on parameter)
///
/// # Effect and Parameter Reference
/// - EFFECT_LOWPASS_FILTER (0):
///   - FILTER_PARAM_CUTOFF (0): 20-20000 Hz
///   - FILTER_PARAM_RESONANCE (1): 0.0-0.95
/// - EFFECT_DELAY (1):
///   - DELAY_PARAM_TIMING (0): DELAY_TIMING_* constant (0-8)
///   - DELAY_PARAM_FEEDBACK (1): 0.0-0.95
///   - DELAY_PARAM_MIX (2): 0.0-1.0
///   - DELAY_PARAM_FILTER_CUTOFF (3): 20-20000 Hz
///   - DELAY_PARAM_PINGPONG (4): 0.0 = off, >= 0.5 = on
/// - EFFECT_SATURATION (2):
///   - SATURATION_PARAM_DRIVE (0): 0.0-1.0
///   - SATURATION_PARAM_WARMTH (1): 0.0-1.0
///   - SATURATION_PARAM_MIX (2): 0.0-1.0
/// - EFFECT_COMPRESSOR (3):
///   - COMPRESSOR_PARAM_THRESHOLD (0): -60.0 to 0.0 dB
///   - COMPRESSOR_PARAM_RATIO (1): 1.0-20.0
///   - COMPRESSOR_PARAM_ATTACK (2): 0.1-100.0 ms
///   - COMPRESSOR_PARAM_RELEASE (3): 5.0-1000.0 ms
///   - COMPRESSOR_PARAM_MIX (4): 0.0-1.0
/// - EFFECT_LIMITER (5):
///   - LIMITER_PARAM_THRESHOLD (0): 0.001-1.0
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_global_effect_param(
    engine: *mut GooeyEngine,
    effect: u32,
    param: u32,
    value: f32,
) {
    if engine.is_null() {
        return;
    }

    let engine = &mut *engine;

    match effect {
        EFFECT_LOWPASS_FILTER => match param {
            FILTER_PARAM_CUTOFF => engine.lowpass_filter.set_cutoff_freq(value),
            FILTER_PARAM_RESONANCE => engine.lowpass_filter.set_resonance(value),
            _ => {} // Unknown parameter, ignore
        },
        EFFECT_DELAY => match param {
            DELAY_PARAM_TIMING => {
                if let Some(timing) = DelayTiming::from_timing_constant(value as u32) {
                    engine.delay.set_timing(timing);
                }
            }
            DELAY_PARAM_FEEDBACK => engine.delay.set_feedback(value),
            DELAY_PARAM_MIX => engine.delay.set_mix(value),
            DELAY_PARAM_FILTER_CUTOFF => engine.delay.set_filter_cutoff(value),
            DELAY_PARAM_PINGPONG => engine.delay.set_pingpong(value >= 0.5),
            _ => {} // Unknown parameter, ignore
        },
        EFFECT_SATURATION => match param {
            SATURATION_PARAM_DRIVE => engine.saturation.set_drive(value),
            SATURATION_PARAM_WARMTH => engine.saturation.set_warmth(value),
            SATURATION_PARAM_MIX => engine.saturation.set_mix(value),
            _ => {} // Unknown parameter, ignore
        },
        EFFECT_COMPRESSOR => match param {
            COMPRESSOR_PARAM_THRESHOLD => engine.compressor.set_threshold(value),
            COMPRESSOR_PARAM_RATIO => engine.compressor.set_ratio(value),
            COMPRESSOR_PARAM_ATTACK => engine.compressor.set_attack(value),
            COMPRESSOR_PARAM_RELEASE => engine.compressor.set_release(value),
            COMPRESSOR_PARAM_MIX => engine.compressor.set_mix(value),
            _ => {} // Unknown parameter, ignore
        },
        EFFECT_TILT_FILTER => match param {
            TILT_PARAM_CUTOFF => engine.tilt_filter.set_cutoff(value),
            TILT_PARAM_RESONANCE => engine.tilt_filter.set_resonance(value),
            _ => {} // Unknown parameter, ignore
        },
        EFFECT_WAVESHAPER => match param {
            WAVESHAPER_PARAM_DRIVE => engine.waveshaper.set_drive(value),
            WAVESHAPER_PARAM_MIX => engine.waveshaper.set_mix(value),
            _ => {}
        },
        EFFECT_FEEDBACK_WAVESHAPER => match param {
            FEEDBACK_WAVESHAPER_PARAM_DRIVE => engine.feedback_waveshaper.set_drive(value),
            FEEDBACK_WAVESHAPER_PARAM_FEEDBACK => engine.feedback_waveshaper.set_feedback(value),
            FEEDBACK_WAVESHAPER_PARAM_FILTER_CUTOFF => {
                engine.feedback_waveshaper.set_filter_cutoff(value)
            }
            FEEDBACK_WAVESHAPER_PARAM_MIX => engine.feedback_waveshaper.set_mix(value),
            _ => {}
        },
        EFFECT_REVERB => match param {
            REVERB_PARAM_DECAY => engine.reverb.set_decay(value),
            REVERB_PARAM_MIX => engine.reverb.set_mix(value),
            REVERB_PARAM_DAMPING => engine.reverb.set_damping(value),
            _ => {} // Unknown parameter, ignore
        },
        EFFECT_PLATE_REVERB => match param {
            PLATE_PARAM_DECAY => engine.plate_reverb.set_decay(value),
            PLATE_PARAM_MIX => engine.plate_reverb.set_mix(value),
            PLATE_PARAM_DAMPING => engine.plate_reverb.set_damping(value),
            PLATE_PARAM_PREDELAY => engine.plate_reverb.set_predelay(value),
            PLATE_PARAM_WIDTH => engine.plate_reverb.set_width(value),
            PLATE_PARAM_SIZE => engine.plate_reverb.set_size(value),
            _ => {} // Unknown parameter, ignore
        },
        EFFECT_LIMITER => match param {
            LIMITER_PARAM_THRESHOLD => engine.limiter.set_threshold(value),
            _ => {} // Unknown parameter, ignore
        },
        _ => {} // Unknown effect, ignore
    }
}

/// Get a parameter value from a global effect
///
/// This provides a generic interface for reading any global effect's parameters.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `effect` - Effect ID (see EFFECT_* constants)
/// * `param` - Parameter ID (depends on effect type)
///
/// # Returns
/// The current parameter value, or -1.0 if the effect or parameter is invalid
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_global_effect_param(
    engine: *mut GooeyEngine,
    effect: u32,
    param: u32,
) -> f32 {
    if engine.is_null() {
        return -1.0;
    }

    let engine = &*engine;

    match effect {
        EFFECT_LOWPASS_FILTER => match param {
            FILTER_PARAM_CUTOFF => engine.lowpass_filter.get_cutoff_freq(),
            FILTER_PARAM_RESONANCE => engine.lowpass_filter.get_resonance(),
            _ => -1.0, // Unknown parameter
        },
        EFFECT_DELAY => match param {
            DELAY_PARAM_TIMING => engine.delay.get_timing() as f32,
            DELAY_PARAM_FEEDBACK => engine.delay.get_feedback(),
            DELAY_PARAM_MIX => engine.delay.get_mix(),
            DELAY_PARAM_FILTER_CUTOFF => engine.delay.get_filter_cutoff(),
            DELAY_PARAM_PINGPONG => {
                if engine.delay.get_pingpong() {
                    1.0
                } else {
                    0.0
                }
            }
            _ => -1.0, // Unknown parameter
        },
        EFFECT_SATURATION => match param {
            SATURATION_PARAM_DRIVE => engine.saturation.get_drive(),
            SATURATION_PARAM_WARMTH => engine.saturation.get_warmth(),
            SATURATION_PARAM_MIX => engine.saturation.get_mix(),
            _ => -1.0, // Unknown parameter
        },
        EFFECT_COMPRESSOR => match param {
            COMPRESSOR_PARAM_THRESHOLD => engine.compressor.get_threshold(),
            COMPRESSOR_PARAM_RATIO => engine.compressor.get_ratio(),
            COMPRESSOR_PARAM_ATTACK => engine.compressor.get_attack(),
            COMPRESSOR_PARAM_RELEASE => engine.compressor.get_release(),
            COMPRESSOR_PARAM_MIX => engine.compressor.get_mix(),
            _ => -1.0, // Unknown parameter
        },
        EFFECT_TILT_FILTER => match param {
            TILT_PARAM_CUTOFF => engine.tilt_filter.get_cutoff(),
            TILT_PARAM_RESONANCE => engine.tilt_filter.get_resonance(),
            _ => -1.0, // Unknown parameter
        },
        EFFECT_REVERB => match param {
            REVERB_PARAM_DECAY => engine.reverb.get_decay(),
            REVERB_PARAM_MIX => engine.reverb.get_mix(),
            REVERB_PARAM_DAMPING => engine.reverb.get_damping(),
            _ => -1.0, // Unknown parameter
        },
        EFFECT_PLATE_REVERB => match param {
            PLATE_PARAM_DECAY => engine.plate_reverb.get_decay(),
            PLATE_PARAM_MIX => engine.plate_reverb.get_mix(),
            PLATE_PARAM_DAMPING => engine.plate_reverb.get_damping(),
            PLATE_PARAM_PREDELAY => engine.plate_reverb.get_predelay(),
            PLATE_PARAM_WIDTH => engine.plate_reverb.get_width(),
            PLATE_PARAM_SIZE => engine.plate_reverb.get_size(),
            _ => -1.0, // Unknown parameter
        },
        EFFECT_LIMITER => match param {
            LIMITER_PARAM_THRESHOLD => engine.limiter.get_threshold(),
            _ => -1.0, // Unknown parameter
        },
        _ => -1.0, // Unknown effect
    }
}

/// Enable or disable a global effect
///
/// When disabled, the effect is bypassed and does not process audio.
/// This is useful for A/B comparison or saving CPU when an effect is not needed.
/// Saturation, compressor, and limiter are disabled by default.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `effect` - Effect ID (see EFFECT_* constants)
/// * `enabled` - Whether the effect should be active
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_global_effect_enabled(
    engine: *mut GooeyEngine,
    effect: u32,
    enabled: bool,
) {
    if engine.is_null() {
        return;
    }

    let engine = &mut *engine;

    match effect {
        EFFECT_LOWPASS_FILTER => engine.lowpass_filter_enabled = enabled,
        EFFECT_DELAY => engine.delay_enabled = enabled,
        EFFECT_SATURATION => engine.saturation_enabled = enabled,
        EFFECT_COMPRESSOR => engine.compressor_enabled = enabled,
        EFFECT_TILT_FILTER => engine.tilt_filter_enabled = enabled,
        EFFECT_LIMITER => engine.limiter_enabled = enabled,
        EFFECT_REVERB => engine.reverb_enabled = enabled,
        EFFECT_PLATE_REVERB => engine.plate_reverb_enabled = enabled,
        EFFECT_WAVESHAPER => engine.waveshaper_enabled = enabled,
        EFFECT_FEEDBACK_WAVESHAPER => engine.feedback_waveshaper_enabled = enabled,
        _ => {} // Unknown effect, ignore
    }
}

/// Check if a global effect is enabled
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `effect` - Effect ID (see EFFECT_* constants)
///
/// # Returns
/// `true` if the effect is enabled, `false` if disabled or if the effect ID is invalid
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_global_effect_enabled(
    engine: *mut GooeyEngine,
    effect: u32,
) -> bool {
    if engine.is_null() {
        return false;
    }

    let engine = &*engine;

    match effect {
        EFFECT_LOWPASS_FILTER => engine.lowpass_filter_enabled,
        EFFECT_DELAY => engine.delay_enabled,
        EFFECT_SATURATION => engine.saturation_enabled,
        EFFECT_COMPRESSOR => engine.compressor_enabled,
        EFFECT_TILT_FILTER => engine.tilt_filter_enabled,
        EFFECT_LIMITER => engine.limiter_enabled,
        EFFECT_REVERB => engine.reverb_enabled,
        EFFECT_PLATE_REVERB => engine.plate_reverb_enabled,
        EFFECT_WAVESHAPER => engine.waveshaper_enabled,
        EFFECT_FEEDBACK_WAVESHAPER => engine.feedback_waveshaper_enabled,
        _ => false, // Unknown effect
    }
}

// =============================================================================
// Compressor sidechain control
// =============================================================================

/// Set the compressor sidechain source instrument
///
/// When set to an instrument (INSTRUMENT_KICK, INSTRUMENT_SNARE, INSTRUMENT_HIHAT,
/// INSTRUMENT_TOM), the compressor's envelope follower tracks that instrument's output
/// while gain reduction is applied to the full mix. This enables techniques like
/// kick-driven ducking.
///
/// Use COMPRESSOR_SIDECHAIN_NONE (0xFFFFFFFF) to use the main mix as input (default).
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_compressor_sidechain(
    engine: *mut GooeyEngine,
    instrument: u32,
) {
    if engine.is_null() {
        return;
    }

    let engine = &mut *engine;
    engine.compressor_sidechain = instrument;
}

/// Get the current compressor sidechain source
///
/// # Returns
/// An INSTRUMENT_* constant, or COMPRESSOR_SIDECHAIN_NONE if using the main mix
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_compressor_sidechain(engine: *mut GooeyEngine) -> u32 {
    if engine.is_null() {
        return COMPRESSOR_SIDECHAIN_NONE;
    }

    let engine = &*engine;
    engine.compressor_sidechain
}

// =============================================================================
// Master gain
// =============================================================================

/// Set the master output gain.
///
/// This gain is applied to the complete instrument sum before global effects.
/// It is smoothed over 30ms to prevent clicks.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `gain` - Linear gain, clamped to 0.0-2.0. The default is 0.25.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_master_gain(engine: *mut GooeyEngine, gain: f32) {
    if engine.is_null() || !gain.is_finite() {
        return;
    }
    (*engine).master_gain.set_target(gain);
}

/// Get the current master output gain target.
///
/// # Returns
/// The current linear gain target, or 0.25 if `engine` is null.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_master_gain(engine: *const GooeyEngine) -> f32 {
    if engine.is_null() {
        return DEFAULT_MASTER_GAIN;
    }
    (*engine).master_gain.target()
}

// =============================================================================
// BPM control
// =============================================================================

/// Set the global BPM (beats per minute)
///
/// This affects all sequencer timing (kick, snare, hihat, tom).
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `bpm` - Beats per minute (typically 60-200)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_bpm(engine: *mut GooeyEngine, bpm: f32) {
    if engine.is_null() {
        return;
    }

    let engine = &mut *engine;
    engine.bpm = bpm;
    for seq in engine.sequencers_iter_mut() {
        seq.set_bpm(bpm);
    }

    // Update delay BPM for clocked timing
    engine.delay.set_bpm(bpm);

    // Update LFO BPM values for BPM-synced LFOs
    for lfo in &mut engine.lfos {
        lfo.set_bpm(bpm);
    }

    // Seed BPM for any future note-synced per-channel loop effects.
    engine.mixer.set_bpm(bpm);

    // Propagate BPM to note-synced effects in per-track racks.
    engine.graph.set_bpm(bpm);
}

/// Get the current BPM.
///
/// # Returns
/// The current beats per minute, or 120.0 if `engine` is null.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_bpm(engine: *const GooeyEngine) -> f32 {
    if engine.is_null() {
        return 120.0;
    }
    let engine = &*engine;
    engine.bpm
}

/// Mark the engine as being driven by an external tempo source (e.g. Ableton Link).
///
/// When enabled, the host should route local BPM changes through the external
/// sync source instead of calling `gooey_engine_set_bpm` directly, to avoid
/// feedback loops.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_link_enabled(engine: *mut GooeyEngine, enabled: bool) {
    if engine.is_null() {
        return;
    }
    let engine = &*engine;
    engine.link_enabled.store(enabled, Ordering::Release);
}

/// Check whether the engine is being driven by an external tempo source.
///
/// # Returns
/// `true` if an external sync source (e.g. Ableton Link) is active.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_is_link_enabled(engine: *const GooeyEngine) -> bool {
    if engine.is_null() {
        return false;
    }
    let engine = &*engine;
    engine.link_enabled.load(Ordering::Acquire)
}

/// Get the current sequencer beat position in quarter notes.
///
/// Uses the kick sequencer as reference (all sequencers are synchronized).
/// Returns the fractional beat position, e.g. 0.0 = bar start, 1.25 = one
/// beat and one 16th note in.  The fraction within the current step is
/// derived from the actual swing-aware step boundaries, so the value
/// progresses monotonically even when swing shifts step durations.
///
/// This is useful for drift detection when syncing to an external clock
/// (e.g. Ableton Link): compare the returned value with the external
/// beat position and call `gooey_engine_sequencer_set_beat_position` if
/// the drift exceeds a threshold.
///
/// # Returns
/// Current position in quarter notes, or 0.0 if the engine pointer is null.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_get_beat_position(
    engine: *const GooeyEngine,
) -> f64 {
    if engine.is_null() {
        return 0.0;
    }
    let engine = &*engine;
    let Some(seq) = engine.reference_sequencer() else {
        return 0.0;
    };

    let step = seq.current_step() as f64;

    // Interpolate within the current step using swing-aware boundaries.
    // step_start_sample and next_trigger_sample define the true duration
    // of this step (which differs from samples_per_step when swing != 0.5).
    let step_start = seq.step_start_sample();
    let step_end = seq.next_trigger_sample();
    let step_duration = step_end.saturating_sub(step_start);
    let frac = if step_duration > 0 {
        let elapsed = seq.sample_count().saturating_sub(step_start);
        (elapsed as f64 / step_duration as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };

    // Each step is a 16th note; 4 steps = 1 quarter note
    (step + frac) / 4.0
}

/// Set the global swing amount for all sequencers (0.0-1.0, where 0.5 = no swing)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_swing(engine: *mut GooeyEngine, swing: f32) {
    if engine.is_null() {
        return;
    }

    let engine = &mut *engine;
    let clamped = swing.clamp(0.0, 1.0);
    engine.swing = clamped;
    for seq in engine.sequencers_iter_mut() {
        seq.set_swing(clamped);
    }
}

/// Get the current global swing amount
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_swing(engine: *mut GooeyEngine) -> f32 {
    if engine.is_null() {
        return 0.5;
    }

    let engine = &*engine;
    engine.swing
}

// =============================================================================
// Sequencer control (all instruments)
// =============================================================================

/// Start all sequencers.
///
/// Cancels any pending `gooey_engine_sequencer_start_at_host_time` arm.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_start(engine: *mut GooeyEngine) {
    if engine.is_null() {
        return;
    }

    let engine = &mut *engine;
    engine.pending_arm_host_time = None;
    for seq in engine.sequencers_iter_mut() {
        seq.start();
    }
}

/// Stop all sequencers.
///
/// Cancels any pending `gooey_engine_sequencer_start_at_host_time` arm.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_stop(engine: *mut GooeyEngine) {
    if engine.is_null() {
        return;
    }

    let engine = &mut *engine;
    engine.pending_arm_host_time = None;
    for seq in engine.sequencers_iter_mut() {
        seq.stop();
    }
}

/// Reset all sequencers to step 0.
///
/// Cancels any pending `gooey_engine_sequencer_start_at_host_time` arm.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_reset(engine: *mut GooeyEngine) {
    if engine.is_null() {
        return;
    }

    let engine = &mut *engine;
    engine.pending_arm_host_time = None;
    for seq in engine.sequencers_iter_mut() {
        seq.reset();
    }
}

/// Set all sequencers to a specific beat position in quarter notes.
///
/// Silently teleports the cursor — no step is fired by this call, including
/// the step that the new position lands on. The step at the target position
/// fires when the cursor next crosses its boundary on the regular tick path.
/// Suitable for external phase locking (e.g. Ableton Link drift correction)
/// and for AUv3 host transport sync.
///
/// Each step is a 16th note (4 steps per quarter-note beat).
///
/// Cancels any pending `gooey_engine_sequencer_start_at_host_time` arm. Call
/// this before `sequencer_start()` when the host resumes transport.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `beat_position` - Position in quarter notes (e.g. 0.0 = bar start, 1.0 = second beat)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_set_beat_position(
    engine: *mut GooeyEngine,
    beat_position: f64,
) {
    if engine.is_null() {
        return;
    }

    let engine = &mut *engine;
    engine.pending_arm_host_time = None;
    for seq in engine.sequencers_iter_mut() {
        seq.set_beat_position(beat_position);
    }
}

/// Tell the engine the host time corresponding to sample 0 of the next
/// render call, plus the host-tick-to-sample conversion factor. The engine
/// uses this to evaluate any pending
/// `gooey_engine_sequencer_start_at_host_time` arm.
///
/// Call this once per buffer from the audio callback, immediately before
/// `gooey_engine_render`. Only required while a host-time arm is pending;
/// calling it otherwise is a cheap no-op that simply updates the host-clock
/// reference.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `host_time_first_sample` - mach_absolute_time of the first sample to be
///   rendered.
/// * `host_ticks_per_sample` - Host-clock ticks per audio sample
///   (e.g. host_ticks_per_second / sample_rate). Must be > 0 to take effect.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_render_host_time(
    engine: *mut GooeyEngine,
    host_time_first_sample: u64,
    host_ticks_per_sample: f64,
) {
    if engine.is_null() || !host_ticks_per_sample.is_finite() || host_ticks_per_sample <= 0.0 {
        return;
    }
    let engine = &mut *engine;
    engine.host_clock_anchor = Some(HostClockAnchor {
        host_time_first_sample,
        host_ticks_per_sample,
    });
}

/// Arm all sequencers to start playback at `start_host_time` (in
/// mach_absolute_time units) with the cursor at `beat_position` at that
/// instant. Until the host clock reaches `start_host_time`, render produces
/// silence and emits no step events. From `start_host_time` onward the
/// cursor advances normally starting at `beat_position`.
///
/// Requires the audio callback to call `gooey_engine_set_render_host_time`
/// before each `gooey_engine_render` while armed. Without a host clock
/// reference the arm fires immediately (fail-safe behaviour).
///
/// Safe to call from the main thread; takes effect on the next render call.
/// If `start_host_time` has already been crossed by the time render reaches
/// it, behaves like an immediate start at `beat_position`. Subsequent calls
/// to `sequencer_set_beat_position`, `sequencer_start`, `sequencer_stop`, or
/// `sequencer_reset` cancel the pending arm.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `start_host_time` - Absolute host time at which the sequencer should
///   start (mach_absolute_time units).
/// * `beat_position` - Cursor position in quarter notes at the moment the
///   arm fires (e.g. 0.0 = bar start).
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_start_at_host_time(
    engine: *mut GooeyEngine,
    start_host_time: u64,
    beat_position: f64,
) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    // Stage the arm; it is resolved against the host clock at render time.
    // Stop the underlying sequencers so they emit nothing until the arm fires.
    engine.pending_arm_host_time = Some(PendingArm {
        start_host_time,
        beat_position,
    });
    for seq in engine.sequencers_iter_mut() {
        seq.cancel_arm();
        seq.stop();
    }
}

/// Set a sequencer step on or off for the kick drum (legacy, prefer per-instrument functions)
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `step` - Step index (0-15 for a 16-step sequencer)
/// * `enabled` - Whether the step should trigger
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_set_step(
    engine: *mut GooeyEngine,
    step: u32,
    enabled: bool,
) {
    if engine.is_null() {
        return;
    }

    let engine = &mut *engine;
    if let Some(seq) = engine.sequencer_for_instrument(INSTRUMENT_KICK) {
        seq.set_step(step as usize, enabled);
    }
}

/// Get the current sequencer step (uses kick sequencer, all are synchronized)
///
/// # Returns
/// The current step index (0-15), or -1 if the sequencer is not running
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_get_current_step(engine: *mut GooeyEngine) -> i32 {
    if engine.is_null() {
        return -1;
    }

    let engine = &*engine;
    match engine.reference_sequencer() {
        Some(seq) if seq.is_running() => seq.current_step() as i32,
        _ => -1,
    }
}

/// Get the sequencer step that will be playing after a lookahead period
///
/// This compensates for audio buffer latency by looking ahead.
/// Use this for UI display to sync visuals with audio output.
/// Uses first sequencer as reference (all sequencers are synchronized).
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `lookahead_samples` - Number of samples to look ahead (typically audio buffer size)
///
/// # Returns
/// The step index that will be playing after the lookahead, or -1 if not running
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_get_step_with_lookahead(
    engine: *mut GooeyEngine,
    lookahead_samples: u32,
) -> i32 {
    if engine.is_null() {
        return -1;
    }

    let engine = &*engine;
    match engine.reference_sequencer() {
        Some(seq) if seq.is_running() => seq.step_at_lookahead(lookahead_samples as u64) as i32,
        _ => -1,
    }
}

// =============================================================================
// Per-instrument sequencer control
// =============================================================================

/// Helper to get a mutable reference to an instrument's sequencer
impl GooeyEngine {
    fn sequencer_for_instrument(&mut self, instrument: u32) -> Option<&mut Sequencer> {
        self.voice_mut(instrument as usize)
            .map(|v| &mut v.sequencer)
    }

    fn sequencer_for_instrument_ref(&self, instrument: u32) -> Option<&Sequencer> {
        self.voice(instrument as usize).map(|v| &v.sequencer)
    }

    /// Iterate all voice sequencers in index order (kit drums then bass).
    fn sequencers_iter_mut(&mut self) -> impl Iterator<Item = &mut Sequencer> {
        self.voices_iter_mut().map(|v| &mut v.sequencer)
    }

    /// Borrow the reference sequencer (voice 0 / kick). All sequencers are kept
    /// sample-synchronized, so any voice can serve as the position reference.
    fn reference_sequencer(&self) -> Option<&Sequencer> {
        self.voice(0).map(|v| &v.sequencer)
    }
}

/// Set a sequencer step on or off for a specific instrument
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
/// * `step` - Step index (0-15 for a 16-step sequencer)
/// * `enabled` - Whether the step should trigger
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_set_instrument_step(
    engine: *mut GooeyEngine,
    instrument: u32,
    step: u32,
    enabled: bool,
) {
    if engine.is_null() {
        return;
    }

    let engine = &mut *engine;
    if let Some(sequencer) = engine.sequencer_for_instrument(instrument) {
        sequencer.set_step(step as usize, enabled);
    }
}

/// Set the velocity for a specific step in an instrument's sequencer
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
/// * `step` - Step index (0-15 for a 16-step sequencer)
/// * `velocity` - Velocity from 0.0 to 1.0
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_set_instrument_step_velocity(
    engine: *mut GooeyEngine,
    instrument: u32,
    step: u32,
    velocity: f32,
) {
    if engine.is_null() {
        return;
    }

    let engine = &mut *engine;
    if let Some(sequencer) = engine.sequencer_for_instrument(instrument) {
        sequencer.set_step_velocity(step as usize, velocity);
    }
}

/// Set both enabled state and velocity for a sequencer step
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
/// * `step` - Step index (0-15 for a 16-step sequencer)
/// * `enabled` - Whether the step should trigger
/// * `velocity` - Velocity from 0.0 to 1.0
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_set_instrument_step_with_velocity(
    engine: *mut GooeyEngine,
    instrument: u32,
    step: u32,
    enabled: bool,
    velocity: f32,
) {
    if engine.is_null() {
        return;
    }

    let engine = &mut *engine;
    if let Some(sequencer) = engine.sequencer_for_instrument(instrument) {
        sequencer.set_step_with_velocity(step as usize, enabled, velocity);
    }
}

/// Set a step with optional velocity, optional blend setting, and optional MIDI note.
///
/// Omitted settings are left unchanged.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
/// * `step` - Step index (0-15 for a 16-step sequencer)
/// * `enabled` - Whether the step should trigger
/// * `set_velocity` - Whether to apply the `velocity` value
/// * `velocity` - Velocity from 0.0 to 1.0 (used only when `set_velocity` is true)
/// * `set_blend` - Whether to apply blend X/Y
/// * `blend_x` - Blend X position (0.0-1.0, used only when `set_blend` is true)
/// * `blend_y` - Blend Y position (0.0-1.0, used only when `set_blend` is true)
/// * `set_note` - Whether to apply the `midi_note` value
/// * `midi_note` - MIDI note number (0-127, or STEP_NOTE_NONE); only applied when `set_note` is true
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_set_instrument_step_settings(
    engine: *mut GooeyEngine,
    instrument: u32,
    step: u32,
    enabled: bool,
    set_velocity: bool,
    velocity: f32,
    set_blend: bool,
    blend_x: f32,
    blend_y: f32,
    set_note: bool,
    midi_note: u8,
) {
    if engine.is_null() {
        return;
    }

    let engine = &mut *engine;
    if let Some(sequencer) = engine.sequencer_for_instrument(instrument) {
        let settings = SequencerStepSettings {
            velocity: if set_velocity { Some(velocity) } else { None },
            blend: if set_blend {
                Some(SequencerBlendSetting::new(blend_x, blend_y))
            } else {
                None
            },
            note: None,
        };
        sequencer.set_step_with_settings(step as usize, enabled, settings);
        // Handle note separately: set_note=true with STEP_NOTE_NONE clears the note
        if set_note {
            let step_idx = step as usize;
            if midi_note == STEP_NOTE_NONE {
                sequencer.clear_step_note(step_idx);
            } else {
                sequencer.set_step_note(step_idx, midi_note);
            }
        }
    }
}

/// Set an absolute blend setting for a specific step (0.0-1.0 X/Y)
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
/// * `step` - Step index (0-15 for a 16-step sequencer)
/// * `x` - Blend X position (0.0-1.0)
/// * `y` - Blend Y position (0.0-1.0)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_set_instrument_step_blend(
    engine: *mut GooeyEngine,
    instrument: u32,
    step: u32,
    x: f32,
    y: f32,
) {
    if engine.is_null() {
        return;
    }

    let engine = &mut *engine;
    if let Some(sequencer) = engine.sequencer_for_instrument(instrument) {
        sequencer.set_step_blend(step as usize, x, y);
    }
}

/// Legacy alias for `gooey_engine_sequencer_set_instrument_step_blend`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_set_instrument_step_blend_override(
    engine: *mut GooeyEngine,
    instrument: u32,
    step: u32,
    x: f32,
    y: f32,
) {
    gooey_engine_sequencer_set_instrument_step_blend(engine, instrument, step, x, y);
}

/// Clear the blend setting for a specific step
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
/// * `step` - Step index (0-15 for a 16-step sequencer)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_clear_instrument_step_blend(
    engine: *mut GooeyEngine,
    instrument: u32,
    step: u32,
) {
    if engine.is_null() {
        return;
    }

    let engine = &mut *engine;
    if let Some(sequencer) = engine.sequencer_for_instrument(instrument) {
        sequencer.clear_step_blend(step as usize);
    }
}

/// Legacy alias for `gooey_engine_sequencer_clear_instrument_step_blend`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_clear_instrument_step_blend_override(
    engine: *mut GooeyEngine,
    instrument: u32,
    step: u32,
) {
    gooey_engine_sequencer_clear_instrument_step_blend(engine, instrument, step);
}

/// Set the MIDI note for a specific step in an instrument's sequencer.
///
/// When a step with a note triggers, the engine sets the instrument's
/// frequency parameter to the note's frequency at the exact trigger sample.
/// Steps without a note (STEP_NOTE_NONE) use the instrument's global
/// frequency parameter as before (backward compatible).
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_BASS, etc.)
/// * `step` - Step index (0-15)
/// * `midi_note` - MIDI note number (0-127), or STEP_NOTE_NONE to clear
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_set_instrument_step_note(
    engine: *mut GooeyEngine,
    instrument: u32,
    step: u32,
    midi_note: u8,
) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    if let Some(sequencer) = engine.sequencer_for_instrument(instrument) {
        if midi_note == STEP_NOTE_NONE {
            sequencer.clear_step_note(step as usize);
        } else {
            sequencer.set_step_note(step as usize, midi_note);
        }
    }
}

/// Get the MIDI note for a specific step.
///
/// # Returns
/// The MIDI note number (0-127), or STEP_NOTE_NONE (255) if no note is set.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_get_instrument_step_note(
    engine: *const GooeyEngine,
    instrument: u32,
    step: u32,
) -> u8 {
    if engine.is_null() {
        return STEP_NOTE_NONE;
    }
    let engine = &*engine;
    if let Some(sequencer) = engine.sequencer_for_instrument_ref(instrument) {
        sequencer
            .get_step_note(step as usize)
            .unwrap_or(STEP_NOTE_NONE)
    } else {
        STEP_NOTE_NONE
    }
}

/// Clear the MIDI note for a step (reverts to global frequency).
/// Equivalent to set_step_note(..., STEP_NOTE_NONE).
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_clear_instrument_step_note(
    engine: *mut GooeyEngine,
    instrument: u32,
    step: u32,
) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    if let Some(sequencer) = engine.sequencer_for_instrument(instrument) {
        sequencer.clear_step_note(step as usize);
    }
}

/// Set MIDI notes for all 16 steps of an instrument's sequencer.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID
/// * `notes` - Pointer to 16 uint8_t values (MIDI notes, or STEP_NOTE_NONE)
///
/// # Safety
/// - `engine` must be a valid pointer returned by `gooey_engine_new`
/// - `notes` must point to at least 16 bytes of allocated memory
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_set_instrument_note_pattern(
    engine: *mut GooeyEngine,
    instrument: u32,
    notes: *const u8,
) {
    if engine.is_null() || notes.is_null() {
        return;
    }
    let engine = &mut *engine;
    let notes_slice = slice::from_raw_parts(notes, 16);
    if let Some(sequencer) = engine.sequencer_for_instrument(instrument) {
        sequencer.set_note_pattern(notes_slice);
    }
}

/// Set the entire 16-step pattern for an instrument's sequencer
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
/// * `pattern` - Pointer to 16 bools representing step states
///
/// # Safety
/// - `engine` must be a valid pointer returned by `gooey_engine_new`
/// - `pattern` must point to at least 16 bools of allocated memory
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_set_instrument_pattern(
    engine: *mut GooeyEngine,
    instrument: u32,
    pattern: *const bool,
) {
    if engine.is_null() || pattern.is_null() {
        return;
    }

    let engine = &mut *engine;
    let pattern_slice = slice::from_raw_parts(pattern, 16);
    let pattern_vec: Vec<bool> = pattern_slice.to_vec();

    if let Some(sequencer) = engine.sequencer_for_instrument(instrument) {
        sequencer.set_pattern(pattern_vec);
    }
}

/// Get the current step for an instrument's sequencer
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
///
/// # Returns
/// The current step index (0-15), or -1 if the sequencer is not running or invalid instrument
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_get_instrument_step(
    engine: *mut GooeyEngine,
    instrument: u32,
) -> i32 {
    if engine.is_null() {
        return -1;
    }

    let engine = &*engine;
    if let Some(sequencer) = engine.sequencer_for_instrument_ref(instrument) {
        if sequencer.is_running() {
            return sequencer.current_step() as i32;
        }
    }
    -1
}

/// Get the step that will be playing after a lookahead period for an instrument
///
/// This compensates for audio buffer latency by looking ahead.
/// Use this for UI display to sync visuals with audio output.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
/// * `lookahead_samples` - Number of samples to look ahead (typically audio buffer size)
///
/// # Returns
/// The step index that will be playing after the lookahead, or -1 if not running
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_get_instrument_step_with_lookahead(
    engine: *mut GooeyEngine,
    instrument: u32,
    lookahead_samples: u32,
) -> i32 {
    if engine.is_null() {
        return -1;
    }

    let engine = &*engine;
    if let Some(sequencer) = engine.sequencer_for_instrument_ref(instrument) {
        if sequencer.is_running() {
            return sequencer.step_at_lookahead(lookahead_samples as u64) as i32;
        }
    }
    -1
}

/// Get the velocity for a specific step in an instrument's sequencer
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
/// * `step` - Step index (0-15 for a 16-step sequencer)
///
/// # Returns
/// The velocity (0.0-1.0), or 0.0 if invalid engine/instrument/step
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_get_instrument_step_velocity(
    engine: *mut GooeyEngine,
    instrument: u32,
    step: u32,
) -> f32 {
    if engine.is_null() {
        return 0.0;
    }

    let engine = &*engine;
    if let Some(sequencer) = engine.sequencer_for_instrument_ref(instrument) {
        return sequencer.get_step_velocity(step as usize);
    }
    0.0
}

/// Get the blend X setting for a specific step
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
/// * `step` - Step index (0-15 for a 16-step sequencer)
///
/// # Returns
/// The X position (0.0-1.0), or -1.0 if no blend setting or invalid engine/instrument/step
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_get_instrument_step_blend_x(
    engine: *mut GooeyEngine,
    instrument: u32,
    step: u32,
) -> f32 {
    if engine.is_null() {
        return -1.0;
    }

    let engine = &*engine;
    if let Some(sequencer) = engine.sequencer_for_instrument_ref(instrument) {
        if let Some(blend_setting) = sequencer.get_step_blend(step as usize) {
            return blend_setting.x;
        }
    }
    -1.0
}

/// Legacy alias for `gooey_engine_sequencer_get_instrument_step_blend_x`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_get_instrument_step_blend_override_x(
    engine: *mut GooeyEngine,
    instrument: u32,
    step: u32,
) -> f32 {
    gooey_engine_sequencer_get_instrument_step_blend_x(engine, instrument, step)
}

/// Get the blend Y setting for a specific step
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
/// * `step` - Step index (0-15 for a 16-step sequencer)
///
/// # Returns
/// The Y position (0.0-1.0), or -1.0 if no blend setting or invalid engine/instrument/step
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_get_instrument_step_blend_y(
    engine: *mut GooeyEngine,
    instrument: u32,
    step: u32,
) -> f32 {
    if engine.is_null() {
        return -1.0;
    }

    let engine = &*engine;
    if let Some(sequencer) = engine.sequencer_for_instrument_ref(instrument) {
        if let Some(blend_setting) = sequencer.get_step_blend(step as usize) {
            return blend_setting.y;
        }
    }
    -1.0
}

/// Legacy alias for `gooey_engine_sequencer_get_instrument_step_blend_y`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_get_instrument_step_blend_override_y(
    engine: *mut GooeyEngine,
    instrument: u32,
    step: u32,
) -> f32 {
    gooey_engine_sequencer_get_instrument_step_blend_y(engine, instrument, step)
}

/// Get the enabled state for a specific step in an instrument's sequencer
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
/// * `step` - Step index (0-15 for a 16-step sequencer)
///
/// # Returns
/// Whether the step is enabled (true/false), or false if invalid engine/instrument/step
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_get_instrument_step_enabled(
    engine: *mut GooeyEngine,
    instrument: u32,
    step: u32,
) -> bool {
    if engine.is_null() {
        return false;
    }

    let engine = &*engine;
    if let Some(sequencer) = engine.sequencer_for_instrument_ref(instrument) {
        return sequencer.get_step_enabled(step as usize);
    }
    false
}

// =============================================================================
// Utility functions
// =============================================================================

/// Get the number of kick parameters
#[no_mangle]
pub extern "C" fn gooey_engine_kick_param_count() -> u32 {
    8
}

/// Get the number of hi-hat parameters
#[no_mangle]
pub extern "C" fn gooey_engine_hihat_param_count() -> u32 {
    6
}

/// Get the number of sequencer steps
#[no_mangle]
pub extern "C" fn gooey_engine_sequencer_step_count() -> u32 {
    16
}

/// Get the number of available instruments
#[no_mangle]
pub extern "C" fn gooey_engine_instrument_count() -> u32 {
    INSTRUMENT_COUNT
}

/// Get the number of available global effects
#[no_mangle]
pub extern "C" fn gooey_engine_global_effect_count() -> u32 {
    EFFECT_COUNT
}

// =============================================================================
// Effect chain reordering
// =============================================================================

/// Get the number of reorderable effects in the chain.
///
/// The optional limiter is excluded because it is pinned at the end of the
/// chain when enabled. Only the other effects can be reordered.
#[no_mangle]
pub extern "C" fn gooey_engine_reorderable_effect_count() -> u32 {
    REORDERABLE_EFFECT_COUNT
}

/// Returns true iff `id` is a valid reorderable effect ID
/// (any `EFFECT_*` constant except `EFFECT_LIMITER`).
fn is_reorderable_effect(id: u32) -> bool {
    matches!(
        id,
        EFFECT_WAVESHAPER
            | EFFECT_LOWPASS_FILTER
            | EFFECT_DELAY
            | EFFECT_SATURATION
            | EFFECT_COMPRESSOR
            | EFFECT_TILT_FILTER
            | EFFECT_FEEDBACK_WAVESHAPER
            | EFFECT_REVERB
            | EFFECT_PLATE_REVERB
    )
}

/// Set the full effect-chain order in one call.
///
/// `ids` must point to exactly `REORDERABLE_EFFECT_COUNT` u32 values, each a
/// distinct reorderable effect ID (any of `EFFECT_LOWPASS_FILTER`,
/// `EFFECT_DELAY`, `EFFECT_SATURATION`, `EFFECT_COMPRESSOR`,
/// `EFFECT_TILT_FILTER`, `EFFECT_REVERB`, `EFFECT_PLATE_REVERB`,
/// `EFFECT_WAVESHAPER`, `EFFECT_FEEDBACK_WAVESHAPER`). `EFFECT_LIMITER` is
/// pinned at the end of the chain and must not appear in `ids`.
///
/// On success, the chain order is replaced and the internal state of every
/// reorderable effect is reset (delay buffer, reverb tail, compressor envelope,
/// filter poles) so a stale routing does not bleed into the new one.
///
/// Returns `true` on success. Returns `false` (leaving the chain unchanged) if
/// `len != REORDERABLE_EFFECT_COUNT`, any ID is not reorderable, or any ID
/// appears more than once.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`. `ids` must
/// point to at least `len` u32 values. Caller must ensure the engine is not
/// concurrently mutated from another thread.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_effect_order(
    engine: *mut GooeyEngine,
    ids: *const u32,
    len: u32,
) -> bool {
    if engine.is_null() || ids.is_null() {
        return false;
    }
    if len != REORDERABLE_EFFECT_COUNT {
        return false;
    }

    let slice = std::slice::from_raw_parts(ids, len as usize);
    let mut new_order = [0u32; REORDERABLE_EFFECT_COUNT as usize];
    for (i, &id) in slice.iter().enumerate() {
        if !is_reorderable_effect(id) {
            return false;
        }
        if slice[..i].contains(&id) {
            return false;
        }
        new_order[i] = id;
    }

    let engine = &mut *engine;
    engine.effect_order = new_order;
    engine.reset_effect_states();
    true
}

/// Move a single effect to `new_position` (0-indexed within the reorderable
/// section of the chain). Other effects shift to fill the gap, preserving
/// their relative order.
///
/// On success, internal state of every reorderable effect is reset (see
/// `gooey_engine_set_effect_order` for rationale).
///
/// Returns `false` (leaving the chain unchanged) if `effect_id` is not
/// reorderable (e.g. `EFFECT_LIMITER`) or `new_position >=
/// REORDERABLE_EFFECT_COUNT`.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`. Caller
/// must ensure the engine is not concurrently mutated from another thread.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_move_effect(
    engine: *mut GooeyEngine,
    effect_id: u32,
    new_position: u32,
) -> bool {
    if engine.is_null() {
        return false;
    }
    if !is_reorderable_effect(effect_id) {
        return false;
    }
    if new_position >= REORDERABLE_EFFECT_COUNT {
        return false;
    }

    let engine = &mut *engine;
    let Some(current_pos) = engine.effect_order.iter().position(|&id| id == effect_id) else {
        return false;
    };
    let new_pos = new_position as usize;
    if current_pos == new_pos {
        return true;
    }

    if new_pos > current_pos {
        // Shift left: elements (current_pos+1 ..= new_pos) move down by one.
        for i in current_pos..new_pos {
            engine.effect_order[i] = engine.effect_order[i + 1];
        }
    } else {
        // Shift right: elements (new_pos .. current_pos) move up by one.
        for i in (new_pos..current_pos).rev() {
            engine.effect_order[i + 1] = engine.effect_order[i];
        }
    }
    engine.effect_order[new_pos] = effect_id;
    engine.reset_effect_states();
    true
}

/// Read the current effect-chain order. Writes up to `max_len` IDs into
/// `out_ids` in chain order (position 0 first). The optional limiter is pinned
/// last when enabled and is not written.
///
/// Returns the number of IDs written, which equals `REORDERABLE_EFFECT_COUNT`
/// when `max_len` is large enough, or `max_len` otherwise. Returns `0` if
/// `engine` or `out_ids` is null.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`. `out_ids`
/// must point to a buffer of at least `max_len` u32 values.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_effect_order(
    engine: *const GooeyEngine,
    out_ids: *mut u32,
    max_len: u32,
) -> u32 {
    if engine.is_null() || out_ids.is_null() || max_len == 0 {
        return 0;
    }
    let engine = &*engine;
    let n = (max_len as usize).min(engine.effect_order.len());
    let dst = std::slice::from_raw_parts_mut(out_ids, n);
    dst.copy_from_slice(&engine.effect_order[..n]);
    n as u32
}

// =============================================================================
// LFO control
// =============================================================================

/// Get the number of LFOs in the pool
#[no_mangle]
pub extern "C" fn gooey_engine_lfo_count() -> u32 {
    LFO_COUNT as u32
}

/// Get the number of LFO timing options
#[no_mangle]
pub extern "C" fn gooey_engine_lfo_timing_count() -> u32 {
    8 // FourBars, TwoBars, OneBar, Half, Quarter, Eighth, Sixteenth, ThirtySecond
}

/// Enable or disable an LFO
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `lfo_index` - LFO index (0-7)
/// * `enabled` - Whether the LFO should be active
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_lfo_enabled(
    engine: *mut GooeyEngine,
    lfo_index: u32,
    enabled: bool,
) {
    if engine.is_null() || lfo_index as usize >= LFO_COUNT {
        return;
    }
    let engine = &mut *engine;
    engine.lfo_enabled[lfo_index as usize] = enabled;
}

/// Check if an LFO is enabled
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `lfo_index` - LFO index (0-7)
///
/// # Returns
/// `true` if the LFO is enabled, `false` otherwise
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_lfo_enabled(
    engine: *const GooeyEngine,
    lfo_index: u32,
) -> bool {
    if engine.is_null() || lfo_index as usize >= LFO_COUNT {
        return false;
    }
    let engine = &*engine;
    engine.lfo_enabled[lfo_index as usize]
}

/// Set the timing (musical division) for an LFO
///
/// The LFO will sync to the global BPM using the specified timing.
/// Phase is preserved when changing timing, allowing smooth transitions.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `lfo_index` - LFO index (0-7)
/// * `timing` - LFO timing constant (LFO_TIMING_QUARTER, etc.)
///
/// # Timing constants
/// - LFO_TIMING_FOUR_BARS (0): 4 bars / 16 beats
/// - LFO_TIMING_TWO_BARS (1): 2 bars / 8 beats
/// - LFO_TIMING_ONE_BAR (2): 1 bar / 4 beats
/// - LFO_TIMING_HALF (3): Half note / 2 beats
/// - LFO_TIMING_QUARTER (4): Quarter note / 1 beat
/// - LFO_TIMING_EIGHTH (5): Eighth note / 1/2 beat
/// - LFO_TIMING_SIXTEENTH (6): Sixteenth note / 1/4 beat
/// - LFO_TIMING_THIRTY_SECOND (7): Thirty-second note / 1/8 beat
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_lfo_timing(
    engine: *mut GooeyEngine,
    lfo_index: u32,
    timing: u32,
) {
    if engine.is_null() || lfo_index as usize >= LFO_COUNT {
        return;
    }
    let engine = &mut *engine;

    if let Some(division) = MusicalDivision::from_timing_constant(timing) {
        engine.lfos[lfo_index as usize].set_sync_mode(division);
    }
}

/// Get the current timing for an LFO
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `lfo_index` - LFO index (0-7)
///
/// # Returns
/// The current timing constant, or LFO_INVALID if invalid
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_lfo_timing(
    engine: *const GooeyEngine,
    lfo_index: u32,
) -> u32 {
    if engine.is_null() || lfo_index as usize >= LFO_COUNT {
        return LFO_INVALID;
    }
    let engine = &*engine;

    match engine.lfos[lfo_index as usize].sync_mode() {
        crate::engine::lfo::LfoSyncMode::BpmSync(division) => match division {
            MusicalDivision::FourBars => LFO_TIMING_FOUR_BARS,
            MusicalDivision::TwoBars => LFO_TIMING_TWO_BARS,
            MusicalDivision::OneBar => LFO_TIMING_ONE_BAR,
            MusicalDivision::Half => LFO_TIMING_HALF,
            MusicalDivision::Quarter => LFO_TIMING_QUARTER,
            MusicalDivision::Eighth => LFO_TIMING_EIGHTH,
            MusicalDivision::Sixteenth => LFO_TIMING_SIXTEENTH,
            MusicalDivision::ThirtySecond => LFO_TIMING_THIRTY_SECOND,
        },
        crate::engine::lfo::LfoSyncMode::Hz(_) => LFO_INVALID, // Hz mode, not BPM synced
    }
}

/// Set the global modulation amount for an LFO
///
/// This scales the LFO's sine wave amplitude before it's distributed to routes.
/// Final modulation = (offset + sine * amount) * route_depth
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `lfo_index` - LFO index (0-7)
/// * `amount` - Global amplitude scale (0.0 to 1.0, default 1.0)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_lfo_amount(
    engine: *mut GooeyEngine,
    lfo_index: u32,
    amount: f32,
) {
    if engine.is_null() || lfo_index as usize >= LFO_COUNT {
        return;
    }
    let engine = &mut *engine;
    engine.lfos[lfo_index as usize].amount = amount;
}

/// Get the global modulation amount for an LFO
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `lfo_index` - LFO index (0-7)
///
/// # Returns
/// The current global amplitude scale, or 0.0 if invalid
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_lfo_amount(
    engine: *const GooeyEngine,
    lfo_index: u32,
) -> f32 {
    if engine.is_null() || lfo_index as usize >= LFO_COUNT {
        return 0.0;
    }
    let engine = &*engine;
    engine.lfos[lfo_index as usize].amount
}

/// Set the center offset (DC bias) for an LFO
///
/// This adds a constant value to the LFO output before distribution to routes.
/// Final modulation = (offset + sine * amount) * route_depth
///
/// Use offset to bias the modulation (e.g., offset=0.5 with amount=0.5 gives 0.0-1.0 range)
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `lfo_index` - LFO index (0-7)
/// * `offset` - DC bias (-1.0 to 1.0, default 0.0 for centered bipolar modulation)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_lfo_offset(
    engine: *mut GooeyEngine,
    lfo_index: u32,
    offset: f32,
) {
    if engine.is_null() || lfo_index as usize >= LFO_COUNT {
        return;
    }
    let engine = &mut *engine;
    engine.lfos[lfo_index as usize].offset = offset;
}

/// Get the center offset (DC bias) for an LFO
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `lfo_index` - LFO index (0-7)
///
/// # Returns
/// The current DC bias, or 0.0 if invalid
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_lfo_offset(
    engine: *const GooeyEngine,
    lfo_index: u32,
) -> f32 {
    if engine.is_null() || lfo_index as usize >= LFO_COUNT {
        return 0.0;
    }
    let engine = &*engine;
    engine.lfos[lfo_index as usize].offset
}

/// Add a route from an LFO to an instrument parameter
///
/// Each LFO can have multiple routes to different parameters.
/// Final modulation applied to target = (offset + sine * amount) * depth
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `lfo_index` - LFO index (0-7)
/// * `instrument` - Target instrument (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
/// * `param` - Target parameter index (KICK_PARAM_FREQUENCY, etc.)
/// * `depth` - Per-route depth (0.0 to 1.0) - scales the LFO output for this target
///
/// # Returns
/// A route ID that can be used to remove this specific route, or LFO_INVALID on error
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_add_lfo_route(
    engine: *mut GooeyEngine,
    lfo_index: u32,
    instrument: u32,
    param: u32,
    depth: f32,
) -> u32 {
    if engine.is_null() || lfo_index as usize >= LFO_COUNT {
        return LFO_INVALID;
    }
    let engine = &mut *engine;
    let idx = lfo_index as usize;

    // Check if we've hit the max routes limit
    if engine.lfo_routes[idx].len() >= LFO_MAX_ROUTES {
        return LFO_INVALID;
    }

    let route_id = engine.lfo_next_route_id[idx];
    engine.lfo_next_route_id[idx] = route_id.wrapping_add(1);

    engine.lfo_routes[idx].push(LfoRoute {
        id: route_id,
        instrument,
        param,
        depth,
    });

    route_id
}

/// Remove a specific route from an LFO by route ID
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `lfo_index` - LFO index (0-7)
/// * `route_id` - The route ID returned by `gooey_engine_add_lfo_route`
///
/// # Returns
/// `true` if the route was found and removed, `false` otherwise
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_remove_lfo_route(
    engine: *mut GooeyEngine,
    lfo_index: u32,
    route_id: u32,
) -> bool {
    if engine.is_null() || lfo_index as usize >= LFO_COUNT {
        return false;
    }
    let engine = &mut *engine;
    let idx = lfo_index as usize;

    if let Some(pos) = engine.lfo_routes[idx].iter().position(|r| r.id == route_id) {
        engine.lfo_routes[idx].remove(pos);
        true
    } else {
        false
    }
}

/// Clear all routes for an LFO
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `lfo_index` - LFO index (0-7)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_clear_lfo_routes(engine: *mut GooeyEngine, lfo_index: u32) {
    if engine.is_null() || lfo_index as usize >= LFO_COUNT {
        return;
    }
    let engine = &mut *engine;
    engine.lfo_routes[lfo_index as usize].clear();
}

/// Get the number of routes for an LFO
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `lfo_index` - LFO index (0-7)
///
/// # Returns
/// The number of active routes, or 0 if invalid
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_lfo_route_count(
    engine: *const GooeyEngine,
    lfo_index: u32,
) -> u32 {
    if engine.is_null() || lfo_index as usize >= LFO_COUNT {
        return 0;
    }
    let engine = &*engine;
    engine.lfo_routes[lfo_index as usize].len() as u32
}

/// Reset an LFO's phase to 0
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `lfo_index` - LFO index (0-7)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_reset_lfo_phase(engine: *mut GooeyEngine, lfo_index: u32) {
    if engine.is_null() || lfo_index as usize >= LFO_COUNT {
        return;
    }
    let engine = &mut *engine;
    engine.lfos[lfo_index as usize].reset();
}

/// Get an LFO's current phase
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `lfo_index` - LFO index (0-7)
///
/// # Returns
/// The current phase (0.0 to 1.0), or -1.0 if invalid
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_lfo_phase(
    engine: *const GooeyEngine,
    lfo_index: u32,
) -> f32 {
    if engine.is_null() || lfo_index as usize >= LFO_COUNT {
        return -1.0;
    }
    let engine = &*engine;
    engine.lfos[lfo_index as usize].phase()
}

/// Get the number of snare parameters
#[no_mangle]
pub extern "C" fn gooey_engine_snare_param_count() -> u32 {
    20 // frequency, decay, brightness, volume, tonal, noise, pitch_drop,
       // tonal_decay, noise_decay, noise_tail_decay, filter_cutoff, filter_resonance,
       // filter_type, xfade, phase_mod_amount, overdrive, amp_decay, amp_decay_curve,
       // tonal_decay_curve, tuning
}

/// Get the number of tom parameters
#[no_mangle]
pub extern "C" fn gooey_engine_tom_param_count() -> u32 {
    9 // tune, bend, tone, color, decay, membrane, membrane_q, volume, tuning (Tom2)
}

// =============================================================================
// Instrument mute/solo control
// =============================================================================

/// Set the mute state for an instrument
///
/// When muted, the instrument's audio output is silenced.
/// Solo takes precedence: if an instrument is both muted and soloed, it will play.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
/// * `muted` - Whether the instrument should be muted
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_instrument_mute(
    engine: *mut GooeyEngine,
    instrument: u32,
    muted: bool,
) {
    if engine.is_null() {
        return;
    }
    if let Some(voice) = (*engine).voice(instrument as usize) {
        voice.muted.store(muted, Ordering::Release);
    }
}

/// Get the mute state for an instrument
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
///
/// # Returns
/// `true` if the instrument is muted, `false` otherwise (or if invalid instrument)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_instrument_mute(
    engine: *const GooeyEngine,
    instrument: u32,
) -> bool {
    if engine.is_null() {
        return false;
    }
    (*engine)
        .voice(instrument as usize)
        .is_some_and(|v| v.muted.load(Ordering::Acquire))
}

/// Set the solo state for an instrument
///
/// When any instrument is soloed, only soloed instruments produce audio.
/// Multiple instruments can be soloed simultaneously.
/// Solo takes precedence over mute.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
/// * `soloed` - Whether the instrument should be soloed
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_instrument_solo(
    engine: *mut GooeyEngine,
    instrument: u32,
    soloed: bool,
) {
    if engine.is_null() {
        return;
    }
    if let Some(voice) = (*engine).voice(instrument as usize) {
        voice.soloed.store(soloed, Ordering::Release);
    }
}

/// Get the solo state for an instrument
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
///
/// # Returns
/// `true` if the instrument is soloed, `false` otherwise (or if invalid instrument)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_instrument_solo(
    engine: *const GooeyEngine,
    instrument: u32,
) -> bool {
    if engine.is_null() {
        return false;
    }
    (*engine)
        .voice(instrument as usize)
        .is_some_and(|v| v.soloed.load(Ordering::Acquire))
}

// =============================================================================
// Per-instrument channel gain (mixer fader, independent of blend system)
// =============================================================================

/// Set the channel gain for an instrument (0.0–1.0)
///
/// This gain is applied after synthesis and blending, so it acts as a mixer
/// fader that the blend system cannot override. Smoothed over 10ms to prevent
/// clicks.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
/// * `gain` - Gain level, clamped to 0.0–1.0
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_instrument_gain(
    engine: *mut GooeyEngine,
    instrument: u32,
    gain: f32,
) {
    if engine.is_null() {
        return;
    }
    let gain = gain.clamp(0.0, 1.0);
    if let Some(voice) = (*engine).voice_mut(instrument as usize) {
        voice.channel_gain.set_target(gain);
    }
}

/// Get the channel gain for an instrument
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
///
/// # Returns
/// The current gain target (0.0–1.0), or 1.0 if invalid instrument
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_instrument_gain(
    engine: *const GooeyEngine,
    instrument: u32,
) -> f32 {
    if engine.is_null() {
        return 1.0;
    }
    (*engine)
        .voice(instrument as usize)
        .map_or(1.0, |v| v.channel_gain.target())
}

/// Set the stereo pan for an instrument.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
/// * `pan` - Pan position, clamped to 0.0 (hard left) – 0.5 (center) – 1.0
///   (hard right). Uses an equal-power law.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_set_instrument_pan(
    engine: *mut GooeyEngine,
    instrument: u32,
    pan: f32,
) {
    if engine.is_null() {
        return;
    }
    let pan = pan.clamp(0.0, 1.0);
    if let Some(voice) = (*engine).voice_mut(instrument as usize) {
        voice.pan.set_target(pan);
    }
}

/// Get the stereo pan for an instrument
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, INSTRUMENT_SNARE, etc.)
///
/// # Returns
/// The current pan target (0.0–1.0), or 0.5 (center) if invalid instrument
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_get_instrument_pan(
    engine: *const GooeyEngine,
    instrument: u32,
) -> f32 {
    if engine.is_null() {
        return 0.5;
    }
    (*engine)
        .voice(instrument as usize)
        .map_or(0.5, |v| v.pan.target())
}

// =============================================================================
// Preset blend (2D X/Y pad interpolation)
// =============================================================================

/// Enable preset blend mode for an instrument
///
/// When enabled, use `gooey_engine_blend_set_position` to blend between
/// the 4 corner presets using X/Y coordinates.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, etc.)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_blend_enable(engine: *mut GooeyEngine, instrument: u32) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    if let Some(voice) = engine.voice_mut(instrument as usize) {
        voice.blend_enabled = true;
    }
}

/// Disable preset blend mode for an instrument
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, etc.)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_blend_disable(engine: *mut GooeyEngine, instrument: u32) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    if let Some(voice) = engine.voice_mut(instrument as usize) {
        voice.blend_enabled = false;
    }
}

/// Check if preset blend mode is enabled for an instrument
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, etc.)
///
/// # Returns
/// `true` if blend mode is enabled, `false` otherwise
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_blend_is_enabled(
    engine: *const GooeyEngine,
    instrument: u32,
) -> bool {
    if engine.is_null() {
        return false;
    }
    let engine = &*engine;
    engine
        .voice(instrument as usize)
        .is_some_and(|v| v.blend_enabled)
}

/// Set the X/Y blend position for an instrument
///
/// Performs bilinear interpolation between the 4 corner presets
/// and applies the blended config to the instrument with smoothing.
///
/// Coordinate space:
/// ```text
///        Y=1
///    TL ---- TR
///     |      |
///     |      |
///    BL ---- BR
///        Y=0
///   X=0      X=1
/// ```
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, etc.)
/// * `x` - Horizontal position (0.0 = left, 1.0 = right)
/// * `y` - Vertical position (0.0 = bottom, 1.0 = top)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_blend_set_position(
    engine: *mut GooeyEngine,
    instrument: u32,
    x: f32,
    y: f32,
) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    let Some(voice) = engine.voice_mut(instrument as usize) else {
        return;
    };
    if !voice.blend_enabled {
        return;
    }

    voice.blend_x = x.clamp(0.0, 1.0);
    voice.blend_y = y.clamp(0.0, 1.0);

    let (x, y) = (voice.blend_x, voice.blend_y);
    voice.blender.blend_and_apply(&mut voice.instrument, x, y);
}

/// Get the current X blend position for an instrument
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, etc.)
///
/// # Returns
/// X position (0.0-1.0), or -1.0 if invalid
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_blend_get_position_x(
    engine: *const GooeyEngine,
    instrument: u32,
) -> f32 {
    if engine.is_null() {
        return -1.0;
    }
    let engine = &*engine;
    engine
        .voice(instrument as usize)
        .map_or(-1.0, |v| v.blend_x)
}

/// Get the current Y blend position for an instrument
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, etc.)
///
/// # Returns
/// Y position (0.0-1.0), or -1.0 if invalid
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_blend_get_position_y(
    engine: *const GooeyEngine,
    instrument: u32,
) -> f32 {
    if engine.is_null() {
        return -1.0;
    }
    let engine = &*engine;
    engine
        .voice(instrument as usize)
        .map_or(-1.0, |v| v.blend_y)
}

/// Set a corner preset by preset ID
///
/// Allows customizing which preset is at each corner of the blend space.
/// Changes take effect immediately on the next blend position update.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, etc.)
/// * `corner` - Corner position (BLEND_CORNER_BOTTOM_LEFT, etc.)
/// * `preset_id` - Preset ID (KICK_PRESET_TIGHT, etc.)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_blend_set_corner_preset(
    engine: *mut GooeyEngine,
    instrument: u32,
    corner: u32,
    preset_id: u32,
) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    let corner_idx = corner as usize;
    if corner_idx >= 4 {
        return;
    }
    if let Some(voice) = engine.voice_mut(instrument as usize) {
        voice.blend_corner_presets[corner_idx] = preset_id;
        voice.blender.set_corner_preset(corner, preset_id);
    }
}

/// Get the preset ID at a corner
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, etc.)
/// * `corner` - Corner position (BLEND_CORNER_BOTTOM_LEFT, etc.)
///
/// # Returns
/// Preset ID, or 0xFFFFFFFF if invalid
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_blend_get_corner_preset(
    engine: *const GooeyEngine,
    instrument: u32,
    corner: u32,
) -> u32 {
    if engine.is_null() {
        return 0xFFFFFFFF;
    }
    let engine = &*engine;
    let corner_idx = corner as usize;
    if corner_idx >= 4 {
        return 0xFFFFFFFF;
    }
    engine
        .voice(instrument as usize)
        .map_or(0xFFFFFFFF, |v| v.blend_corner_presets[corner_idx])
}

/// Reset blend corners to default presets
///
/// Restores the default corner configuration for an instrument.
/// For kick: BL=Tight, BR=Punch, TL=Loose, TR=Dirt
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `instrument` - Instrument ID (INSTRUMENT_KICK, etc.)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_blend_reset_corners(
    engine: *mut GooeyEngine,
    instrument: u32,
) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    if let Some(voice) = engine.voice_mut(instrument as usize) {
        let inst_type = voice.instrument.instrument_type();
        voice.blender = ChannelBlender::default_for_type(inst_type);
        voice.blend_corner_presets = ChannelBlender::default_corner_preset_ids(inst_type);
    }
}

// =============================================================================
// Poly Synth — chord playback via music theory
// =============================================================================

// Preset IDs
pub const POLY_PRESET_DEFAULT: u32 = 0;
pub const POLY_PRESET_PAD: u32 = 1;
pub const POLY_PRESET_PLUCK: u32 = 2;
pub const POLY_PRESET_KEYS: u32 = 3;
pub const POLY_PRESET_STRINGS: u32 = 4;

// Scale type IDs
pub const SCALE_MAJOR: u32 = 0;
pub const SCALE_MINOR: u32 = 1;

// Voicing type IDs
pub const VOICING_ROOT_POSITION: u32 = 0;
pub const VOICING_FIRST_INVERSION: u32 = 1;
pub const VOICING_SECOND_INVERSION: u32 = 2;
pub const VOICING_THIRD_INVERSION: u32 = 3;
pub const VOICING_OPEN: u32 = 4;
pub const VOICING_DROP2: u32 = 5;
pub const VOICING_DROP3: u32 = 6;
pub const VOICING_SPREAD: u32 = 7;
pub const VOICING_SHELL: u32 = 8;
pub const VOICING_ROOTLESS: u32 = 9;

fn voicing_from_id(id: u32) -> VoicingType {
    match id {
        VOICING_FIRST_INVERSION => VoicingType::FirstInversion,
        VOICING_SECOND_INVERSION => VoicingType::SecondInversion,
        VOICING_THIRD_INVERSION => VoicingType::ThirdInversion,
        VOICING_OPEN => VoicingType::OpenVoicing,
        VOICING_DROP2 => VoicingType::Drop2,
        VOICING_DROP3 => VoicingType::Drop3,
        VOICING_SPREAD => VoicingType::Spread,
        VOICING_SHELL => VoicingType::Shell,
        VOICING_ROOTLESS => VoicingType::Rootless,
        _ => VoicingType::RootPosition,
    }
}

fn scale_from_id(id: u32) -> ScaleType {
    match id {
        SCALE_MINOR => ScaleType::NaturalMinor,
        _ => ScaleType::Major,
    }
}

fn root_from_id(id: u32) -> NoteName {
    NoteName::from_index(id as u8 % 12)
}

fn preset_config(id: u32) -> PolySynthConfig {
    match id {
        POLY_PRESET_PAD => PolySynthConfig::pad(),
        POLY_PRESET_PLUCK => PolySynthConfig::pluck(),
        POLY_PRESET_KEYS => PolySynthConfig::keys(),
        POLY_PRESET_STRINGS => PolySynthConfig::strings(),
        _ => PolySynthConfig::default(),
    }
}

/// Trigger a diatonic chord from a key.
///
/// Builds the chord from music theory (root + scale → diatonic chord at degree),
/// applies the requested voicing, and triggers all notes on the poly synth.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `root` - Root note (0=C, 1=C#, 2=D, ... 11=B)
/// * `scale_type` - Scale (SCALE_MAJOR=0, SCALE_MINOR=1)
/// * `degree` - Diatonic degree (0-6, i.e. I through VII)
/// * `voicing` - Voicing type ID (VOICING_ROOT_POSITION, etc.)
/// * `preset` - Preset ID (POLY_PRESET_DEFAULT, etc.)
/// * `octave` - Base octave (typically 3-5)
/// * `velocity` - Note velocity (0.0-1.0)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_poly_trigger_chord(
    engine: *mut GooeyEngine,
    root: u32,
    scale_type: u32,
    degree: u32,
    voicing: u32,
    preset: u32,
    octave: i32,
    velocity: f32,
) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;

    let root_note = root_from_id(root);
    let scale = scale_from_id(scale_type);
    let key = Key::new(root_note, scale);
    let voicing_type = voicing_from_id(voicing);
    let octave = octave.clamp(0, 8) as i8;
    let velocity = velocity.clamp(0.0, 1.0);

    // Apply preset
    engine.poly_synth.set_config(preset_config(preset));
    engine.poly_synth.snap_params();

    // Get diatonic seventh chords and pick the requested degree
    let chords = key.diatonic_sevenths();
    let degree = degree as usize % chords.len();
    let chord = &chords[degree];

    // Apply voicing to get MIDI notes
    let midi_notes = apply_voicing(chord, voicing_type, octave);

    // Release any currently sounding notes, then trigger the new chord
    engine.poly_synth.release_all();
    for note in &midi_notes {
        engine.poly_synth.trigger_note(*note, velocity);
    }
}

/// Release all sounding poly synth notes.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_poly_release(engine: *mut GooeyEngine) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    engine.poly_synth.release_all();
}

/// Set the poly synth preset.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `preset` - Preset ID (POLY_PRESET_DEFAULT, etc.)
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_poly_set_preset(engine: *mut GooeyEngine, preset: u32) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    engine.poly_synth.set_config(preset_config(preset));
}

/// Set a single poly synth parameter by index.
///
/// Parameter indices:
/// - 0: osc_shape (0=saw, 1=square)
/// - 1: detune_amount
/// - 2: filter_cutoff
/// - 3: filter_resonance
/// - 4: filter_env_amount
/// - 5: amp_attack
/// - 6: amp_decay
/// - 7: amp_sustain
/// - 8: amp_release
/// - 9: filter_attack
/// - 10: filter_decay
/// - 11: filter_sustain
/// - 12: filter_release
/// - 13: volume
///
/// All values are normalized 0.0-1.0.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_poly_set_param(
    engine: *mut GooeyEngine,
    param: u32,
    value: f32,
) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    let value = value.clamp(0.0, 1.0);

    match param {
        0 => engine.poly_synth.params.osc_shape.set_target(value),
        1 => engine.poly_synth.params.detune_amount.set_target(value),
        2 => engine.poly_synth.params.filter_cutoff.set_target(value),
        3 => engine.poly_synth.params.filter_resonance.set_target(value),
        4 => engine.poly_synth.params.filter_env_amount.set_target(value),
        5 => engine.poly_synth.params.amp_attack.set_target(value),
        6 => engine.poly_synth.params.amp_decay.set_target(value),
        7 => engine.poly_synth.params.amp_sustain.set_target(value),
        8 => engine.poly_synth.params.amp_release.set_target(value),
        9 => engine.poly_synth.params.filter_attack.set_target(value),
        10 => engine.poly_synth.params.filter_decay.set_target(value),
        11 => engine.poly_synth.params.filter_sustain.set_target(value),
        12 => engine.poly_synth.params.filter_release.set_target(value),
        13 => engine.poly_synth.params.volume.set_target(value),
        _ => {}
    }
}

/// Query how many voicings are available for a given chord quality.
///
/// The chord quality is determined by root + scale + degree.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_poly_available_voicing_count(
    root: u32,
    scale_type: u32,
    degree: u32,
) -> u32 {
    let root_note = root_from_id(root);
    let scale = scale_from_id(scale_type);
    let key = Key::new(root_note, scale);
    let chords = key.diatonic_sevenths();
    let degree = degree as usize % chords.len();
    available_voicings(&chords[degree].quality).len() as u32
}

// ---------------------------------------------------------------------------
// Granulator
// ---------------------------------------------------------------------------

/// Load a mono sample buffer into the granulator.
///
/// Copies `len` samples from `samples` into the engine. Replaces any
/// previously loaded buffer and kills all currently active grains.
///
/// The granulator is mono only. Stereo or multichannel hosts should downmix
/// before calling this function.
///
/// Returns `true` on success, `false` if any argument is invalid (null engine,
/// null `samples`, `len == 0`, non-finite or non-positive `sample_rate`, or
/// any non-finite value in the sample slice).
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`. `samples`
/// must point to at least `len` valid `f32` values when `len > 0`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_granulator_set_buffer(
    engine: *mut GooeyEngine,
    samples: *const f32,
    len: u32,
    sample_rate: f32,
) -> bool {
    if engine.is_null() || samples.is_null() || len == 0 {
        return false;
    }
    let engine = &mut *engine;
    let slice = slice::from_raw_parts(samples, len as usize);
    let owned = slice.to_vec();
    match SampleBuffer::from_mono(owned, sample_rate) {
        Ok(buffer) => {
            engine.granulator.set_buffer(buffer);
            true
        }
        Err(_) => false,
    }
}

// =============================================================================
// Mixer graph: host-defined source routing, submix tracks, and track effects
// =============================================================================
//
// The mixer graph sits above the drum kit, bass, poly synth, granulator, and
// loop mixer. Hosts can create named tracks, route sources to tracks, adjust
// track strips, and add track-level effects. Graph mutation is intended to be
// serialized with rendering, matching the existing loop-effect API contract.

/// Restore the default graph layout: Drums, Bass, Synth, Loops.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_mixer_reset_default_layout(engine: *mut GooeyEngine) {
    if let Some(engine) = engine.as_mut() {
        engine.graph = MixerGraph::with_default_layout(engine.sample_rate, engine.bpm);
    }
}

/// Clear every graph track and source route. All graph-routed sources become silent.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_mixer_clear_layout(engine: *mut GooeyEngine) {
    if let Some(engine) = engine.as_mut() {
        engine.graph.reset();
    }
}

/// Add a named mixer track. Returns the new track index, or -1 on failure.
///
/// # Safety
/// `name` must point to a valid null-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_mixer_add_track(
    engine: *mut GooeyEngine,
    name: *const c_char,
) -> i32 {
    match (engine.as_mut(), name.as_ref()) {
        (Some(engine), Some(_)) => {
            let name = CStr::from_ptr(name).to_owned();
            engine.graph.add_track(name) as i32
        }
        _ => -1,
    }
}

/// Return the number of mixer graph tracks.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_mixer_get_track_count(engine: *const GooeyEngine) -> u32 {
    engine
        .as_ref()
        .map_or(0, |engine| engine.graph.track_count() as u32)
}

/// Return a track's engine-owned name pointer, or null for a bad track index.
///
/// The pointer remains valid until the track is renamed, the layout is cleared,
/// or the engine is freed.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_mixer_get_track_name(
    engine: *const GooeyEngine,
    track: u32,
) -> *const c_char {
    engine
        .as_ref()
        .and_then(|engine| engine.graph.track_name(track as usize))
        .map_or(std::ptr::null(), CStr::as_ptr)
}

/// Rename a mixer track. Returns false for null input or a bad track index.
///
/// # Safety
/// `name` must point to a valid null-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_mixer_set_track_name(
    engine: *mut GooeyEngine,
    track: u32,
    name: *const c_char,
) -> bool {
    match (engine.as_mut(), name.as_ref()) {
        (Some(engine), Some(_)) => {
            let name = CStr::from_ptr(name).to_owned();
            engine.graph.set_track_name(track as usize, name)
        }
        _ => false,
    }
}

/// Find the first track with `name`. Returns -1 if none is found.
///
/// # Safety
/// `name` must point to a valid null-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_mixer_find_track(
    engine: *const GooeyEngine,
    name: *const c_char,
) -> i32 {
    match (engine.as_ref(), name.as_ref()) {
        (Some(engine), Some(_)) => {
            let name = CStr::from_ptr(name);
            engine
                .graph
                .track_index_by_name(name)
                .map_or(-1, |track| track as i32)
        }
        _ => -1,
    }
}

/// Route an engine source (`SOURCE_*`) to a mixer track.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_mixer_route_source(
    engine: *mut GooeyEngine,
    source: u32,
    track: u32,
) -> bool {
    match engine.as_mut() {
        Some(engine) => engine.graph.route(source, track as usize),
        None => false,
    }
}

/// Unroute an engine source. Returns false for invalid or already-unrouted sources.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_mixer_unroute_source(
    engine: *mut GooeyEngine,
    source: u32,
) -> bool {
    match engine.as_mut() {
        Some(engine) => engine.graph.unroute(source),
        None => false,
    }
}

/// Return the track a source is routed to, or -1 if invalid/unrouted.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_mixer_get_source_route(
    engine: *const GooeyEngine,
    source: u32,
) -> i32 {
    engine
        .as_ref()
        .and_then(|engine| engine.graph.route_of(source))
        .map_or(-1, |track| track as i32)
}

/// Set a track fader gain (`0.0..=2.0`).
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_mixer_set_track_gain(
    engine: *mut GooeyEngine,
    track: u32,
    gain: f32,
) {
    if let Some(engine) = engine.as_mut() {
        engine.graph.set_track_gain(track as usize, gain);
    }
}

/// Get a track fader gain, or 1.0 for null/bad track.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_mixer_get_track_gain(
    engine: *const GooeyEngine,
    track: u32,
) -> f32 {
    engine
        .as_ref()
        .map_or(1.0, |engine| engine.graph.track_gain(track as usize))
}

/// Set a track stereo balance (`0.0` left, `0.5` center, `1.0` right).
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_mixer_set_track_pan(
    engine: *mut GooeyEngine,
    track: u32,
    pan: f32,
) {
    if let Some(engine) = engine.as_mut() {
        engine.graph.set_track_pan(track as usize, pan);
    }
}

/// Get a track stereo balance, or 0.5 for null/bad track.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_mixer_get_track_pan(
    engine: *const GooeyEngine,
    track: u32,
) -> f32 {
    engine
        .as_ref()
        .map_or(0.5, |engine| engine.graph.track_pan(track as usize))
}

/// Mute or unmute a track.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_mixer_set_track_mute(
    engine: *mut GooeyEngine,
    track: u32,
    muted: bool,
) {
    if let Some(engine) = engine.as_ref() {
        engine.graph.set_track_mute(track as usize, muted);
    }
}

/// Return a track's mute state, or false for null/bad track.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_mixer_get_track_mute(
    engine: *const GooeyEngine,
    track: u32,
) -> bool {
    engine
        .as_ref()
        .is_some_and(|engine| engine.graph.track_mute(track as usize))
}

/// Solo or un-solo a track.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_mixer_set_track_solo(
    engine: *mut GooeyEngine,
    track: u32,
    soloed: bool,
) {
    if let Some(engine) = engine.as_ref() {
        engine.graph.set_track_solo(track as usize, soloed);
    }
}

/// Return a track's solo state, or false for null/bad track.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_mixer_get_track_solo(
    engine: *const GooeyEngine,
    track: u32,
) -> bool {
    engine
        .as_ref()
        .is_some_and(|engine| engine.graph.track_solo(track as usize))
}

/// Read and reset a track's post-strip peak. Returns 0.0 for null/bad track.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_mixer_get_track_peak(
    engine: *const GooeyEngine,
    track: u32,
) -> f32 {
    engine
        .as_ref()
        .and_then(|engine| engine.graph.track_peak_swap(track as usize))
        .unwrap_or(0.0)
}

/// Append an effect to a mixer track. Returns the new slot, or -1 on failure.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_track_effect_add(
    engine: *mut GooeyEngine,
    track: u32,
    effect_id: u32,
) -> i32 {
    match engine.as_mut() {
        Some(engine) => engine
            .graph
            .effect_add(track as usize, effect_id)
            .map_or(-1, |slot| slot as i32),
        None => -1,
    }
}

/// Remove an effect from a mixer track.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_track_effect_remove(
    engine: *mut GooeyEngine,
    track: u32,
    slot: u32,
) -> bool {
    match engine.as_mut() {
        Some(engine) => engine.graph.effect_remove(track as usize, slot as usize),
        None => false,
    }
}

/// Move an effect within a mixer track's rack.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_track_effect_move(
    engine: *mut GooeyEngine,
    track: u32,
    slot: u32,
    new_position: u32,
) -> bool {
    match engine.as_mut() {
        Some(engine) => {
            engine
                .graph
                .effect_move(track as usize, slot as usize, new_position as usize)
        }
        None => false,
    }
}

/// Clear all effects from a mixer track.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_track_effect_clear(engine: *mut GooeyEngine, track: u32) {
    if let Some(engine) = engine.as_mut() {
        engine.graph.effect_clear(track as usize);
    }
}

/// Set a parameter on a mixer track effect.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_track_effect_set_param(
    engine: *mut GooeyEngine,
    track: u32,
    slot: u32,
    param: u32,
    value: f32,
) {
    if let Some(engine) = engine.as_ref() {
        engine
            .graph
            .effect_set_param(track as usize, slot as usize, param, value);
    }
}

/// Return the number of effects on a mixer track.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_track_effect_count(
    engine: *const GooeyEngine,
    track: u32,
) -> u32 {
    engine
        .as_ref()
        .map_or(0, |engine| engine.graph.effect_count(track as usize) as u32)
}

/// Return the `EFFECT_*` id at a mixer track effect slot, or -1.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_track_effect_type_at(
    engine: *const GooeyEngine,
    track: u32,
    slot: u32,
) -> i32 {
    engine
        .as_ref()
        .and_then(|engine| engine.graph.effect_type_at(track as usize, slot as usize))
        .map_or(-1, |id| id as i32)
}

// =============================================================================
// Loop mixer: 4 stereo loop channels with per-channel effect chains
// =============================================================================
//
// These functions control the engine's `Mixer` (see `src/mixer/`). Each channel
// is a stereo loop player with its own gain fader, mute/solo, loop window,
// varispeed, and an ordered chain of effects. The host decodes audio files and
// passes raw interleaved f32 frames; the engine owns playback and mixing.
//
// `channel` is a 0-based index in `[0, LOOP_CHANNEL_COUNT)`; out-of-range
// indices are ignored (or return a sensible default).

/// Pitch mode: no BPM-driven tempo warp; playback rate follows `speed` alone.
pub const PITCH_MODE_OFF: u32 = 0;
/// Pitch mode: naive resample warp — tempo changes shift pitch.
pub const PITCH_MODE_RESAMPLE: u32 = 1;
/// Pitch mode: WSOLA time-stretch — tempo changes without shifting pitch.
pub const PITCH_MODE_PRESERVE_PITCH: u32 = 2;

/// Load (or replace) a loop channel's stereo buffer from interleaved f32 frames.
///
/// `samples` points to `frames * channels` interleaved values. A `channels`
/// value of 1 duplicates the mono signal to both sides; 2+ uses channels 0/1 as
/// left/right. Loading resets the channel's playhead to its loop start.
///
/// Returns `true` on success, `false` on a null/short pointer, bad channel
/// index, or invalid buffer (zero length, non-finite samples, bad sample rate).
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`. `samples`
/// must point to at least `frames * channels` readable `f32` values.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_load(
    engine: *mut GooeyEngine,
    channel: u32,
    samples: *const f32,
    frames: u32,
    channels: u32,
    sample_rate: f32,
) -> bool {
    if engine.is_null() || samples.is_null() || frames == 0 || channels == 0 {
        return false;
    }
    let engine = &mut *engine;
    let total = frames as usize * channels as usize;
    let slice = slice::from_raw_parts(samples, total);
    match StereoSampleBuffer::from_interleaved(slice, channels as usize, sample_rate) {
        Ok(buffer) => engine.mixer.load(channel as usize, buffer),
        Err(_) => false,
    }
}

/// Start or stop playback of a loop channel.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_set_playing(
    engine: *mut GooeyEngine,
    channel: u32,
    playing: bool,
) {
    if let Some(engine) = engine.as_mut() {
        engine.mixer.set_playing(channel as usize, playing);
    }
}

/// Set a loop channel's fader gain (0.0 = silence, 1.0 = unity, up to 2.0).
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_set_gain(
    engine: *mut GooeyEngine,
    channel: u32,
    gain: f32,
) {
    if let Some(engine) = engine.as_mut() {
        engine.mixer.set_gain(channel as usize, gain);
    }
}

/// Mute or unmute a loop channel (click-free, applied to the post-effect signal).
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_set_mute(
    engine: *mut GooeyEngine,
    channel: u32,
    muted: bool,
) {
    if let Some(engine) = engine.as_mut() {
        engine.mixer.set_muted(channel as usize, muted);
    }
}

/// Solo or un-solo a loop channel. While any channel is soloed, only soloed
/// channels are audible.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_set_solo(
    engine: *mut GooeyEngine,
    channel: u32,
    soloed: bool,
) {
    if let Some(engine) = engine.as_mut() {
        engine.mixer.set_soloed(channel as usize, soloed);
    }
}

/// Set a loop channel's loop start point as a normalized `[0, 1]` position in
/// the buffer.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_set_start(
    engine: *mut GooeyEngine,
    channel: u32,
    normalized: f32,
) {
    if let Some(engine) = engine.as_mut() {
        engine.mixer.set_loop_start(channel as usize, normalized);
    }
}

/// Set a loop channel's loop end point as a normalized `[0, 1]` position in
/// the buffer.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_set_end(
    engine: *mut GooeyEngine,
    channel: u32,
    normalized: f32,
) {
    if let Some(engine) = engine.as_mut() {
        engine.mixer.set_loop_end(channel as usize, normalized);
    }
}

/// Set a loop channel's playback speed (varispeed). 1.0 = normal, 0.5 = half
/// speed/down an octave, negative = reverse. Clamped to `[-4, 4]`.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_set_speed(
    engine: *mut GooeyEngine,
    channel: u32,
    speed: f32,
) {
    if let Some(engine) = engine.as_mut() {
        engine.mixer.set_speed(channel as usize, speed);
    }
}

/// Tag a loop channel's loaded buffer with the tempo its source material was
/// authored at. Used by the tempo-warp pitch modes (see
/// `gooey_engine_loop_set_pitch_mode`) to compute a warp ratio against the
/// engine's BPM (set via `gooey_engine_set_bpm`). Pass `0.0` to clear the tag
/// (disables warping for this channel regardless of pitch mode). No-op if no
/// buffer is loaded — call after `gooey_engine_loop_load`.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_set_source_bpm(
    engine: *mut GooeyEngine,
    channel: u32,
    source_bpm: f32,
) {
    if let Some(engine) = engine.as_mut() {
        let bpm = if source_bpm > 0.0 {
            Some(source_bpm)
        } else {
            None
        };
        engine.mixer.set_source_bpm(channel as usize, bpm);
    }
}

/// Get a loop channel's tagged source BPM, or `0.0` if unset, no buffer is
/// loaded, or the channel index is out of range.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_get_source_bpm(
    engine: *const GooeyEngine,
    channel: u32,
) -> f32 {
    engine
        .as_ref()
        .and_then(|e| e.mixer.source_bpm(channel as usize))
        .unwrap_or(0.0)
}

/// Set a loop channel's tempo-warp/pitch mode. See `PITCH_MODE_*` constants.
/// Out-of-range values are treated as `PITCH_MODE_OFF`.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_set_pitch_mode(
    engine: *mut GooeyEngine,
    channel: u32,
    mode: u32,
) {
    if let Some(engine) = engine.as_mut() {
        let mode = match mode {
            PITCH_MODE_RESAMPLE => PitchMode::Resample,
            PITCH_MODE_PRESERVE_PITCH => PitchMode::PreservePitch,
            _ => PitchMode::Off,
        };
        engine.mixer.set_pitch_mode(channel as usize, mode);
    }
}

/// Get a loop channel's tempo-warp/pitch mode. See `PITCH_MODE_*` constants.
/// Returns `PITCH_MODE_OFF` for an out-of-range channel index.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_get_pitch_mode(
    engine: *const GooeyEngine,
    channel: u32,
) -> u32 {
    match engine
        .as_ref()
        .map(|e| e.mixer.pitch_mode(channel as usize))
    {
        Some(PitchMode::Resample) => PITCH_MODE_RESAMPLE,
        Some(PitchMode::PreservePitch) => PITCH_MODE_PRESERVE_PITCH,
        _ => PITCH_MODE_OFF,
    }
}

/// Restart a loop channel from its loop start point.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_restart(engine: *mut GooeyEngine, channel: u32) {
    if let Some(engine) = engine.as_mut() {
        engine.mixer.restart(channel as usize);
    }
}

/// Set a loop channel's playhead to a normalized `[0, 1]` position within its
/// loop region. Clamped into the active loop window; no-op if no buffer is loaded.
/// Inverse of `gooey_engine_loop_get_position`. Lets a caller swap a channel's
/// buffer and resume at the same phase instead of restarting at the loop start.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_set_position(
    engine: *mut GooeyEngine,
    channel: u32,
    normalized: f32,
) {
    if let Some(engine) = engine.as_mut() {
        engine.mixer.set_position(channel as usize, normalized);
    }
}

/// Stage a buffer to atomically replace this channel's loop at the next bar-grid
/// boundary. `divisions` splits the loop region into equal segments (pass the
/// loop's bar count for bar-quantized swaps; 1 for whole-phrase). When the playing
/// cursor next crosses a segment boundary, the queued buffer becomes active and its
/// playhead resets to the loop start (restart from the top on the downbeat).
/// Replaces any previously-queued buffer on the channel. Returns false on a null
/// engine, bad channel, or empty buffer. Samples are interleaved, `channels` deep.
///
/// `source_bpm` tags the *pending* take so its tempo warp is correct the instant the
/// swap lands (mirrors [`gooey_engine_loop_set_source_bpm`], which only tags the
/// currently active buffer). Pass `0.0` (or negative) to leave it untagged, in which
/// case a warping channel plays the swapped-in loop at its original tempo.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`; `samples` must
/// point to at least `frames * channels` floats.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_queue_swap(
    engine: *mut GooeyEngine,
    channel: u32,
    samples: *const f32,
    frames: u32,
    channels: u32,
    sample_rate: f32,
    source_bpm: f32,
    divisions: u32,
) -> bool {
    if engine.is_null() || samples.is_null() || frames == 0 || channels == 0 {
        return false;
    }
    let engine = &mut *engine;
    let total = frames as usize * channels as usize;
    let slice = slice::from_raw_parts(samples, total);
    match StereoSampleBuffer::from_interleaved(slice, channels as usize, sample_rate) {
        Ok(mut buffer) => {
            // Tag the pending take so `warp_ratio()` is correct the moment it lands,
            // rather than falling back to 1.0 until the host retags post-swap.
            // `set_source_bpm` filters non-finite/<= 0, so 0.0 means "untagged".
            buffer.set_source_bpm((source_bpm > 0.0).then_some(source_bpm));
            engine.mixer.queue_swap(channel as usize, buffer, divisions)
        }
        Err(_) => false,
    }
}

/// Drop a pending queued swap on a loop channel (Cancel / re-select the playing
/// take / transport stop). No-op if nothing is queued.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_cancel_queued_swap(
    engine: *mut GooeyEngine,
    channel: u32,
) {
    if let Some(engine) = engine.as_mut() {
        engine.mixer.cancel_queued_swap(channel as usize);
    }
}

/// Count of queued swaps that have completed on this channel since engine creation.
/// The host samples this when it queues and watches it increment to learn the swap
/// landed (drives the UI's "queued -> playing" flip). Returns 0 for a null engine
/// or out-of-range channel.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_swaps_completed(
    engine: *const GooeyEngine,
    channel: u32,
) -> u32 {
    match engine.as_ref() {
        Some(engine) => engine.mixer.swaps_completed(channel as usize),
        None => 0,
    }
}

/// Get a loop channel's current playhead as a normalized `[0, 1]` position.
/// Returns 0.0 for a null engine or empty/out-of-range channel.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_get_position(
    engine: *const GooeyEngine,
    channel: u32,
) -> f32 {
    match engine.as_ref() {
        Some(engine) => engine
            .mixer
            .channel(channel as usize)
            .map_or(0.0, |ch| ch.position_normalized()),
        None => 0.0,
    }
}

/// Append an effect (an `EFFECT_*` id) to a loop channel's chain. Returns the
/// new effect's slot index, or -1 on failure (null engine, bad channel, or an
/// effect id that is not a per-channel effect, e.g. the master limiter).
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_effect_add(
    engine: *mut GooeyEngine,
    channel: u32,
    effect_id: u32,
) -> i32 {
    match engine.as_mut() {
        Some(engine) => engine
            .mixer
            .effect_add(channel as usize, effect_id)
            .map_or(-1, |slot| slot as i32),
        None => -1,
    }
}

/// Remove the effect at `slot` from a loop channel's chain. Returns `true` if an
/// effect was removed.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_effect_remove(
    engine: *mut GooeyEngine,
    channel: u32,
    slot: u32,
) -> bool {
    match engine.as_mut() {
        Some(engine) => engine.mixer.effect_remove(channel as usize, slot as usize),
        None => false,
    }
}

/// Move the effect at `slot` to `new_position` within a loop channel's chain.
/// Returns `true` on success.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_effect_move(
    engine: *mut GooeyEngine,
    channel: u32,
    slot: u32,
    new_position: u32,
) -> bool {
    match engine.as_mut() {
        Some(engine) => {
            engine
                .mixer
                .effect_move(channel as usize, slot as usize, new_position as usize)
        }
        None => false,
    }
}

/// Remove all effects from a loop channel's chain.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_effect_clear(engine: *mut GooeyEngine, channel: u32) {
    if let Some(engine) = engine.as_mut() {
        engine.mixer.effect_clear(channel as usize);
    }
}

/// Set a parameter (`*_PARAM_*` id) on the effect at `slot` of a loop channel.
/// Parameter ids and value ranges match `gooey_engine_set_global_effect_param`.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_effect_set_param(
    engine: *mut GooeyEngine,
    channel: u32,
    slot: u32,
    param: u32,
    value: f32,
) {
    if let Some(engine) = engine.as_ref() {
        engine
            .mixer
            .effect_set_param(channel as usize, slot as usize, param, value);
    }
}

/// Return the number of effects in a loop channel's chain (0 for a null engine
/// or out-of-range channel).
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_effect_count(
    engine: *const GooeyEngine,
    channel: u32,
) -> u32 {
    match engine.as_ref() {
        Some(engine) => engine.mixer.effect_count(channel as usize) as u32,
        None => 0,
    }
}

/// Return the `EFFECT_*` id of the effect at `slot` of a loop channel, or -1 if
/// the engine is null or the channel/slot is out of range.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_loop_effect_type_at(
    engine: *const GooeyEngine,
    channel: u32,
    slot: u32,
) -> i32 {
    match engine.as_ref() {
        Some(engine) => engine
            .mixer
            .effect_type_at(channel as usize, slot as usize)
            .map_or(-1, |id| id as i32),
        None => -1,
    }
}

/// Return the length, in samples, of the granulator's currently loaded buffer.
///
/// Returns 0 if `engine` is null. Immediately after `gooey_engine_new`, the
/// granulator holds a 1-sample silent placeholder, so a return value of 1
/// (with no `set_buffer` call) indicates "no host buffer loaded yet".
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_granulator_buffer_len(engine: *const GooeyEngine) -> u32 {
    if engine.is_null() {
        return 0;
    }
    let engine = &*engine;
    engine.granulator.buffer_len() as u32
}

/// Return the sample rate of the granulator's currently loaded buffer.
///
/// Returns 0.0 if `engine` is null.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_granulator_buffer_sample_rate(
    engine: *const GooeyEngine,
) -> f32 {
    if engine.is_null() {
        return 0.0;
    }
    let engine = &*engine;
    engine.granulator.buffer_sample_rate()
}

/// Trigger the granulator with a velocity, starting a cloud of grains.
///
/// Unlike drum-channel triggers, granulator triggers are applied immediately
/// against the engine's current time and are not deferred through an atomic
/// pending flag. The caller is expected to invoke this from the host thread
/// (e.g. UI / MIDI input handler), not from inside the audio render callback.
///
/// `velocity` is clamped to 0.0-1.0.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_granulator_trigger(engine: *mut GooeyEngine, velocity: f32) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    let velocity = velocity.clamp(0.0, 1.0);
    engine
        .granulator
        .trigger_with_velocity(engine.current_time, velocity);
}

/// Set a granulator parameter by index. All values are normalized 0.0-1.0.
///
/// Parameter indices: see the `GRANULATOR_PARAM_*` constants. Unknown indices
/// are silently ignored, matching the pattern used by other instrument
/// setters in this file.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_granulator_set_param(
    engine: *mut GooeyEngine,
    param: u32,
    value: f32,
) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    let value = value.clamp(0.0, 1.0);
    match param {
        GRANULATOR_PARAM_SCAN_POSITION => engine.granulator.set_scan_position(value),
        GRANULATOR_PARAM_GRAIN_LENGTH => engine.granulator.set_grain_length(value),
        GRANULATOR_PARAM_SPRAY => engine.granulator.set_spray(value),
        GRANULATOR_PARAM_PITCH => engine.granulator.set_pitch(value),
        GRANULATOR_PARAM_DENSITY => engine.granulator.set_density(value),
        GRANULATOR_PARAM_TEXTURE => engine.granulator.set_texture(value),
        GRANULATOR_PARAM_DIRECTION => engine.granulator.set_direction(value),
        GRANULATOR_PARAM_CLOUD_DURATION => engine.granulator.set_cloud_duration(value),
        GRANULATOR_PARAM_VOLUME => engine.granulator.set_volume(value),
        GRANULATOR_PARAM_RANDOM_TIMING => engine.granulator.set_random_timing(value),
        GRANULATOR_PARAM_RANDOM_AMP => engine.granulator.set_random_amp(value),
        GRANULATOR_PARAM_DRIVE => engine.granulator.set_drive(value),
        _ => {}
    }
}

/// Read the most-recently-set value of a granulator parameter, in the same
/// normalized 0.0-1.0 range used by `gooey_engine_granulator_set_param`.
///
/// Returns `f32::NAN` if `engine` is null or `param` is unrecognized.
/// Reads the `SmoothedParam::target()` so callers see the value they last
/// wrote even before the audio thread has finished smoothing into it.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_granulator_get_param(
    engine: *const GooeyEngine,
    param: u32,
) -> f32 {
    if engine.is_null() {
        return f32::NAN;
    }
    let engine = &*engine;
    match param {
        GRANULATOR_PARAM_SCAN_POSITION => engine.granulator.scan_position(),
        GRANULATOR_PARAM_GRAIN_LENGTH => engine.granulator.grain_length(),
        GRANULATOR_PARAM_SPRAY => engine.granulator.spray(),
        GRANULATOR_PARAM_PITCH => engine.granulator.pitch(),
        GRANULATOR_PARAM_DENSITY => engine.granulator.density(),
        GRANULATOR_PARAM_TEXTURE => engine.granulator.texture(),
        GRANULATOR_PARAM_DIRECTION => engine.granulator.direction(),
        GRANULATOR_PARAM_CLOUD_DURATION => engine.granulator.cloud_duration(),
        GRANULATOR_PARAM_VOLUME => engine.granulator.volume(),
        GRANULATOR_PARAM_RANDOM_TIMING => engine.granulator.random_timing(),
        GRANULATOR_PARAM_RANDOM_AMP => engine.granulator.random_amp(),
        GRANULATOR_PARAM_DRIVE => engine.granulator.drive(),
        _ => f32::NAN,
    }
}

/// Seed the granulator's grain-spray PRNG. Used for reproducible output in
/// tests; callers that don't need determinism can ignore this.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_granulator_set_seed(engine: *mut GooeyEngine, seed: u32) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    engine.granulator.set_seed(seed);
}

/// Return the number of grains currently sounding inside the granulator.
/// Returns 0 if `engine` is null. Useful for UI metering.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_granulator_active_grain_count(
    engine: *const GooeyEngine,
) -> u32 {
    if engine.is_null() {
        return 0;
    }
    let engine = &*engine;
    engine.granulator.active_grain_count() as u32
}

/// Snap all granulator smoothed parameters to their target values, bypassing
/// the usual ~15 ms exponential smoothing. Useful when loading a preset or
/// resetting state between songs without introducing audible glides.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_granulator_snap_params(engine: *mut GooeyEngine) {
    if engine.is_null() {
        return;
    }
    let engine = &mut *engine;
    engine.granulator.snap_params();
}

// ---------------------------------------------------------------------------
// Offline bounce
// ---------------------------------------------------------------------------

impl GooeyEngine {
    /// Render `bars` bars of audio offline into a new buffer.
    fn bounce_to_buffer(&mut self, bars: u32) -> Vec<f32> {
        let samples_per_bar = 4.0_f64 * (60.0 / self.bpm as f64) * self.sample_rate as f64;
        let total_samples = (bars as f64 * samples_per_bar).round() as usize;

        // Reset engine to a clean state
        self.current_time = 0.0;
        for seq in self.sequencers_iter_mut() {
            seq.reset();
            seq.start();
        }
        for lfo in &mut self.lfos {
            lfo.reset();
        }
        for voice in self.voices_iter_mut() {
            voice.mute_gain.snap();
            voice.channel_gain.snap();
            voice.pan.snap();
        }
        self.graph.snap_strip_params();
        self.master_gain.snap();

        // Render in chunks using the same path as real-time playback. `render`
        // writes interleaved stereo (`[l, r]` per frame), so each frame is
        // downmixed to a single mono sample for this mono bounce buffer —
        // otherwise a panned channel (e.g. hard-left `[l, 0]`) would be written
        // as alternating samples and zeros.
        let mut output = Vec::with_capacity(total_samples);
        let frames_per_chunk = 512;
        let mut chunk_buf = vec![0.0_f32; frames_per_chunk * 2];

        let mut remaining = total_samples;
        while remaining > 0 {
            let frames = remaining.min(frames_per_chunk);
            let slice = &mut chunk_buf[..frames * 2];
            for s in slice.iter_mut() {
                *s = 0.0;
            }
            self.render(slice);
            for frame in slice.chunks_exact(2) {
                output.push(0.5 * (frame[0] + frame[1]));
            }
            remaining -= frames;
        }

        for seq in self.sequencers_iter_mut() {
            seq.stop();
        }

        output
    }
}

/// Bounce the engine offline for the given number of bars.
///
/// Returns a heap-allocated `f32` buffer. The caller must free it with
/// [`gooey_engine_free_buffer`]. The number of samples is written to
/// `out_length`.
///
/// # Safety
/// - `engine` must be a valid pointer returned by `gooey_engine_new`.
/// - `out_length` must be a valid pointer to a `u32`.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_bounce_to_buffer(
    engine: *mut GooeyEngine,
    bars: u32,
    out_length: *mut u32,
) -> *mut f32 {
    if engine.is_null() || out_length.is_null() {
        return std::ptr::null_mut();
    }
    let engine = &mut *engine;
    let buffer = engine.bounce_to_buffer(bars);
    if buffer.len() > u32::MAX as usize {
        return std::ptr::null_mut();
    }
    let len = buffer.len() as u32;
    let boxed = buffer.into_boxed_slice();
    let ptr = Box::into_raw(boxed) as *mut f32;
    *out_length = len;
    ptr
}

/// Free a buffer returned by [`gooey_engine_bounce_to_buffer`].
///
/// # Safety
/// - `buffer` must have been returned by `gooey_engine_bounce_to_buffer`.
/// - `length` must match the `out_length` value from that call.
/// - Must only be called once per buffer.
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_free_buffer(buffer: *mut f32, length: u32) {
    if buffer.is_null() {
        return;
    }
    let slice = std::slice::from_raw_parts_mut(buffer, length as usize);
    drop(Box::from_raw(slice as *mut [f32]));
}

/// Bounce the engine offline and write a WAV file.
///
/// Returns `true` on success. The file is written as mono 16-bit PCM
/// at the engine's sample rate.
///
/// # Safety
/// - `engine` must be a valid pointer returned by `gooey_engine_new`.
/// - `path` must be a valid null-terminated UTF-8 string.
#[cfg(feature = "bounce")]
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_bounce_to_wav(
    engine: *mut GooeyEngine,
    bars: u32,
    path: *const std::os::raw::c_char,
) -> bool {
    if engine.is_null() || path.is_null() {
        return false;
    }
    let engine = &mut *engine;
    let path_str = match std::ffi::CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };

    let buffer = engine.bounce_to_buffer(bars);
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: engine.sample_rate as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer = match hound::WavWriter::create(path_str, spec) {
        Ok(w) => w,
        Err(_) => return false,
    };

    let scale = i16::MAX as f32;
    for &sample in &buffer {
        if writer
            .write_sample((sample * scale).round() as i16)
            .is_err()
        {
            return false;
        }
    }

    writer.finalize().is_ok()
}
