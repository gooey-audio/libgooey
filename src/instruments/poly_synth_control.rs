//! Cross-thread projection and render-boundary handoff for editable poly presets.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use super::{PolyModRoute, PolySynthConfig};

pub(crate) const POLY_PRESET_BANK_SIZE: usize = 5;

struct ControlState {
    projected: [PolySynthConfig; POLY_PRESET_BANK_SIZE],
    pending: [Option<PolySynthConfig>; POLY_PRESET_BANK_SIZE],
    pending_active: Option<u32>,
}

struct SharedControl {
    state: Mutex<ControlState>,
    active_preset: AtomicU32,
    has_pending: AtomicBool,
}

#[derive(Clone)]
pub(crate) struct PolySynthControl {
    shared: Arc<SharedControl>,
}

pub(crate) struct PolySynthPending {
    pub(crate) presets: [Option<PolySynthConfig>; POLY_PRESET_BANK_SIZE],
    pub(crate) active_preset: Option<u32>,
}

impl Default for PolySynthPending {
    fn default() -> Self {
        Self {
            presets: [None; POLY_PRESET_BANK_SIZE],
            active_preset: None,
        }
    }
}

impl PolySynthControl {
    pub(crate) fn new(
        presets: [PolySynthConfig; POLY_PRESET_BANK_SIZE],
        active_preset: u32,
    ) -> Self {
        Self {
            shared: Arc::new(SharedControl {
                state: Mutex::new(ControlState {
                    projected: presets,
                    pending: [None; POLY_PRESET_BANK_SIZE],
                    pending_active: None,
                }),
                active_preset: AtomicU32::new(active_preset),
                has_pending: AtomicBool::new(false),
            }),
        }
    }

    fn edit_preset(&self, preset: u32, edit: impl FnOnce(&mut PolySynthConfig) -> bool) -> bool {
        let Some(index) = valid_index(preset) else {
            return false;
        };
        let Ok(mut state) = self.shared.state.lock() else {
            return false;
        };
        let mut candidate = state.projected[index];
        if !edit(&mut candidate) {
            return false;
        }
        state.projected[index] = candidate;
        state.pending[index] = Some(candidate);
        self.shared.has_pending.store(true, Ordering::Release);
        true
    }

    pub(crate) fn set_active_preset(&self, preset: u32) -> bool {
        if valid_index(preset).is_none() {
            return false;
        }
        let Ok(mut state) = self.shared.state.lock() else {
            return false;
        };
        state.pending_active = Some(preset);
        self.shared.active_preset.store(preset, Ordering::Release);
        self.shared.has_pending.store(true, Ordering::Release);
        true
    }

    pub(crate) fn active_preset(&self) -> u32 {
        self.shared.active_preset.load(Ordering::Acquire)
    }

    /// Publish an audio-thread-selected preset (for example a clip event) to
    /// legacy getters. The atomic publication cannot be lost when the producer
    /// mutex is contended and never waits on the render path.
    pub(crate) fn publish_active_from_audio(&self, preset: u32) {
        debug_assert!(valid_index(preset).is_some());
        self.shared.active_preset.store(preset, Ordering::Release);
    }

    pub(crate) fn preset(&self, preset: u32) -> Option<PolySynthConfig> {
        let index = valid_index(preset)?;
        self.shared
            .state
            .lock()
            .ok()
            .map(|state| state.projected[index])
    }

    pub(crate) fn param(&self, preset: u32, param: u32) -> Option<f32> {
        self.preset(preset)?.param(param)
    }

    pub(crate) fn active_param(&self, param: u32) -> Option<f32> {
        let active_preset = self.active_preset();
        let index = valid_index(active_preset)?;
        let state = self.shared.state.lock().ok()?;
        state.projected[index].param(param)
    }

    pub(crate) fn set_param(&self, preset: u32, param: u32, value: f32) -> bool {
        self.edit_preset(preset, |config| config.set_param(param, value))
    }

    /// Update the host projection of the active preset without scheduling a
    /// render-side config apply. Used when something that already writes the
    /// live synth (a macro) must be reflected by getters and survive later
    /// whole-config applies from unrelated preset edits.
    pub(crate) fn set_projected_param(&self, param: u32, value: f32) -> bool {
        let Some(index) = valid_index(self.active_preset()) else {
            return false;
        };
        let Ok(mut state) = self.shared.state.lock() else {
            return false;
        };
        if !state.projected[index].set_param(param, value) {
            return false;
        }
        if let Some(pending) = state.pending[index].as_mut() {
            pending.set_param(param, value);
        }
        true
    }

