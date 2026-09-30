//! Loop Studio editor mounted in the shared GUI shell; never an audio host.
use super::egui::{self, Color32, RichText};
use super::{
    control_lock, parameter, parameter_logarithmic, AudioHealth, BlockRenderer, GuiPanel,
    InterleavedAdapter,
};
use crate::{
    ffi::*,
    studio::{Control, Session, Studio, LOOP_TICKS, TRACK_NAMES},
};
use anyhow::Result;
use std::sync::{atomic::Ordering, Arc, Mutex};
pub struct StudioPanel {
    engine: Arc<Mutex<Studio>>,
    health: Arc<AudioHealth>,
    active: bool,
    path: String,
    wav_path: String,
    import_path: String,
    bars: u32,
    status: String,
    root: u32,
    minor: bool,
    octave: i32,
    preset: u32,
    held: Option<u32>,
    held_pointer: bool,
    exporting: Option<std::sync::mpsc::Receiver<String>>,
    loading: Option<std::sync::mpsc::Receiver<std::result::Result<Studio, String>>>,
    lifecycle_epoch: u64,
    loading_epoch: u64,
    selected_lane: usize,
}
fn knob(ui: &mut egui::Ui, s: &mut Studio, c: Control, label: &str) {
    let mut v = s.session().value(c);
    let changed = if matches!(c, Control::Cutoff(_)) {
        parameter_logarithmic(ui, label, &mut v, c.range())
    } else {
        parameter(ui, label, &mut v, c.range())
    };
    if changed {
        let _ = s.set_control(c, v);
    }
}
// Held-key autorepeat is not a new musical gesture. In particular, a key kept
// down while changing panels must not restart the returning studio's voice.
fn key_pressed(ctx: &egui::Context, key: egui::Key) -> bool {
    ctx.input(|input| {
        input.events.iter().any(|event| matches!(event,
        egui::Event::Key { key: event_key, pressed: true, repeat: false, .. } if *event_key == key
    ))
    })
}
impl GuiPanel for StudioPanel {
    fn name(&self) -> &str {
        "Loop Studio"
    }
    fn renderer(&mut self, rate: f32, health: Arc<AudioHealth>) -> Box<dyn BlockRenderer> {
        self.active = true;
        self.health = Arc::clone(&health);
        let engine = Arc::clone(&self.engine);
        // Construction happens in the factory at the negotiated rate. Mounting
        // again must retain the song and never reconstruct DSP on the callback.
        assert_eq!(control_lock(&engine, &health).sample_rate(), rate as u32);
        Box::new(InterleavedAdapter::new(
            move |out: &mut [f32], _: f32| match engine.try_lock() {
                Ok(mut studio) => {
                    if studio.render(out).is_err() {
                        out.fill(0.0);
                        health.render_failures.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Err(_) => {
                    out.fill(0.0);
                    health.contention.fetch_add(1, Ordering::Relaxed);
                }
            },
        ))
    }
    fn deactivate(&mut self) {
        let mut studio = control_lock(&self.engine, &self.health);
        studio.chord_off();
        studio.record(false);
        studio.play(false);
        self.held = None;
        self.held_pointer = false;
        self.active = false;
        self.lifecycle_epoch = self.lifecycle_epoch.wrapping_add(1);
    }
    fn exercise(&mut self) {
        let mut studio = control_lock(&self.engine, &self.health);
        studio.play(true);
        let _ = studio.hit(INSTRUMENT_KICK, 36, 0.8);
    }
    fn exercise_step(&mut self, step: u64) {
        let mut studio = control_lock(&self.engine, &self.health);
        studio.record(step % 4 != 3);
        studio.play(true);
        let _ = studio.hit(INSTRUMENT_KICK, 36, 0.8);
        if step.is_multiple_of(2) {
            let _ = studio.chord_on((step % 7) as u32, 0, true, 4, POLY_PRESET_PAD);
        } else {
            studio.chord_off();
        }
        let value = (step % 5) as f32 / 4.0;
        for control in [
            Control::Gain(0),
            Control::Pan(2),
            Control::BassTone,
            Control::MasterReverb,
        ] {
            let _ = studio.set_control(control, value);
        }
    }
    fn ui(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.exporting {
            if let Ok(msg) = rx.try_recv() {
                self.status = msg;
                self.exporting = None;
            }
        }
        if let Some(rx) = &self.loading {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok(mut replacement) => {
                        if !self.active || self.loading_epoch != self.lifecycle_epoch {
                            replacement.chord_off();
                            replacement.record(false);
                            replacement.play(false);
                        }
                        let playing = replacement.playing();
                        // Construction and file I/O occur off the audio lock;
                        // reclaim the old engine only after releasing the lock.
                        let old = {
                            let mut live = control_lock(&self.engine, &self.health);
                            live.chord_off();
                            live.record(false);
                            live.play(false);
                            std::mem::replace(&mut *live, replacement)
                        };
                        drop(old);
                        self.held = None;
                        self.held_pointer = false;
                        self.status = if playing {
                            "Rewound session"
                        } else {
                            "Loaded session (stopped; press Play)"
                        }
                        .into();
                    }
                    Err(error) => self.status = error,
                }
                self.loading = None;
            }
        }
        if !self.active {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.heading("Loop Studio audio is stopped");
                ui.label(
                    "Resume audio in the shared shell to edit or play. Your song is retained.",
                );
            });
            return;
        }
        let engine = self.engine.clone();
        let mut s = control_lock(&engine, &self.health);
        // Keyboard gestures are real engine controls, not a scripted UI demo.
        if ctx.input(|i| i.focused && !i.modifiers.ctrl && !i.modifiers.alt && !i.modifiers.mac_cmd)
            && !ctx.wants_keyboard_input()
        {
            if key_pressed(ctx, egui::Key::Space) {
                let on = !s.playing();
                s.play(on);
            }
            if key_pressed(ctx, egui::Key::R) {
                let on = !s.recording();
                s.record(on);
            }
            if key_pressed(ctx, egui::Key::Num0) {
                s.chord_off();
                self.held = None;
            }
            for (degree, key) in [
                egui::Key::Num1,
                egui::Key::Num2,
                egui::Key::Num3,
                egui::Key::Num4,
                egui::Key::Num5,
                egui::Key::Num6,
                egui::Key::Num7,
            ]
            .into_iter()
            .enumerate()
            {
                if key_pressed(ctx, key) {
                    let _ = s.chord_on(
                        degree as u32,
                        self.root,
                        self.minor,
                        self.octave,
                        self.preset,
                    );
                    self.held = Some(degree as u32);
                    self.held_pointer = false;
                }
            }
            for (key, id) in [
                (egui::Key::Z, INSTRUMENT_KICK),
                (egui::Key::X, INSTRUMENT_SNARE),
                (egui::Key::C, INSTRUMENT_HIHAT),
                (egui::Key::V, INSTRUMENT_TOM),
                (egui::Key::B, INSTRUMENT_BASS),
            ] {
                if key_pressed(ctx, key) {
                    let _ = s.hit(id, 36, 0.8);
                }
            }
        }
        // A text field taking keyboard focus must not swallow a held pad's
        // key-up and leave its voice sounding indefinitely.
        for (degree, key) in [
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
            egui::Key::Num6,
            egui::Key::Num7,
        ]
        .into_iter()
        .enumerate()
        {
            if ctx.input(|i| i.key_released(key)) && self.held == Some(degree as u32) {
                s.chord_off();
                self.held = None;
            }
        }
        if (!ctx.input(|i| i.focused) || ctx.wants_keyboard_input()) && self.held.is_some() {
            s.chord_off();
            self.held = None;
        }
        egui::TopBottomPanel::top("transport").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("GOOEY / LOOP STUDIO");
                ui.separator();
                ui.label("Shared host · stereo block renderer");
            });
            ui.horizontal(|ui| {
                if ui
                    .button(if s.playing() { "■ Stop" } else { "▶ Play" })
                    .clicked()
                {
                    let on = !s.playing();
                    s.play(on);
                }
                if ui
                    .add_enabled(self.loading.is_none(), egui::Button::new("↤ Rewind"))
                    .clicked()
                {
                    let song = s.snapshot_session();
                    let rate = s.sample_rate();
                    let playing = s.playing();
                    self.load_worker(move || {
                        let mut replacement = Studio::new(song, rate)?;
                        replacement.play(playing);
                        Ok(replacement)
                    });
                }
                let rec = s.recording();
                if ui.selectable_label(rec, "● Record / overdub [R]").clicked() {
                    s.record(!rec);
                }
                let mut bpm = s.session().bpm;
                if parameter(ui, "BPM", &mut bpm, 40.0..=240.0) {
                    let _ = s.set_tempo(bpm);
                }
                ui.label(format!(
                    "Step {:02}/16 · {}",
                    s.tick() / 24 + 1,
                    if s.chord_recording() {
                        "CAPTURING CHORDS"
                    } else if rec {
                        "CHORDS ARMED: next loop"
                    } else {
                        "Space = play/stop"
                    }
                ));
            });
        });
        egui::TopBottomPanel::bottom("files").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Session");
                ui.add(egui::TextEdit::singleline(&mut self.path).desired_width(260.0));
                if ui
                    .add_enabled(self.exporting.is_none(), egui::Button::new("Save"))
                    .clicked()
                {
                    let song = s.snapshot_session();
                    let path = self.path.clone();
                    self.save_worker(song, path);
                }
                if ui
                    .add_enabled(self.loading.is_none(), egui::Button::new("Load"))
                    .clicked()
                {
                    let path = self.path.clone();
                    let rate = s.sample_rate();
                    self.load_worker(move || Studio::new(Session::load(path)?, rate));
                }
                if ui
                    .add_enabled(self.loading.is_none(), egui::Button::new("Demo song"))
                    .clicked()
                {
                    let rate = s.sample_rate();
                    self.load_worker(move || Studio::new(Session::demo(), rate));
                }
                if ui
                    .add_enabled(self.loading.is_none(), egui::Button::new("New empty"))
                    .clicked()
                {
                    let rate = s.sample_rate();
                    self.load_worker(move || Studio::new(Session::default(), rate));
                }
            });
            ui.horizontal(|ui| {
                ui.label("Mixdown");
                ui.add(egui::TextEdit::singleline(&mut self.wav_path).desired_width(250.0));
                ui.add(
                    egui::DragValue::new(&mut self.bars)
                        .range(1..=1024)
                        .prefix("bars "),
                );
                if ui
                    .add_enabled(self.exporting.is_none(), egui::Button::new("Export WAV"))
                    .clicked()
                {
                    let song = s.snapshot_session();
                    let path = self.wav_path.clone();
                    let bars = self.bars;
                    self.export_worker(song, path, bars);
                }
                ui.label("48 kHz stereo float WAV + 2s tails");
            });
            ui.label(&self.status);
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading(&s.session().title);
                ui.label("One-bar clips • click steps to toggle • bass notes are editable • all controls drive the engine");
                draw_steps(ui, &mut s);
                ui.separator();
                draw_mixer(ui, &mut s);
                ui.separator();
                self.draw_performance(ui, ctx, &mut s);
                ui.separator();
                self.draw_automation(ui, &mut s);
                ui.separator();
                self.draw_import(ui, &mut s);
            });
        });
    }
}
fn draw_steps(ui: &mut egui::Ui, s: &mut Studio) {
    let current = (s.tick() / 24) as usize;
    egui::Grid::new("steps").spacing([7.0, 5.0]).show(ui, |ui| {
        ui.label("SEQUENCER");
        for i in 0..16 {
            let color = if s.playing() && i == current {
                Color32::YELLOW
            } else {
                Color32::GRAY
            };
            ui.label(RichText::new(format!("{:02}", i + 1)).color(color));
        }
        ui.end_row();
        for (row, name) in ["Kick", "Snare", "Hi-hat", "Tom", "Bass"]
            .into_iter()
            .enumerate()
        {
            ui.label(name);
            for step in 0..16 {
                let active = s.session().steps[row][step] > 0.0;
                let color = if active {
                    Color32::from_rgb(35, 155, 145)
                } else {
                    Color32::from_rgb(45, 48, 58)
                };
                let text = if row == 4 {
                    s.session().bass_notes[step].to_string()
                } else if active {
                    "●".into()
                } else {
                    "·".into()
                };
                let mut button = egui::Button::new(text)
                    .fill(color)
                    .min_size(egui::vec2(35.0, 27.0));
                if s.playing() && current == step {
                    button = button.stroke(egui::Stroke::new(2.0_f32, Color32::YELLOW));
                }
                if ui.add(button).clicked() {
                    let note = s.session().bass_notes[step];
                    let velocity = if active {
                        0.0
                    } else if row == 2 {
                        0.4
                    } else {
                        0.8
                    };
                    let _ = s.set_step(row, step, velocity, note);
                }
            }
            ui.end_row();
        }
        ui.label("Bass MIDI");
        for step in 0..16 {
            let mut note = s.session().bass_notes[step];
            if ui
                .add(egui::DragValue::new(&mut note).range(24..=60).speed(0.2))
                .changed()
            {
                let velocity = s.session().steps[4][step];
                let _ = s.set_step(4, step, velocity, note);
            }
        }
        ui.end_row();
    });
}
fn draw_mixer(ui: &mut egui::Ui, s: &mut Studio) {
    let peaks = s.peaks();
    ui.columns(5, |cols| {
        for t in 0..4 {
            let ui = &mut cols[t];
            ui.heading(TRACK_NAMES[t]);
            let strip = s.session().strips[t].clone();
            ui.horizontal(|ui| {
                if ui.selectable_label(strip.mute, "Mute").clicked() {
                    let _ = s.set_mute(t, !strip.mute);
                }
                if ui.selectable_label(strip.solo, "Solo").clicked() {
                    let _ = s.set_solo(t, !strip.solo);
                }
            });
            ui.add(
                egui::ProgressBar::new(peaks[t].clamp(0.0, 1.0))
                    .text(format!("peak {:.2}", peaks[t])),
            );
            for (control, label) in [
                (Control::Gain(t), "Gain"),
                (Control::Pan(t), "Pan"),
                (Control::Cutoff(t), "Filter Hz"),
                (Control::Delay(t), "Delay mix"),
                (Control::Reverb(t), "Reverb mix"),
            ] {
                knob(ui, s, control, label);
            }
            if t == 1 {
                knob(ui, s, Control::BassTone, "Synth tone");
            }
            if t == 2 {
                knob(ui, s, Control::ChordTone, "Synth tone");
            }
        }
        let ui = &mut cols[4];
        ui.heading("MASTER");
        knob(ui, s, Control::MasterGain, "Gain");
        knob(ui, s, Control::MasterDelay, "Delay mix");
        knob(ui, s, Control::MasterReverb, "Reverb mix");
        ui.label("Engine limiter enabled\nTrack rack: filter → delay → reverb");
    });
}
impl StudioPanel {
    fn save_worker(&mut self, song: Session, path: String) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.exporting = Some(rx);
        std::thread::spawn(move || {
            let status = match song.save(&path) {
                Ok(()) => format!("Saved {path}"),
                Err(e) => e.to_string(),
            };
            let _ = tx.send(status);
        });
        self.status = "Saving session…".into();
    }
    fn export_worker(&mut self, song: Session, path: String, bars: u32) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.exporting = Some(rx);
        std::thread::spawn(move || {
            let msg = match song.export_wav(&path, bars, 2.0) {
                Ok(r) => format!(
                    "Exported {path} · {} frames · peak {:.3} · RMS {:.3}",
                    r.frames, r.peak, r.rms
                ),
                Err(e) => format!("Export failed: {e}"),
            };
            let _ = tx.send(msg);
        });
        self.status = "Rendering fresh-engine mixdown…".into();
    }
    fn load_worker(&mut self, prepare: impl FnOnce() -> Result<Studio> + Send + 'static) {
        self.loading_epoch = self.lifecycle_epoch;
        let (tx, rx) = std::sync::mpsc::channel();
        self.loading = Some(rx);
        self.status = "Preparing session on worker…".into();
        std::thread::spawn(move || {
            let _ = tx.send(prepare().map_err(|e| e.to_string()));
        });
    }
    fn draw_performance(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, s: &mut Studio) {
        ui.horizontal(|ui| {
            ui.heading("PERFORMANCE / NEBULA");
            ui.add(
                egui::DragValue::new(&mut self.root)
                    .range(0..=11)
                    .prefix("root "),
            );
            ui.checkbox(&mut self.minor, "Minor");
            ui.add(
                egui::DragValue::new(&mut self.octave)
                    .range(1..=6)
                    .prefix("octave "),
            );
            let names = ["Default", "Pad", "Pluck", "Keys", "Strings"];
            egui::ComboBox::from_id_salt("preset")
                .selected_text(names[self.preset as usize])
                .show_ui(ui, |ui| {
                    for (i, name) in names.into_iter().enumerate() {
                        ui.selectable_value(&mut self.preset, i as u32, name);
                    }
                });
        });
        ui.horizontal(|ui| {
            for (i, name) in [
                "I / 1", "II / 2", "III / 3", "IV / 4", "V / 5", "VI / 6", "VII / 7",
            ]
            .into_iter()
            .enumerate()
            {
                let color = if self.held == Some(i as u32) {
                    Color32::from_rgb(40, 160, 150)
                } else {
                    Color32::from_rgb(65, 60, 85)
                };
                let response = ui.add(
                    egui::Button::new(name)
                        .min_size(egui::vec2(75.0, 40.0))
                        .fill(color),
                );
                if response.is_pointer_button_down_on() && self.held != Some(i as u32) {
                    let _ = s.chord_on(i as u32, self.root, self.minor, self.octave, self.preset);
                    self.held = Some(i as u32);
                    self.held_pointer = true;
                }
            }
            if ui.button("Release / 0").clicked() {
                s.chord_off();
                self.held = None;
            }
        });
        if ctx.input(|i| i.pointer.any_released()) && self.held.is_some() && self.held_pointer {
            s.chord_off();
            self.held = None;
        }
        ui.horizontal(|ui| {
            for (id, name) in [
                (INSTRUMENT_KICK, "Kick [Z]"),
                (INSTRUMENT_SNARE, "Snare [X]"),
                (INSTRUMENT_HIHAT, "Hat [C]"),
                (INSTRUMENT_TOM, "Tom [V]"),
                (INSTRUMENT_BASS, "Bass C2 [B]"),
            ] {
                if ui.button(name).clicked() {
                    let _ = s.hit(id, 36, 0.85);
                }
            }
            if ui.button("Clear performance").clicked() {
                s.clear_performance();
            }
            ui.label(format!(
                "{} chords · {} live hits",
                s.session().chords.len(),
                s.session().hits.len()
            ));
        });
        s.refresh_performance();
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 42.0), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 3.0, Color32::from_rgb(30, 33, 42));
        for event in &s.session().chords {
            let x = rect.left() + rect.width() * event.tick as f32 / LOOP_TICKS as f32;
            let width = rect.width() * event.duration as f32 / LOOP_TICKS as f32;
            let r = egui::Rect::from_min_size(
                egui::pos2(x, rect.top() + 5.0),
                egui::vec2(width.max(4.0), 30.0),
            );
            painter.rect_filled(r, 3.0, Color32::from_rgb(75, 90, 145));
            let label = ["I", "II", "III", "IV", "V", "VI", "VII"][event.degree as usize];
            painter.text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                format!("{label} · {:.2} beats", event.duration as f32 / 96.0),
                egui::FontId::proportional(12.0),
                Color32::WHITE,
            );
        }
    }
    fn draw_automation(&mut self, ui: &mut egui::Ui, s: &mut Studio) {
        ui.horizontal(|ui| {
            ui.heading("AUTOMATION");
            ui.label("Record + move a slider → 96-PPQ step-held lane; replay when record is off");
            if ui.button("Clear lanes").clicked() {
                s.clear_automation();
            }
        });
        let lanes = &s.session().automation;
        if lanes.is_empty() {
            ui.label("No lanes yet. Arm Record, press Play and move Gain, Pan, Filter, FX or synth tone.");
            return;
        }
        self.selected_lane = self.selected_lane.min(lanes.len() - 1);
        egui::ComboBox::from_id_salt("lane")
            .selected_text(lanes[self.selected_lane].control.label())
            .show_ui(ui, |ui| {
                for (i, lane) in lanes.iter().enumerate() {
                    ui.selectable_value(
                        &mut self.selected_lane,
                        i,
                        format!("{} ({} points)", lane.control.label(), lane.points.len()),
                    );
                }
            });
        let lane = &lanes[self.selected_lane];
        ui.label(format!(
            "{} · range {:.2}–{:.2} · beats 0 / 1 / 2 / 3 / 4",
            lane.control.label(),
            lane.control.range().start(),
            lane.control.range().end()
        ));
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 95.0), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 3.0, Color32::from_rgb(24, 27, 36));
        let range = lane.control.range();
        let low = *range.start();
        let high = *range.end();
        let pos = |tick: u32, value: f32| {
            egui::pos2(
                rect.left() + rect.width() * tick as f32 / LOOP_TICKS as f32,
                rect.bottom() - 5.0 - (rect.height() - 10.0) * (value - low) / (high - low),
            )
        };
        for beat in 0..=4 {
            let x = rect.left() + rect.width() * beat as f32 / 4.0;
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(1.0_f32, Color32::from_gray(50)),
            );
        }
        for (i, point) in lane.points.iter().enumerate() {
            let a = pos(point.tick, point.value);
            let end = lane.points.get(i + 1).map_or(LOOP_TICKS, |p| p.tick);
            let b = pos(end, point.value);
            painter.line_segment(
                [a, b],
                egui::Stroke::new(2.0_f32, Color32::from_rgb(80, 220, 180)),
            );
            painter.circle_filled(a, 3.0, Color32::WHITE);
            if let Some(next) = lane.points.get(i + 1) {
                painter.line_segment(
                    [b, pos(next.tick, next.value)],
                    egui::Stroke::new(1.0_f32, Color32::from_rgb(80, 220, 180)),
                );
            }
        }
        let x = pos(s.tick(), low).x;
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0_f32, Color32::YELLOW),
        );
        if let Some(pointer) = response.hover_pos() {
            let tick = ((pointer.x - rect.left()) / rect.width() * LOOP_TICKS as f32)
                .clamp(0.0, 383.0) as u32;
            if let Some(point) = lane
                .points
                .iter()
                .rev()
                .find(|p| p.tick <= tick)
                .or_else(|| lane.points.last())
            {
                response.on_hover_text(format!(
                    "Beat {:.2}: {:.3}",
                    tick as f32 / 96.0,
                    point.value
                ));
            }
        }
        ui.label(
            lane.points
                .iter()
                .take(12)
                .map(|p| format!("beat {:.2}: {:.3}", p.tick as f32 / 96.0, p.value))
                .collect::<Vec<_>>()
                .join("   |   "),
        );
    }
    fn draw_import(&mut self, ui: &mut egui::Ui, s: &mut Studio) {
        ui.horizontal(|ui| {
            ui.label("Audio loop WAV");
            ui.add(egui::TextEdit::singleline(&mut self.import_path).desired_width(350.0));
            if ui
                .add_enabled(self.loading.is_none(), egui::Button::new("Import loop"))
                .clicked()
            {
                let mut song = s.snapshot_session();
                let path = self.import_path.clone();
                let rate = s.sample_rate();
                self.load_worker(move || {
                    song.import_wav(path)?;
                    Studio::new(song, rate)
                });
            }
        });
        if let Some(audio) = &s.session().audio_loop {
            ui.label(format!(
                "{} · {:.2}s · source {:.0} BPM",
                audio.name,
                audio.samples.len() as f32 / 2.0 / audio.sample_rate as f32,
                audio.source_bpm
            ));
            let mut source_bpm = audio.source_bpm;
            if parameter(
                ui,
                "Loop source BPM (not automated)",
                &mut source_bpm,
                40.0..=240.0,
            ) {
                let _ = s.set_loop_source_tempo(source_bpm);
            }
        }
    }
}
impl StudioPanel {
    pub fn new(rate: f32, song: Session) -> Self {
        Self {
            engine: Arc::new(Mutex::new(
                Studio::new(song, rate as u32).expect("validated factory session and host rate"),
            )),
            health: Arc::default(),
            active: false,
            path: "studio-session.json".into(),
            wav_path: "studio-mix.wav".into(),
            import_path: String::new(),
            bars: 4,
            status: "Ready. Space: transport · R: record · 1–7: hold chords · Z/X/C/V/B: hits"
                .into(),
            root: 0,
            minor: true,
            octave: 4,
            preset: POLY_PRESET_PAD,
            held: None,
            held_pointer: false,
            exporting: None,
            loading: None,
            lifecycle_epoch: 0,
            loading_epoch: 0,
            selected_lane: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::{OmniApp, PanelFactory, BLOCK_SIZE};
    use crate::StereoFrame;

    fn draw(
        panel: &mut StudioPanel,
        ctx: &egui::Context,
        size: [f32; 2],
        events: Vec<egui::Event>,
        focused: bool,
    ) {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(size[0], size[1]),
                )),
                events,
                focused,
                ..Default::default()
            },
            |ctx| panel.ui(ctx),
        );
        assert!(!output.shapes.is_empty());
    }

    #[test]
    fn factory_mount_keeps_song_at_actual_rate_and_try_lock_never_waits() {
        for rate in [44_100.0, 48_000.0] {
            let mut panel = StudioPanel::new(rate, Session::demo());
            let song = control_lock(&panel.engine, &panel.health).snapshot_session();
            let health = Arc::new(AudioHealth::default());
            let mut renderer = panel.renderer(rate, Arc::clone(&health));
            let guard = panel.engine.lock().unwrap();
            let mut out = [StereoFrame { l: 1.0, r: 1.0 }; BLOCK_SIZE * 2 + 1];
            renderer.render(&mut out, rate);
            assert!(out.iter().all(|frame| frame.l == 0.0 && frame.r == 0.0));
            assert_eq!(health.contention.load(Ordering::Relaxed), 3);
            drop(guard);
            drop(renderer);
            panel.deactivate();
            let _renderer = panel.renderer(rate, Arc::clone(&health));
            let mut engine = control_lock(&panel.engine, &health);
            assert_eq!(engine.sample_rate(), rate as u32);
            assert_eq!(
                serde_json::to_string(&song).unwrap(),
                serde_json::to_string(&engine.snapshot_session()).unwrap()
            );
        }
    }

    #[test]
    fn studio_headless_all_sections_keyboard_focus_and_inactive_input() {
        let mut panel = StudioPanel::new(44_100.0, Session::demo());
        let _renderer = panel.renderer(44_100.0, Arc::default());
        let ctx = egui::Context::default();
        let key = |key, pressed| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        };
        for size in [[900.0, 700.0], [1440.0, 1000.0]] {
            draw(
                &mut panel,
                &ctx,
                size,
                vec![key(egui::Key::Num3, true)],
                true,
            );
            assert_eq!(panel.held, Some(2));
            draw(
                &mut panel,
                &ctx,
                size,
                vec![key(egui::Key::Num3, false)],
                true,
            );
            assert_eq!(panel.held, None);
            draw(
                &mut panel,
                &ctx,
                size,
                vec![key(egui::Key::Num1, true)],
                true,
            );
            draw(&mut panel, &ctx, size, vec![], false);
            assert_eq!(panel.held, None);
        }
        // Draw every section directly, not only those above the scroll viewport.
        let engine = Arc::clone(&panel.engine);
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut studio = control_lock(&engine, &panel.health);
                draw_steps(ui, &mut studio);
                draw_mixer(ui, &mut studio);
                panel.draw_performance(ui, ctx, &mut studio);
                panel.draw_automation(ui, &mut studio);
                panel.draw_import(ui, &mut studio);
            });
        });
        panel.deactivate();
        draw(
            &mut panel,
            &ctx,
            [900.0, 700.0],
            vec![
                key(egui::Key::Num2, true),
                key(egui::Key::Space, true),
                key(egui::Key::R, true),
            ],
            true,
        );
        assert_eq!(panel.held, None);
        let studio = control_lock(&panel.engine, &panel.health);
        assert!(!studio.playing() && !studio.recording());
    }

    #[test]
    fn recording_survives_deactivation_save_load_and_worker_export() {
        let mut panel = StudioPanel::new(48_000.0, Session::default());
        let mut renderer = panel.renderer(48_000.0, Arc::default());
        let mut frames = [StereoFrame::default(); BLOCK_SIZE];
        {
            let mut studio = control_lock(&panel.engine, &panel.health);
            studio.record(true);
            studio.play(true);
        }
        renderer.render(&mut frames, 48_000.0);
        {
            let mut studio = control_lock(&panel.engine, &panel.health);
            studio.chord_on(3, 0, true, 4, POLY_PRESET_PAD).unwrap();
            studio.hit(INSTRUMENT_KICK, 36, 0.9).unwrap();
            studio.set_control(Control::Gain(0), 0.3).unwrap();
            panel.held = Some(3);
        }
        for _ in 0..48 {
            renderer.render(&mut frames, 48_000.0);
        }
        drop(renderer); // Same ordering as the host: stop first, then finalize.
        panel.deactivate();
        let song = {
            let mut studio = control_lock(&panel.engine, &panel.health);
            assert!(!studio.playing() && !studio.recording());
            studio.snapshot_session()
        };
        assert_eq!(song.chords.len(), 1);
        assert!(song.chords[0].duration > 20);
        assert_eq!(song.hits.len(), 1);
        assert_eq!(song.automation.len(), 1);
        assert_eq!(panel.held, None);
        let base =
            std::env::temp_dir().join(format!("gooey-panel-recording-{}", std::process::id()));
        let json = base.with_extension("json");
        let wav = base.with_extension("wav");
        let ctx = egui::Context::default();
        // Use the same workers as the actual Save/Export WAV buttons.
        panel.save_worker(song.clone(), json.to_string_lossy().into_owned());
        for _ in 0..1000 {
            draw(&mut panel, &ctx, [900.0, 700.0], vec![], true);
            if panel.exporting.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(panel.status.starts_with("Saved "), "{}", panel.status);
        panel.export_worker(song, wav.to_string_lossy().into_owned(), 1);
        for _ in 0..2000 {
            draw(&mut panel, &ctx, [900.0, 700.0], vec![], true);
            if panel.exporting.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(panel.status.starts_with("Exported "), "{}", panel.status);
        let json_worker = json.clone();
        panel.load_worker(move || Studio::new(Session::load(json_worker)?, 48_000));
        for _ in 0..200 {
            draw(&mut panel, &ctx, [900.0, 700.0], vec![], true);
            if panel.loading.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(panel.loading.is_none(), "load worker did not finish");
        let mut renderer = panel.renderer(48_000.0, Arc::default());
        {
            let mut studio = control_lock(&panel.engine, &panel.health);
            assert_eq!(studio.session().chords.len(), 1);
            assert_eq!(studio.session().hits.len(), 1);
            assert_eq!(studio.session().automation.len(), 1);
            studio.play(true);
        }
        let mut peak = 0.0_f32;
        for _ in 0..200 {
            renderer.render(&mut frames, 48_000.0);
            for frame in frames {
                peak = peak.max(frame.l.abs()).max(frame.r.abs());
            }
        }
        assert!(peak > 0.01);
        let mut reader = hound::WavReader::open(&wav).unwrap();
        assert_eq!(reader.spec().channels, 2);
        assert_eq!(reader.spec().sample_rate, 48_000);
        assert!(reader.duration() > 48_000);
        let energy: f64 = reader
            .samples::<f32>()
            .map(|sample| {
                let sample = sample.unwrap();
                assert!(sample.is_finite());
                f64::from(sample).powi(2)
            })
            .sum();
        assert!(energy > 1.0);
        std::fs::remove_file(json).unwrap();
        std::fs::remove_file(wav).unwrap();
    }

    #[test]
    fn real_shell_switches_stop_studio_and_retain_saved_song() {
        let probe = Arc::new(Mutex::new(None));
        let factory_probe = Arc::clone(&probe);
        let factories: Vec<PanelFactory> = vec![
            Box::new(move |rate| {
                let panel = StudioPanel::new(rate, Session::demo());
                *factory_probe.lock().unwrap() = Some(Arc::clone(&panel.engine));
                Box::new(panel)
            }),
            Box::new(|rate| Box::new(StudioPanel::new(rate, Session::default()))),
        ];
        let mut app = OmniApp::new(factories, Some("Loop Studio"), true).unwrap();
        let engine = probe.lock().unwrap().as_ref().unwrap().clone();
        for _ in 0..8 {
            {
                let mut studio = engine.lock().unwrap();
                studio.play(true);
                studio.record(true);
                studio.chord_on(0, 0, true, 4, POLY_PRESET_PAD).unwrap();
            }
            app.select(1).unwrap();
            {
                let studio = engine.lock().unwrap();
                assert!(!studio.playing() && !studio.recording());
                assert!(studio.session().steps[0].iter().any(|value| *value > 0.0));
            }
            app.select(0).unwrap();
            assert!(!engine.lock().unwrap().playing());
        }
        drop(app);
        assert!(!engine.lock().unwrap().recording());
    }

    #[test]
    fn pending_rewind_cannot_restart_transport_after_deactivate_and_remount() {
        let mut panel = StudioPanel::new(44_100.0, Session::demo());
        let renderer = panel.renderer(44_100.0, Arc::default());
        let ctx = egui::Context::default();
        draw(
            &mut panel,
            &ctx,
            [900.0, 700.0],
            vec![egui::Event::Key {
                key: egui::Key::Num1,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::default(),
            }],
            true,
        );
        assert_eq!(panel.held, Some(0));
        let (tx, rx) = std::sync::mpsc::channel();
        panel.load_worker(move || {
            rx.recv().unwrap();
            let mut replacement = Studio::new(Session::demo(), 44_100)?;
            replacement.play(true);
            Ok(replacement)
        });
        drop(renderer);
        panel.deactivate();
        let _renderer = panel.renderer(44_100.0, Arc::default());
        tx.send(()).unwrap();
        for _ in 0..200 {
            draw(&mut panel, &ctx, [900.0, 700.0], vec![], true);
            if panel.loading.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(panel.loading.is_none());
        assert!(!control_lock(&panel.engine, &panel.health).playing());
        assert_eq!(panel.held, None);
        draw(
            &mut panel,
            &ctx,
            [900.0, 700.0],
            vec![egui::Event::Key {
                key: egui::Key::Num1,
                physical_key: None,
                pressed: true,
                repeat: true,
                modifiers: egui::Modifiers::default(),
            }],
            true,
        );
        assert_eq!(panel.held, None);
    }
}
