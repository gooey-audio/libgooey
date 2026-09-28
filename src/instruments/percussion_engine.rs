//! A stable selector over the available resonant percussion architectures.
//!
//! Musical roles such as kick and snare are presets. `PercussionEngineKind`
//! chooses the signal topology that interprets those presets.

use crate::engine::{Instrument, Modulatable};
use crate::instruments::{
    ResonatorVoice, ResonatorVoiceConfig, TwinCorePercConfig, TwinCorePercVoice,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PercussionEngineKind {
    RoutingMatrix = 0,
    TwinCore = 1,
}

impl PercussionEngineKind {
    pub const ALL: [Self; 2] = [Self::RoutingMatrix, Self::TwinCore];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::RoutingMatrix => "Routing Matrix",
            Self::TwinCore => "Twin Core",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PercussionPreset {
    Kick = 0,
    Tom = 1,
    Snare = 2,
    ClapHybrid = 3,
    Metallic = 4,
}

impl PercussionPreset {
    pub const ALL: [Self; 5] = [
        Self::Kick,
        Self::Tom,
        Self::Snare,
        Self::ClapHybrid,
        Self::Metallic,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Kick => "Kick",
            Self::Tom => "Tom",
            Self::Snare => "Snare",
            Self::ClapHybrid => "Clap / Hybrid",
            Self::Metallic => "Metallic",
        }
    }
}

enum PercussionEngineVoice {
    RoutingMatrix(ResonatorVoice),
    TwinCore(TwinCorePercVoice),
}

pub struct PercussionEngine {
    sample_rate: f32,
    kind: PercussionEngineKind,
    preset: PercussionPreset,
    voice: PercussionEngineVoice,
}

impl PercussionEngine {
    const ROUTING_MATRIX_PARAMETERS: [&'static str; 9] = [
        "pitch",
        "pitch_sweep",
        "sweep_time",
        "decay",
        "body_character",
        "noise",
        "coupling",
        "drive",
        "volume",
    ];
    const TWIN_CORE_PARAMETERS: [&'static str; 12] = [
        "tune",
        "detune",
        "length",
        "body_bias",
        "fm_decay",
        "fm_depth",
        "trigger_delay",
        "harmonics",
        "noise_filter",
        "noise_decay",
        "noise_bias",
        "volume",
    ];

    pub fn new(sample_rate: f32) -> Self {
        Self::with_selection(
            sample_rate,
            PercussionEngineKind::RoutingMatrix,
            PercussionPreset::Kick,
        )
    }

    pub fn with_selection(
        sample_rate: f32,
        kind: PercussionEngineKind,
        preset: PercussionPreset,
    ) -> Self {
        let sample_rate = sample_rate.max(1.0);
        Self {
            sample_rate,
            kind,
            preset,
            voice: Self::make_voice(sample_rate, kind, preset),
        }
    }

    pub fn kind(&self) -> PercussionEngineKind {
        self.kind
    }

    pub fn preset(&self) -> PercussionPreset {
        self.preset
    }

    pub fn select_engine(&mut self, kind: PercussionEngineKind) {
        if kind != self.kind {
            self.kind = kind;
            self.voice = Self::make_voice(self.sample_rate, kind, self.preset);
        }
    }

    pub fn select_preset(&mut self, preset: PercussionPreset) {
        self.preset = preset;
        self.voice = Self::make_voice(self.sample_rate, self.kind, preset);
    }

    pub fn parameter_count(&self) -> usize {
        match self.kind {
            PercussionEngineKind::RoutingMatrix => Self::ROUTING_MATRIX_PARAMETERS.len(),
            PercussionEngineKind::TwinCore => Self::TWIN_CORE_PARAMETERS.len(),
        }
    }

    pub fn parameter_name(&self, index: usize) -> Option<&'static str> {
        match self.kind {
            PercussionEngineKind::RoutingMatrix => {
                Self::ROUTING_MATRIX_PARAMETERS.get(index).copied()
            }
            PercussionEngineKind::TwinCore => Self::TWIN_CORE_PARAMETERS.get(index).copied(),
        }
    }

    pub fn parameter_normalized(&self, index: usize) -> Option<f32> {
        match &self.voice {
            PercussionEngineVoice::RoutingMatrix(voice) => Some(match index {
                0 => voice.params.pitch.target(),
                1 => voice.params.pitch_sweep.target(),
                2 => voice.params.sweep_time.target(),
                3 => voice.params.decay.target(),
                4 => voice.params.body_character.target(),
                5 => voice.params.noise.target(),
                6 => voice.params.coupling.target(),
                7 => voice.params.drive.target(),
                8 => voice.params.volume.target(),
                _ => return None,
            }),
            PercussionEngineVoice::TwinCore(voice) => voice.parameter_normalized(index),
        }
    }

    pub fn set_parameter_normalized(&mut self, index: usize, value: f32) -> bool {
        if !value.is_finite() {
            return false;
        }
        let routing_name = Self::ROUTING_MATRIX_PARAMETERS.get(index).copied();
        match &mut self.voice {
            PercussionEngineVoice::RoutingMatrix(voice) => {
                routing_name.is_some_and(|name| voice.set_macro(name, value).is_ok())
            }
            PercussionEngineVoice::TwinCore(voice) => {
                if index >= Self::TWIN_CORE_PARAMETERS.len() {
                    return false;
                }
                voice.set_parameter_normalized(index, value);
                true
            }
        }
    }

    pub fn routing_matrix(&self) -> Option<&ResonatorVoice> {
        match &self.voice {
            PercussionEngineVoice::RoutingMatrix(voice) => Some(voice),
            PercussionEngineVoice::TwinCore(_) => None,
        }
    }

    pub fn routing_matrix_mut(&mut self) -> Option<&mut ResonatorVoice> {
        match &mut self.voice {
            PercussionEngineVoice::RoutingMatrix(voice) => Some(voice),
            PercussionEngineVoice::TwinCore(_) => None,
        }
    }

    pub fn twin_core(&self) -> Option<&TwinCorePercVoice> {
        match &self.voice {
            PercussionEngineVoice::TwinCore(voice) => Some(voice),
            PercussionEngineVoice::RoutingMatrix(_) => None,
        }
    }

    pub fn twin_core_mut(&mut self) -> Option<&mut TwinCorePercVoice> {
        match &mut self.voice {
            PercussionEngineVoice::TwinCore(voice) => Some(voice),
            PercussionEngineVoice::RoutingMatrix(_) => None,
        }
    }

    pub fn reset(&mut self) {
        match &mut self.voice {
            PercussionEngineVoice::RoutingMatrix(voice) => voice.reset(),
            PercussionEngineVoice::TwinCore(voice) => voice.reset(),
        }
    }

    fn make_voice(
        sample_rate: f32,
        kind: PercussionEngineKind,
        preset: PercussionPreset,
    ) -> PercussionEngineVoice {
        match kind {
            PercussionEngineKind::RoutingMatrix => {
                let config = match preset {
                    PercussionPreset::Kick => ResonatorVoiceConfig::kick(),
                    PercussionPreset::Tom => ResonatorVoiceConfig::tom(),
                    PercussionPreset::Snare => ResonatorVoiceConfig::snare(),
                    PercussionPreset::ClapHybrid => ResonatorVoiceConfig::hybrid(),
                    PercussionPreset::Metallic => ResonatorVoiceConfig::metallic_drone(),
                };
                PercussionEngineVoice::RoutingMatrix(ResonatorVoice::with_config(
                    sample_rate,
                    config,
                ))
            }
            PercussionEngineKind::TwinCore => {
                let config = match preset {
                    PercussionPreset::Kick => TwinCorePercConfig::kick(),
                    PercussionPreset::Tom => TwinCorePercConfig::tom(),
                    PercussionPreset::Snare => TwinCorePercConfig::snare(),
                    PercussionPreset::ClapHybrid => TwinCorePercConfig::clap(),
                    PercussionPreset::Metallic => TwinCorePercConfig::metallic(),
                };
                PercussionEngineVoice::TwinCore(TwinCorePercVoice::with_config(sample_rate, config))
            }
        }
    }
}

