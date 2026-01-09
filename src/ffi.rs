//! C FFI bindings for the gooey audio engine
//!
//! This module exposes the audio engine to C/Swift via C-compatible functions.
//! Designed for integration with iOS (and other platforms in the future).

use crate::effects::{BrickWallLimiter, Effect};
use crate::engine::Sequencer;
use crate::instruments::{HiHat, KickDrum, SnareDrum, TomDrum};
use std::slice;
use std::sync::atomic::{AtomicBool, Ordering};

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
    tom: TomDrum,

    // Per-instrument sequencers (sample-accurate, synchronized)
    kick_sequencer: Sequencer,
    snare_sequencer: Sequencer,
    hihat_sequencer: Sequencer,
    tom_sequencer: Sequencer,

    // Effects
    limiter: BrickWallLimiter,

    // Engine state
    sample_rate: f32,
    bpm: f32,
    current_time: f32,

    // Per-instrument manual trigger flags
    kick_trigger_pending: AtomicBool,
    snare_trigger_pending: AtomicBool,
    hihat_trigger_pending: AtomicBool,
    tom_trigger_pending: AtomicBool,
}

impl GooeyEngine {
    fn new(sample_rate: f32) -> Self {
        let bpm = 120.0;

        // Create all instruments
        let kick = KickDrum::new(sample_rate);
        let snare = SnareDrum::new(sample_rate);
        let hihat = HiHat::new(sample_rate);
        let tom = TomDrum::new(sample_rate);

        // Create a 16-step sequencer for each instrument
        // All use 16th notes at default 120 BPM, starting with all steps off
        let kick_sequencer = Sequencer::with_pattern(bpm, sample_rate, vec![false; 16], "kick");
        let snare_sequencer = Sequencer::with_pattern(bpm, sample_rate, vec![false; 16], "snare");
        let hihat_sequencer = Sequencer::with_pattern(bpm, sample_rate, vec![false; 16], "hihat");
        let tom_sequencer = Sequencer::with_pattern(bpm, sample_rate, vec![false; 16], "tom");

        Self {
            kick,
            snare,
            hihat,
            tom,
            kick_sequencer,
            snare_sequencer,
            hihat_sequencer,
            tom_sequencer,
            limiter: BrickWallLimiter::new(1.0),
            sample_rate,
            bpm,
            current_time: 0.0,
            kick_trigger_pending: AtomicBool::new(false),
            snare_trigger_pending: AtomicBool::new(false),
            hihat_trigger_pending: AtomicBool::new(false),
            tom_trigger_pending: AtomicBool::new(false),
        }
    }

    fn render(&mut self, buffer: &mut [f32]) {
        // Check for pending manual triggers (all instruments)
        if self.kick_trigger_pending.swap(false, Ordering::Acquire) {
            self.kick.trigger(self.current_time);
        }
        if self.snare_trigger_pending.swap(false, Ordering::Acquire) {
            self.snare.trigger(self.current_time);
        }
        if self.hihat_trigger_pending.swap(false, Ordering::Acquire) {
            self.hihat.trigger(self.current_time);
        }
        if self.tom_trigger_pending.swap(false, Ordering::Acquire) {
            self.tom.trigger(self.current_time);
        }

        let sample_period = 1.0 / self.sample_rate;

        for sample in buffer.iter_mut() {
            // Tick ALL sequencers first to ensure sample-accurate synchronization
            // (if two instruments trigger on the same step, they fire at exactly the same sample)
            let kick_trigger = self.kick_sequencer.tick().is_some();
            let snare_trigger = self.snare_sequencer.tick().is_some();
            let hihat_trigger = self.hihat_sequencer.tick().is_some();
            let tom_trigger = self.tom_sequencer.tick().is_some();

            // Apply triggers after all sequencers have been ticked
            if kick_trigger {
                self.kick.trigger(self.current_time);
            }
            if snare_trigger {
                self.snare.trigger(self.current_time);
            }
            if hihat_trigger {
                self.hihat.trigger(self.current_time);
            }
            if tom_trigger {
                self.tom.trigger(self.current_time);
            }

            // Generate and mix audio from all instruments
            let output = self.kick.tick(self.current_time)
                + self.snare.tick(self.current_time)
                + self.hihat.tick(self.current_time)
                + self.tom.tick(self.current_time);

            // Apply limiter to prevent clipping
            *sample = self.limiter.process(output);

            self.current_time += sample_period;
        }
    }
}

// =============================================================================
// Kick drum parameter indices (must match Swift KickParam enum)
// =============================================================================

/// Kick parameter: base frequency (20-200 Hz)
pub const KICK_PARAM_FREQUENCY: u32 = 0;
/// Kick parameter: punch/mid presence (0-1)
pub const KICK_PARAM_PUNCH: u32 = 1;
/// Kick parameter: sub bass presence (0-1)
pub const KICK_PARAM_SUB: u32 = 2;
/// Kick parameter: click/transient amount (0-1)
pub const KICK_PARAM_CLICK: u32 = 3;
/// Kick parameter: decay time (0.01-5.0 seconds)
pub const KICK_PARAM_DECAY: u32 = 4;
/// Kick parameter: pitch drop amount (0-1)
pub const KICK_PARAM_PITCH_DROP: u32 = 5;
/// Kick parameter: overall volume (0-1)
pub const KICK_PARAM_VOLUME: u32 = 6;

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

/// Trigger any instrument manually by ID
///
/// Use this for manual triggering outside of the sequencer (e.g., user tap).
/// The trigger will be processed on the next call to `gooey_engine_render`.
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
    if let Some(engine) = engine.as_ref() {
        match instrument {
            INSTRUMENT_KICK => engine.kick_trigger_pending.store(true, Ordering::Release),
            INSTRUMENT_SNARE => engine.snare_trigger_pending.store(true, Ordering::Release),
            INSTRUMENT_HIHAT => engine.hihat_trigger_pending.store(true, Ordering::Release),
            INSTRUMENT_TOM => engine.tom_trigger_pending.store(true, Ordering::Release),
            _ => {} // Unknown instrument, ignore
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
/// - 0 (FREQUENCY): 20-200 Hz
/// - 1 (PUNCH): 0-1
/// - 2 (SUB): 0-1
/// - 3 (CLICK): 0-1
/// - 4 (DECAY): 0.01-5.0 seconds
/// - 5 (PITCH_DROP): 0-1
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
        KICK_PARAM_DECAY => engine.kick.set_decay(value),
        KICK_PARAM_PITCH_DROP => engine.kick.set_pitch_drop(value),
        KICK_PARAM_VOLUME => engine.kick.set_volume(value),
        _ => {} // Unknown parameter, ignore
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

// =============================================================================
// Utility functions
// =============================================================================

/// Get the number of kick parameters
#[no_mangle]
pub extern "C" fn gooey_engine_kick_param_count() -> u32 {
    7
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
