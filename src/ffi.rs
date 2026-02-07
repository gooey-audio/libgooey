//! C FFI bindings for the gooey audio engine
//!
//! This module exposes the audio engine to C/Swift via C-compatible functions.
//! Designed for integration with iOS (and other platforms in the future).

use crate::effects::{BrickWallLimiter, DelayEffect, Effect, LowpassFilterEffect, TubeSaturation};
use crate::engine::lfo::{Lfo, MusicalDivision};
use crate::engine::{Instrument, Sequencer};
use crate::instruments::{HiHat, KickConfig, KickDrum, SnareConfig, SnareDrum, Tom2, Tom2Config};
use crate::utils::{PresetBlender, SmoothedParam};
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

/// Opaque wrapper around the audio engine for FFI
///
/// This struct provides a simplified C-compatible interface for iOS integration.
/// It manages 4 drum instruments (kick, snare, hihat, tom), each with its own
/// 16-step sequencer with sample-accurate timing.
///
/// Parameter smoothing is handled internally by each instrument,
/// so all parameter changes are automatically smoothed to prevent clicks/pops.
pub struct GooeyEngine {
    // Instruments
    kick: KickDrum,
    snare: SnareDrum,
    hihat: HiHat,
    tom: Tom2,

    // Per-instrument sequencers (sample-accurate, synchronized)
    kick_sequencer: Sequencer,
    snare_sequencer: Sequencer,
    hihat_sequencer: Sequencer,
    tom_sequencer: Sequencer,

    // Global effects (applied in order: delay -> lowpass filter -> saturation -> limiter)
    delay: DelayEffect,
    delay_enabled: bool,
    lowpass_filter: LowpassFilterEffect,
    lowpass_filter_enabled: bool,
    saturation: TubeSaturation,
    saturation_enabled: bool,
    limiter: BrickWallLimiter,

    // Engine state
    sample_rate: f32,
    bpm: f32,
    current_time: f32,

    // Per-instrument manual trigger flags and velocities
    kick_trigger_pending: AtomicBool,
    kick_trigger_velocity: AtomicU32, // f32 bits stored atomically
    snare_trigger_pending: AtomicBool,
    snare_trigger_velocity: AtomicU32,
    hihat_trigger_pending: AtomicBool,
    hihat_trigger_velocity: AtomicU32,
    tom_trigger_pending: AtomicBool,
    tom_trigger_velocity: AtomicU32,

    // LFO pool (8 LFOs with multi-target routing)
    lfos: [Lfo; LFO_COUNT],
    lfo_enabled: [bool; LFO_COUNT],
    lfo_routes: [Vec<LfoRoute>; LFO_COUNT],
    lfo_next_route_id: [u32; LFO_COUNT],

    // Per-instrument mute/solo state (indexed by INSTRUMENT_* constants)
    instrument_muted: [AtomicBool; NUM_INSTRUMENTS],
    instrument_soloed: [AtomicBool; NUM_INSTRUMENTS],

    // Smoothed gain multipliers for click-free mute/solo transitions
    instrument_gains: [SmoothedParam; NUM_INSTRUMENTS],

    // Per-instrument preset blend state (2D X/Y pad interpolation)
    kick_blender: PresetBlender<KickConfig>,
    snare_blender: PresetBlender<SnareConfig>,
    tom_blender: PresetBlender<Tom2Config>,
    blend_enabled: [bool; INSTRUMENT_COUNT as usize],
    blend_x: [f32; INSTRUMENT_COUNT as usize],
    blend_y: [f32; INSTRUMENT_COUNT as usize],
    blend_corner_presets: [[u32; 4]; INSTRUMENT_COUNT as usize],
}