impl Instrument for PercussionEngine {
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        match &mut self.voice {
            PercussionEngineVoice::RoutingMatrix(voice) => {
                voice.trigger_with_velocity(time, velocity)
            }
            PercussionEngineVoice::TwinCore(voice) => voice.trigger_with_velocity(time, velocity),
        }
    }

    fn tick(&mut self, current_time: f64) -> f32 {
        match &mut self.voice {
            PercussionEngineVoice::RoutingMatrix(voice) => voice.tick(current_time),
            PercussionEngineVoice::TwinCore(voice) => voice.tick(current_time),
        }
    }

    fn is_active(&self) -> bool {
        match &self.voice {
            PercussionEngineVoice::RoutingMatrix(voice) => voice.is_active(),
            PercussionEngineVoice::TwinCore(voice) => voice.is_active(),
        }
    }

    fn set_midi_note(&mut self, note: u8) {
        match &mut self.voice {
            PercussionEngineVoice::RoutingMatrix(voice) => voice.set_midi_note(note),
            PercussionEngineVoice::TwinCore(voice) => voice.set_midi_note(note),
        }
    }

    fn as_modulatable(&mut self) -> Option<&mut dyn Modulatable> {
        Some(self)
    }
}

impl Modulatable for PercussionEngine {
    fn modulatable_parameters(&self) -> Vec<&'static str> {
        match &self.voice {
            PercussionEngineVoice::RoutingMatrix(voice) => voice.modulatable_parameters(),
            PercussionEngineVoice::TwinCore(voice) => voice.modulatable_parameters(),
        }
    }

    fn apply_modulation(&mut self, parameter: &str, value: f32) -> Result<(), String> {
        match &mut self.voice {
            PercussionEngineVoice::RoutingMatrix(voice) => voice.apply_modulation(parameter, value),
            PercussionEngineVoice::TwinCore(voice) => voice.apply_modulation(parameter, value),
        }
    }

    fn parameter_range(&self, parameter: &str) -> Option<(f32, f32)> {
        match &self.voice {
            PercussionEngineVoice::RoutingMatrix(voice) => voice.parameter_range(parameter),
            PercussionEngineVoice::TwinCore(voice) => voice.parameter_range(parameter),
        }
    }
}
