//! Desktop loop studio. `--silent` runs the real engine without an audio device.
use anyhow::{Context, Result};
use eframe::egui::{self, Color32, RichText};
use gooey::{
    ffi::*,
    studio::{Control, Session, Studio, LOOP_TICKS, TRACK_NAMES},
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

struct AudioClock {
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    #[cfg(feature = "native")]
    _stream: Option<cpal::Stream>,
}
impl Drop for AudioClock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
impl AudioClock {
    fn silent(engine: Arc<Mutex<Studio>>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let worker = std::thread::spawn(move || {
            let rate = engine.lock().unwrap().sample_rate();
            let period = Duration::from_secs_f64(512.0 / rate as f64);
            let mut deadline = Instant::now();
            let mut buffer = [0.0; 1024];
            while !flag.load(Ordering::Acquire) {
                if let Ok(mut s) = engine.lock() {
                    let _ = s.render(&mut buffer);
                }
                deadline += period;
                if let Some(delay) = deadline.checked_duration_since(Instant::now()) {
                    std::thread::sleep(delay);
                } else {
                    deadline = Instant::now();
                }
            }
        });
        Self {
            stop,
            worker: Some(worker),
            #[cfg(feature = "native")]
            _stream: None,
        }
    }
}
#[cfg(feature = "native")]
fn native(session: Session) -> Result<(Arc<Mutex<Studio>>, AudioClock, String)> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let device = cpal::default_host()
        .default_output_device()
        .context("no audio device")?;
    let supported = device.default_output_config()?;
    let config: cpal::StreamConfig = supported.clone().into();
    let engine = Arc::new(Mutex::new(Studio::new(session, config.sample_rate.0)?));
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => make_stream::<f32>(&device, &config, engine.clone())?,
        cpal::SampleFormat::I16 => make_stream::<i16>(&device, &config, engine.clone())?,
        cpal::SampleFormat::U16 => make_stream::<u16>(&device, &config, engine.clone())?,
        other => anyhow::bail!("unsupported audio format {other}; use --silent"),
    };
    stream.play()?;
    Ok((
        engine,
        AudioClock {
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
            _stream: Some(stream),
        },
        format!(
            "LIVE AUDIO · {} · {} Hz",
            device.name()?,
            config.sample_rate.0
        ),
    ))
}
#[cfg(feature = "native")]
fn make_stream<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    engine: Arc<Mutex<Studio>>,
) -> Result<cpal::Stream> {
    use cpal::traits::DeviceTrait;
    let channels = config.channels as usize;
    let mut scratch = [0.0_f32; 2048];
    Ok(device.build_output_stream(
        config,
        move |out: &mut [T], _| {
            if let Ok(mut s) = engine.lock() {
                for block in out.chunks_mut(channels * 1024) {
                    let frames = block.len() / channels;
                    if s.render(&mut scratch[..frames * 2]).is_err() {
                        scratch.fill(0.0);
                    }
                    for (i, frame) in block.chunks_mut(channels).enumerate() {
                        let l = scratch[i * 2];
                        let r = scratch[i * 2 + 1];
                        for (ch, v) in frame.iter_mut().enumerate() {
                            *v = T::from_sample(if channels == 1 || ch > 1 {
                                (l + r) * 0.5
                            } else if ch == 0 {
                                l
                            } else {
                                r
                            });
                        }
                    }
                }
            } else {
                for v in out {
                    *v = T::from_sample(0.0);
                }
            }
        },
        |e| eprintln!("audio stream: {e}"),
        None,
    )?)
}
struct App {
    engine: Arc<Mutex<Studio>>,
    _clock: AudioClock,
    mode: String,
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
    exporting: Option<std::sync::mpsc::Receiver<String>>,
    loading: Option<std::sync::mpsc::Receiver<std::result::Result<Studio, String>>>,
    selected_lane: usize,
}
fn knob(ui: &mut egui::Ui, s: &mut Studio, c: Control, label: &str) {
    let mut v = s.session().value(c);
    let slider = egui::Slider::new(&mut v, c.range())
        .text(label)
        .logarithmic(matches!(c, Control::Cutoff(_)));
    if ui.add(slider).changed() {
        let _ = s.set_control(c, v);
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(33));
        if let Some(rx) = &self.exporting {
            if let Ok(msg) = rx.try_recv() {
                self.status = msg;
                self.exporting = None;
            }
        }
        if let Some(rx) = &self.loading {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok(replacement) => {
                        // Construction and file I/O occur off the audio lock;
                        // reclaim the old engine only after releasing the lock.
                        let old = {
                            let mut live = self.engine.lock().unwrap();
                            std::mem::replace(&mut *live, replacement)
                        };
                        drop(old);
                        self.held = None;
                        self.status = "Loaded session (stopped; press Play)".into();
                    }
                    Err(error) => self.status = error,
                }
                self.loading = None;
            }
        }
        let engine = self.engine.clone();
        let mut s = engine.lock().unwrap();
        // Keyboard gestures are real engine controls, not a scripted UI demo.
        if !ctx.wants_keyboard_input() {
            if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
                let on = !s.playing();
                s.play(on);
            }
            if ctx.input(|i| i.key_pressed(egui::Key::R)) {
                let on = !s.recording();
                s.record(on);
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Num0)) {
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
                if ctx.input(|i| i.key_pressed(key)) {
                    let _ = s.chord_on(
                        degree as u32,
                        self.root,
                        self.minor,
                        self.octave,
                        self.preset,
                    );
                    self.held = Some(degree as u32);
                }
            }
            for (key, id) in [
                (egui::Key::Z, INSTRUMENT_KICK),
                (egui::Key::X, INSTRUMENT_SNARE),
                (egui::Key::C, INSTRUMENT_HIHAT),
                (egui::Key::V, INSTRUMENT_TOM),
                (egui::Key::B, INSTRUMENT_BASS),
            ] {
                if ctx.input(|i| i.key_pressed(key)) {
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
        if !ctx.input(|i| i.focused) && self.held.is_some() {
            s.chord_off();
            self.held = None;
        }
        egui::TopBottomPanel::top("transport").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("GOOEY / LOOP STUDIO");
                ui.separator();
                ui.label(RichText::new(&self.mode).color(Color32::from_rgb(100, 200, 180)));
            });
            ui.horizontal(|ui| {
                if ui
                    .button(if s.playing() { "■ Stop" } else { "▶ Play" })
                    .clicked()
                {
                    let on = !s.playing();
                    s.play(on);
                }
                if ui.button("↤ Rewind").clicked() {
                    if let Err(e) = s.rewind() {
                        self.status = e.to_string();
                    }
                }
                let rec = s.recording();
                if ui.selectable_label(rec, "● Record / overdub [R]").clicked() {
                    s.record(!rec);
                }
                let mut bpm = s.session().bpm;
                if ui
                    .add(egui::Slider::new(&mut bpm, 40.0..=240.0).text("BPM"))
                    .changed()
                {
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
                if ui
                    .add_enabled(self.loading.is_none(), egui::Button::new("Load"))
                    .clicked()
                {
                    let path = self.path.clone();
                    let rate = s.sample_rate();
                    self.load_worker(move || Studio::new(Session::load(path)?, rate));
                }
                if ui.button("Demo song").clicked() {
                    if let Err(e) = s.replace(Session::demo()) {
                        self.status = e.to_string();
                    }
                }
                if ui.button("New empty").clicked() {
                    if let Err(e) = s.replace(Session::default()) {
                        self.status = e.to_string();
                    }
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
impl App {
    fn load_worker(&mut self, prepare: impl FnOnce() -> Result<Studio> + Send + 'static) {
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
                }
            }
            if ui.button("Release / 0").clicked() {
                s.chord_off();
                self.held = None;
            }
        });
        if ctx.input(|i| i.pointer.any_released()) && self.held.is_some() {
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
            if ui
                .add(
                    egui::Slider::new(&mut source_bpm, 40.0..=240.0)
                        .text("Loop source BPM (not automated)"),
                )
                .changed()
            {
                let _ = s.set_loop_source_tempo(source_bpm);
            }
        }
    }
}
fn main() -> Result<()> {
    let mut silent = false;
    let mut song = Session::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--silent" => silent = true,
            "--demo" => song = Session::demo(),
            "--load" => song = Session::load(args.next().context("--load needs a path")?)?,
            "--help" => {
                println!("loop_studio [--silent] [--demo] [--load session.json]");
                return Ok(());
            }
            _ => anyhow::bail!("unknown argument {arg}"),
        }
    }
    #[cfg(feature = "native")]
    let audio = if silent {
        None
    } else {
        match native(song.clone()) {
            Ok(audio) => Some(audio),
            Err(e) => {
                eprintln!("Audio unavailable ({e}); using silent render clock");
                None
            }
        }
    };
    #[cfg(not(feature = "native"))]
    let audio: Option<(Arc<Mutex<Studio>>, AudioClock, String)> = {
        let _ = silent;
        None
    };
    let (engine, clock, mode) = if let Some(audio) = audio {
        audio
    } else {
        let engine = Arc::new(Mutex::new(Studio::new(song, 48000)?));
        let clock = AudioClock::silent(engine.clone());
        (
            engine,
            clock,
            "SILENT CLOCK · engine active · no device output".into(),
        )
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1380.0, 940.0])
            .with_title("Gooey Loop Studio"),
        ..Default::default()
    };
    eframe::run_native(
        "Gooey Loop Studio",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(App {
                engine,
                _clock: clock,
                mode,
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
                exporting: None,
                loading: None,
                selected_lane: 0,
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!(e.to_string()))
}
