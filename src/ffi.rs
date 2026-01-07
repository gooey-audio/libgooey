//! C FFI bindings for the gooey audio engine
//!
//! This module exposes the audio engine to C/Swift via C-compatible functions.
//! Designed for integration with iOS (and other platforms in the future).

use crate::effects::{BrickWallLimiter, Effect};
use crate::engine::Sequencer;
use crate::instruments::KickDrum;
use std::slice;
use std::sync::atomic::{AtomicBool, Ordering};

/// Opaque wrapper around the audio engine for FFI
/// 
/// This struct provides a simplified C-compatible interface for iOS integration.
/// It manages a kick drum instrument and a 16-step sequencer with sample-accurate timing.
pub struct GooeyEngine {
    kick: KickDrum,
    sequencer: Sequencer,
    limiter: BrickWallLimiter,
    sample_rate: f32,
    bpm: f32,
    current_time: f32,
    trigger_pending: AtomicBool,
}

impl GooeyEngine {
    fn new(sample_rate: f32) -> Self {
        let kick = KickDrum::new(sample_rate);
        
        // Create a 16-step sequencer targeting the kick drum
        // Uses 16th notes at default 120 BPM
        let sequencer = Sequencer::with_pattern(
            120.0,
            sample_rate,
            vec![false; 16], // Start with all steps off
            "kick",
        );
        
        Self {
            kick,
            sequencer,
            limiter: BrickWallLimiter::new(1.0),
            sample_rate,
            bpm: 120.0,
            current_time: 0.0,
            trigger_pending: AtomicBool::new(false),
        }
    }
    
    fn render(&mut self, buffer: &mut [f32]) {
        // Check for pending manual trigger
        if self.trigger_pending.swap(false, Ordering::Acquire) {
            self.kick.trigger(self.current_time);
        }

        let sample_period = 1.0 / self.sample_rate;
        
        for sample in buffer.iter_mut() {
            // Check if sequencer wants to trigger
            if let Some(_instrument_name) = self.sequencer.tick() {
                self.kick.trigger(self.current_time);
            }
            
            // Generate audio from kick
            let raw_output = self.kick.tick(self.current_time);
            
            // Apply limiter
            *sample = self.limiter.process(raw_output);
            
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
// Kick drum control
// =============================================================================

/// Trigger the kick drum manually
///
/// Use this for manual triggering outside of the sequencer (e.g., user tap).
/// The trigger will be processed on the next call to `gooey_engine_render`.
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_trigger_kick(engine: *mut GooeyEngine) {
    if let Some(engine) = engine.as_ref() {
        engine.trigger_pending.store(true, Ordering::Release);
    }
}

/// Set a kick drum parameter
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
/// This affects the sequencer timing.
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
    engine.sequencer.set_bpm(bpm);
}

// =============================================================================
// Sequencer control
// =============================================================================

/// Start the sequencer
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_start(engine: *mut GooeyEngine) {
    if engine.is_null() {
        return;
    }

    let engine = &mut *engine;
    engine.sequencer.start();
}

/// Stop the sequencer
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_stop(engine: *mut GooeyEngine) {
    if engine.is_null() {
        return;
    }

    let engine = &mut *engine;
    engine.sequencer.stop();
}

/// Set a sequencer step on or off
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
    engine.sequencer.set_step(step as usize, enabled);
}

/// Get the current sequencer step
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
    if engine.sequencer.is_running() {
        engine.sequencer.current_step() as i32
    } else {
        -1
    }
}

/// Reset the sequencer to step 0
///
/// # Safety
/// `engine` must be a valid pointer returned by `gooey_engine_new`
#[no_mangle]
pub unsafe extern "C" fn gooey_engine_sequencer_reset(engine: *mut GooeyEngine) {
    if engine.is_null() {
        return;
    }

    let engine = &mut *engine;
    engine.sequencer.reset();
}

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