impl GooeyEngine {
    fn new(sample_rate: f32) -> Self {
        let bpm = 120.0;

        // Create all instruments
        let kick = KickDrum::new(sample_rate);
        let snare = SnareDrum::new(sample_rate);
        let hihat = HiHat::new(sample_rate);
        let tom = Tom2::new(sample_rate);

        // Create a 16-step sequencer for each instrument
        // All use 16th notes at default 120 BPM, starting with all steps off
        let kick_sequencer = Sequencer::with_pattern(bpm, sample_rate, vec![false; 16], "kick");
        let snare_sequencer = Sequencer::with_pattern(bpm, sample_rate, vec![false; 16], "snare");
        let hihat_sequencer = Sequencer::with_pattern(bpm, sample_rate, vec![false; 16], "hihat");
        let tom_sequencer = Sequencer::with_pattern(bpm, sample_rate, vec![false; 16], "tom");

        // Create delay with default settings (0.25s delay, no feedback, no mix)
        let delay = DelayEffect::new(sample_rate, 0.25, 0.0, 0.0);

        // Create lowpass filter with default settings (fully open, no resonance)
        // Default cutoff at 20kHz means filter is effectively bypassed when enabled
        let lowpass_filter = LowpassFilterEffect::new(sample_rate, 20000.0, 0.0);

        // Create saturation with default light warmth settings
        // drive: 0.3, warmth: 0.4, mix: 0.5 for subtle analog warmth
        let saturation = TubeSaturation::new(sample_rate, 0.3, 0.4, 0.5);

        // Create LFO pool (8 LFOs, all disabled by default with quarter note timing)
        let lfos = std::array::from_fn(|_| Lfo::with_sample_rate(sample_rate));
        let lfo_routes: [Vec<LfoRoute>; LFO_COUNT] = std::array::from_fn(|_| Vec::new());

        // Create kick preset blender with default corner presets
        // BL(0,0)=Tight, BR(1,0)=Punch, TL(0,1)=Loose, TR(1,1)=Dirt
        let kick_blender = PresetBlender::new(
            KickConfig::tight(),
            KickConfig::punch(),
            KickConfig::loose(),
            KickConfig::dirt(),
        );

        // Create snare preset blender with default corner presets
        // BL(0,0)=Tight, BR(1,0)=Loose, TL(0,1)=Hiss, TR(1,1)=Smack
        let snare_blender = PresetBlender::new(
            SnareConfig::tight(),
            SnareConfig::loose(),
            SnareConfig::hiss(),
            SnareConfig::smack(),
        );

        // Create tom preset blender with default corner presets
        // BL(0,0)=Derp, BR(1,0)=Ring, TL(0,1)=Brush, TR(1,1)=Void
        let tom_blender = PresetBlender::new(
            Tom2Config::derp(),
            Tom2Config::ring(),
            Tom2Config::brush(),
            Tom2Config::void_preset(),
        );

        Self {
            kick,
            snare,
            hihat,
            tom,
            kick_sequencer,
            snare_sequencer,
            hihat_sequencer,
            tom_sequencer,
            delay,
            delay_enabled: false, // Disabled by default
            lowpass_filter,
            lowpass_filter_enabled: false, // Disabled by default
            saturation,
            saturation_enabled: true, // Enabled by default for light warmth
            limiter: BrickWallLimiter::new(1.0),
            sample_rate,
            bpm,
            current_time: 0.0,
            kick_trigger_pending: AtomicBool::new(false),
            kick_trigger_velocity: AtomicU32::new(1.0_f32.to_bits()),
            snare_trigger_pending: AtomicBool::new(false),
            snare_trigger_velocity: AtomicU32::new(1.0_f32.to_bits()),
            hihat_trigger_pending: AtomicBool::new(false),
            hihat_trigger_velocity: AtomicU32::new(1.0_f32.to_bits()),
            tom_trigger_pending: AtomicBool::new(false),
            tom_trigger_velocity: AtomicU32::new(1.0_f32.to_bits()),
            // LFO pool
            lfos,
            lfo_enabled: [false; LFO_COUNT],
            lfo_routes,
            lfo_next_route_id: [0; LFO_COUNT],
            // Mute/solo state (all unmuted, none soloed by default)
            instrument_muted: std::array::from_fn(|_| AtomicBool::new(false)),
            instrument_soloed: std::array::from_fn(|_| AtomicBool::new(false)),
            // Smoothed gains for click-free mute/solo transitions (10ms smoothing)
            instrument_gains: std::array::from_fn(|_| SmoothedParam::new(1.0, 0.0, 1.0, sample_rate, 10.0)),
            // Preset blend state
            kick_blender,
            snare_blender,
            tom_blender,
            blend_enabled: [false; INSTRUMENT_COUNT as usize],
            blend_x: [0.5; INSTRUMENT_COUNT as usize],
            blend_y: [0.5; INSTRUMENT_COUNT as usize],
            blend_corner_presets: [
                [KICK_PRESET_TIGHT, KICK_PRESET_PUNCH, KICK_PRESET_LOOSE, KICK_PRESET_DIRT],
                [SNARE_PRESET_TIGHT, SNARE_PRESET_LOOSE, SNARE_PRESET_HISS, SNARE_PRESET_SMACK],
                [0, 1, 2, 3], // HiHat: placeholder
                [TOM_PRESET_DERP, TOM_PRESET_RING, TOM_PRESET_BRUSH, TOM_PRESET_VOID],
            ],
        }
    }

