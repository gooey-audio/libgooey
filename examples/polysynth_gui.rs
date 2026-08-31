//! Native keyboard-driven laboratory for the expressive poly synth.
//!
//! Run with:
//!
//!     cargo run --example polysynth_gui --features native,visualization

use std::ffi::CString;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use glfw::{Action, Context, GlfwReceiver, Key, Modifiers, WindowEvent};

use gooey::engine::{Engine, EngineOutput, Instrument};
use gooey::instruments::{
    PolyModRoute, PolyModSource, PolySynth, PolySynthConfig, POLY_MOD_ROUTE_COUNT,
    POLY_PARAM_AMP_ATTACK, POLY_PARAM_AMP_ATTACK_CURVE, POLY_PARAM_AMP_DECAY,
    POLY_PARAM_AMP_FALL_CURVE, POLY_PARAM_AMP_RELEASE, POLY_PARAM_AMP_SUSTAIN, POLY_PARAM_COUNT,
    POLY_PARAM_DETUNE, POLY_PARAM_FILTER_ATTACK, POLY_PARAM_FILTER_ATTACK_CURVE,
    POLY_PARAM_FILTER_CUTOFF, POLY_PARAM_FILTER_DECAY, POLY_PARAM_FILTER_ENV_AMOUNT,
    POLY_PARAM_FILTER_FALL_CURVE, POLY_PARAM_FILTER_RELEASE, POLY_PARAM_FILTER_RESONANCE,
    POLY_PARAM_FILTER_SUSTAIN, POLY_PARAM_OSC_A_LEVEL, POLY_PARAM_OSC_A_WAVEFORM,
    POLY_PARAM_OSC_B_LEVEL, POLY_PARAM_OSC_B_WAVEFORM, POLY_PARAM_PITCH_ATTACK,
    POLY_PARAM_PITCH_ATTACK_CURVE, POLY_PARAM_PITCH_DECAY, POLY_PARAM_PITCH_ENV_AMOUNT,
    POLY_PARAM_PITCH_FALL_CURVE, POLY_PARAM_PITCH_RELEASE, POLY_PARAM_PITCH_SUSTAIN,
    POLY_PARAM_SATURATION, POLY_PARAM_STEREO_WIDTH, POLY_PARAM_VOLUME,
};
use gooey::StereoFrame;

const SAMPLE_RATE: f32 = 44_100.0;
const SCOPE_CAPACITY: usize = 2_048;
const PAGE_COUNT: usize = 6;

const PARAM_NAMES: [&str; POLY_PARAM_COUNT as usize] = [
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

const OSC_PARAMS: [u32; 6] = [
    POLY_PARAM_OSC_A_WAVEFORM,
    POLY_PARAM_OSC_A_LEVEL,
    POLY_PARAM_OSC_B_WAVEFORM,
    POLY_PARAM_OSC_B_LEVEL,
    POLY_PARAM_DETUNE,
    POLY_PARAM_STEREO_WIDTH,
];
const AMP_PARAMS: [u32; 6] = [
    POLY_PARAM_AMP_ATTACK,
    POLY_PARAM_AMP_DECAY,
    POLY_PARAM_AMP_SUSTAIN,
    POLY_PARAM_AMP_RELEASE,
    POLY_PARAM_AMP_ATTACK_CURVE,
    POLY_PARAM_AMP_FALL_CURVE,
];
const PITCH_PARAMS: [u32; 7] = [
    POLY_PARAM_PITCH_ENV_AMOUNT,
    POLY_PARAM_PITCH_ATTACK,
    POLY_PARAM_PITCH_DECAY,
    POLY_PARAM_PITCH_SUSTAIN,
    POLY_PARAM_PITCH_RELEASE,
    POLY_PARAM_PITCH_ATTACK_CURVE,
    POLY_PARAM_PITCH_FALL_CURVE,
];
const FILTER_PARAMS: [u32; 9] = [
    POLY_PARAM_FILTER_CUTOFF,
    POLY_PARAM_FILTER_RESONANCE,
    POLY_PARAM_FILTER_ENV_AMOUNT,
    POLY_PARAM_FILTER_ATTACK,
    POLY_PARAM_FILTER_DECAY,
    POLY_PARAM_FILTER_SUSTAIN,
    POLY_PARAM_FILTER_RELEASE,
    POLY_PARAM_FILTER_ATTACK_CURVE,
    POLY_PARAM_FILTER_FALL_CURVE,
];
const EXPRESSION_PARAMS: [u32; 2] = [POLY_PARAM_SATURATION, POLY_PARAM_VOLUME];
const PAGE_NAMES: [&str; PAGE_COUNT] = [
    "Oscillators",
    "Amp envelope",
    "Pitch envelope",
    "Filter",
    "Expression",
    "Mod matrix",
];
const PRESET_NAMES: [&str; 5] = ["Default", "Pad", "Pluck", "Keys", "Strings"];

fn page_params(page: usize) -> &'static [u32] {
    match page {
        0 => &OSC_PARAMS,
        1 => &AMP_PARAMS,
        2 => &PITCH_PARAMS,
        3 => &FILTER_PARAMS,
        4 => &EXPRESSION_PARAMS,
        _ => &[],
    }
}

