//! Readable native graph editor and beat sequencer for `ResonatorVoice`.
//!
//! Run with:
//!
//!     cargo run --example resonator_voice_gui --features native,visualization

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use gooey::engine::{Engine, EngineOutput, Instrument, Sequencer};
use gooey::instruments::{
    ResonatorExciterShape, ResonatorOutputTap, ResonatorVoice, ResonatorVoiceConfig,
};

const SAMPLE_RATE: f32 = 44_100.0;
const SCOPE_CAPACITY: usize = 2_048;
const TRACK_NAMES: [&str; 4] = ["Kick", "Snare", "Tom", "Hybrid"];
const PATCH_NAMES: [&str; 5] = ["Kick", "Tom", "Snare", "Hybrid", "Metallic drone"];
const INSTRUMENT_NAMES: [&str; 4] = ["graph_kick", "graph_snare", "graph_tom", "graph_hybrid"];
const EDGE_NAMES: [&str; 9] = [
    "Transient → Body core",
    "Transient → Character core",
    "Filtered noise → Body core",
    "Filtered noise → Character core",
    "Body core → Character core",
    "Transient → Output",
    "Filtered noise → Output",
    "Body core → Output",
    "Character core → Output",
];

fn factory_patch(index: usize) -> ResonatorVoiceConfig {
    match index.min(PATCH_NAMES.len() - 1) {
        0 => ResonatorVoiceConfig::kick(),
        1 => ResonatorVoiceConfig::tom(),
        2 => ResonatorVoiceConfig::snare(),
        3 => ResonatorVoiceConfig::hybrid(),
        _ => ResonatorVoiceConfig::metallic_drone(),
    }
}

fn initial_patch_for_track(index: usize) -> usize {
    [0, 2, 1, 3][index]
}

fn track_config(index: usize) -> ResonatorVoiceConfig {
    factory_patch(initial_patch_for_track(index))
}

struct Scope {
    samples: Box<[AtomicU32]>,
    cursor: AtomicUsize,
}

impl Scope {
    fn new(capacity: usize) -> Self {
        Self {
            samples: (0..capacity)
                .map(|_| AtomicU32::new(0.0_f32.to_bits()))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            cursor: AtomicUsize::new(0),
        }
    }

    fn push(&self, sample: f32) {
        let sequence = self.cursor.load(Ordering::Relaxed);
        self.samples[sequence % self.samples.len()].store(sample.to_bits(), Ordering::Relaxed);
        self.cursor
            .store(sequence.wrapping_add(1), Ordering::Release);
    }

    fn snapshot(&self) -> Vec<f32> {
        let end = self.cursor.load(Ordering::Acquire);
        let count = end.min(self.samples.len());
        (end.wrapping_sub(count)..end)
            .map(|sequence| {
                f32::from_bits(self.samples[sequence % self.samples.len()].load(Ordering::Relaxed))
            })
            .collect()
    }
}

struct SharedVoice {
    voice: Arc<Mutex<ResonatorVoice>>,
    scope: Arc<Scope>,
}

