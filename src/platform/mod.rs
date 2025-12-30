/// Platform abstraction for audio output
/// This module provides a unified interface for audio playback across different platforms
/// (native CPAL, web audio, iOS, etc.)

use crate::stage::Stage;
use std::sync::{Arc, Mutex};

/// Trait for platform-specific audio output implementations
pub trait AudioOutput {
    /// Initialize the audio output with the given sample rate
    fn initialize(&mut self, sample_rate: f32) -> Result<(), anyhow::Error>;
    
    /// Start the audio stream
    fn start(&mut self) -> Result<(), anyhow::Error>;
    
    /// Stop the audio stream
    fn stop(&mut self) -> Result<(), anyhow::Error>;
    
    /// Get the current sample rate
    fn sample_rate(&self) -> f32;
    
    /// Check if the audio output is active
    fn is_active(&self) -> bool;
}

/// Shared audio state for communication between main thread and audio callback
pub struct AudioState {
    pub should_trigger: bool,
    pub trigger_time: f32,
}

impl AudioState {
    pub fn new() -> Self {
        Self {
            should_trigger: false,
            trigger_time: 0.0,
        }
    }
}

/// Audio engine that connects a Stage to platform-specific audio output
pub struct AudioEngine {
    stage: Arc<Mutex<Stage>>,
    audio_state: Arc<Mutex<AudioState>>,
    sample_rate: f32,
}

impl AudioEngine {
    /// Create a new audio engine with the given sample rate
    pub fn new(sample_rate: f32) -> Self {
        Self {
            stage: Arc::new(Mutex::new(Stage::new(sample_rate))),
            audio_state: Arc::new(Mutex::new(AudioState::new())),
            sample_rate,
        }
    }
    
    /// Get the stage for use with audio output
    pub fn stage(&self) -> Arc<Mutex<Stage>> {
        self.stage.clone()
    }
    
    /// Get the audio state for triggering from other threads
    pub fn audio_state(&self) -> Arc<Mutex<AudioState>> {
        self.audio_state.clone()
    }
    
    /// Trigger all instruments in the stage
    pub fn trigger_all(&self) {
        let mut state = self.audio_state.lock().unwrap();
        state.should_trigger = true;
    }
    
    /// Get the current sample rate
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }
    
    /// Modify the stage (for configuration)
    pub fn with_stage<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Stage) -> R,
    {
        let mut stage = self.stage.lock().unwrap();
        f(&mut stage)
    }
    
    /// Get a locked reference to the stage for direct access
    pub fn stage_mut(&self) -> std::sync::MutexGuard<Stage> {
        self.stage.lock().unwrap()
    }
}

// Platform-specific implementations
#[cfg(feature = "native")]
pub mod cpal_output;

// #[cfg(feature = "web")]
// pub mod web_output;

// Re-export platform-specific types
#[cfg(feature = "native")]
pub use self::cpal_output::CpalOutput;

// #[cfg(feature = "web")]
// pub use web_output::WebOutput;