fn factory_presets() -> [PolySynthConfig; 5] {
    [
        PolySynthConfig::default(),
        PolySynthConfig::pad(),
        PolySynthConfig::pluck(),
        PolySynthConfig::keys(),
        PolySynthConfig::strings(),
    ]
}

fn factory_preset(index: usize) -> PolySynthConfig {
    factory_presets()[index.min(PRESET_NAMES.len() - 1)]
}

/// Single-writer, lock-free stereo history. The audio callback never waits
/// for the GUI; the GUI may read one stale sample while a frame is published.
struct StereoScope {
    left: Box<[AtomicU32]>,
    right: Box<[AtomicU32]>,
    cursor: AtomicUsize,
}

impl StereoScope {
    fn new(capacity: usize) -> Self {
        let channel = || {
            (0..capacity)
                .map(|_| AtomicU32::new(0.0_f32.to_bits()))
                .collect::<Vec<_>>()
                .into_boxed_slice()
        };
        Self {
            left: channel(),
            right: channel(),
            cursor: AtomicUsize::new(0),
        }
    }

    fn push(&self, frame: StereoFrame) {
        let sequence = self.cursor.load(Ordering::Relaxed);
        let index = sequence % self.left.len();
        self.left[index].store(frame.l.to_bits(), Ordering::Relaxed);
        self.right[index].store(frame.r.to_bits(), Ordering::Relaxed);
        self.cursor
            .store(sequence.wrapping_add(1), Ordering::Release);
    }

    fn snapshot(&self) -> (Vec<f32>, Vec<f32>) {
        let end = self.cursor.load(Ordering::Acquire);
        let count = end.min(self.left.len());
        let start = end.wrapping_sub(count);
        let mut left = Vec::with_capacity(count);
        let mut right = Vec::with_capacity(count);
        for sequence in start..end {
            let index = sequence % self.left.len();
            left.push(f32::from_bits(self.left[index].load(Ordering::Relaxed)));
            right.push(f32::from_bits(self.right[index].load(Ordering::Relaxed)));
        }
        (left, right)
    }
}

struct SharedLabSynth {
    synth: Arc<Mutex<PolySynth>>,
    scope: Arc<StereoScope>,
}

impl Instrument for SharedLabSynth {
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        self.synth
            .lock()
            .unwrap()
            .trigger_with_velocity(time, velocity);
    }

    fn tick(&mut self, current_time: f64) -> f32 {
        let sample = self.synth.lock().unwrap().tick(current_time);
        self.scope.push(StereoFrame::mono(sample));
        sample
    }

    fn tick_stereo(&mut self, current_time: f64) -> Option<StereoFrame> {
        let frame = self.synth.lock().unwrap().tick_frame(current_time);
        self.scope.push(frame);
        Some(frame)
    }

    fn is_active(&self) -> bool {
        self.synth.lock().unwrap().is_active()
    }

    fn set_midi_note(&mut self, note: u8) {
        self.synth.lock().unwrap().set_midi_note(note);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RouteField {
    Enabled,
    Source,
    Destination,
    Depth,
    Curve,
    KeyScale,
}

impl RouteField {
    fn shifted(self, direction: i32) -> Self {
        const FIELDS: [RouteField; 6] = [
            RouteField::Enabled,
            RouteField::Source,
            RouteField::Destination,
            RouteField::Depth,
            RouteField::Curve,
            RouteField::KeyScale,
        ];
        let index = FIELDS.iter().position(|field| *field == self).unwrap();
        FIELDS[(index as i32 + direction).rem_euclid(FIELDS.len() as i32) as usize]
    }

    fn label(self) -> &'static str {
        match self {
            Self::Enabled => "Enabled",
            Self::Source => "Source",
            Self::Destination => "Destination",
            Self::Depth => "Depth",
            Self::Curve => "Curve",
            Self::KeyScale => "Key scale",
        }
    }
}