impl Instrument for SharedVoice {
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        self.voice
            .lock()
            .unwrap()
            .trigger_with_velocity(time, velocity);
    }

    fn tick(&mut self, time: f64) -> f32 {
        let sample = self.voice.lock().unwrap().tick(time);
        self.scope.push(sample);
        sample
    }

    fn is_active(&self) -> bool {
        self.voice.lock().unwrap().is_active()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum View {
    Topology,
    Sequencer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Inspector {
    Trigger,
    Noise,
    Body,
    Character,
    Output,
    Routing,
}

struct ResonatorLab {
    engine: Arc<Mutex<Engine>>,
    _output: EngineOutput,
    voices: [Arc<Mutex<ResonatorVoice>>; 4],
    configs: [ResonatorVoiceConfig; 4],
    track_patches: [usize; 4],
    scope: Arc<Scope>,
    view: View,
    inspector: Inspector,
    track: usize,
    velocity: f32,
    bpm: f32,
    playing: bool,
    patterns: [[bool; 16]; 4],
}

impl ResonatorLab {
    fn new(
        engine: Arc<Mutex<Engine>>,
        output: EngineOutput,
        voices: [Arc<Mutex<ResonatorVoice>>; 4],
        scope: Arc<Scope>,
    ) -> Self {
        Self {
            engine,
            _output: output,
            voices,
            configs: std::array::from_fn(track_config),
            track_patches: std::array::from_fn(initial_patch_for_track),
            scope,
            view: View::Topology,
            inspector: Inspector::Body,
            track: 0,
            velocity: 0.8,
            bpm: 120.0,
            playing: false,
            patterns: starter_patterns(),
        }
    }

    fn trigger(&self) {
        self.engine
            .lock()
            .unwrap()
            .trigger_instrument_with_velocity(INSTRUMENT_NAMES[self.track], self.velocity);
    }

    fn set_transport(&mut self, playing: bool) {
        self.playing = playing;
        let mut engine = self.engine.lock().unwrap();
        for index in 0..4 {
            if let Some(sequence) = engine.sequencer_mut(index) {
                if playing {
                    sequence.start();
                } else {
                    sequence.stop();
                }
            }
        }
    }

    fn update_tempo(&self) {
        let mut engine = self.engine.lock().unwrap();
        engine.set_bpm(self.bpm);
        for index in 0..4 {
            if let Some(sequence) = engine.sequencer_mut(index) {
                sequence.set_bpm(self.bpm);
            }
        }
    }

    fn toggle_step(&mut self, track: usize, step: usize) {
        self.patterns[track][step] = !self.patterns[track][step];
        if let Some(sequence) = self.engine.lock().unwrap().sequencer_mut(track) {
            sequence.set_step(step, self.patterns[track][step]);
        }
    }

    fn apply_track(&self) {
        let config = self.configs[self.track];
        let mut voice = self.voices[self.track].lock().unwrap();
        voice.set_base_frequency_hz(config.base_frequency_hz);
        voice.set_sweep_seconds(config.sweep_seconds);
        voice.set_exciter_shape(config.exciter.shape);
        voice.set_exciter_width_ms(config.exciter.width_ms);
        voice.set_exciter_level(config.exciter.level);
        voice.set_noise_config(config.noise);
        for (index, mode) in [config.mode1, config.mode2].into_iter().enumerate() {
            voice.set_mode_frequency_ratio(index, mode.frequency_ratio);
            voice.set_mode_tuning_semitones(index, mode.tuning_semitones);
            voice.set_mode_pitch_sweep_octaves(index, mode.pitch_sweep_octaves);
            voice.set_mode_decay_seconds(index, mode.decay_seconds);
            voice.set_mode_feedback(index, mode.feedback);
            voice.set_mode_damping_override(index, mode.damping_override);
            voice.set_mode_drive(index, mode.drive);
            voice.set_mode_level(index, mode.level);
            voice.set_output_tap(index, mode.tap);
        }
        voice.set_routing(config.routing);
        voice.set_output_volume(config.volume);
    }

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Resonator Percussion Lab");
                ui.separator();
                ui.selectable_value(&mut self.view, View::Topology, "Signal topology");
                ui.selectable_value(&mut self.view, View::Sequencer, "Beat sequencer");
                ui.separator();
                egui::ComboBox::from_id_salt("track")
                    .selected_text(format!("Editing: {}", TRACK_NAMES[self.track]))
                    .show_ui(ui, |ui| {
                        for (index, name) in TRACK_NAMES.iter().enumerate() {
                            ui.selectable_value(&mut self.track, index, *name);
                        }
                    });
                let previous_patch = self.track_patches[self.track];
                egui::ComboBox::from_id_salt("factory_patch")
                    .selected_text(format!(
                        "Patch: {}",
                        PATCH_NAMES[self.track_patches[self.track]]
                    ))
                    .show_ui(ui, |ui| {
                        for (index, name) in PATCH_NAMES.iter().enumerate() {
                            ui.selectable_value(&mut self.track_patches[self.track], index, *name);
                        }
                    });
                if self.track_patches[self.track] != previous_patch {
                    self.configs[self.track] = factory_patch(self.track_patches[self.track]);
                    *self.voices[self.track].lock().unwrap() =
                        ResonatorVoice::with_config(SAMPLE_RATE, self.configs[self.track]);
                }
                if ui.button("Trigger hit  [Space]").clicked() {
                    self.trigger();
                }
                ui.add(egui::Slider::new(&mut self.velocity, 0.0..=1.0).text("velocity"));
            });
        });
    }

    fn inspector(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("inspector")
            .default_width(330.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading("Inspector");
                ui.label("Every value here controls the running audio graph.");
                ui.separator();
                let config = &mut self.configs[self.track];
                let mut changed = false;
                match self.inspector {
                    Inspector::Trigger => {
                        ui.heading("Trigger & transient exciter");
                        ui.label("A short pulse, sampled click, or noise burst that strikes either resonant core and may also reach the output directly.");
                        egui::ComboBox::from_label("Exciter shape")
                            .selected_text(format!("{:?}", config.exciter.shape))
                            .show_ui(ui, |ui| {
                                changed |= ui.selectable_value(&mut config.exciter.shape, ResonatorExciterShape::Pulse, "Pulse").changed();
                                changed |= ui.selectable_value(&mut config.exciter.shape, ResonatorExciterShape::Click, "Click table").changed();
                                changed |= ui.selectable_value(&mut config.exciter.shape, ResonatorExciterShape::Noise, "Noise burst").changed();
                            });
                        changed |= ui.add(egui::Slider::new(&mut config.exciter.width_ms, 0.25..=8.0).logarithmic(true).text("width (ms)")).changed();
                        changed |= ui.add(egui::Slider::new(&mut config.exciter.level, 0.0..=2.0).text("level")).changed();
                        changed |= ui.add(egui::Slider::new(&mut config.sweep_seconds, 0.002..=4.0).logarithmic(true).text("pitch sweep time (s)")).changed();
                    }
                    Inspector::Noise => {
                        ui.heading("Noise generator, filter & envelope");
                        ui.label("Deterministic white noise passes through the current band-pass filter, then an independent attack/decay amplitude envelope.");
                        changed |= ui.add(egui::Slider::new(&mut config.noise.attack_seconds, 0.0001..=1.0).logarithmic(true).text("attack (s)")).changed();
                        changed |= ui.add(egui::Slider::new(&mut config.noise.decay_seconds, 0.001..=8.0).logarithmic(true).text("decay / T60 (s)")).changed();
                        changed |= ui.add(egui::Slider::new(&mut config.noise.filter_hz, 20.0..=20_000.0).logarithmic(true).text("band-pass frequency (Hz)")).changed();
                        changed |= ui.add(egui::Slider::new(&mut config.noise.filter_q, 0.5..=20.0).logarithmic(true).text("filter Q")).changed();
                        changed |= ui.add(egui::Slider::new(&mut config.noise.level, 0.0..=2.0).text("noise VCA level")).changed();
                        changed |= ui.add(egui::Slider::new(&mut config.noise.velocity_response, 0.0..=1.0).text("velocity response")).changed();
                    }
                    Inspector::Body => {
                        ui.heading("Body resonant core");
                        ui.label("The primary pitched resonator. Its exponential pitch envelope, decay, saturating internal feedback, tap, and driven output define the drum body.");
                        changed |= ui.add(egui::Slider::new(&mut config.base_frequency_hz, 20.0..=400.0).logarithmic(true).text("base pitch (Hz)")).changed();
                        changed |= mode_controls(ui, &mut config.mode1);
                    }
                    Inspector::Character => {
                        ui.heading("Character resonant core");
                        ui.label("A second independently tuned mode. It can receive transient, filtered noise, and the raw Body response for parallel or serial topologies.");
                        changed |= mode_controls(ui, &mut config.mode2);
                    }
                    Inspector::Output => {
                        ui.heading("Drive & output mixer");
                        ui.label("Each core tap is oversampled and soft-clipped, then mixed with optional direct transient and noise paths.");
                        changed |= ui.add(egui::Slider::new(&mut config.volume, 0.0..=2.0).text("final output level")).changed();
                        ui.separator();
                        ui.label("The engine currently uses soft clipping rather than the Ultra-Perc's analog wave-folder.");
                    }
                    Inspector::Routing => {
                        ui.heading("Routing matrix");
                        ui.label("These nine feed-forward gains create parallel, serial, and hybrid voices without a cyclic inter-core feedback path.");
                        let mut values = routing_values(config);
                        for (index, value) in values.iter_mut().enumerate() {
                            changed |= ui.add(egui::Slider::new(value, 0.0..=1.0).text(EDGE_NAMES[index])).changed();
                        }
                        if changed {
                            set_routing_values(config, values);
                        }
                    }
                }
                if changed {
                    self.apply_track();
                }
                ui.separator();
                if ui.button("Reset this track patch").clicked() {
                    self.configs[self.track] = factory_patch(self.track_patches[self.track]);
                    *self.voices[self.track].lock().unwrap() =
                        ResonatorVoice::with_config(SAMPLE_RATE, self.configs[self.track]);
                }
            });
    }

    fn topology(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(format!("{} topology", TRACK_NAMES[self.track]));
            ui.label("Click a block or cable group to inspect and edit it.");
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Reference vocabulary:");
            ui.hyperlink_to(
                "SSF Entity Ultra-Perc twin-core percussion architecture",
                "https://steadystatefate.com/products/entity-ultra-perc",
            );
            ui.label("— the blocks below describe this engine's actual implementation.");
        });
        ui.label("Signal travels left → right. Cable width and brightness show gain; a hairline means that route is effectively muted. Click any block, or empty graph space for all nine routing gains.");
        let available = ui.available_size();
        let height = (available.y - 170.0).max(420.0);
        let (response, painter) =
            ui.allocate_painter(Vec2::new(available.x, height), Sense::click());
        let area = response.rect.shrink(18.0);
        let node_size = Vec2::new((area.width() * 0.19).clamp(155.0, 240.0), 112.0);
        let node = |x: f32, y: f32| {
            Rect::from_min_size(
                Pos2::new(
                    area.left() + area.width() * x,
                    area.top() + area.height() * y,
                ),
                node_size,
            )
        };
        let trigger = node(0.01, 0.12);
        let noise = node(0.01, 0.60);
        let body = node(0.31, 0.12);
        let character = node(0.55, 0.48);
        let output = node(0.80, 0.28);
        let values = routing_values(&self.configs[self.track]);
        let edges = [
            (trigger, body, values[0]),
            (trigger, character, values[1]),
            (noise, body, values[2]),
            (noise, character, values[3]),
            (body, character, values[4]),
            (trigger, output, values[5]),
            (noise, output, values[6]),
            (body, output, values[7]),
            (character, output, values[8]),
        ];
        for (from, to, gain) in edges {
            draw_cable(&painter, from.right_center(), to.left_center(), gain);
        }
        if response.clicked() {
            if let Some(position) = response.interact_pointer_pos() {
                self.inspector = if trigger.contains(position) {
                    Inspector::Trigger
                } else if noise.contains(position) {
                    Inspector::Noise
                } else if body.contains(position) {
                    Inspector::Body
                } else if character.contains(position) {
                    Inspector::Character
                } else if output.contains(position) {
                    Inspector::Output
                } else {
                    Inspector::Routing
                };
            }
        }
        draw_node(
            &painter,
            trigger,
            "TRIGGER / EXCITER",
            "pulse · click · noise burst",
            self.inspector == Inspector::Trigger,
            Color32::from_rgb(151, 76, 48),
        );
        draw_node(
            &painter,
            noise,
            "NOISE VOICE",
            "RNG → band-pass → attack/decay VCA",
            self.inspector == Inspector::Noise,
            Color32::from_rgb(94, 64, 139),
        );
        draw_node(
            &painter,
            body,
            "BODY CORE",
            "pitch env → resonator → tap → drive",
            self.inspector == Inspector::Body,
            Color32::from_rgb(33, 120, 154),
        );
        draw_node(
            &painter,
            character,
            "CHARACTER CORE",
            "ratio/detune → resonator → tap → drive",
            self.inspector == Inspector::Character,
            Color32::from_rgb(38, 137, 101),
        );
        draw_node(
            &painter,
            output,
            "OUTPUT MIXER",
            "5 paths → final gain → engine limiter",
            self.inspector == Inspector::Output,
            Color32::from_rgb(160, 111, 40),
        );
        let route_rect = Rect::from_min_size(
            Pos2::new(area.left() + area.width() * 0.34, area.bottom() - 50.0),
            Vec2::new(260.0, 34.0),
        );
        painter.rect_filled(
            route_rect,
            6.0,
            if self.inspector == Inspector::Routing {
                Color32::from_rgb(105, 84, 31)
            } else {
                Color32::from_rgb(42, 48, 57)
            },
        );
        painter.text(
            route_rect.center(),
            Align2::CENTER_CENTER,
            "ROUTING MATRIX — click empty space",
            FontId::proportional(14.0),
            Color32::WHITE,
        );
        self.scope(ui);
    }

    fn scope(&self, ui: &mut egui::Ui) {
        ui.separator();
        ui.label("Mixed output waveform");
        let (response, painter) =
            ui.allocate_painter(Vec2::new(ui.available_width(), 105.0), Sense::hover());
        painter.rect_filled(response.rect, 5.0, Color32::from_rgb(15, 19, 27));
        painter.hline(
            response.rect.x_range(),
            response.rect.center().y,
            Stroke::new(1.0, Color32::from_gray(55)),
        );
        let samples = self.scope.snapshot();
        if samples.len() > 1 {
            let points = samples
                .iter()
                .enumerate()
                .map(|(index, sample)| {
                    Pos2::new(
                        egui::lerp(
                            response.rect.x_range(),
                            index as f32 / (samples.len() - 1) as f32,
                        ),
                        response.rect.center().y
                            - (sample * response.rect.height() * 0.42).clamp(
                                -response.rect.height() * 0.45,
                                response.rect.height() * 0.45,
                            ),
                    )
                })
                .collect::<Vec<_>>();
            painter.add(egui::Shape::line(
                points,
                Stroke::new(1.5, Color32::from_rgb(63, 225, 230)),
            ));
        }
    }

    fn sequencer(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("16-step beat sequencer");
            if ui
                .button(if self.playing { "■ Stop" } else { "▶ Play" })
                .clicked()
            {
                self.set_transport(!self.playing);
            }
            if ui
                .add(egui::Slider::new(&mut self.bpm, 40.0..=240.0).text("BPM"))
                .changed()
            {
                self.update_tempo();
            }
            if ui.button("Clear").clicked() {
                for track in 0..4 {
                    for step in 0..16 {
                        if self.patterns[track][step] {
                            self.toggle_step(track, step);
                        }
                    }
                }
            }
        });
        ui.label("Each row is an independent ResonatorVoice. Click cells to build a beat; red outlines show the sample-accurate playhead.");
        let playheads: [usize; 4] = {
            let engine = self.engine.lock().unwrap();
            std::array::from_fn(|index| {
                engine
                    .sequencer(index)
                    .map(|sequence| sequence.current_step())
                    .unwrap_or(0)
            })
        };
        egui::Grid::new("beat_grid")
            .spacing([5.0, 7.0])
            .show(ui, |ui| {
                ui.label("");
                for step in 0..16 {
                    ui.label(egui::RichText::new(format!("{}", step + 1)).small().color(
                        if step % 4 == 0 {
                            Color32::WHITE
                        } else {
                            Color32::GRAY
                        },
                    ));
                }
                ui.end_row();
                for track in 0..4 {
                    if ui
                        .selectable_label(self.track == track, TRACK_NAMES[track])
                        .clicked()
                    {
                        self.track = track;
                    }
                    for step in 0..16 {
                        let on = self.patterns[track][step];
                        let playhead = self.playing && playheads[track] == step;
                        let button = egui::Button::new(if on { "●" } else { "·" })
                            .min_size(Vec2::splat(32.0))
                            .fill(if on {
                                Color32::from_rgb(30, 145, 122)
                            } else {
                                Color32::from_rgb(31, 37, 48)
                            })
                            .stroke(if playhead {
                                Stroke::new(3.0, Color32::LIGHT_RED)
                            } else if step % 4 == 0 {
                                Stroke::new(1.0, Color32::from_gray(100))
                            } else {
                                Stroke::NONE
                            });
                        if ui.add(button).clicked() {
                            self.toggle_step(track, step);
                        }
                    }
                    ui.end_row();
                }
            });
        ui.separator();
        ui.label("Starter pattern: four-on-the-floor body, backbeat noise/snare, syncopated tom. The Hybrid row starts empty for experiments.");
        self.scope(ui);
    }
}

