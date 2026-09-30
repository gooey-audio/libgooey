use super::{AudioHealth, BlockRenderer, EngineRenderer, GuiPanel};
use crate::effects::*;
use crate::engine::{Engine, Instrument, Lfo, Modulatable, Sequencer};
use crate::instruments::*;
use eframe::egui;
use std::sync::{Arc, Mutex};

const INSTRUMENTS: [&str; 8] = [
    "Kick",
    "Snare",
    "HiHat / HiHat2",
    "Tom",
    "Tom2",
    "Bass",
    "ResoKick",
    "Membrane",
];
const EFFECTS: [&str; 7] = [
    "Dry",
    "Delay",
    "Reverb",
    "Lowpass",
    "Saturation",
    "Tilt",
    "Plate",
];
struct Membrane {
    envelope: crate::max_curve::MaxCurveEnvelope,
    resonator: crate::filters::MembraneResonator,
    active: bool,
    noise: u32,
}
impl Membrane {
    fn new(rate: f32) -> Self {
        let mut resonator = crate::filters::MembraneResonator::new(rate);
        resonator.set_q_scale(0.01);
        resonator.set_gain_scale(0.001);
        Self {
            envelope: crate::max_curve::MaxCurveEnvelope::new(vec![
                (1.0, 5.0, 0.8),
                (0.0, 2000.0, -0.83),
            ]),
            resonator,
            active: false,
            noise: 1,
        }
    }
}
impl Instrument for Membrane {
    fn trigger_with_velocity(&mut self, time: f64, _: f32) {
        self.active = true;
        self.envelope.trigger(time);
        self.resonator.reset();
    }
    fn tick(&mut self, time: f64) -> f32 {
        if !self.active {
            return 0.0;
        }
        let env = self.envelope.get_value(time);
        self.active = !self.envelope.is_complete();
        self.noise ^= self.noise << 13;
        self.noise ^= self.noise >> 17;
        self.noise ^= self.noise << 5;
        self.resonator
            .process((self.noise as f32 / u32::MAX as f32 * 2.0 - 1.0) * env * 0.99)
    }
    fn is_active(&self) -> bool {
        self.active
    }
    fn as_modulatable(&mut self) -> Option<&mut dyn Modulatable> {
        Some(self)
    }
}
impl Modulatable for Membrane {
    fn modulatable_parameters(&self) -> Vec<&'static str> {
        vec!["q_scale", "gain_scale"]
    }
    fn apply_modulation(&mut self, name: &str, value: f32) -> Result<(), String> {
        let value = (value + 1.0) * 0.5;
        match name {
            "q_scale" => self.resonator.set_q_scale(0.001 + value * 0.099),
            "gain_scale" => self.resonator.set_gain_scale(0.0001 + value * 0.0099),
            _ => return Err(format!("Unknown membrane parameter {name}")),
        }
        Ok(())
    }
    fn parameter_range(&self, _: &str) -> Option<(f32, f32)> {
        Some((0.0, 1.0))
    }
}
fn instrument(index: usize, rate: f32) -> Box<dyn Instrument> {
    match index {
        0 => Box::new(KickDrum::new(rate)),
        1 => Box::new(SnareDrum::new(rate)),
        2 => Box::new(HiHat2::new(rate)),
        3 => Box::new(TomDrum::new(rate)),
        4 => Box::new(Tom2::new(rate)),
        5 => Box::new(BassSynth::new(rate)),
        6 => Box::new(ResoKick::new(rate)),
        _ => Box::new(Membrane::new(rate)),
    }
}
pub struct Experiments {
    engine: Arc<Mutex<Engine>>,
    rate: f32,
    selected: usize,
    params: Vec<(&'static str, f32)>,
    edits: std::collections::BTreeMap<&'static str, f32>,
    effect: usize,
    mix: f32,
    feedback: f32,
    damping: f32,
    cutoff: f32,
    delay_timing: u32,
    pingpong: bool,
    predelay: f32,
    width: f32,
    size: f32,
    bpm: f32,
    gain: f32,
    velocity: f32,
    playing: bool,
    pattern: [bool; 16],
    lfo_enabled: bool,
    lfo_target: usize,
    lfo_amount: f32,
    lfo_hz: f32,
    error: Option<String>,
}
impl Experiments {
    pub fn configured(
        rate: f32,
        instrument_name: &str,
        effect_name: &str,
        sequence: bool,
        modulation: bool,
    ) -> Self {
        let mut result = Self::new(rate);
        result.selected = INSTRUMENTS
            .iter()
            .position(|name| name.starts_with(instrument_name))
            .unwrap_or(0);
        result.effect = EFFECTS
            .iter()
            .position(|name| name.eq_ignore_ascii_case(effect_name))
            .unwrap_or(0);
        result.replace_instrument();
        result.update_effect();
        result.lfo_enabled = modulation;
        result.update_lfo();
        if sequence {
            result.playing = true;
            result.exercise();
        }
        result
    }
    pub fn new(rate: f32) -> Self {
        let mut result = Self {
            engine: Arc::new(Mutex::new(Engine::new(rate))),
            rate,
            selected: 0,
            params: vec![],
            edits: Default::default(),
            effect: 0,
            mix: 0.3,
            feedback: 0.4,
            damping: 0.5,
            cutoff: 5000.0,
            delay_timing: 3,
            pingpong: false,
            predelay: 0.0,
            width: 1.0,
            size: 0.5,
            bpm: 120.0,
            gain: 0.6,
            velocity: 0.8,
            playing: false,
            pattern: std::array::from_fn(|i| i % 4 == 0),
            lfo_enabled: false,
            lfo_target: 0,
            lfo_amount: 0.2,
            lfo_hz: 1.0,
            error: None,
        };
        result.replace_instrument();
        result.update_effect();
        result
    }
    fn replace_instrument(&mut self) {
        self.edits.clear();
        let mut voice = instrument(self.selected, self.rate);
        self.params = voice
            .as_modulatable()
            .map(|p| {
                p.modulatable_parameters()
                    .into_iter()
                    .map(|name| (name, 0.5))
                    .collect()
            })
            .unwrap_or_default();
        if let Ok(mut engine) = self.engine.lock() {
            engine.stop_all_sequencers();
            engine.add_instrument("audition", voice);
            if engine.sequencer_count() == 0 {
                engine.add_sequencer(Sequencer::with_pattern(
                    self.bpm,
                    self.rate,
                    self.pattern.to_vec(),
                    "audition",
                ));
            }
            engine.set_master_gain(self.gain);
        }
        self.playing = false;
        self.lfo_enabled = false;
        self.lfo_target = 0;
        self.update_lfo();
    }
    fn update_effect(&mut self) {
        let effect: Option<Box<dyn Effect>> = match self.effect {
            1 => {
                let delay = DelayEffect::new(
                    self.rate,
                    DelayTiming::from_timing_constant(self.delay_timing)
                        .unwrap_or(DelayTiming::Eighth),
                    self.bpm,
                    self.feedback,
                    self.mix,
                    self.cutoff,
                );
                delay.set_pingpong(self.pingpong);
                Some(Box::new(delay))
            }
            2 => Some(Box::new(SpringReverbEffect::new(
                self.rate,
                self.feedback,
                self.mix,
                self.damping,
            ))),
            3 => Some(Box::new(LowpassFilterEffect::new(
                self.rate,
                self.cutoff,
                self.feedback,
            ))),
            4 => Some(Box::new(TubeSaturation::new(
                self.rate,
                self.feedback,
                self.damping,
                self.mix,
            ))),
            5 => {
                let filter = TiltFilterEffect::new(self.rate);
                filter.set_cutoff(self.mix);
                filter.set_resonance(self.feedback);
                Some(Box::new(filter))
            }
            6 => {
                let plate =
                    PlateReverbEffect::new(self.rate, self.feedback, self.mix, self.damping);
                plate.set_predelay(self.predelay);
                plate.set_width(self.width);
                plate.set_size(self.size);
                Some(Box::new(plate))
            }
            _ => None,
        };
        if let Ok(mut engine) = self.engine.lock() {
            engine.clear_global_effects();
            if let Some(effect) = effect {
                engine.add_global_effect(Box::new(GuiEffect(effect)));
            }
        }
    }
    fn update_lfo(&mut self) {
        if let Ok(mut engine) = self.engine.lock() {
            if engine.lfo(0).is_none() {
                engine.add_lfo(Lfo::new(self.lfo_hz, self.rate));
            }
            if let Some(lfo) = engine.lfo_mut(0) {
                lfo.set_frequency(self.lfo_hz);
                lfo.amount = if self.lfo_enabled {
                    self.lfo_amount
                } else {
                    0.0
                };
                lfo.target_instrument.clear();
            }
            if self.lfo_enabled && !self.params.is_empty() {
                self.error = engine
                    .map_lfo_to_parameter(
                        0,
                        "audition",
                        self.params[self.lfo_target].0,
                        self.lfo_amount,
                    )
                    .err();
            }
        }
    }
}
// Effect objects are constructed and swapped under the engine lock, never in
// the callback. This adapter preserves each effect's stereo implementation.
struct GuiEffect(Box<dyn Effect>);
impl Effect for GuiEffect {
    fn process(&self, input: f32) -> f32 {
        self.0.process(input)
    }
    fn process_stereo(&self, frame: crate::StereoFrame) -> crate::StereoFrame {
        self.0.process_stereo(frame)
    }
}
impl GuiPanel for Experiments {
    fn name(&self) -> &str {
        "Experiments"
    }
    fn renderer(&mut self, _: f32, health: Arc<AudioHealth>) -> Box<dyn BlockRenderer> {
        Box::new(EngineRenderer::new(Arc::clone(&self.engine), health))
    }
    fn deactivate(&mut self) {
        self.playing = false;
        if let Ok(mut engine) = self.engine.lock() {
            let mut voice = instrument(self.selected, self.rate);
            if let Some(params) = voice.as_modulatable() {
                for (name, value) in &self.edits {
                    self.error = params.apply_modulation(name, *value * 2.0 - 1.0).err();
                }
            }
            *engine = Engine::new(self.rate);
            engine.set_master_gain(self.gain);
            engine.add_instrument("audition", voice);
            engine.add_sequencer(Sequencer::with_pattern(
                self.bpm,
                self.rate,
                self.pattern.to_vec(),
                "audition",
            ));
        }
        self.update_effect();
        self.update_lfo();
    }
    fn exercise(&mut self) {
        if let Ok(mut engine) = self.engine.lock() {
            engine.trigger_instrument_with_velocity("audition", self.velocity);
            if let Some(sequence) = engine.sequencer_mut(0) {
                sequence.start();
            }
        }
    }
    fn exercise_step(&mut self, step: u64) {
        self.selected = step as usize % INSTRUMENTS.len();
        self.replace_instrument();
        self.effect = (step as usize / INSTRUMENTS.len()) % EFFECTS.len();
        self.delay_timing = step as u32 % 9;
        self.pingpong = step & 1 == 0;
        self.mix = 0.15 + (step % 7) as f32 * 0.1;
        self.feedback = 0.2 + (step % 5) as f32 * 0.1;
        self.update_effect();
        if let Ok(mut engine) = self.engine.lock() {
            if let Some(params) = engine
                .instrument_mut("audition")
                .and_then(|voice| voice.as_modulatable())
            {
                for (index, (name, marker)) in self.params.iter_mut().enumerate() {
                    *marker = 0.2 + ((index as u64 + step) % 7) as f32 * 0.1;
                    self.error = params.apply_modulation(name, *marker * 2.0 - 1.0).err();
                    self.edits.insert(name, *marker);
                }
            }
        }
        self.lfo_enabled = true;
        self.lfo_target = step as usize % self.params.len().max(1);
        self.update_lfo();
        self.exercise();
    }
    fn ui(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().id_salt("experiments-editor").show(ui, |ui| {
            ui.heading("Instrument / effects / transport experiments");
            let old = self.selected;
            ui.horizontal_wrapped(|ui| { for (i, name) in INSTRUMENTS.iter().enumerate() { ui.selectable_value(&mut self.selected, i, *name); } });
            if old != self.selected { self.replace_instrument(); }
            ui.horizontal(|ui| {
                if ui.button("Trigger / Space").clicked() || (!ctx.wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::Space))) { if let Ok(mut engine) = self.engine.lock() { engine.trigger_instrument_with_velocity("audition", self.velocity); } }
                super::parameter(ui, "Velocity", &mut self.velocity, 0.0..=1.0);
                if super::parameter(ui, "Master", &mut self.gain, 0.0..=1.0) { if let Ok(mut engine) = self.engine.lock() { engine.set_master_gain(self.gain); } }
            });
            ui.columns(2, |columns| {
                egui::ScrollArea::vertical().max_height(420.0).show(&mut columns[0], |ui| {
                    ui.label("Normalized parameters · values apply when edited (default markers start at midpoint)");
                    for (id, (name, value)) in self.params.iter_mut().enumerate() {
                        if super::ParameterDescriptor::normalized(id as u32, name).ui(ui, value) {
                            self.edits.insert(name, *value);
                            if let Ok(mut engine) = self.engine.lock() {
                                if let Some(params) = engine.instrument_mut("audition").and_then(|voice| voice.as_modulatable()) { self.error = params.apply_modulation(name, *value * 2.0 - 1.0).err(); }
                            }
                        }
                    }
                });
                let ui = &mut columns[1];
                let mut changed = false;
                egui::ComboBox::from_label("Effect").selected_text(EFFECTS[self.effect]).show_ui(ui, |ui| { for (i, name) in EFFECTS.iter().enumerate() { changed |= ui.selectable_value(&mut self.effect, i, *name).changed(); } });
                changed |= super::parameter(ui, "Mix / tilt", &mut self.mix, 0.0..=1.0);
                changed |= super::parameter(ui, "Feedback / decay / resonance / drive", &mut self.feedback, 0.0..=0.9);
                changed |= super::parameter(ui, "Damping / warmth", &mut self.damping, 0.0..=1.0);
                changed |= super::parameter(ui, "Cutoff Hz", &mut self.cutoff, 20.0..=20_000.0);
                if self.effect == 1 {
                    egui::ComboBox::from_label("Delay division").selected_text(format!("{:?}", DelayTiming::from_timing_constant(self.delay_timing))).show_ui(ui, |ui| { for id in 0..9 { if let Some(timing) = DelayTiming::from_timing_constant(id) { changed |= ui.selectable_value(&mut self.delay_timing, id, format!("{timing:?}")).changed(); } } });
                    changed |= ui.checkbox(&mut self.pingpong, "Ping-pong stereo").changed();
                }
                if self.effect == 6 {
                    changed |= super::parameter(ui, "Predelay", &mut self.predelay, 0.0..=1.0);
                    changed |= super::parameter(ui, "Stereo width", &mut self.width, 0.0..=1.0);
                    changed |= super::parameter(ui, "Plate size", &mut self.size, 0.0..=1.0);
                }
                if changed { self.update_effect(); }
                ui.small("POC: editing an effect replaces it and clears its tail; controls are not an automation interface.");
                ui.separator();
                let mut lfo_changed = ui.checkbox(&mut self.lfo_enabled, "LFO modulation").changed();
                lfo_changed |= super::parameter(ui, "Rate Hz", &mut self.lfo_hz, 0.05..=20.0);
                lfo_changed |= super::parameter(ui, "Amount", &mut self.lfo_amount, 0.0..=1.0);
                if !self.params.is_empty() { egui::ComboBox::from_label("Target").selected_text(self.params[self.lfo_target].0).show_ui(ui, |ui| { for (i, (name, _)) in self.params.iter().enumerate() { lfo_changed |= ui.selectable_value(&mut self.lfo_target, i, *name).changed(); } }); }
                if lfo_changed { self.update_lfo(); }
            });
            ui.separator();
            let transport_changed = ui.checkbox(&mut self.playing, "Play pattern").changed();
            let tempo_changed = super::parameter(ui, "BPM", &mut self.bpm, 40.0..=240.0);
            ui.horizontal_wrapped(|ui| { for step in 0..16 { if ui.selectable_label(self.pattern[step], format!("{:02}", step + 1)).clicked() { self.pattern[step] = !self.pattern[step]; if let Ok(mut engine) = self.engine.lock() { if let Some(sequence) = engine.sequencer_mut(0) { sequence.set_step(step, self.pattern[step]); } } } } });
            if transport_changed || tempo_changed { if let Ok(mut engine) = self.engine.lock() { engine.set_bpm(self.bpm); if let Some(sequence) = engine.sequencer_mut(0) { sequence.set_bpm(self.bpm); if self.playing { sequence.start(); } else { sequence.stop(); } } } }
            if let Some(error) = &self.error { ui.colored_label(egui::Color32::RED, error); }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_instruments_parameters_effects_transport_and_lfo_are_real() {
        for rate in [44_100.0, 48_000.0] {
            for selected in 0..INSTRUMENTS.len() {
                let mut lab = Experiments::new(rate);
                lab.selected = selected;
                lab.replace_instrument();
                for (name, _) in &lab.params {
                    let mut engine = lab.engine.lock().expect("engine");
                    let voice = engine
                        .instrument_mut("audition")
                        .expect("voice")
                        .as_modulatable()
                        .expect("params");
                    for value in [-0.8, 0.0, 0.8] {
                        voice
                            .apply_modulation(name, value)
                            .expect("parameter applies");
                    }
                }
                lab.lfo_enabled = true;
                lab.update_lfo();
                assert!(lab.error.is_none());
                for effect in 0..EFFECTS.len() {
                    lab.effect = effect;
                    lab.update_effect();
                    lab.exercise();
                    let mut renderer = lab.renderer(rate, Arc::default());
                    let telemetry = crate::gui::exercise(renderer.as_mut(), rate, 40);
                    assert_eq!(
                        telemetry
                            .health
                            .non_finite
                            .load(std::sync::atomic::Ordering::Relaxed),
                        0,
                        "{} / {}",
                        INSTRUMENTS[selected],
                        EFFECTS[effect]
                    );
                    assert_eq!(
                        lab.engine.lock().expect("engine").global_effect_count(),
                        usize::from(effect != 0)
                    );
                }
                lab.deactivate();
                assert!(!lab
                    .engine
                    .lock()
                    .expect("engine")
                    .sequencer(0)
                    .expect("sequence")
                    .is_running());
            }
        }
    }
}