struct LabState {
    page: usize,
    selected: usize,
    route_field: RouteField,
    velocity: f32,
    octave: i32,
    preset: usize,
    presets: [PolySynthConfig; 5],
    held_keys: Vec<(Key, Vec<u8>)>,
}

impl LabState {
    fn new() -> Self {
        Self {
            page: 0,
            selected: 0,
            route_field: RouteField::Depth,
            velocity: 0.8,
            octave: 4,
            preset: 0,
            presets: factory_presets(),
            held_keys: Vec::new(),
        }
    }

    fn item_count(&self) -> usize {
        if self.page == PAGE_COUNT - 1 {
            POLY_MOD_ROUTE_COUNT
        } else {
            page_params(self.page).len()
        }
    }

    fn selected_param(&self) -> Option<u32> {
        page_params(self.page).get(self.selected).copied()
    }

    fn select_page(&mut self, direction: i32) {
        self.page = (self.page as i32 + direction).rem_euclid(PAGE_COUNT as i32) as usize;
        self.selected = self.selected.min(self.item_count().saturating_sub(1));
    }

    fn select_item(&mut self, direction: i32) {
        self.selected =
            (self.selected as i32 + direction).rem_euclid(self.item_count() as i32) as usize;
    }
}

impl LabState {
    fn edit_selected(&mut self, synth: &mut PolySynth, direction: f32, coarse: bool) {
        if let Some(param) = self.selected_param() {
            let step = if coarse { 0.05 } else { 0.01 };
            let value = synth.param(param).unwrap_or(0.5) + direction * step;
            synth.set_param(param, value);
            self.presets[self.preset].set_param(param, value);
            return;
        }

        let Some(mut route) = synth.mod_route(self.selected) else {
            return;
        };
        match self.route_field {
            RouteField::Enabled => route.enabled = !route.enabled,
            RouteField::Source => {
                route.source = match route.source {
                    PolyModSource::Velocity => PolyModSource::KeyPosition,
                    PolyModSource::KeyPosition => PolyModSource::Velocity,
                };
            }
            RouteField::Destination => {
                route.destination = (route.destination as i32 + direction.signum() as i32)
                    .rem_euclid(POLY_PARAM_COUNT as i32) as u32;
            }
            RouteField::Depth => {
                let step = if coarse { 0.1 } else { 0.02 };
                route.depth = (route.depth + direction * step).clamp(-1.0, 1.0);
            }
            RouteField::Curve => {
                let step = if coarse { 0.1 } else { 0.02 };
                route.curve = (route.curve + direction * step).clamp(0.0, 1.0);
            }
            RouteField::KeyScale => {
                let step = if coarse { 0.1 } else { 0.02 };
                route.key_scale = (route.key_scale + direction * step).clamp(-1.0, 1.0);
            }
        }
        synth.set_mod_route(self.selected, route);
        self.presets[self.preset].set_mod_route(self.selected, route);
    }

    fn set_selected_absolute(&mut self, synth: &mut PolySynth, normalized: f32) {
        if let Some(param) = self.selected_param() {
            synth.set_param(param, normalized);
            self.presets[self.preset].set_param(param, normalized);
            return;
        }

        let Some(mut route) = synth.mod_route(self.selected) else {
            return;
        };
        match self.route_field {
            RouteField::Enabled => route.enabled = normalized >= 0.5,
            RouteField::Source => {
                route.source = if normalized >= 0.5 {
                    PolyModSource::KeyPosition
                } else {
                    PolyModSource::Velocity
                };
            }
            RouteField::Destination => {
                route.destination =
                    (normalized.clamp(0.0, 1.0) * (POLY_PARAM_COUNT - 1) as f32).round() as u32;
            }
            RouteField::Depth => route.depth = normalized.clamp(0.0, 1.0) * 2.0 - 1.0,
            RouteField::Curve => route.curve = normalized.clamp(0.0, 1.0),
            RouteField::KeyScale => route.key_scale = normalized.clamp(0.0, 1.0) * 2.0 - 1.0,
        }
        synth.set_mod_route(self.selected, route);
        self.presets[self.preset].set_mod_route(self.selected, route);
    }

