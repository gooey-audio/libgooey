//! Shared audio engine logic for native (CPAL) and iOS targets

pub mod envelope;
pub mod filters;

// New organized modules
pub mod effects;
pub mod engine;
pub mod ffi;
pub mod gen;
pub mod instruments;
pub mod sequencer;
pub mod utils;

// Visualization module (optional)
#[cfg(feature = "visualization")]
pub mod visualization;
