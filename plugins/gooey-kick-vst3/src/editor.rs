use crate::params::{AtomicParameters, ParamId};
use egui::{Color32, RichText, Sense, Stroke, Vec2};
use egui_baseview::{App, Frame, Key, KeyCapture};
use std::sync::Arc;

pub const EDITOR_WIDTH: f64 = 560.0;
pub const EDITOR_HEIGHT: f64 = 300.0;
const KNOB_DRAG_SENSITIVITY: f32 = 0.006;

fn knob_value_from_drag(start: f32, total_drag_y: f32) -> f32 {
    (start - total_drag_y * KNOB_DRAG_SENSITIVITY).clamp(0.0, 1.0)
}

fn editor_key_capture(standalone: bool) -> KeyCapture {
    if standalone {
        KeyCapture::CaptureKeys(vec![Key::Character(" ".into())])
    } else {
        KeyCapture::IgnoreAll
    }
}

pub trait EditorHost: Send + 'static {
    fn parameters(&self) -> &AtomicParameters;
    fn begin_edit(&self, id: ParamId);
    fn perform_edit(&self, id: ParamId, value: f32);
    fn end_edit(&self, id: ParamId);
    fn audition(&self) {}
}

pub struct KickEditor {
    host: Box<dyn EditorHost>,
    standalone: bool,
    gesture_active: [bool; crate::PARAM_COUNT],
    gesture_start: [f32; crate::PARAM_COUNT],
}

impl KickEditor {
    pub fn new(host: Box<dyn EditorHost>, standalone: bool) -> Self {
        Self {
            host,
            standalone,
            gesture_active: [false; crate::PARAM_COUNT],
            gesture_start: [0.0; crate::PARAM_COUNT],
        }
    }

    fn parameter(&mut self, ui: &mut egui::Ui, id: ParamId) {
        let mut value = self.host.parameters().get(id);
        ui.vertical(|ui| {
            ui.set_width(70.0);
            ui.label(RichText::new(id.name()).small());
            let index = id.index();
            let (rect, mut response) =
                ui.allocate_exact_size(Vec2::splat(56.0), Sense::click_and_drag());
            if response.drag_started() {
                self.host.begin_edit(id);
                self.gesture_active[index] = true;
                self.gesture_start[index] = value;
            }
            if response.dragged() {
                let dragged = knob_value_from_drag(
                    self.gesture_start[index],
                    response.total_drag_delta().unwrap_or_default().y,
                );
                if dragged != value {
                    value = dragged;
                    response.mark_changed();
                }
            }
            if response.double_clicked() {
                value = crate::Parameters::default().get(id);
                response.mark_changed();
            }
            if response.changed() {
                if !self.gesture_active[index] {
                    self.host.begin_edit(id);
                    self.gesture_active[index] = true;
                }
                self.host.perform_edit(id, value);
                if !response.dragged() {
                    self.host.end_edit(id);
                    self.gesture_active[index] = false;
                }
            }
            if response.drag_stopped() && self.gesture_active[index] {
                self.host.end_edit(id);
                self.gesture_active[index] = false;
            }
            let visuals = ui.style().interact(&response);
            let center = rect.center();
            let radius = rect.width() * 0.42;
            ui.painter().circle(
                center,
                radius,
                Color32::from_rgb(42, 46, 57),
                Stroke::new(2.0, visuals.fg_stroke.color),
            );
            let angle = std::f32::consts::PI * (0.75 + value * 1.5);
            let direction = Vec2::new(angle.cos(), angle.sin());
            ui.painter().line_segment(
                [center, center + direction * (radius - 5.0)],
                Stroke::new(3.0, Color32::from_rgb(239, 172, 72)),
            );
            ui.label(RichText::new(id.display(value as f64)).monospace().small());
        });
    }
}

impl Drop for KickEditor {
    fn drop(&mut self) {
        for id in ParamId::ALL {
            if self.gesture_active[id.index()] {
                self.host.end_edit(id);
            }
        }
    }
}

impl App for KickEditor {
    fn build(
        &mut self,
        _egui_ctx: egui::Context,
        frame: &mut Frame,
    ) -> Result<(), egui_baseview::baseview::HandlerError> {
        frame.set_key_capture(editor_key_capture(self.standalone));
        Ok(())
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut Frame) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(Color32::from_rgb(20, 22, 28)))
            .show(ui, |ui| {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.heading(
                        RichText::new("Gooey Kick POC").color(Color32::from_rgb(239, 172, 72)),
                    );
                    ui.label("libgooey · Rust VST3");
                });
                ui.separator();
                ui.add_space(8.0);
                ui.horizontal_centered(|ui| {
                    for id in ParamId::ALL {
                        self.parameter(ui, id);
                    }
                });

                if self.standalone {
                    let mut audition_requested = false;
                    ui.add_space(8.0);
                    ui.horizontal_centered(|ui| {
                        if ui.button("Audition").clicked() {
                            audition_requested = true;
                        }
                        let keyboard_focus = ui.add(
                            egui::Label::new("or press Space")
                                .sense(Sense::focusable_noninteractive()),
                        );
                        if ui.memory(|memory| memory.focused().is_none()) {
                            keyboard_focus.request_focus();
                        }
                    });
                    if ui.input(|input| input.key_pressed(egui::Key::Space)) {
                        audition_requested = true;
                    }
                    if audition_requested {
                        self.host.audition();
                    }
                }
            });
    }
}

pub struct AtomicEditorHost {
    pub parameters: Arc<AtomicParameters>,
    pub audition: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl EditorHost for AtomicEditorHost {
    fn parameters(&self) -> &AtomicParameters {
        &self.parameters
    }

    fn begin_edit(&self, _id: ParamId) {}

    fn perform_edit(&self, id: ParamId, value: f32) {
        self.parameters.set(id, value);
    }

    fn end_edit(&self, _id: ParamId) {}

    fn audition(&self) {
        if let Some(audition) = &self.audition {
            audition();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cumulative_vertical_drag_changes_knob_monotonically_and_clamps() {
        let upward = [-10.0, -20.0, -30.0].map(|delta| knob_value_from_drag(0.5, delta));
        assert!(upward.windows(2).all(|values| values[0] < values[1]));

        let downward = [10.0, 20.0, 30.0].map(|delta| knob_value_from_drag(0.5, delta));
        assert!(downward.windows(2).all(|values| values[0] > values[1]));

        assert_eq!(knob_value_from_drag(0.5, -1_000.0), 1.0);
        assert_eq!(knob_value_from_drag(0.5, 1_000.0), 0.0);
    }

    #[test]
    fn keyboard_capture_is_scoped_to_standalone_spacebar() {
        assert_eq!(
            editor_key_capture(true),
            KeyCapture::CaptureKeys(vec![Key::Character(" ".into())])
        );
        assert_eq!(editor_key_capture(false), KeyCapture::IgnoreAll);
    }
}
