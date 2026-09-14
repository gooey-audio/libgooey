//! Gooey Kick POC shared DSP, UI, and VST3 implementation.

pub mod dsp;
pub mod editor;
pub mod params;
pub mod schedule;

#[cfg(feature = "vst3-plugin")]
mod vst3_plugin;

pub use dsp::KickAdapter;
pub use params::{AtomicParameters, ParamId, Parameters, PARAM_COUNT};
