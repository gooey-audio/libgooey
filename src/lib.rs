//! Shared audio engine logic for native (CPAL) and iOS targets

pub mod dsl;
pub mod envelope;
pub mod filters;
pub mod max_curve;

// New organized modules
pub mod effects;
pub mod engine;
pub mod ffi;
pub mod gen;
pub mod instruments;
pub mod music;
pub mod sequencer;
pub mod utils;

pub mod bounce;

// Visualization module (optional)
#[cfg(feature = "visualization")]
pub mod visualization;