impl eframe::App for ResonatorLab {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|input| input.key_pressed(egui::Key::Space)) {
            if self.view == View::Topology {
                self.trigger();
            } else {
                self.set_transport(!self.playing);
            }
        }
        self.top_bar(ctx);
        if self.view == View::Topology {
            self.inspector(ctx);
        }
        egui::CentralPanel::default().show(ctx, |ui| match self.view {
            View::Topology => self.topology(ui),
            View::Sequencer => self.sequencer(ui),
        });
        ctx.request_repaint_after(Duration::from_millis(16));
    }
}

fn mode_controls(ui: &mut egui::Ui, mode: &mut gooey::instruments::ResonatorModeConfig) -> bool {
    let mut changed = false;
    changed |= ui
        .add(
            egui::Slider::new(&mut mode.frequency_ratio, 0.0625..=16.0)
                .logarithmic(true)
                .text("frequency ratio"),
        )
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut mode.tuning_semitones, -24.0..=24.0).text("detune (semitones)"))
        .changed();
    changed |= ui
        .add(
            egui::Slider::new(&mut mode.pitch_sweep_octaves, -8.0..=8.0)
                .text("FM / pitch sweep (octaves)"),
        )
        .changed();
    changed |= ui
        .add(
            egui::Slider::new(&mut mode.decay_seconds, 0.005..=20.0)
                .logarithmic(true)
                .text("ring decay / T60 (s)"),
        )
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut mode.feedback, 0.0..=0.8).text("internal feedback"))
        .changed();
    changed |= ui
        .add(
            egui::Slider::new(&mut mode.damping_override, 0.0..=1.0)
                .text("damping override (0 = T60)"),
        )
        .changed();
    changed |= ui
        .add(
            egui::Slider::new(&mut mode.drive, 0.1..=24.0)
                .logarithmic(true)
                .text("soft-clip drive"),
        )
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut mode.level, 0.0..=2.0).text("core output level"))
        .changed();
    egui::ComboBox::from_label("output tap")
        .selected_text(format!("{:?}", mode.tap))
        .show_ui(ui, |ui| {
            changed |= ui
                .selectable_value(&mut mode.tap, ResonatorOutputTap::Lowpass, "Low-pass body")
                .changed();
            changed |= ui
                .selectable_value(
                    &mut mode.tap,
                    ResonatorOutputTap::Bandpass,
                    "Band-pass motion",
                )
                .changed();
        });
    changed
}