    fn reset_selected(&mut self, synth: &mut PolySynth) {
        let factory = factory_preset(self.preset);
        if let Some(param) = self.selected_param() {
            let value = factory.param(param).unwrap();
            synth.set_param(param, value);
            self.presets[self.preset].set_param(param, value);
        } else {
            let route = factory.mod_routes[self.selected];
            synth.set_mod_route(self.selected, route);
            self.presets[self.preset].set_mod_route(self.selected, route);
        }
    }

    fn reset_preset(&mut self, synth: &mut PolySynth) {
        self.panic(synth);
        let factory = factory_preset(self.preset);
        self.presets[self.preset] = factory;
        synth.set_config(factory);
        synth.snap_params();
    }

    fn select_preset(&mut self, synth: &mut PolySynth, preset: usize) {
        self.panic(synth);
        self.preset = preset.min(PRESET_NAMES.len() - 1);
        synth.set_config(self.presets[self.preset]);
        synth.snap_params();
    }

    fn note_on(&mut self, synth: &mut PolySynth, key: Key, notes: Vec<u8>) {
        if self.held_keys.iter().any(|(held, _)| *held == key) {
            return;
        }
        for note in &notes {
            synth.trigger_note(*note, self.velocity);
        }
        self.held_keys.push((key, notes));
    }

    fn note_off(&mut self, synth: &mut PolySynth, key: Key) {
        let Some(index) = self.held_keys.iter().position(|(held, _)| *held == key) else {
            return;
        };
        let (_, notes) = self.held_keys.remove(index);
        for note in notes {
            synth.release_note(note);
        }
    }

    fn panic(&mut self, synth: &mut PolySynth) {
        synth.release_all();
        self.held_keys.clear();
    }

    fn title(&self, synth: &PolySynth) -> String {
        let selection = if let Some(param) = self.selected_param() {
            format!(
                "{} = {:.3}",
                PARAM_NAMES[param as usize],
                synth.param(param).unwrap_or(f32::NAN)
            )
        } else {
            let route = synth
                .mod_route(self.selected)
                .unwrap_or_else(PolyModRoute::disabled);
            format!(
                "Route {} [{}] {} {} -> {} | d {:+.2} c {:.2} key {:+.2}",
                self.selected + 1,
                self.route_field.label(),
                if route.enabled { "ON" } else { "off" },
                match route.source {
                    PolyModSource::Velocity => "velocity",
                    PolyModSource::KeyPosition => "key",
                },
                PARAM_NAMES[route.destination as usize],
                route.depth,
                route.curve,
                route.key_scale,
            )
        };
        format!(
            "PolySynth Lab | {} | {} | {} | vel {:.2} oct {}",
            PRESET_NAMES[self.preset], PAGE_NAMES[self.page], selection, self.velocity, self.octave
        )
    }
}

fn note_offset(key: Key) -> Option<i32> {
    Some(match key {
        Key::Z => 0,
        Key::S => 1,
        Key::X => 2,
        Key::D => 3,
        Key::C => 4,
        Key::V => 5,
        Key::G => 6,
        Key::B => 7,
        Key::H => 8,
        Key::N => 9,
        Key::J => 10,
        Key::M => 11,
        Key::Q => 12,
        Key::Num2 => 13,
        Key::W => 14,
        Key::Num3 => 15,
        Key::E => 16,
        Key::R => 17,
        Key::Num5 => 18,
        Key::T => 19,
        Key::Num6 => 20,
        Key::Y => 21,
        Key::Num7 => 22,
        Key::U => 23,
        Key::I => 24,
        _ => return None,
    })
}