    fn render(&mut self, buffer: &mut [f32]) {
        // Check for pending manual triggers with velocity (all instruments)
        if self.kick_trigger_pending.swap(false, Ordering::Acquire) {
            let velocity = f32::from_bits(self.kick_trigger_velocity.load(Ordering::Acquire));
            self.kick.trigger_with_velocity(self.current_time, velocity);
        }
        if self.snare_trigger_pending.swap(false, Ordering::Acquire) {
            let velocity = f32::from_bits(self.snare_trigger_velocity.load(Ordering::Acquire));
            self.snare
                .trigger_with_velocity(self.current_time, velocity);
        }
        if self.hihat_trigger_pending.swap(false, Ordering::Acquire) {
            let velocity = f32::from_bits(self.hihat_trigger_velocity.load(Ordering::Acquire));
            self.hihat
                .trigger_with_velocity(self.current_time, velocity);
        }
        if self.tom_trigger_pending.swap(false, Ordering::Acquire) {
            let velocity = f32::from_bits(self.tom_trigger_velocity.load(Ordering::Acquire));
            self.tom.trigger_with_velocity(self.current_time, velocity);
        }

        let sample_period = 1.0 / self.sample_rate;

        // Update mute/solo gain targets (check once per buffer for efficiency)
        let any_soloed = self
            .instrument_soloed
            .iter()
            .any(|s| s.load(Ordering::Relaxed));
        for i in 0..NUM_INSTRUMENTS {
            let target = Self::calculate_instrument_gain(
                self.instrument_muted[i].load(Ordering::Relaxed),
                self.instrument_soloed[i].load(Ordering::Relaxed),
                any_soloed,
            );
            self.instrument_gains[i].set_target(target);
        }

        for sample in buffer.iter_mut() {
            // Tick ALL sequencers first to ensure sample-accurate synchronization
            // (if two instruments trigger on the same step, they fire at exactly the same sample)
            // Returns Option<(&str, f32)> with instrument name and velocity
            let kick_trigger = self.kick_sequencer.tick();
            let snare_trigger = self.snare_sequencer.tick();
            let hihat_trigger = self.hihat_sequencer.tick();
            let tom_trigger = self.tom_sequencer.tick();

            // Apply triggers with velocity after all sequencers have been ticked
            if let Some((_, velocity)) = kick_trigger {
                self.kick.trigger_with_velocity(self.current_time, velocity);
            }
            if let Some((_, velocity)) = snare_trigger {
                self.snare
                    .trigger_with_velocity(self.current_time, velocity);
            }
            if let Some((_, velocity)) = hihat_trigger {
                self.hihat
                    .trigger_with_velocity(self.current_time, velocity);
            }
            if let Some((_, velocity)) = tom_trigger {
                self.tom.trigger_with_velocity(self.current_time, velocity);
            }

            // Process LFOs and apply modulation to routed parameters
            // Use index-based iteration to avoid allocation on audio thread
            for lfo_idx in 0..LFO_COUNT {
                if self.lfo_enabled[lfo_idx] {
                    let lfo_value = self.lfos[lfo_idx].tick();
                    let route_count = self.lfo_routes[lfo_idx].len();

                    for route_idx in 0..route_count {
                        // Access route data with short-lived borrows
                        let instrument = self.lfo_routes[lfo_idx][route_idx].instrument;
                        let param = self.lfo_routes[lfo_idx][route_idx].param;
                        let depth = self.lfo_routes[lfo_idx][route_idx].depth;
                        let modulation = lfo_value * depth;
                        self.apply_modulation_by_index(instrument, param, modulation);
                    }
                }
            }

            // Generate and mix audio from all instruments with mute/solo gains
            let mut output = self.kick.tick(self.current_time)
                * self.instrument_gains[INSTRUMENT_KICK as usize].tick()
                + self.snare.tick(self.current_time)
                    * self.instrument_gains[INSTRUMENT_SNARE as usize].tick()
                + self.hihat.tick(self.current_time)
                    * self.instrument_gains[INSTRUMENT_HIHAT as usize].tick()
                + self.tom.tick(self.current_time)
                    * self.instrument_gains[INSTRUMENT_TOM as usize].tick();

            // Apply global effects chain
            // 1. Delay (if enabled)
            if self.delay_enabled {
                output = self.delay.process(output);
            }

            // 2. Lowpass filter (if enabled)
            if self.lowpass_filter_enabled {
                output = self.lowpass_filter.process(output);
            }

            // 3. Saturation (if enabled)
            if self.saturation_enabled {
                output = self.saturation.process(output);
            }

            // 4. Limiter (always on - protects output from clipping)
            *sample = self.limiter.process(output);

            self.current_time += sample_period;
        }
    }

