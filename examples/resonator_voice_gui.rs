//! Native graph laboratory for the flexible dual-resonator percussion voice.
//!
//! Run with:
//!
//!     cargo run --example resonator_voice_gui --features native,visualization

use std::ffi::CString;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use glfw::{Action, Context, GlfwReceiver, Key, Modifiers, MouseButton, WindowEvent};

use gooey::engine::{Engine, EngineOutput, Instrument, Sequencer};
use gooey::instruments::{
    ResonatorExciterShape, ResonatorOutputTap, ResonatorVoice, ResonatorVoiceConfig,
};

const SAMPLE_RATE: f32 = 44_100.0;
const SCOPE_CAPACITY: usize = 2_048;
const PRESET_NAMES: [&str; 5] = ["Kick", "Tom", "Snare", "Hybrid", "Metallic drone"];
const TRACK_NAMES: [&str; 4] = ["KICK", "SNARE", "TOM", "HYBRID"];
const INSTRUMENT_NAMES: [&str; 4] = ["graph_kick", "graph_snare", "graph_tom", "graph_hybrid"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum View {
    Graph,
    Sequencer,
}

fn factory_preset(index: usize) -> ResonatorVoiceConfig {
    match index.min(4) {
        0 => ResonatorVoiceConfig::kick(),
        1 => ResonatorVoiceConfig::tom(),
        2 => ResonatorVoiceConfig::snare(),
        3 => ResonatorVoiceConfig::hybrid(),
        _ => ResonatorVoiceConfig::metallic_drone(),
    }
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
enum Selection {
    BasePitch,
    SweepTime,
    TransientLevel,
    TransientWidth,
    TransientShape,
    NoiseLevel,
    NoiseDecay,
    NoiseFilter,
    Mode1Ratio,
    Mode1Decay,
    Mode1Feedback,
    Mode1Drive,
    Mode1Tap,
    Mode2Ratio,
    Mode2Decay,
    Mode2Feedback,
    Mode2Drive,
    Mode2Tap,
    Edge(usize),
    OutputLevel,
}

const SELECTIONS: [Selection; 28] = [
    Selection::BasePitch,
    Selection::SweepTime,
    Selection::TransientLevel,
    Selection::TransientWidth,
    Selection::TransientShape,
    Selection::NoiseLevel,
    Selection::NoiseDecay,
    Selection::NoiseFilter,
    Selection::Mode1Ratio,
    Selection::Mode1Decay,
    Selection::Mode1Feedback,
    Selection::Mode1Drive,
    Selection::Mode1Tap,
    Selection::Mode2Ratio,
    Selection::Mode2Decay,
    Selection::Mode2Feedback,
    Selection::Mode2Drive,
    Selection::Mode2Tap,
    Selection::Edge(0),
    Selection::Edge(1),
    Selection::Edge(2),
    Selection::Edge(3),
    Selection::Edge(4),
    Selection::Edge(5),
    Selection::Edge(6),
    Selection::Edge(7),
    Selection::Edge(8),
    Selection::OutputLevel,
];

const EDGE_NAMES: [&str; 9] = [
    "transient -> mode 1",
    "transient -> mode 2",
    "noise -> mode 1",
    "noise -> mode 2",
    "mode 1 -> mode 2",
    "transient -> output",
    "noise -> output",
    "mode 1 -> output",
    "mode 2 -> output",
];

struct LabState {
    preset: usize,
    config: ResonatorVoiceConfig,
    selected: usize,
    velocity: f32,
    view: View,
    sequence_track: usize,
    sequence_step: usize,
    patterns: [[bool; 16]; 4],
    playing: bool,
    bpm: f32,
}

impl LabState {
    fn new() -> Self {
        Self {
            preset: 0,
            config: factory_preset(0),
            selected: 0,
            velocity: 0.8,
            view: View::Graph,
            sequence_track: 0,
            sequence_step: 0,
            patterns: [
                [
                    true, false, false, false, true, false, false, false, true, false, false,
                    false, true, false, false, false,
                ],
                [
                    false, false, false, false, true, false, false, false, false, false, false,
                    false, true, false, false, false,
                ],
                [
                    false, false, true, false, false, false, false, false, false, false, true,
                    false, false, false, false, false,
                ],
                [false; 16],
            ],
            playing: false,
            bpm: 120.0,
        }
    }

    fn selection(&self) -> Selection {
        SELECTIONS[self.selected]
    }

    fn select(&mut self, direction: i32) {
        self.selected =
            (self.selected as i32 + direction).rem_euclid(SELECTIONS.len() as i32) as usize;
    }

    fn edge_values(&self) -> [f32; 9] {
        let r = self.config.routing;
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

    fn set_edge(&mut self, index: usize, value: f32) {
        let value = value.clamp(0.0, 1.0);
        match index {
            0 => self.config.routing.transient_to_mode1 = value,
            1 => self.config.routing.transient_to_mode2 = value,
            2 => self.config.routing.noise_to_mode1 = value,
            3 => self.config.routing.noise_to_mode2 = value,
            4 => self.config.routing.mode1_to_mode2 = value,
            5 => self.config.routing.transient_to_output = value,
            6 => self.config.routing.noise_to_output = value,
            7 => self.config.routing.mode1_to_output = value,
            8 => self.config.routing.mode2_to_output = value,
            _ => {}
        }
    }

    fn normalized_value(&self) -> f32 {
        match self.selection() {
            Selection::BasePitch => {
                ((self.config.base_frequency_hz / 20.0).ln() / 20.0_f32.ln()).clamp(0.0, 1.0)
            }
            Selection::SweepTime => {
                ((self.config.sweep_seconds / 0.002).ln() / (4.0_f32 / 0.002).ln()).clamp(0.0, 1.0)
            }
            Selection::TransientLevel => self.config.exciter.level / 2.0,
            Selection::TransientWidth => (self.config.exciter.width_ms - 0.25) / 7.75,
            Selection::TransientShape => match self.config.exciter.shape {
                ResonatorExciterShape::Pulse => 0.0,
                ResonatorExciterShape::Click => 0.5,
                ResonatorExciterShape::Noise => 1.0,
            },
            Selection::NoiseLevel => self.config.noise.level / 2.0,
            Selection::NoiseDecay => self.config.noise.decay_seconds / 2.0,
            Selection::NoiseFilter => ((self.config.noise.filter_hz / 20.0).ln()
                / (20_000.0_f32 / 20.0).ln())
            .clamp(0.0, 1.0),
            Selection::Mode1Ratio => self.config.mode1.frequency_ratio / 8.0,
            Selection::Mode1Decay => self.config.mode1.decay_seconds / 5.0,
            Selection::Mode1Feedback => self.config.mode1.feedback / 0.8,
            Selection::Mode1Drive => self.config.mode1.drive / 24.0,
            Selection::Mode1Tap => {
                (self.config.mode1.tap == ResonatorOutputTap::Bandpass) as u8 as f32
            }
            Selection::Mode2Ratio => self.config.mode2.frequency_ratio / 8.0,
            Selection::Mode2Decay => self.config.mode2.decay_seconds / 5.0,
            Selection::Mode2Feedback => self.config.mode2.feedback / 0.8,
            Selection::Mode2Drive => self.config.mode2.drive / 24.0,
            Selection::Mode2Tap => {
                (self.config.mode2.tap == ResonatorOutputTap::Bandpass) as u8 as f32
            }
            Selection::Edge(index) => self.edge_values()[index],
            Selection::OutputLevel => self.config.volume / 2.0,
        }
        .clamp(0.0, 1.0)
    }

    fn describe(&self) -> String {
        if self.view == View::Sequencer {
            return format!(
                "Resonator Graph Lab | Sequencer | {} step {} | {:.0} BPM | {}",
                TRACK_NAMES[self.sequence_track],
                self.sequence_step + 1,
                self.bpm,
                if self.playing { "playing" } else { "stopped" }
            );
        }
        let detail = match self.selection() {
            Selection::BasePitch => format!("base pitch {:.1} Hz", self.config.base_frequency_hz),
            Selection::SweepTime => format!("sweep {:.1} ms", self.config.sweep_seconds * 1_000.0),
            Selection::TransientLevel => {
                format!("transient level {:.2}", self.config.exciter.level)
            }
            Selection::TransientWidth => {
                format!("transient width {:.2} ms", self.config.exciter.width_ms)
            }
            Selection::TransientShape => format!("transient shape {:?}", self.config.exciter.shape),
            Selection::NoiseLevel => format!("noise level {:.2}", self.config.noise.level),
            Selection::NoiseDecay => {
                format!("noise decay {:.3} s", self.config.noise.decay_seconds)
            }
            Selection::NoiseFilter => format!("noise filter {:.0} Hz", self.config.noise.filter_hz),
            Selection::Mode1Ratio => {
                format!("mode 1 ratio {:.3}", self.config.mode1.frequency_ratio)
            }
            Selection::Mode1Decay => {
                format!("mode 1 decay {:.3} s", self.config.mode1.decay_seconds)
            }
            Selection::Mode1Feedback => {
                format!("mode 1 feedback {:.2}", self.config.mode1.feedback)
            }
            Selection::Mode1Drive => format!("mode 1 drive {:.2}", self.config.mode1.drive),
            Selection::Mode1Tap => format!("mode 1 tap {:?}", self.config.mode1.tap),
            Selection::Mode2Ratio => {
                format!("mode 2 ratio {:.3}", self.config.mode2.frequency_ratio)
            }
            Selection::Mode2Decay => {
                format!("mode 2 decay {:.3} s", self.config.mode2.decay_seconds)
            }
            Selection::Mode2Feedback => {
                format!("mode 2 feedback {:.2}", self.config.mode2.feedback)
            }
            Selection::Mode2Drive => format!("mode 2 drive {:.2}", self.config.mode2.drive),
            Selection::Mode2Tap => format!("mode 2 tap {:?}", self.config.mode2.tap),
            Selection::Edge(index) => {
                format!("{} {:.2}", EDGE_NAMES[index], self.edge_values()[index])
            }
            Selection::OutputLevel => format!("output level {:.2}", self.config.volume),
        };
        format!(
            "Resonator Graph Lab | {} | {} | velocity {:.2}",
            PRESET_NAMES[self.preset], detail, self.velocity
        )
    }

    fn selection_label(&self) -> &'static str {
        match self.selection() {
            Selection::BasePitch => "BASE PITCH",
            Selection::SweepTime => "SWEEP TIME",
            Selection::TransientLevel => "TRANSIENT LEVEL",
            Selection::TransientWidth => "TRANSIENT WIDTH",
            Selection::TransientShape => "TRANSIENT SHAPE",
            Selection::NoiseLevel => "NOISE LEVEL",
            Selection::NoiseDecay => "NOISE DECAY",
            Selection::NoiseFilter => "NOISE FILTER",
            Selection::Mode1Ratio => "MODE 1 RATIO",
            Selection::Mode1Decay => "MODE 1 DECAY",
            Selection::Mode1Feedback => "MODE 1 FEEDBACK",
            Selection::Mode1Drive => "MODE 1 DRIVE",
            Selection::Mode1Tap => "MODE 1 TAP",
            Selection::Mode2Ratio => "MODE 2 RATIO",
            Selection::Mode2Decay => "MODE 2 DECAY",
            Selection::Mode2Feedback => "MODE 2 FEEDBACK",
            Selection::Mode2Drive => "MODE 2 DRIVE",
            Selection::Mode2Tap => "MODE 2 TAP",
            Selection::Edge(index) => EDGE_NAMES[index],
            Selection::OutputLevel => "OUTPUT LEVEL",
        }
    }

    fn apply(&self, voice: &mut ResonatorVoice) {
        voice.set_base_frequency_hz(self.config.base_frequency_hz);
        voice.set_sweep_seconds(self.config.sweep_seconds);
        voice.set_exciter_level(self.config.exciter.level);
        voice.set_exciter_width_ms(self.config.exciter.width_ms);
        voice.set_exciter_shape(self.config.exciter.shape);
        voice.set_noise_config(self.config.noise);
        for (index, mode) in [self.config.mode1, self.config.mode2]
            .into_iter()
            .enumerate()
        {
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
        voice.set_routing(self.config.routing);
        voice.set_output_volume(self.config.volume);
    }

    fn set_normalized(&mut self, voice: &mut ResonatorVoice, value: f32) {
        let value = value.clamp(0.0, 1.0);
        match self.selection() {
            Selection::BasePitch => self.config.base_frequency_hz = 20.0 * 20.0_f32.powf(value),
            Selection::SweepTime => self.config.sweep_seconds = 0.002 * 2_000.0_f32.powf(value),
            Selection::TransientLevel => self.config.exciter.level = value * 2.0,
            Selection::TransientWidth => self.config.exciter.width_ms = 0.25 + value * 7.75,
            Selection::TransientShape => {
                self.config.exciter.shape = if value < 0.25 {
                    ResonatorExciterShape::Pulse
                } else if value < 0.75 {
                    ResonatorExciterShape::Click
                } else {
                    ResonatorExciterShape::Noise
                };
            }
            Selection::NoiseLevel => self.config.noise.level = value * 2.0,
            Selection::NoiseDecay => self.config.noise.decay_seconds = 0.001 + value * 1.999,
            Selection::NoiseFilter => self.config.noise.filter_hz = 20.0 * 1_000.0_f32.powf(value),
            Selection::Mode1Ratio => self.config.mode1.frequency_ratio = 0.125 + value * 7.875,
            Selection::Mode1Decay => self.config.mode1.decay_seconds = 0.005 + value * 4.995,
            Selection::Mode1Feedback => self.config.mode1.feedback = value * 0.8,
            Selection::Mode1Drive => self.config.mode1.drive = 0.1 + value * 23.9,
            Selection::Mode1Tap => {
                self.config.mode1.tap = if value < 0.5 {
                    ResonatorOutputTap::Lowpass
                } else {
                    ResonatorOutputTap::Bandpass
                }
            }
            Selection::Mode2Ratio => self.config.mode2.frequency_ratio = 0.125 + value * 7.875,
            Selection::Mode2Decay => self.config.mode2.decay_seconds = 0.005 + value * 4.995,
            Selection::Mode2Feedback => self.config.mode2.feedback = value * 0.8,
            Selection::Mode2Drive => self.config.mode2.drive = 0.1 + value * 23.9,
            Selection::Mode2Tap => {
                self.config.mode2.tap = if value < 0.5 {
                    ResonatorOutputTap::Lowpass
                } else {
                    ResonatorOutputTap::Bandpass
                }
            }
            Selection::Edge(index) => self.set_edge(index, value),
            Selection::OutputLevel => self.config.volume = value * 2.0,
        }
        self.apply(voice);
    }

    fn load_preset(&mut self, voice: &mut ResonatorVoice, index: usize) {
        self.preset = index.min(4);
        self.config = factory_preset(self.preset);
        *voice = ResonatorVoice::with_config(SAMPLE_RATE, self.config);
    }
}

#[derive(Clone, Copy)]
struct Node {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

const TRANSIENT: Node = Node {
    x: -0.88,
    y: 0.55,
    w: 0.22,
    h: 0.18,
};
const NOISE: Node = Node {
    x: -0.88,
    y: 0.08,
    w: 0.22,
    h: 0.18,
};
const MODE1: Node = Node {
    x: -0.30,
    y: 0.55,
    w: 0.25,
    h: 0.22,
};
const MODE2: Node = Node {
    x: 0.18,
    y: 0.08,
    w: 0.25,
    h: 0.22,
};
const OUTPUT: Node = Node {
    x: 0.66,
    y: 0.34,
    w: 0.20,
    h: 0.22,
};

fn center(node: Node) -> (f32, f32) {
    (node.x + node.w * 0.5, node.y + node.h * 0.5)
}

fn edge_points(index: usize) -> ((f32, f32), (f32, f32)) {
    match index {
        0 => (center(TRANSIENT), center(MODE1)),
        1 => (center(TRANSIENT), center(MODE2)),
        2 => (center(NOISE), center(MODE1)),
        3 => (center(NOISE), center(MODE2)),
        4 => (center(MODE1), center(MODE2)),
        5 => (center(TRANSIENT), center(OUTPUT)),
        6 => (center(NOISE), center(OUTPUT)),
        7 => (center(MODE1), center(OUTPUT)),
        _ => (center(MODE2), center(OUTPUT)),
    }
}

fn glyph(character: char) -> [&'static str; 7] {
    match character.to_ascii_uppercase() {
        'A' => [
            "01110", "10001", "10001", "11111", "10001", "10001", "10001",
        ],
        'B' => [
            "11110", "10001", "10001", "11110", "10001", "10001", "11110",
        ],
        'C' => [
            "01111", "10000", "10000", "10000", "10000", "10000", "01111",
        ],
        'D' => [
            "11110", "10001", "10001", "10001", "10001", "10001", "11110",
        ],
        'E' => [
            "11111", "10000", "10000", "11110", "10000", "10000", "11111",
        ],
        'F' => [
            "11111", "10000", "10000", "11110", "10000", "10000", "10000",
        ],
        'G' => [
            "01111", "10000", "10000", "10111", "10001", "10001", "01111",
        ],
        'H' => [
            "10001", "10001", "10001", "11111", "10001", "10001", "10001",
        ],
        'I' => [
            "11111", "00100", "00100", "00100", "00100", "00100", "11111",
        ],
        'J' => [
            "00111", "00010", "00010", "00010", "10010", "10010", "01100",
        ],
        'K' => [
            "10001", "10010", "10100", "11000", "10100", "10010", "10001",
        ],
        'L' => [
            "10000", "10000", "10000", "10000", "10000", "10000", "11111",
        ],
        'M' => [
            "10001", "11011", "10101", "10101", "10001", "10001", "10001",
        ],
        'N' => [
            "10001", "11001", "10101", "10011", "10001", "10001", "10001",
        ],
        'O' => [
            "01110", "10001", "10001", "10001", "10001", "10001", "01110",
        ],
        'P' => [
            "11110", "10001", "10001", "11110", "10000", "10000", "10000",
        ],
        'Q' => [
            "01110", "10001", "10001", "10001", "10101", "10010", "01101",
        ],
        'R' => [
            "11110", "10001", "10001", "11110", "10100", "10010", "10001",
        ],
        'S' => [
            "01111", "10000", "10000", "01110", "00001", "00001", "11110",
        ],
        'T' => [
            "11111", "00100", "00100", "00100", "00100", "00100", "00100",
        ],
        'U' => [
            "10001", "10001", "10001", "10001", "10001", "10001", "01110",
        ],
        'V' => [
            "10001", "10001", "10001", "10001", "10001", "01010", "00100",
        ],
        'W' => [
            "10001", "10001", "10001", "10101", "10101", "10101", "01010",
        ],
        'X' => [
            "10001", "10001", "01010", "00100", "01010", "10001", "10001",
        ],
        'Y' => [
            "10001", "10001", "01010", "00100", "00100", "00100", "00100",
        ],
        'Z' => [
            "11111", "00001", "00010", "00100", "01000", "10000", "11111",
        ],
        '0' => [
            "01110", "10001", "10011", "10101", "11001", "10001", "01110",
        ],
        '1' => [
            "00100", "01100", "00100", "00100", "00100", "00100", "01110",
        ],
        '2' => [
            "01110", "10001", "00001", "00010", "00100", "01000", "11111",
        ],
        '3' => [
            "11110", "00001", "00001", "01110", "00001", "00001", "11110",
        ],
        '4' => [
            "00010", "00110", "01010", "10010", "11111", "00010", "00010",
        ],
        '5' => [
            "11111", "10000", "10000", "11110", "00001", "00001", "11110",
        ],
        '6' => [
            "01110", "10000", "10000", "11110", "10001", "10001", "01110",
        ],
        '7' => [
            "11111", "00001", "00010", "00100", "01000", "01000", "01000",
        ],
        '8' => [
            "01110", "10001", "10001", "01110", "10001", "10001", "01110",
        ],
        '9' => [
            "01110", "10001", "10001", "01111", "00001", "00001", "01110",
        ],
        '-' => [
            "00000", "00000", "00000", "11111", "00000", "00000", "00000",
        ],
        _ => ["00000"; 7],
    }
}

struct LabWindow {
    glfw: glfw::Glfw,
    window: glfw::PWindow,
    events: GlfwReceiver<(f64, WindowEvent)>,
    width: i32,
    height: i32,
    shader: u32,
    color_location: i32,
    vao: u32,
    vbo: u32,
}

impl LabWindow {
    fn new(width: u32, height: u32) -> anyhow::Result<Self> {
        let mut glfw = glfw::init(glfw::fail_on_errors)
            .map_err(|error| anyhow::anyhow!("failed to initialize GLFW: {error:?}"))?;
        glfw.window_hint(glfw::WindowHint::ContextVersion(3, 3));
        glfw.window_hint(glfw::WindowHint::OpenGlProfile(
            glfw::OpenGlProfileHint::Core,
        ));
        glfw.window_hint(glfw::WindowHint::OpenGlForwardCompat(true));
        let (mut window, events) = glfw
            .create_window(
                width,
                height,
                "Resonator Graph Lab",
                glfw::WindowMode::Windowed,
            )
            .ok_or_else(|| anyhow::anyhow!("failed to create Resonator Graph Lab window"))?;
        window.make_current();
        window.set_key_polling(true);
        window.set_mouse_button_polling(true);
        window.set_cursor_pos_polling(true);
        window.set_framebuffer_size_polling(true);
        glfw.set_swap_interval(glfw::SwapInterval::Sync(1));
        gl::load_with(|symbol| window.get_proc_address(symbol) as *const _);
        let shader = create_shader_program()?;
        let (vao, vbo) = create_buffers();
        let color_location = unsafe { gl::GetUniformLocation(shader, c"color".as_ptr()) };
        Ok(Self {
            glfw,
            window,
            events,
            width: width as i32,
            height: height as i32,
            shader,
            color_location,
            vao,
            vbo,
        })
    }

    fn poll_events(&mut self) -> Vec<WindowEvent> {
        self.glfw.poll_events();
        glfw::flush_messages(&self.events)
            .map(|(_, event)| event)
            .collect()
    }

    fn draw(&self, vertices: &[f32], mode: u32, color: (f32, f32, f32), width: f32) {
        unsafe {
            gl::BindVertexArray(self.vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                std::mem::size_of_val(vertices) as isize,
                vertices.as_ptr().cast(),
                gl::DYNAMIC_DRAW,
            );
            gl::Uniform3f(self.color_location, color.0, color.1, color.2);
            gl::LineWidth(width);
            gl::DrawArrays(mode, 0, (vertices.len() / 2) as i32);
        }
    }

    fn rect(&self, node: Node, color: (f32, f32, f32)) {
        self.draw(
            &[
                node.x,
                node.y,
                node.x + node.w,
                node.y,
                node.x + node.w,
                node.y + node.h,
                node.x,
                node.y + node.h,
            ],
            gl::TRIANGLE_FAN,
            color,
            1.0,
        );
    }

    fn outline(&self, node: Node, color: (f32, f32, f32), width: f32) {
        self.draw(
            &[
                node.x,
                node.y,
                node.x + node.w,
                node.y,
                node.x + node.w,
                node.y + node.h,
                node.x,
                node.y + node.h,
            ],
            gl::LINE_LOOP,
            color,
            width,
        );
    }

    fn text(&self, text: &str, x: f32, y: f32, scale: f32, color: (f32, f32, f32)) {
        for (character_index, character) in text.chars().enumerate() {
            for (row, pattern) in glyph(character).iter().enumerate() {
                for (column, pixel) in pattern.bytes().enumerate() {
                    if pixel == b'1' {
                        self.rect(
                            Node {
                                x: x + character_index as f32 * scale * 6.0 + column as f32 * scale,
                                y: y - row as f32 * scale,
                                w: scale * 0.82,
                                h: scale * 0.82,
                            },
                            color,
                        );
                    }
                }
            }
        }
    }

    fn render_sequencer(&self, state: &LabState, playheads: [usize; 4]) {
        self.text("SEQUENCER", -0.94, 0.88, 0.018, (0.85, 0.9, 0.95));
        self.text(
            if state.playing { "PLAYING" } else { "STOPPED" },
            0.48,
            0.88,
            0.014,
            if state.playing {
                (0.3, 0.95, 0.5)
            } else {
                (0.9, 0.4, 0.3)
            },
        );
        let left = -0.68;
        let top = 0.58;
        let cell_w = 0.09;
        let cell_h = 0.23;
        for step in 0..16 {
            self.text(
                &(step + 1).to_string(),
                left + step as f32 * cell_w + 0.018,
                top + 0.15,
                0.008,
                (0.55, 0.62, 0.7),
            );
        }
        for track in 0..4 {
            let y = top - track as f32 * cell_h;
            self.text(
                TRACK_NAMES[track],
                -0.94,
                y + 0.055,
                0.012,
                if track == state.sequence_track {
                    (1.0, 0.85, 0.25)
                } else {
                    (0.65, 0.72, 0.8)
                },
            );
            for step in 0..16 {
                let node = Node {
                    x: left + step as f32 * cell_w,
                    y,
                    w: cell_w - 0.012,
                    h: cell_h - 0.055,
                };
                let selected = track == state.sequence_track && step == state.sequence_step;
                let playhead = state.playing && step == playheads[track];
                self.rect(
                    node,
                    if state.patterns[track][step] {
                        (0.15, 0.68, 0.62)
                    } else {
                        (0.10, 0.13, 0.18)
                    },
                );
                self.outline(
                    node,
                    if selected {
                        (1.0, 0.85, 0.2)
                    } else if playhead {
                        (0.95, 0.35, 0.25)
                    } else {
                        (0.25, 0.32, 0.4)
                    },
                    if selected || playhead { 3.0 } else { 1.0 },
                );
            }
        }
        self.text("SPACE PLAY STOP", -0.72, -0.48, 0.011, (0.6, 0.7, 0.8));
        self.text("ENTER TOGGLE", -0.18, -0.48, 0.011, (0.6, 0.7, 0.8));
        self.text("TAB GRAPH", 0.32, -0.48, 0.011, (0.6, 0.7, 0.8));
        self.text(
            &format!("BPM {}", state.bpm.round() as u32),
            -0.12,
            -0.66,
            0.016,
            (0.95, 0.62, 0.2),
        );
    }

    fn render(&mut self, state: &LabState, scope: &Scope, playheads: [usize; 4]) {
        self.window.set_title(&state.describe());
        unsafe {
            gl::Viewport(0, 0, self.width, self.height);
            gl::ClearColor(0.025, 0.035, 0.055, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);
            gl::UseProgram(self.shader);
        }
        if state.view == View::Sequencer {
            self.render_sequencer(state, playheads);
            self.window.swap_buffers();
            return;
        }
        let edge_values = state.edge_values();
        for (index, value) in edge_values.iter().enumerate() {
            let (a, b) = edge_points(index);
            let selected = state.selection() == Selection::Edge(index);
            self.draw(
                &[a.0, a.1, b.0, b.1],
                gl::LINES,
                if selected {
                    (1.0, 0.82, 0.2)
                } else {
                    (
                        0.16 + 0.35 * value,
                        0.32 + 0.55 * value,
                        0.42 + 0.45 * value,
                    )
                },
                if selected { 6.0 } else { 1.0 + 5.0 * value },
            );
        }
        let node_selected =
            |range: std::ops::RangeInclusive<usize>| range.contains(&state.selected);
        for (node, color, selected) in [
            (TRANSIENT, (0.75, 0.30, 0.20), node_selected(2..=4)),
            (NOISE, (0.45, 0.28, 0.65), node_selected(5..=7)),
            (MODE1, (0.12, 0.55, 0.70), node_selected(8..=12)),
            (MODE2, (0.12, 0.68, 0.48), node_selected(13..=17)),
            (
                OUTPUT,
                (0.72, 0.52, 0.15),
                state.selection() == Selection::OutputLevel,
            ),
        ] {
            self.rect(node, color);
            self.outline(
                node,
                if selected {
                    (1.0, 0.9, 0.25)
                } else {
                    (0.5, 0.58, 0.65)
                },
                if selected { 4.0 } else { 1.5 },
            );
        }
        self.text(
            "TRANSIENT",
            TRANSIENT.x + 0.018,
            TRANSIENT.y + 0.115,
            0.010,
            (1.0, 0.9, 0.82),
        );
        self.text(
            "NOISE",
            NOISE.x + 0.052,
            NOISE.y + 0.115,
            0.012,
            (0.95, 0.88, 1.0),
        );
        self.text(
            "MODE 1",
            MODE1.x + 0.055,
            MODE1.y + 0.135,
            0.013,
            (0.86, 0.96, 1.0),
        );
        self.text(
            "MODE 2",
            MODE2.x + 0.055,
            MODE2.y + 0.135,
            0.013,
            (0.86, 1.0, 0.92),
        );
        self.text(
            "OUTPUT",
            OUTPUT.x + 0.037,
            OUTPUT.y + 0.135,
            0.012,
            (1.0, 0.94, 0.75),
        );
        self.text("TAB SEQUENCER", -0.93, -0.45, 0.012, (0.58, 0.68, 0.78));
        self.text("SELECTED", -0.93, -0.84, 0.010, (0.55, 0.62, 0.7));
        self.text(
            state.selection_label(),
            -0.62,
            -0.84,
            0.012,
            (1.0, 0.82, 0.25),
        );
        let value = state.normalized_value();
        let meter = Node {
            x: 0.91,
            y: -0.45,
            w: 0.045,
            h: 1.18,
        };
        self.outline(meter, (0.3, 0.35, 0.42), 1.0);
        self.rect(
            Node {
                x: meter.x + 0.006,
                y: meter.y + 0.006,
                w: meter.w - 0.012,
                h: (meter.h - 0.012) * value,
            },
            (0.95, 0.62, 0.18),
        );
        let samples = scope.snapshot();
        if samples.len() > 1 {
            let mut vertices = Vec::with_capacity(samples.len() * 2);
            for (index, sample) in samples.iter().enumerate() {
                vertices.extend_from_slice(&[
                    -0.94 + index as f32 / (samples.len() - 1) as f32 * 1.72,
                    -0.72 + (sample * 0.22).clamp(-0.18, 0.18),
                ]);
            }
            self.draw(&vertices, gl::LINE_STRIP, (0.25, 0.9, 0.92), 1.4);
        }
        self.window.swap_buffers();
    }
}

impl Drop for LabWindow {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteProgram(self.shader);
            gl::DeleteBuffers(1, &self.vbo);
            gl::DeleteVertexArrays(1, &self.vao);
        }
    }
}

fn create_shader_program() -> anyhow::Result<u32> {
    let vertex_source = CString::new("#version 330 core\nlayout (location=0) in vec2 aPos;\nvoid main(){gl_Position=vec4(aPos,0.0,1.0);}")?;
    let fragment_source = CString::new("#version 330 core\nout vec4 FragColor;\nuniform vec3 color;\nvoid main(){FragColor=vec4(color,1.0);}")?;
    unsafe {
        let vertex = gl::CreateShader(gl::VERTEX_SHADER);
        gl::ShaderSource(vertex, 1, &vertex_source.as_ptr(), std::ptr::null());
        gl::CompileShader(vertex);
        check_shader(vertex, "vertex")?;
        let fragment = gl::CreateShader(gl::FRAGMENT_SHADER);
        gl::ShaderSource(fragment, 1, &fragment_source.as_ptr(), std::ptr::null());
        gl::CompileShader(fragment);
        check_shader(fragment, "fragment")?;
        let program = gl::CreateProgram();
        gl::AttachShader(program, vertex);
        gl::AttachShader(program, fragment);
        gl::LinkProgram(program);
        gl::DeleteShader(vertex);
        gl::DeleteShader(fragment);
        let mut success = 0;
        gl::GetProgramiv(program, gl::LINK_STATUS, &mut success);
        if success == 0 {
            anyhow::bail!("shader link failed");
        }
        Ok(program)
    }
}

unsafe fn check_shader(shader: u32, label: &str) -> anyhow::Result<()> {
    let mut success = 0;
    gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut success);
    if success != 0 {
        return Ok(());
    }
    anyhow::bail!("{label} shader compile failed")
}

fn create_buffers() -> (u32, u32) {
    unsafe {
        let mut vao = 0;
        let mut vbo = 0;
        gl::GenVertexArrays(1, &mut vao);
        gl::GenBuffers(1, &mut vbo);
        gl::BindVertexArray(vao);
        gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
        gl::VertexAttribPointer(
            0,
            2,
            gl::FLOAT,
            gl::FALSE,
            (2 * std::mem::size_of::<f32>()) as i32,
            std::ptr::null(),
        );
        gl::EnableVertexAttribArray(0);
        (vao, vbo)
    }
}

fn select_at_cursor(state: &mut LabState, window: &glfw::PWindow) {
    let (x, y) = window.get_cursor_pos();
    let (w, h) = window.get_size();
    let point = (
        x as f32 / w.max(1) as f32 * 2.0 - 1.0,
        1.0 - y as f32 / h.max(1) as f32 * 2.0,
    );
    let nodes = [
        (TRANSIENT, 2),
        (NOISE, 5),
        (MODE1, 8),
        (MODE2, 13),
        (OUTPUT, 27),
    ];
    if let Some((_, index)) = nodes.into_iter().find(|(n, _)| {
        point.0 >= n.x && point.0 <= n.x + n.w && point.1 >= n.y && point.1 <= n.y + n.h
    }) {
        state.selected = index;
        return;
    }
    let mut best = (f32::MAX, 0usize);
    for index in 0..9 {
        let (a, b) = edge_points(index);
        let ab = (b.0 - a.0, b.1 - a.1);
        let ap = (point.0 - a.0, point.1 - a.1);
        let t = ((ap.0 * ab.0 + ap.1 * ab.1) / (ab.0 * ab.0 + ab.1 * ab.1)).clamp(0.0, 1.0);
        let d =
            ((point.0 - (a.0 + t * ab.0)).powi(2) + (point.1 - (a.1 + t * ab.1)).powi(2)).sqrt();
        if d < best.0 {
            best = (d, index);
        }
    }
    if best.0 < 0.06 {
        state.selected = 18 + best.1;
    }
}

fn sequencer_cell_at_cursor(window: &glfw::PWindow) -> Option<(usize, usize)> {
    let (x, y) = window.get_cursor_pos();
    let (w, h) = window.get_size();
    let point = (
        x as f32 / w.max(1) as f32 * 2.0 - 1.0,
        1.0 - y as f32 / h.max(1) as f32 * 2.0,
    );
    let left = -0.68;
    let top = 0.58;
    let cell_w = 0.09;
    let cell_h = 0.23;
    let step = ((point.0 - left) / cell_w).floor() as isize;
    let track = ((top + cell_h - point.1) / cell_h).floor() as isize;
    (track >= 0 && track < 4 && step >= 0 && step < 16).then_some((track as usize, step as usize))
}

fn print_help() {
    println!("Resonator Graph GUI Lab");
    println!("  Hit: Space       Presets: F1-F5       Velocity: -/=");
    println!("  Select: Up/Down or click a node/cable  Edit: Left/Right");
    println!("  Min/mid/max: Home/\\/End              Reset preset: Backspace");
    println!("  Quit: Cmd/Ctrl-Q");
    println!("  Views: Tab        Sequencer: arrows/click, Enter toggles, Space plays");
    println!("Cable thickness/brightness is routing gain; the title shows the selected control.");
}

fn main() -> anyhow::Result<()> {
    print_help();
    let mut state = LabState::new();
    let state_config = factory_preset(0);
    let voice = Arc::new(Mutex::new(ResonatorVoice::with_config(
        SAMPLE_RATE,
        state_config,
    )));
    let other_voices = [
        Arc::new(Mutex::new(ResonatorVoice::with_config(
            SAMPLE_RATE,
            factory_preset(2),
        ))),
        Arc::new(Mutex::new(ResonatorVoice::with_config(
            SAMPLE_RATE,
            factory_preset(1),
        ))),
        Arc::new(Mutex::new(ResonatorVoice::with_config(
            SAMPLE_RATE,
            factory_preset(3),
        ))),
    ];
    let scope = Arc::new(Scope::new(SCOPE_CAPACITY));
    let mut engine = Engine::new(SAMPLE_RATE);
    engine.add_instrument(
        INSTRUMENT_NAMES[0],
        Box::new(SharedVoice {
            voice: Arc::clone(&voice),
            scope: Arc::clone(&scope),
        }),
    );
    for (index, extra) in other_voices.iter().enumerate() {
        engine.add_instrument(
            INSTRUMENT_NAMES[index + 1],
            Box::new(SharedVoice {
                voice: Arc::clone(extra),
                scope: Arc::clone(&scope),
            }),
        );
    }
    for track in 0..4 {
        engine.add_sequencer(Sequencer::with_pattern(
            state.bpm,
            SAMPLE_RATE,
            state.patterns[track].to_vec(),
            INSTRUMENT_NAMES[track],
        ));
    }
    engine.set_master_gain(0.8);
    let engine = Arc::new(Mutex::new(engine));
    let trigger_engine = Arc::clone(&engine);
    let mut output = EngineOutput::new();
    output.initialize(SAMPLE_RATE)?;
    output.create_stream_with_engine(Arc::clone(&engine))?;
    output.start()?;
    let mut ui = LabWindow::new(1_280, 800)?;
    while !ui.window.should_close() {
        for event in ui.poll_events() {
            match event {
                WindowEvent::FramebufferSize(w, h) => {
                    ui.width = w.max(1);
                    ui.height = h.max(1);
                }
                WindowEvent::MouseButton(MouseButton::Button1, Action::Press, _) => {
                    if state.view == View::Graph {
                        select_at_cursor(&mut state, &ui.window)
                    } else if let Some((track, step)) = sequencer_cell_at_cursor(&ui.window) {
                        state.sequence_track = track;
                        state.sequence_step = step;
                        state.patterns[track][step] = !state.patterns[track][step];
                        if let Some(seq) = trigger_engine.lock().unwrap().sequencer_mut(track) {
                            seq.set_step(step, state.patterns[track][step]);
                        }
                    }
                }
                WindowEvent::Key(key, _, action, mods) if action != Action::Release => {
                    let command =
                        mods.contains(Modifiers::Control) || mods.contains(Modifiers::Super);
                    let coarse = mods.contains(Modifiers::Shift);
                    let press = action == Action::Press;
                    match key {
                        Key::Q if command && press => ui.window.set_should_close(true),
                        Key::Tab if press => {
                            state.view = if state.view == View::Graph {
                                View::Sequencer
                            } else {
                                View::Graph
                            }
                        }
                        Key::Space if press && state.view == View::Graph => trigger_engine
                            .lock()
                            .unwrap()
                            .trigger_instrument_with_velocity(INSTRUMENT_NAMES[0], state.velocity),
                        Key::Space if press => {
                            state.playing = !state.playing;
                            let mut engine = trigger_engine.lock().unwrap();
                            for index in 0..4 {
                                if let Some(seq) = engine.sequencer_mut(index) {
                                    if state.playing {
                                        seq.start();
                                    } else {
                                        seq.stop();
                                    }
                                }
                            }
                        }
                        Key::Enter if press && state.view == View::Sequencer => {
                            let track = state.sequence_track;
                            let step = state.sequence_step;
                            state.patterns[track][step] = !state.patterns[track][step];
                            if let Some(seq) = trigger_engine.lock().unwrap().sequencer_mut(track) {
                                seq.set_step(step, state.patterns[track][step]);
                            }
                        }
                        Key::Up
                            if (press || action == Action::Repeat) && state.view == View::Graph =>
                        {
                            state.select(-1)
                        }
                        Key::Down
                            if (press || action == Action::Repeat) && state.view == View::Graph =>
                        {
                            state.select(1)
                        }
                        Key::Up if press || action == Action::Repeat => {
                            state.sequence_track = (state.sequence_track + 3) % 4
                        }
                        Key::Down if press || action == Action::Repeat => {
                            state.sequence_track = (state.sequence_track + 1) % 4
                        }
                        Key::Left
                            if (press || action == Action::Repeat)
                                && state.view == View::Sequencer =>
                        {
                            state.sequence_step = (state.sequence_step + 15) % 16
                        }
                        Key::Right
                            if (press || action == Action::Repeat)
                                && state.view == View::Sequencer =>
                        {
                            state.sequence_step = (state.sequence_step + 1) % 16
                        }
                        Key::Left | Key::Right if press || action == Action::Repeat => {
                            let step = if coarse { 0.1 } else { 0.02 };
                            let direction = if key == Key::Left { -1.0 } else { 1.0 };
                            let value = state.normalized_value() + direction * step;
                            state.set_normalized(&mut voice.lock().unwrap(), value);
                        }
                        Key::Home if press => state.set_normalized(&mut voice.lock().unwrap(), 0.0),
                        Key::Backslash if press => {
                            state.set_normalized(&mut voice.lock().unwrap(), 0.5)
                        }
                        Key::End if press => state.set_normalized(&mut voice.lock().unwrap(), 1.0),
                        Key::Minus
                            if (press || action == Action::Repeat)
                                && state.view == View::Sequencer =>
                        {
                            state.bpm = (state.bpm - if coarse { 5.0 } else { 1.0 }).max(40.0);
                            let mut engine = trigger_engine.lock().unwrap();
                            engine.set_bpm(state.bpm);
                            for i in 0..4 {
                                if let Some(seq) = engine.sequencer_mut(i) {
                                    seq.set_bpm(state.bpm);
                                }
                            }
                        }
                        Key::Equal
                            if (press || action == Action::Repeat)
                                && state.view == View::Sequencer =>
                        {
                            state.bpm = (state.bpm + if coarse { 5.0 } else { 1.0 }).min(240.0);
                            let mut engine = trigger_engine.lock().unwrap();
                            engine.set_bpm(state.bpm);
                            for i in 0..4 {
                                if let Some(seq) = engine.sequencer_mut(i) {
                                    seq.set_bpm(state.bpm);
                                }
                            }
                        }
                        Key::Minus if press || action == Action::Repeat => {
                            state.velocity =
                                (state.velocity - if coarse { 0.1 } else { 0.02 }).max(0.0)
                        }
                        Key::Equal if press || action == Action::Repeat => {
                            state.velocity =
                                (state.velocity + if coarse { 0.1 } else { 0.02 }).min(1.0)
                        }
                        Key::Backspace if press => {
                            let preset = state.preset;
                            state.load_preset(&mut voice.lock().unwrap(), preset);
                        }
                        Key::F1 | Key::F2 | Key::F3 | Key::F4 | Key::F5 if press => {
                            let preset = match key {
                                Key::F1 => 0,
                                Key::F2 => 1,
                                Key::F3 => 2,
                                Key::F4 => 3,
                                _ => 4,
                            };
                            state.load_preset(&mut voice.lock().unwrap(), preset);
                        }
                        _ => {}
                    }
                }
                WindowEvent::Close => ui.window.set_should_close(true),
                _ => {}
            }
        }
        let playheads = {
            let engine = trigger_engine.lock().unwrap();
            std::array::from_fn(|index| {
                engine
                    .sequencer(index)
                    .map(|seq| seq.current_step())
                    .unwrap_or(0)
            })
        };
        ui.render(&state, &scope, playheads);
    }
    output.stop()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_pattern_is_a_four_track_backbeat() {
        let state = LabState::new();
        assert_eq!(state.patterns[0].iter().filter(|step| **step).count(), 4);
        assert!(state.patterns[0][0] && state.patterns[0][4]);
        assert!(state.patterns[1][4] && state.patterns[1][12]);
        assert!(state.patterns[2][2] && state.patterns[2][10]);
        assert!(state.patterns[3].iter().all(|step| !step));
    }

    #[test]
    fn every_graph_selection_has_a_visible_label() {
        let mut state = LabState::new();
        for selected in 0..SELECTIONS.len() {
            state.selected = selected;
            assert!(!state.selection_label().is_empty());
        }
    }
}
