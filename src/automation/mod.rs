//! Macros and motions.
//!
//! A macro is one 0-1 control connected to many parameters (see
//! [`macros`]). A motion is a one-shot, retriggerable automation of a macro's
//! value over a beat- or time-based duration (see [`motion`]). The engine
//! owning the parameters decides what a [`ParamTarget`] addresses and performs
//! the writes; this module is allocation-free and engine-agnostic.

pub(crate) mod control;
pub mod macros;
pub mod motion;

pub use macros::{
    active_definition, MacroBank, MacroDefinition, MacroMapping, ParamTarget, MACRO_COUNT,
    MACRO_MAX_MAPPINGS,
};
pub use motion::{
    MotionClock, MotionCurve, MotionDefinition, MotionDuration, MotionEndMode, MotionPhase,
    MotionQuantize, MotionRunner, MOTION_SLOT_COUNT,
};