    /// Apply LFO modulation to an instrument parameter by index
    /// This is used internally by the render loop to apply LFO values to routed parameters
    fn apply_modulation_by_index(&mut self, instrument: u32, param: u32, value: f32) {
        match instrument {
            INSTRUMENT_KICK => match param {
                KICK_PARAM_FREQUENCY => self.kick.params.frequency.set_bipolar(value),
                KICK_PARAM_PUNCH => self.kick.params.punch.set_bipolar(value),
                KICK_PARAM_SUB => self.kick.params.sub.set_bipolar(value),
                KICK_PARAM_CLICK => self.kick.params.click.set_bipolar(value),
                KICK_PARAM_DECAY => self.kick.params.oscillator_decay.set_bipolar(value),
                KICK_PARAM_PITCH_ENVELOPE => self.kick.params.pitch_envelope_amount.set_bipolar(value),
                KICK_PARAM_VOLUME => self.kick.params.volume.set_bipolar(value),
                _ => {}
            },
            INSTRUMENT_SNARE => match param {
                SNARE_PARAM_FREQUENCY => self.snare.params.frequency.set_bipolar(value),
                SNARE_PARAM_DECAY => self.snare.params.decay.set_bipolar(value),
                SNARE_PARAM_BRIGHTNESS => self.snare.params.brightness.set_bipolar(value),
                SNARE_PARAM_VOLUME => self.snare.params.volume.set_bipolar(value),
                SNARE_PARAM_TONAL => self.snare.params.tonal.set_bipolar(value),
                SNARE_PARAM_NOISE => self.snare.params.noise.set_bipolar(value),
                SNARE_PARAM_PITCH_DROP => self.snare.params.pitch_drop.set_bipolar(value),
                SNARE_PARAM_TONAL_DECAY => self.snare.params.tonal_decay.set_bipolar(value),
                SNARE_PARAM_NOISE_DECAY => self.snare.params.noise_decay.set_bipolar(value),
                SNARE_PARAM_NOISE_TAIL_DECAY => self.snare.params.noise_tail_decay.set_bipolar(value),
                SNARE_PARAM_FILTER_CUTOFF => self.snare.params.filter_cutoff.set_bipolar(value),
                SNARE_PARAM_FILTER_RESONANCE => self.snare.params.filter_resonance.set_bipolar(value),
                SNARE_PARAM_XFADE => self.snare.params.xfade.set_bipolar(value),
                SNARE_PARAM_PHASE_MOD_AMOUNT => self.snare.params.phase_mod_amount.set_bipolar(value),
                SNARE_PARAM_OVERDRIVE => self.snare.params.overdrive.set_bipolar(value),
                SNARE_PARAM_AMP_DECAY => self.snare.params.amp_decay.set_bipolar(value),
                SNARE_PARAM_AMP_DECAY_CURVE => self.snare.params.amp_decay_curve.set_bipolar(value),
                SNARE_PARAM_TONAL_DECAY_CURVE => self.snare.params.tonal_decay_curve.set_bipolar(value),
                _ => {}
            },
            INSTRUMENT_HIHAT => match param {
                HIHAT_PARAM_FILTER => self.hihat.params.filter.set_bipolar(value),
                HIHAT_PARAM_FREQUENCY => self.hihat.params.frequency.set_bipolar(value),
                HIHAT_PARAM_DECAY => self.hihat.params.decay.set_bipolar(value),
                HIHAT_PARAM_VOLUME => self.hihat.params.volume.set_bipolar(value),
                _ => {}
            },
            INSTRUMENT_TOM => {
                // Tom2 uses 0-100 range, FFI uses 0-1, so scale by 100
                let scaled = value * 100.0;
                match param {
                    TOM_PARAM_TUNE => self.tom.set_tune(scaled),
                    TOM_PARAM_BEND => self.tom.set_bend(scaled),
                    TOM_PARAM_TONE => self.tom.set_tone(scaled),
                    TOM_PARAM_COLOR => self.tom.set_color(scaled),
                    TOM_PARAM_DECAY => self.tom.set_decay(scaled),
                    TOM_PARAM_MEMBRANE => self.tom.set_membrane(scaled),
                    TOM_PARAM_MEMBRANE_Q => self.tom.set_membrane_q(scaled),
                    _ => {}
                }
            }
            _ => {}
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
/// Total number of global effects
pub const EFFECT_COUNT: u32 = 3;

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

/// Delay parameter: time in seconds (0.0-5.0)
pub const DELAY_PARAM_TIME: u32 = 0;
/// Delay parameter: feedback amount (0.0-0.95)
pub const DELAY_PARAM_FEEDBACK: u32 = 1;
/// Delay parameter: wet/dry mix (0.0-1.0)
pub const DELAY_PARAM_MIX: u32 = 2;

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

// =============================================================================
// Hi-hat parameter indices (must match Swift HiHatParam enum)
// =============================================================================

/// Hi-hat parameter: combined brightness + resonance control (0-1)
pub const HIHAT_PARAM_FILTER: u32 = 0;
/// Hi-hat parameter: filter cutoff frequency (4000-16000 Hz)
pub const HIHAT_PARAM_FREQUENCY: u32 = 1;
/// Hi-hat parameter: decay time (0.005-0.4 seconds)
pub const HIHAT_PARAM_DECAY: u32 = 2;
/// Hi-hat parameter: overall volume (0-1)
pub const HIHAT_PARAM_VOLUME: u32 = 3;

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
/// Total number of instruments
pub const INSTRUMENT_COUNT: u32 = 4;
/// Internal usize version for array indexing
const NUM_INSTRUMENTS: usize = INSTRUMENT_COUNT as usize;

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

/// Snare preset: Tight - short, punchy snare
pub const SNARE_PRESET_TIGHT: u32 = 0;
/// Snare preset: Loose - longer decay, more body
pub const SNARE_PRESET_LOOSE: u32 = 1;
/// Snare preset: Hiss - noise-focused with phase modulation
pub const SNARE_PRESET_HISS: u32 = 2;
/// Snare preset: Smack - DS-style transient with SVF noise
pub const SNARE_PRESET_SMACK: u32 = 3;

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

/// Render audio samples into the provided buffer
///
/// This is the main audio callback function. Call this from your audio thread
/// to generate audio samples.
///
/// # Arguments
/// * `engine` - Pointer to a GooeyEngine
/// * `buffer` - Pointer to a buffer of floats to fill with audio
/// * `frames` - Number of frames (samples) to render
///
/// # Safety
/// - `engine` must be a valid pointer returned by `gooey_engine_new`
/// - `buffer` must point to at least `frames` floats of allocated memory
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_render(
    engine: *mut GooeyEngine,
    buffer: *mut f32,
    frames: u32,
) {
    if engine.is_null() || buffer.is_null() {
        return;
    }

    let engine = &mut *engine;
    let buffer = slice::from_raw_parts_mut(buffer, frames as usize);
    engine.render(buffer);
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
        let vel_clamped = velocity.clamp(0.0, 1.0);
        match instrument {
            INSTRUMENT_KICK => {
                engine
                    .kick_trigger_velocity
                    .store(vel_clamped.to_bits(), Ordering::Release);
                engine.kick_trigger_pending.store(true, Ordering::Release);
            }
            INSTRUMENT_SNARE => {
                engine
                    .snare_trigger_velocity
                    .store(vel_clamped.to_bits(), Ordering::Release);
                engine.snare_trigger_pending.store(true, Ordering::Release);
            }
            INSTRUMENT_HIHAT => {
                engine
                    .hihat_trigger_velocity
                    .store(vel_clamped.to_bits(), Ordering::Release);
                engine.hihat_trigger_pending.store(true, Ordering::Release);
            }
            INSTRUMENT_TOM => {
                engine
                    .tom_trigger_velocity
                    .store(vel_clamped.to_bits(), Ordering::Release);
                engine.tom_trigger_pending.store(true, Ordering::Release);
            }
            _ => {} // Unknown instrument, ignore
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
        engine.kick_trigger_pending.store(true, Ordering::Release);
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

    // KickDrum's setters now handle smoothing internally
    match param {
        KICK_PARAM_FREQUENCY => engine.kick.set_frequency(value),
        KICK_PARAM_PUNCH => engine.kick.set_punch(value),
        KICK_PARAM_SUB => engine.kick.set_sub(value),
        KICK_PARAM_CLICK => engine.kick.set_click(value),
        KICK_PARAM_DECAY => engine.kick.set_oscillator_decay(value),
        KICK_PARAM_PITCH_ENVELOPE => engine.kick.set_pitch_envelope_amount(value),
        KICK_PARAM_VOLUME => engine.kick.set_volume(value),
        _ => {} // Unknown parameter, ignore
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
/// - 0 (FREQUENCY): 4000-16000 Hz - filter cutoff, lower values tame harshness
/// - 1 (BRIGHTNESS): 0-1 - high-frequency emphasis
/// - 2 (RESONANCE): 0-1 - filter resonance boost
/// - 3 (DECAY): 0.01-3.0 seconds
/// - 4 (ATTACK): 0.001-0.1 seconds
/// - 5 (VOLUME): 0-1
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

    // HiHat's setters now handle smoothing internally
    match param {
        HIHAT_PARAM_FILTER => engine.hihat.set_filter(value),
        HIHAT_PARAM_FREQUENCY => engine.hihat.set_frequency(value),
        HIHAT_PARAM_DECAY => engine.hihat.set_decay(value),
        HIHAT_PARAM_VOLUME => engine.hihat.set_volume(value),
        _ => {} // Unknown parameter, ignore
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

    // SnareDrum's setters now handle smoothing internally
    // All parameters use normalized 0-1 range
    match param {
        SNARE_PARAM_FREQUENCY => engine.snare.set_frequency(value),
        SNARE_PARAM_DECAY => engine.snare.set_decay(value),
        SNARE_PARAM_BRIGHTNESS => engine.snare.set_brightness(value),
        SNARE_PARAM_VOLUME => engine.snare.set_volume(value),
        SNARE_PARAM_TONAL => engine.snare.set_tonal(value),
        SNARE_PARAM_NOISE => engine.snare.set_noise(value),
        SNARE_PARAM_PITCH_DROP => engine.snare.set_pitch_drop(value),
        SNARE_PARAM_TONAL_DECAY => engine.snare.set_tonal_decay(value),
        SNARE_PARAM_NOISE_DECAY => engine.snare.set_noise_decay(value),
        SNARE_PARAM_NOISE_TAIL_DECAY => engine.snare.set_noise_tail_decay(value),
        SNARE_PARAM_FILTER_CUTOFF => engine.snare.set_filter_cutoff(value),
        SNARE_PARAM_FILTER_RESONANCE => engine.snare.set_filter_resonance(value),
        SNARE_PARAM_FILTER_TYPE => engine.snare.set_filter_type(value as u8),
        SNARE_PARAM_XFADE => engine.snare.set_xfade(value),
        SNARE_PARAM_PHASE_MOD_AMOUNT => engine.snare.set_phase_mod_amount(value),
        SNARE_PARAM_OVERDRIVE => engine.snare.set_overdrive(value),
        SNARE_PARAM_AMP_DECAY => engine.snare.set_amp_decay(value),
        SNARE_PARAM_AMP_DECAY_CURVE => engine.snare.set_amp_decay_curve(value),
        SNARE_PARAM_TONAL_DECAY_CURVE => engine.snare.set_tonal_decay_curve(value),
        _ => {} // Unknown parameter, ignore
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
///   - DELAY_PARAM_TIME (0): 0.0-5.0 seconds
///   - DELAY_PARAM_FEEDBACK (1): 0.0-0.95
///   - DELAY_PARAM_MIX (2): 0.0-1.0
/// - EFFECT_SATURATION (2):
///   - SATURATION_PARAM_DRIVE (0): 0.0-1.0
///   - SATURATION_PARAM_WARMTH (1): 0.0-1.0
///   - SATURATION_PARAM_MIX (2): 0.0-1.0
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
            DELAY_PARAM_TIME => engine.delay.set_time(value),
            DELAY_PARAM_FEEDBACK => engine.delay.set_feedback(value),
            DELAY_PARAM_MIX => engine.delay.set_mix(value),
            _ => {} // Unknown parameter, ignore
        },
        EFFECT_SATURATION => match param {
            SATURATION_PARAM_DRIVE => engine.saturation.set_drive(value),
            SATURATION_PARAM_WARMTH => engine.saturation.set_warmth(value),
            SATURATION_PARAM_MIX => engine.saturation.set_mix(value),
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
            DELAY_PARAM_TIME => engine.delay.get_time(),
            DELAY_PARAM_FEEDBACK => engine.delay.get_feedback(),
            DELAY_PARAM_MIX => engine.delay.get_mix(),
            _ => -1.0, // Unknown parameter
        },
        EFFECT_SATURATION => match param {
            SATURATION_PARAM_DRIVE => engine.saturation.get_drive(),
            SATURATION_PARAM_WARMTH => engine.saturation.get_warmth(),
            SATURATION_PARAM_MIX => engine.saturation.get_mix(),
            _ => -1.0, // Unknown parameter
        },
        _ => -1.0, // Unknown effect
    }
}

/// Enable or disable a global effect
///
/// When disabled, the effect is bypassed and does not process audio.
/// This is useful for A/B comparison or saving CPU when an effect is not needed.
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
        _ => false, // Unknown effect
    }
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
    engine.kick_sequencer.set_bpm(bpm);
    engine.snare_sequencer.set_bpm(bpm);
    engine.hihat_sequencer.set_bpm(bpm);
    engine.tom_sequencer.set_bpm(bpm);

    // Update LFO BPM values for BPM-synced LFOs
    for lfo in &mut engine.lfos {
        lfo.set_bpm(bpm);
    }
}

// =============================================================================
// Sequencer control (all instruments)
// =============================================================================

/// Start all sequencers
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_start(engine: *mut GooeyEngine) {
    if engine.is_null() {
        return;
    }

    let engine = &mut *engine;
    engine.kick_sequencer.start();
    engine.snare_sequencer.start();
    engine.hihat_sequencer.start();
    engine.tom_sequencer.start();
}

/// Stop all sequencers
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_stop(engine: *mut GooeyEngine) {
    if engine.is_null() {
        return;
    }

    let engine = &mut *engine;
    engine.kick_sequencer.stop();
    engine.snare_sequencer.stop();
    engine.hihat_sequencer.stop();
    engine.tom_sequencer.stop();
}

/// Reset all sequencers to step 0
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_reset(engine: *mut GooeyEngine) {
    if engine.is_null() {
        return;
    }

    let engine = &mut *engine;
    engine.kick_sequencer.reset();
    engine.snare_sequencer.reset();
    engine.hihat_sequencer.reset();
    engine.tom_sequencer.reset();
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
    engine.kick_sequencer.set_step(step as usize, enabled);
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
    if engine.kick_sequencer.is_running() {
        engine.kick_sequencer.current_step() as i32
    } else {
        -1
    }
}

/// Get the sequencer step that will be playing after a lookahead period
///
/// This compensates for audio buffer latency by looking ahead.
/// Use this for UI display to sync visuals with audio output.
/// Uses kick sequencer as reference (all sequencers are synchronized).
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
    if engine.kick_sequencer.is_running() {
        engine
            .kick_sequencer
            .step_at_lookahead(lookahead_samples as u64) as i32
    } else {
        -1
    }
}

// =============================================================================
// Per-instrument sequencer control
// =============================================================================

/// Helper to get a mutable reference to an instrument's sequencer
impl GooeyEngine {
    fn sequencer_for_instrument(&mut self, instrument: u32) -> Option<&mut Sequencer> {
        match instrument {
            INSTRUMENT_KICK => Some(&mut self.kick_sequencer),
            INSTRUMENT_SNARE => Some(&mut self.snare_sequencer),
            INSTRUMENT_HIHAT => Some(&mut self.hihat_sequencer),
            INSTRUMENT_TOM => Some(&mut self.tom_sequencer),
            _ => None,
        }
    }

    fn sequencer_for_instrument_ref(&self, instrument: u32) -> Option<&Sequencer> {
        match instrument {
            INSTRUMENT_KICK => Some(&self.kick_sequencer),
            INSTRUMENT_SNARE => Some(&self.snare_sequencer),
            INSTRUMENT_HIHAT => Some(&self.hihat_sequencer),
            INSTRUMENT_TOM => Some(&self.tom_sequencer),
            _ => None,
        }
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
    4
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
pub unsafe extern "C" fn gooey_engine_clear_lfo_routes(
    engine: *mut GooeyEngine,
    lfo_index: u32,
) {
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
pub unsafe extern "C" fn gooey_engine_reset_lfo_phase(
    engine: *mut GooeyEngine,
    lfo_index: u32,
) {
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
    19 // frequency, decay, brightness, volume, tonal, noise, pitch_drop,
       // tonal_decay, noise_decay, noise_tail_decay, filter_cutoff, filter_resonance,
       // filter_type, xfade, phase_mod_amount, overdrive, amp_decay, amp_decay_curve,
       // tonal_decay_curve
}

/// Get the number of tom parameters
#[no_mangle]
pub extern "C" fn gooey_engine_tom_param_count() -> u32 {
    7 // tune, bend, tone, color, decay, membrane, membrane_q (Tom2)
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
    if engine.is_null() || instrument as usize >= NUM_INSTRUMENTS {
        return;
    }
    (*engine).instrument_muted[instrument as usize].store(muted, Ordering::Release);
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
    if engine.is_null() || instrument as usize >= NUM_INSTRUMENTS {
        return false;
    }
    (*engine).instrument_muted[instrument as usize].load(Ordering::Acquire)
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
    if engine.is_null() || instrument as usize >= NUM_INSTRUMENTS {
        return;
    }
    (*engine).instrument_soloed[instrument as usize].store(soloed, Ordering::Release);
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
    if engine.is_null() || instrument as usize >= NUM_INSTRUMENTS {
        return false;
    }
    (*engine).instrument_soloed[instrument as usize].load(Ordering::Acquire)
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
    let idx = instrument as usize;

    if idx >= INSTRUMENT_COUNT as usize {
        return;
    }

    engine.blend_enabled[idx] = true;
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
    let idx = instrument as usize;

    if idx >= INSTRUMENT_COUNT as usize {
        return;
    }

    engine.blend_enabled[idx] = false;
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
    let idx = instrument as usize;

    if idx >= INSTRUMENT_COUNT as usize {
        return false;
    }

    engine.blend_enabled[idx]
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
    let idx = instrument as usize;

    if idx >= INSTRUMENT_COUNT as usize {
        return;
    }
    if !engine.blend_enabled[idx] {
        return;
    }

    engine.blend_x[idx] = x.clamp(0.0, 1.0);
    engine.blend_y[idx] = y.clamp(0.0, 1.0);

    match instrument {
        INSTRUMENT_KICK => {
            let blended = engine.kick_blender.blend(engine.blend_x[idx], engine.blend_y[idx]);
            engine.kick.set_config(blended);
        }
        INSTRUMENT_SNARE => {
            let blended = engine.snare_blender.blend(engine.blend_x[idx], engine.blend_y[idx]);
            engine.snare.set_config(blended);
        }
        INSTRUMENT_TOM => {
            let blended = engine.tom_blender.blend(engine.blend_x[idx], engine.blend_y[idx]);
            engine.tom.set_config(blended);
        }
        _ => {}
    }
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
    let idx = instrument as usize;

    if idx >= INSTRUMENT_COUNT as usize {
        return -1.0;
    }

    engine.blend_x[idx]
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
    let idx = instrument as usize;

    if idx >= INSTRUMENT_COUNT as usize {
        return -1.0;
    }

    engine.blend_y[idx]
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
    let idx = instrument as usize;
    let corner_idx = corner as usize;

    if idx >= INSTRUMENT_COUNT as usize {
        return;
    }
    if corner_idx >= 4 {
        return;
    }

    engine.blend_corner_presets[idx][corner_idx] = preset_id;

    match instrument {
        INSTRUMENT_KICK => {
            if let Some(config) = GooeyEngine::kick_preset_by_id(preset_id) {
                match corner {
                    BLEND_CORNER_BOTTOM_LEFT => engine.kick_blender.set_bottom_left(config),
                    BLEND_CORNER_BOTTOM_RIGHT => engine.kick_blender.set_bottom_right(config),
                    BLEND_CORNER_TOP_LEFT => engine.kick_blender.set_top_left(config),
                    BLEND_CORNER_TOP_RIGHT => engine.kick_blender.set_top_right(config),
                    _ => {}
                }
            }
        }
        INSTRUMENT_SNARE => {
            if let Some(config) = GooeyEngine::snare_preset_by_id(preset_id) {
                match corner {
                    BLEND_CORNER_BOTTOM_LEFT => engine.snare_blender.set_bottom_left(config),
                    BLEND_CORNER_BOTTOM_RIGHT => engine.snare_blender.set_bottom_right(config),
                    BLEND_CORNER_TOP_LEFT => engine.snare_blender.set_top_left(config),
                    BLEND_CORNER_TOP_RIGHT => engine.snare_blender.set_top_right(config),
                    _ => {}
                }
            }
        }
        INSTRUMENT_TOM => {
            if let Some(config) = GooeyEngine::tom_preset_by_id(preset_id) {
                match corner {
                    BLEND_CORNER_BOTTOM_LEFT => engine.tom_blender.set_bottom_left(config),
                    BLEND_CORNER_BOTTOM_RIGHT => engine.tom_blender.set_bottom_right(config),
                    BLEND_CORNER_TOP_LEFT => engine.tom_blender.set_top_left(config),
                    BLEND_CORNER_TOP_RIGHT => engine.tom_blender.set_top_right(config),
                    _ => {}
                }
            }
        }
        _ => {}
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
    let idx = instrument as usize;
    let corner_idx = corner as usize;

    if idx >= INSTRUMENT_COUNT as usize {
        return 0xFFFFFFFF;
    }
    if corner_idx >= 4 {
        return 0xFFFFFFFF;
    }

    engine.blend_corner_presets[idx][corner_idx]
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
    let idx = instrument as usize;

    if idx >= INSTRUMENT_COUNT as usize {
        return;
    }

    match instrument {
        INSTRUMENT_KICK => {
            engine.kick_blender = PresetBlender::new(
                KickConfig::tight(),
                KickConfig::punch(),
                KickConfig::loose(),
                KickConfig::dirt(),
            );
            engine.blend_corner_presets[idx] =
                [KICK_PRESET_TIGHT, KICK_PRESET_PUNCH, KICK_PRESET_LOOSE, KICK_PRESET_DIRT];
        }
        INSTRUMENT_SNARE => {
            engine.snare_blender = PresetBlender::new(
                SnareConfig::tight(),
                SnareConfig::loose(),
                SnareConfig::hiss(),
                SnareConfig::smack(),
            );
            engine.blend_corner_presets[idx] =
                [SNARE_PRESET_TIGHT, SNARE_PRESET_LOOSE, SNARE_PRESET_HISS, SNARE_PRESET_SMACK];
        }
        INSTRUMENT_TOM => {
            engine.tom_blender = PresetBlender::new(
                Tom2Config::derp(),
                Tom2Config::ring(),
                Tom2Config::brush(),
                Tom2Config::void_preset(),
            );
            engine.blend_corner_presets[idx] =
                [TOM_PRESET_DERP, TOM_PRESET_RING, TOM_PRESET_BRUSH, TOM_PRESET_VOID];
        }
        _ => {}
    }
}