fn routing_values(config: &ResonatorVoiceConfig) -> [f32; 9] {
    let r = config.routing;
    [
        r.transient_to_mode1,
        r.transient_to_mode2,
        r.noise_to_mode1,
        r.noise_to_mode2,
        r.mode1_to_mode2,
        r.transient_to_output,
        r.noise_to_output,
        r.mode1_to_output,
        r.mode2_to_output,
    ]
}

fn set_routing_values(config: &mut ResonatorVoiceConfig, values: [f32; 9]) {
    config.routing.transient_to_mode1 = values[0];
    config.routing.transient_to_mode2 = values[1];
    config.routing.noise_to_mode1 = values[2];
    config.routing.noise_to_mode2 = values[3];
    config.routing.mode1_to_mode2 = values[4];
    config.routing.transient_to_output = values[5];
    config.routing.noise_to_output = values[6];
    config.routing.mode1_to_output = values[7];
    config.routing.mode2_to_output = values[8];
}

fn draw_node(
    painter: &egui::Painter,
    rect: Rect,
    title: &str,
    subtitle: &str,
    selected: bool,
    color: Color32,
) {
    painter.rect_filled(rect, 10.0, color);
    painter.rect_stroke(
        rect,
        10.0,
        Stroke::new(
            if selected { 3.0 } else { 1.0 },
            if selected {
                Color32::YELLOW
            } else {
                Color32::from_gray(170)
            },
        ),
    );
    painter.text(
        rect.center_top() + Vec2::new(0.0, 28.0),
        Align2::CENTER_CENTER,
        title,
        FontId::proportional(17.0),
        Color32::WHITE,
    );
    painter.text(
        rect.center() + Vec2::new(0.0, 15.0),
        Align2::CENTER_CENTER,
        subtitle,
        FontId::proportional(12.0),
        Color32::from_gray(225),
    );
}

