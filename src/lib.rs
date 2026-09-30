//! Shared audio engine logic for native (CPAL) and iOS targets

pub mod automation;
pub mod dsl;
pub mod envelope;
pub mod filters;
pub mod max_curve;
pub mod metronome;

// New organized modules
pub mod effects;
pub mod engine;
pub mod ffi;
pub mod frame;
pub mod gen;
pub mod instruments;
pub(crate) mod live_control;
pub mod mixer;
pub mod music;
pub mod performance;
pub mod sequencer;
pub mod utils;

pub mod bounce;
#[cfg(feature = "studio")]
pub mod studio;

pub use frame::StereoFrame;

#[cfg(feature = "gui")]
pub mod gui;

// Visualization module (optional)
#[cfg(feature = "visualization")]
pub mod visualization;
