use super::{AudioHealth, BlockRenderer, GuiPanel};
use crate::instruments::*;
use crate::StereoFrame;
use eframe::egui;
use std::sync::{atomic::Ordering, Arc, Mutex};

pub const PARAM_NAMES: [&str; POLY_PARAM_COUNT as usize] = [
    "Osc A waveform",
    "Osc A level",
    "Osc B waveform",
    "Osc B level",
    "Detune",
    "Stereo width",
    "Amp attack",
    "Amp decay",
    "Amp sustain",
    "Amp release",
    "Amp attack curve",
    "Amp fall curve",
    "Pitch amount",
    "Pitch attack",
    "Pitch decay",
    "Pitch sustain",
    "Pitch release",
    "Pitch attack curve",
    "Pitch fall curve",
    "Filter cutoff",
    "Filter resonance",
    "Filter envelope amount",
    "Filter attack",
    "Filter decay",
    "Filter sustain",
    "Filter release",
    "Filter attack curve",
    "Filter fall curve",
    "Saturation",
    "Volume",
];
const PAGES: [&str; 6] = [
    "Oscillators",
    "Amp envelope",
    "Pitch envelope",
    "Filter",
    "Expression",
    "Mod matrix",
];
const PRESETS: [&str; 5] = ["Default", "Pad", "Pluck", "Keys", "Strings"];
fn preset(index: usize) -> PolySynthConfig {
    match index {
        1 => PolySynthConfig::pad(),
        2 => PolySynthConfig::pluck(),
        3 => PolySynthConfig::keys(),
        4 => PolySynthConfig::strings(),
        _ => PolySynthConfig::default(),
    }
}
struct PolyRenderer {
    synth: Arc<Mutex<PolySynth>>,
    time: f64,
    health: Arc<AudioHealth>,
}
impl BlockRenderer for PolyRenderer {
    fn set_time(&mut self, seconds: f64) {
        self.time = seconds;
    }
    fn render(&mut self, frames: &mut [StereoFrame], rate: f32) {
        if let Ok(mut synth) = self.synth.try_lock() {
            for frame in frames {
                *frame = synth.tick_frame(self.time);
                self.time += 1.0 / rate as f64;
            }
        } else {
            frames.fill(StereoFrame::default());
            self.health.contention.fetch_add(1, Ordering::Relaxed);
        }
    }
}
pub struct PolyPanel {
    synth: Arc<Mutex<PolySynth>>,
    config: PolySynthConfig,
    page: usize,
    preset: usize,
    keyboard: super::Keyboard,
    velocity: f32,
    presets: [PolySynthConfig; 5],
}
impl PolyPanel {
    pub fn new(rate: f32) -> Self {
        Self {
            synth: Arc::new(Mutex::new(PolySynth::new(rate))),
            config: preset(0),
            page: 0,
            preset: 0,
            keyboard: super::Keyboard::default(),
            velocity: 0.8,
            presets: std::array::from_fn(preset),
        }
    }
}
impl GuiPanel for PolyPanel {
    fn name(&self) -> &str {
        "PolySynth"
    }
    fn renderer(&mut self, _rate: f32, health: Arc<AudioHealth>) -> Box<dyn BlockRenderer> {
        Box::new(PolyRenderer {
            synth: Arc::clone(&self.synth),
            time: 0.0,
            health,
        })
    }
    fn deactivate(&mut self) {
        self.keyboard.release();
        if let Ok(mut synth) = self.synth.lock() {
            synth.release_all();
        }
    }
    fn exercise(&mut self) {
        if let Ok(mut synth) = self.synth.lock() {
            for id in 0..POLY_PARAM_COUNT {
                let value = (id as f32 / POLY_PARAM_COUNT as f32).clamp(0.05, 0.95);
                self.config.set_param(id, value);
                synth.set_param(id, value);
            }
            for slot in 0..POLY_MOD_ROUTE_COUNT {
                let route = PolyModRoute {
                    enabled: true,
                    source: PolyModSource::Velocity,
                    destination: slot as u32 % POLY_PARAM_COUNT,
                    depth: 0.2,
                    curve: 0.5,
                    key_scale: 0.1,
                };
                synth.set_mod_route(slot, route);
            }
            synth.trigger_note(60, 0.8);
        }
    }
    fn exercise_step(&mut self, step: u64) {
        self.page = step as usize % PAGES.len();
        self.preset = step as usize % PRESETS.len();
        self.config = preset(self.preset);
        if let Ok(mut synth) = self.synth.lock() {
            synth.release_all();
            synth.set_config(self.config);
            for id in 0..POLY_PARAM_COUNT {
                let value = 0.15 + ((id as u64 + step) % 10) as f32 * 0.07;
                synth.set_param(id, value);
                self.config.set_param(id, value);
            }
            for slot in 0..POLY_MOD_ROUTE_COUNT {
                let route = PolyModRoute {
                    enabled: true,
                    source: if step & 1 == 0 {
                        PolyModSource::Velocity
                    } else {
                        PolyModSource::KeyPosition
                    },
                    destination: (slot as u64 + step) as u32 % POLY_PARAM_COUNT,
                    depth: 0.2,
                    curve: 0.2 + (step % 6) as f32 * 0.1,
                    key_scale: -0.2,
                };
                synth.set_mod_route(slot, route);
                self.config.set_mod_route(slot, route);
            }
            for note in [48, 60, 67] {
                synth.trigger_note(note, self.velocity);
            }
        }
    }
    fn ui(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Expressive PolySynth laboratory");
            let old = self.preset;
            egui::ComboBox::from_label("Preset")
                .selected_text(PRESETS[self.preset])
                .show_ui(ui, |ui| {
                    for (i, name) in PRESETS.iter().enumerate() {
                        ui.selectable_value(&mut self.preset, i, *name);
                    }
                });
            if old != self.preset {
                self.presets[old] = self.config;
                self.config = self.presets[self.preset];
                self.deactivate();
                if let Ok(mut synth) = self.synth.lock() {
                    synth.set_config(self.config);
                }
            }
            if ui.button("Reset current preset").clicked() {
                self.config = preset(self.preset);
                self.presets[self.preset] = self.config;
                self.deactivate();
                if let Ok(mut synth) = self.synth.lock() {
                    synth.set_config(self.config);
                }
            }
            ui.horizontal_wrapped(|ui| {
                for (i, name) in PAGES.iter().enumerate() {
                    ui.selectable_value(&mut self.page, i, *name);
                }
            });
            egui::ScrollArea::vertical()
                .max_height((ui.available_height() - 150.0).max(100.0))
                .show(ui, |ui| {
                    if self.page < 5 {
                        let range = [0..6, 6..12, 12..19, 19..28, 28..30][self.page].clone();
                        for id in range {
                            let mut value = self.config.param(id as u32).unwrap_or(0.0);
                            if super::ParameterDescriptor::normalized(id as u32, PARAM_NAMES[id])
                                .ui(ui, &mut value)
                            {
                                self.config.set_param(id as u32, value);
                                if let Ok(mut synth) = self.synth.lock() {
                                    synth.set_param(id as u32, value);
                                }
                            }
                        }
                    } else {
                        for slot in 0..POLY_MOD_ROUTE_COUNT {
                            let mut route = self
                                .synth
                                .lock()
                                .ok()
                                .and_then(|synth| synth.mod_route(slot))
                                .unwrap_or_default();
                            let old = route;
                            ui.push_id(slot, |ui| {
                                ui.group(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(format!("Route {}", slot + 1));
                                        ui.checkbox(&mut route.enabled, "Enabled");
                                        egui::ComboBox::from_id_salt("source")
                                            .selected_text(format!("{:?}", route.source))
                                            .show_ui(ui, |ui| {
                                                ui.selectable_value(
                                                    &mut route.source,
                                                    PolyModSource::Velocity,
                                                    "Velocity",
                                                );
                                                ui.selectable_value(
                                                    &mut route.source,
                                                    PolyModSource::KeyPosition,
                                                    "Key position",
                                                );
                                            });
                                        egui::ComboBox::from_id_salt("destination")
                                            .selected_text(PARAM_NAMES[route.destination as usize])
                                            .show_ui(ui, |ui| {
                                                for (i, name) in PARAM_NAMES.iter().enumerate() {
                                                    ui.selectable_value(
                                                        &mut route.destination,
                                                        i as u32,
                                                        *name,
                                                    );
                                                }
                                            });
                                    });
                                    super::parameter(ui, "Depth", &mut route.depth, -1.0..=1.0);
                                    super::parameter(ui, "Curve", &mut route.curve, 0.0..=1.0);
                                    super::parameter(
                                        ui,
                                        "Key scale",
                                        &mut route.key_scale,
                                        -1.0..=1.0,
                                    );
                                });
                            });
                            if old != route {
                                self.config.set_mod_route(slot, route);
                                if let Ok(mut synth) = self.synth.lock() {
                                    synth.set_mod_route(slot, route);
                                }
                            }
                        }
                    }
                });
            super::parameter(ui, "Velocity", &mut self.velocity, 0.0..=1.0);
            ui.add(egui::Slider::new(&mut self.keyboard.octave, 0..=8).text("Octave"));
            ui.label("Play Z–M / Q–I · two chromatic octaves · release on focus loss");
            let events = self.keyboard.ui(ui);
            if let Ok(mut synth) = self.synth.lock() {
                for (note, down) in events {
                    if down {
                        synth.trigger_note(note, self.velocity);
                    } else {
                        synth.release_note(note);
                    }
                }
                if ui.button("Panic / release all").clicked() {
                    synth.release_all();
                    self.keyboard.release();
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_presets_parameters_destinations_and_route_fields_render_finitely() {
        for rate in [44_100.0, 48_000.0] {
            for preset_index in 0..PRESETS.len() {
                let mut synth = PolySynth::with_config(rate, preset(preset_index));
                for id in 0..POLY_PARAM_COUNT {
                    for value in [0.0, 0.5, 1.0] {
                        assert!(synth.set_param(id, value));
                        assert_eq!(synth.param(id), Some(value));
                    }
                    assert!(synth.set_param(id, preset(preset_index).param(id).expect("parameter")));
                    for source in [PolyModSource::Velocity, PolyModSource::KeyPosition] {
                        for slot in 0..POLY_MOD_ROUTE_COUNT {
                            let route = PolyModRoute {
                                enabled: true,
                                source,
                                destination: id,
                                depth: 0.3,
                                curve: 0.7,
                                key_scale: -0.2,
                            };
                            assert!(synth.set_mod_route(slot, route));
                            assert_eq!(synth.mod_route(slot), Some(route));
                        }
                    }
                }
                synth.trigger_note(60, 0.8);
                let mut peak = 0.0_f32;
                for i in 0..4096 {
                    let frame = synth.tick_frame(i as f64 / rate as f64);
                    assert!(frame.l.is_finite() && frame.r.is_finite());
                    peak = peak.max(frame.l.abs()).max(frame.r.abs());
                }
                assert!(peak > 0.0, "preset {} silent", PRESETS[preset_index]);
                synth.release_all();
            }
        }
    }
    #[test]
    fn every_parameter_page_and_matrix_layouts() {
        let mut panel = PolyPanel::new(44_100.0);
        let ctx = egui::Context::default();
        for page in 0..PAGES.len() {
            panel.page = page;
            let output = ctx.run(egui::RawInput::default(), |ctx| panel.ui(ctx));
            assert!(!output.shapes.is_empty());
        }
        assert_eq!(PARAM_NAMES.len(), 30);
    }
}