fn draw_cable(painter: &egui::Painter, from: Pos2, to: Pos2, gain: f32) {
    let control = (to.x - from.x).abs() * 0.45;
    let shape = egui::epaint::CubicBezierShape::from_points_stroke(
        [
            from,
            from + Vec2::new(control, 0.0),
            to - Vec2::new(control, 0.0),
            to,
        ],
        false,
        Color32::TRANSPARENT,
        Stroke::new(
            1.0 + gain * 5.0,
            Color32::from_rgb(
                (65.0 + gain * 120.0) as u8,
                (95.0 + gain * 140.0) as u8,
                (120.0 + gain * 120.0) as u8,
            ),
        ),
    );
    painter.add(shape);
}

fn starter_patterns() -> [[bool; 16]; 4] {
    [
        [
            true, false, false, false, true, false, false, false, true, false, false, false, true,
            false, false, false,
        ],
        [
            false, false, false, false, true, false, false, false, false, false, false, false,
            true, false, false, false,
        ],
        [
            false, false, true, false, false, false, false, false, false, false, true, false,
            false, false, false, false,
        ],
        [false; 16],
    ]
}

fn main() -> anyhow::Result<()> {
    let scope = Arc::new(Scope::new(SCOPE_CAPACITY));
    let voices: [Arc<Mutex<ResonatorVoice>>; 4] = std::array::from_fn(|index| {
        Arc::new(Mutex::new(ResonatorVoice::with_config(
            SAMPLE_RATE,
            track_config(index),
        )))
    });
    let patterns = starter_patterns();
    let mut engine = Engine::new(SAMPLE_RATE);
    engine.set_master_gain(0.8);
    for index in 0..4 {
        engine.add_instrument(
            INSTRUMENT_NAMES[index],
            Box::new(SharedVoice {
                voice: Arc::clone(&voices[index]),
                scope: Arc::clone(&scope),
            }),
        );
        engine.add_sequencer(Sequencer::with_pattern(
            120.0,
            SAMPLE_RATE,
            patterns[index].to_vec(),
            INSTRUMENT_NAMES[index],
        ));
    }
    let engine = Arc::new(Mutex::new(engine));
    let mut output = EngineOutput::new();
    output.initialize(SAMPLE_RATE)?;
    output.create_stream_with_engine(Arc::clone(&engine))?;
    output.start()?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1_440.0, 900.0])
            .with_min_inner_size([1_050.0, 680.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Resonator Percussion Lab",
        options,
        Box::new(move |context| {
            context.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(ResonatorLab::new(engine, output, voices, scope)))
        }),
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_pattern_is_a_four_track_backbeat() {
        let pattern = starter_patterns();
        assert_eq!(pattern[0].iter().filter(|step| **step).count(), 4);
        assert!(pattern[1][4] && pattern[1][12]);
        assert!(pattern[2][2] && pattern[2][10]);
        assert!(pattern[3].iter().all(|step| !step));
    }

    #[test]
    fn routing_round_trips_through_the_editor_projection() {
        let mut config = ResonatorVoiceConfig::hybrid();
        let expected = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9];
        set_routing_values(&mut config, expected);
        assert_eq!(routing_values(&config), expected);
    }
}