    pub(crate) fn set_params(&self, preset: u32, values: &[(u32, f32)]) -> bool {
        self.edit_preset(preset, |config| {
            let mut candidate = *config;
            for &(param, value) in values {
                if !candidate.set_param(param, value) {
                    return false;
                }
            }
            *config = candidate;
            true
        })
    }

    pub(crate) fn reset_preset(&self, preset: u32, config: PolySynthConfig) -> bool {
        self.edit_preset(preset, |candidate| {
            *candidate = config;
            true
        })
    }

    pub(crate) fn set_mod_route(&self, preset: u32, slot: usize, route: PolyModRoute) -> bool {
        self.edit_preset(preset, |config| config.set_mod_route(slot, route))
    }

    pub(crate) fn mod_route(&self, preset: u32, slot: usize) -> Option<PolyModRoute> {
        self.preset(preset)?.mod_routes.get(slot).copied()
    }

    pub(crate) fn clear_mod_route(&self, preset: u32, slot: usize) -> bool {
        self.edit_preset(preset, |config| config.clear_mod_route(slot))
    }

    /// Copy all staged state into render-owned scratch. A contended producer
    /// mutex is never waited on; the complete batch remains pending.
    pub(crate) fn drain_into(&self, destination: &mut PolySynthPending) {
        if !self.shared.has_pending.load(Ordering::Acquire) {
            return;
        }
        let Ok(mut state) = self.shared.state.try_lock() else {
            return;
        };
        for (source, destination) in state.pending.iter_mut().zip(&mut destination.presets) {
            if source.is_some() {
                *destination = source.take();
            }
        }
        if state.pending_active.is_some() {
            destination.active_preset = state.pending_active.take();
        }
        self.shared.has_pending.store(false, Ordering::Release);
    }
}

fn valid_index(preset: u32) -> Option<usize> {
    (preset < POLY_PRESET_BANK_SIZE as u32).then_some(preset as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instruments::POLY_PARAM_VOLUME;

    fn presets() -> [PolySynthConfig; POLY_PRESET_BANK_SIZE] {
        [
            PolySynthConfig::default(),
            PolySynthConfig::pad(),
            PolySynthConfig::pluck(),
            PolySynthConfig::keys(),
            PolySynthConfig::strings(),
        ]
    }

    #[test]
    fn repeated_writes_coalesce_to_one_complete_config() {
        let control = PolySynthControl::new(presets(), 0);
        assert!(control.set_param(0, POLY_PARAM_VOLUME, 0.2));
        assert!(control.set_param(0, POLY_PARAM_VOLUME, 0.8));
        assert_eq!(control.param(0, POLY_PARAM_VOLUME), Some(0.8));

        let mut pending = PolySynthPending::default();
        control.drain_into(&mut pending);
        assert_eq!(
            pending.presets[0].unwrap().param(POLY_PARAM_VOLUME),
            Some(0.8)
        );
        assert!(pending.presets[1..].iter().all(Option::is_none));
    }

    #[test]
    fn batch_validation_is_atomic_and_duplicates_are_last_write_wins() {
        let control = PolySynthControl::new(presets(), 0);
        let before = control.param(0, POLY_PARAM_VOLUME).unwrap();
        assert!(!control.set_params(0, &[(POLY_PARAM_VOLUME, 0.2), (u32::MAX, 0.4)]));
        assert_eq!(control.param(0, POLY_PARAM_VOLUME), Some(before));
        assert!(control.set_params(0, &[(POLY_PARAM_VOLUME, 0.2), (POLY_PARAM_VOLUME, 0.7)]));
        assert_eq!(control.param(0, POLY_PARAM_VOLUME), Some(0.7));
    }

    #[test]
    fn contended_render_drain_defers_without_modifying_scratch() {
        let control = PolySynthControl::new(presets(), 0);
        assert!(control.set_param(0, POLY_PARAM_VOLUME, 0.3));
        let _guard = control.shared.state.lock().unwrap();
        let mut pending = PolySynthPending::default();
        control.drain_into(&mut pending);
        assert!(pending.presets.iter().all(Option::is_none));
    }

    #[test]
    fn audio_selected_preset_publication_survives_control_lock_contention() {
        let control = PolySynthControl::new(presets(), 0);
        let guard = control.shared.state.lock().unwrap();

        control.publish_active_from_audio(2);
        assert_eq!(control.active_preset(), 2);

        drop(guard);
        assert!(control.set_param(control.active_preset(), POLY_PARAM_VOLUME, 0.37));
        assert_eq!(control.param(2, POLY_PARAM_VOLUME), Some(0.37));
        assert_ne!(control.param(0, POLY_PARAM_VOLUME), Some(0.37));
    }
}