fn midi_note(octave: i32, offset: i32) -> u8 {
    ((octave + 1) * 12 + offset).clamp(0, 127) as u8
}

fn handle_key(
    window: &mut glfw::PWindow,
    state: &mut LabState,
    synth: &Arc<Mutex<PolySynth>>,
    key: Key,
    action: Action,
    modifiers: Modifiers,
) {
    let command = modifiers.contains(Modifiers::Control) || modifiers.contains(Modifiers::Super);
    if key == Key::Q && command && action == Action::Press {
        window.set_should_close(true);
        return;
    }

    if !command {
        if let Some(offset) = note_offset(key) {
            let mut synth = synth.lock().unwrap();
            match action {
                Action::Press => {
                    let note = midi_note(state.octave, offset);
                    state.note_on(&mut synth, key, vec![note]);
                }
                Action::Release => state.note_off(&mut synth, key),
                Action::Repeat => {}
            }
            return;
        }
    }

    if key == Key::Space {
        let mut synth = synth.lock().unwrap();
        match action {
            Action::Press => {
                let root = midi_note(state.octave, 0);
                state.note_on(&mut synth, key, vec![root, root + 4, root + 7]);
            }
            Action::Release => state.note_off(&mut synth, key),
            Action::Repeat => {}
        }
        return;
    }

    if action == Action::Release {
        return;
    }
    let press = action == Action::Press;
    let coarse = modifiers.contains(Modifiers::Shift);
    match key {
        Key::Escape if press => state.panic(&mut synth.lock().unwrap()),
        Key::Tab if press => state.select_page(if coarse { -1 } else { 1 }),
        Key::Up if press || action == Action::Repeat => state.select_item(-1),
        Key::Down if press || action == Action::Repeat => state.select_item(1),
        Key::Comma if press && state.page == PAGE_COUNT - 1 => {
            state.route_field = state.route_field.shifted(-1);
        }
        Key::Period if press && state.page == PAGE_COUNT - 1 => {
            state.route_field = state.route_field.shifted(1);
        }
        Key::Left | Key::Right if press || action == Action::Repeat => {
            let toggled_field = state.page == PAGE_COUNT - 1
                && matches!(state.route_field, RouteField::Enabled | RouteField::Source);
            if !toggled_field || press {
                let direction = if key == Key::Left { -1.0 } else { 1.0 };
                state.edit_selected(&mut synth.lock().unwrap(), direction, coarse);
            }
        }
        Key::Home if press => state.set_selected_absolute(&mut synth.lock().unwrap(), 0.0),
        Key::Backslash if press => state.set_selected_absolute(&mut synth.lock().unwrap(), 0.5),
        Key::End if press => state.set_selected_absolute(&mut synth.lock().unwrap(), 1.0),
        Key::Delete if press => state.reset_selected(&mut synth.lock().unwrap()),
        Key::Backspace if press => state.reset_preset(&mut synth.lock().unwrap()),
        Key::Minus if press || action == Action::Repeat => {
            state.velocity = (state.velocity - if coarse { 0.1 } else { 0.02 }).max(0.0);
        }
        Key::Equal if press || action == Action::Repeat => {
            state.velocity = (state.velocity + if coarse { 0.1 } else { 0.02 }).min(1.0);
        }
        Key::LeftBracket if press => state.octave = (state.octave - 1).max(0),
        Key::RightBracket if press => state.octave = (state.octave + 1).min(8),
        Key::F1 if press => state.select_preset(&mut synth.lock().unwrap(), 0),
        Key::F2 if press => state.select_preset(&mut synth.lock().unwrap(), 1),
        Key::F3 if press => state.select_preset(&mut synth.lock().unwrap(), 2),
        Key::F4 if press => state.select_preset(&mut synth.lock().unwrap(), 3),
        Key::F5 if press => state.select_preset(&mut synth.lock().unwrap(), 4),
        _ => {}
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
            .create_window(width, height, "PolySynth Lab", glfw::WindowMode::Windowed)
            .ok_or_else(|| anyhow::anyhow!("failed to create PolySynth Lab window"))?;
        window.make_current();
        window.set_key_polling(true);
        window.set_framebuffer_size_polling(true);
        window.set_focus_polling(true);
        window.set_close_polling(true);
        glfw.set_swap_interval(glfw::SwapInterval::Sync(1));
        gl::load_with(|symbol| window.get_proc_address(symbol) as *const _);

        let shader = create_shader_program()?;
        let (vao, vbo) = create_buffers();
        let color_location = unsafe { gl::GetUniformLocation(shader, c"color".as_ptr()) };
        unsafe {
            gl::Viewport(0, 0, width as i32, height as i32);
        }
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

    fn should_close(&self) -> bool {
        self.window.should_close()
    }

    fn draw(&self, vertices: &[f32], mode: u32, color: (f32, f32, f32), line_width: f32) {
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
            gl::LineWidth(line_width);
            gl::DrawArrays(mode, 0, (vertices.len() / 2) as i32);
        }
    }

    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32, color: (f32, f32, f32)) {
        self.draw(
            &[x0, y0, x1, y0, x1, y1, x0, y1],
            gl::TRIANGLE_FAN,
            color,
            1.0,
        );
    }

    fn outline(&self, x0: f32, y0: f32, x1: f32, y1: f32, color: (f32, f32, f32), width: f32) {
        self.draw(
            &[x0, y0, x1, y0, x1, y1, x0, y1],
            gl::LINE_LOOP,
            color,
            width,
        );
    }

    fn render_scope(&self, left: &[f32], right: &[f32]) {
        self.draw(
            &[-0.96, 0.51, 0.80, 0.51],
            gl::LINES,
            (0.16, 0.2, 0.25),
            1.0,
        );
        self.draw(
            &[-0.96, 0.08, 0.80, 0.08],
            gl::LINES,
            (0.16, 0.2, 0.25),
            1.0,
        );
        if left.len() < 2 || right.len() < 2 {
            return;
        }

        let make_trace = |samples: &[f32], center: f32| {
            let mut vertices = Vec::with_capacity(samples.len() * 2);
            for (index, sample) in samples.iter().enumerate() {
                let x = -0.96 + index as f32 / (samples.len() - 1) as f32 * 1.76;
                let y = center + (sample * 1.8).clamp(-0.18, 0.18);
                vertices.extend_from_slice(&[x, y]);
            }
            vertices
        };
        self.draw(
            &make_trace(left, 0.72),
            gl::LINE_STRIP,
            (0.2, 0.9, 0.95),
            1.4,
        );
        self.draw(
            &make_trace(right, 0.29),
            gl::LINE_STRIP,
            (0.95, 0.35, 0.72),
            1.4,
        );

        let (mid_energy, side_energy) = left.iter().zip(right).fold(
            (0.0_f32, 0.0_f32),
            |(mid_energy, side_energy), (left, right)| {
                let mid = 0.5 * (left + right);
                let side = 0.5 * (left - right);
                (mid_energy + mid * mid, side_energy + side * side)
            },
        );
        let mid = mid_energy.sqrt();
        let side = side_energy.sqrt();
        let width = side / (mid + side).max(1e-9);
        self.outline(0.85, 0.08, 0.90, 0.94, (0.3, 0.35, 0.4), 1.0);
        self.rect(0.855, 0.085, 0.895, 0.085 + width * 0.85, (0.95, 0.7, 0.2));
    }

    fn render_parameter_bars(&self, state: &LabState, values: &[f32]) {
        let count = values.len();
        if count == 0 {
            return;
        }
        let left = -0.96;
        let right = 0.80;
        let gap = 0.025;
        let width = (right - left - gap * (count - 1) as f32) / count as f32;
        for (index, value) in values.iter().enumerate() {
            let x0 = left + index as f32 * (width + gap);
            let x1 = x0 + width;
            let selected = index == state.selected;
            self.outline(
                x0,
                -0.92,
                x1,
                -0.18,
                if selected {
                    (1.0, 0.85, 0.25)
                } else {
                    (0.28, 0.32, 0.38)
                },
                if selected { 3.0 } else { 1.0 },
            );
            self.rect(
                x0 + 0.008,
                -0.91,
                x1 - 0.008,
                -0.91 + value.clamp(0.0, 1.0) * 0.72,
                if selected {
                    (0.95, 0.55, 0.18)
                } else {
                    (0.18, 0.55, 0.72)
                },
            );
        }
    }

    fn render_routes(&self, state: &LabState, routes: &[PolyModRoute]) {
        let left = -0.96;
        let right = 0.80;
        let gap = 0.02;
        let width = (right - left - gap * (routes.len() - 1) as f32) / routes.len() as f32;
        for (index, route) in routes.iter().enumerate() {
            let x0 = left + index as f32 * (width + gap);
            let x1 = x0 + width;
            let selected = index == state.selected;
            self.outline(
                x0,
                -0.92,
                x1,
                -0.18,
                if selected {
                    (1.0, 0.85, 0.25)
                } else if route.enabled {
                    (0.35, 0.7, 0.45)
                } else {
                    (0.22, 0.25, 0.28)
                },
                if selected { 3.0 } else { 1.0 },
            );

            let center = -0.55;
            self.draw(&[x0, center, x1, center], gl::LINES, (0.3, 0.32, 0.35), 1.0);
            let depth_y = center + route.depth * 0.32;
            self.rect(
                x0 + 0.02,
                center.min(depth_y),
                x0 + width * 0.42,
                center.max(depth_y),
                if route.depth >= 0.0 {
                    (0.25, 0.85, 0.4)
                } else {
                    (0.9, 0.3, 0.3)
                },
            );
            let key_y = center + route.key_scale * 0.32;
            self.rect(
                x0 + width * 0.55,
                center.min(key_y),
                x1 - 0.02,
                center.max(key_y),
                (0.25, 0.55, 0.95),
            );
            self.rect(
                x0 + 0.02,
                -0.90,
                x1 - 0.02,
                -0.90 + route.curve * 0.06,
                match route.source {
                    PolyModSource::Velocity => (0.9, 0.55, 0.15),
                    PolyModSource::KeyPosition => (0.65, 0.35, 0.95),
                },
            );
        }
    }

    fn render(&mut self, state: &LabState, synth: &Arc<Mutex<PolySynth>>, scope: &StereoScope) {
        let (title, values, routes) = {
            let synth = synth.lock().unwrap();
            let values = page_params(state.page)
                .iter()
                .map(|param| synth.param(*param).unwrap_or(0.0))
                .collect::<Vec<_>>();
            let routes = if state.page == PAGE_COUNT - 1 {
                (0..POLY_MOD_ROUTE_COUNT)
                    .filter_map(|slot| synth.mod_route(slot))
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            (state.title(&synth), values, routes)
        };
        self.window.set_title(&title);
        let (left, right) = scope.snapshot();

        unsafe {
            gl::Viewport(0, 0, self.width, self.height);
            gl::ClearColor(0.025, 0.035, 0.055, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);
            gl::UseProgram(self.shader);
        }
        self.render_scope(&left, &right);
        if state.page == PAGE_COUNT - 1 {
            self.render_routes(state, &routes);
        } else {
            self.render_parameter_bars(state, &values);
        }

        self.outline(0.93, -0.92, 0.97, -0.18, (0.3, 0.35, 0.4), 1.0);
        self.rect(
            0.935,
            -0.91,
            0.965,
            -0.91 + state.velocity * 0.72,
            (0.3, 0.9, 0.55),
        );
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
    let vertex_source = CString::new(
        "#version 330 core\nlayout (location = 0) in vec2 aPos;\nvoid main() { gl_Position = vec4(aPos, 0.0, 1.0); }",
    )?;
    let fragment_source = CString::new(
        "#version 330 core\nout vec4 FragColor;\nuniform vec3 color;\nvoid main() { FragColor = vec4(color, 1.0); }",
    )?;
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
            let mut length = 0;
            gl::GetProgramiv(program, gl::INFO_LOG_LENGTH, &mut length);
            let mut log = vec![0_u8; length as usize];
            gl::GetProgramInfoLog(
                program,
                length,
                std::ptr::null_mut(),
                log.as_mut_ptr().cast(),
            );
            gl::DeleteProgram(program);
            anyhow::bail!("shader link failed: {}", String::from_utf8_lossy(&log));
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
    let mut length = 0;
    gl::GetShaderiv(shader, gl::INFO_LOG_LENGTH, &mut length);
    let mut log = vec![0_u8; length as usize];
    gl::GetShaderInfoLog(
        shader,
        length,
        std::ptr::null_mut(),
        log.as_mut_ptr().cast(),
    );
    anyhow::bail!(
        "{label} shader compile failed: {}",
        String::from_utf8_lossy(&log)
    );
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

fn print_help() {
    println!("PolySynth GUI Lab");
    println!("  Play: Z-M and Q-I (two chromatic octaves), SPACE=C major chord");
    println!("  Velocity: -/= (hold Shift for coarse)    Octave: [ / ]");
    println!("  Pages: Tab / Shift-Tab    Select: Up/Down    Edit: Left/Right");
    println!("  Matrix field: ,/.         Min/center/max: Home/\\/End");
    println!("  Reset selected: Delete    Reset current preset: Backspace");
    println!("  Presets: F1-F5            Panic: Escape      Quit: Cmd/Ctrl-Q");
    println!();
    println!("The window title shows the exact selected control and value.");
    println!("Cyan/magenta are left/right scope traces; the orange meter is side energy.");
    println!("On the matrix page, green/red is depth, blue is key scale, and the thin");
    println!("orange/purple strip is curve plus velocity/key source.");
}

fn main() -> anyhow::Result<()> {
    print_help();

    let synth = Arc::new(Mutex::new(PolySynth::new(SAMPLE_RATE)));
    let scope = Arc::new(StereoScope::new(SCOPE_CAPACITY));
    let mut engine = Engine::new(SAMPLE_RATE);
    engine.add_instrument(
        "polysynth",
        Box::new(SharedLabSynth {
            synth: Arc::clone(&synth),
            scope: Arc::clone(&scope),
        }),
    );
    engine.set_master_gain(0.8);

    let engine = Arc::new(Mutex::new(engine));
    let mut output = EngineOutput::new();
    output.initialize(SAMPLE_RATE)?;
    output.create_stream_with_engine(engine)?;
    output.start()?;

    let mut state = LabState::new();
    let mut ui = LabWindow::new(1_240, 780)?;
    while !ui.should_close() {
        for event in ui.poll_events() {
            match event {
                WindowEvent::Key(key, _, action, modifiers) => {
                    handle_key(&mut ui.window, &mut state, &synth, key, action, modifiers);
                }
                WindowEvent::FramebufferSize(width, height) => {
                    ui.width = width.max(1);
                    ui.height = height.max(1);
                }
                WindowEvent::Focus(false) => state.panic(&mut synth.lock().unwrap()),
                WindowEvent::Close => ui.window.set_should_close(true),
                _ => {}
            }
        }
        ui.render(&state, &synth, &scope);
    }

    state.panic(&mut synth.lock().unwrap());
    output.stop()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameter_pages_cover_every_public_parameter_once() {
        let mut params = (0..PAGE_COUNT - 1)
            .flat_map(page_params)
            .copied()
            .collect::<Vec<_>>();
        params.sort_unstable();
        assert_eq!(params, (0..POLY_PARAM_COUNT).collect::<Vec<_>>());
    }

    #[test]
    fn computer_keyboard_map_is_chromatic_for_two_octaves() {
        let keys = [
            Key::Z,
            Key::S,
            Key::X,
            Key::D,
            Key::C,
            Key::V,
            Key::G,
            Key::B,
            Key::H,
            Key::N,
            Key::J,
            Key::M,
            Key::Q,
            Key::Num2,
            Key::W,
            Key::Num3,
            Key::E,
            Key::R,
            Key::Num5,
            Key::T,
            Key::Num6,
            Key::Y,
            Key::Num7,
            Key::U,
            Key::I,
        ];
        assert_eq!(
            keys.into_iter().map(note_offset).collect::<Vec<_>>(),
            (0..=24).map(Some).collect::<Vec<_>>()
        );
    }

    #[test]
    fn page_and_matrix_navigation_wraps() {
        let mut state = LabState::new();
        state.select_page(-1);
        assert_eq!(state.page, PAGE_COUNT - 1);
        state.select_item(-1);
        assert_eq!(state.selected, POLY_MOD_ROUTE_COUNT - 1);
        assert_eq!(RouteField::Enabled.shifted(-1), RouteField::KeyScale);
        assert_eq!(RouteField::KeyScale.shifted(1), RouteField::Enabled);
    }
}
