//! Spatial explorer for the modal plate voice.
//!
//! Run with:
//!
//!     cargo run --release --example plate_explorer --features native,visualization
//!
//! The centre view draws the struck body in 3-D (drum head, cymbal, bar, bell
//! or hanging sheet, following the material) and ripples it with the voice's
//! live mode amplitudes, slowed down so the shapes stay visible. Strikers and
//! the two pickups (left and right channel) are points on the surface:
//!
//! - click the surface to tap it, shift-drag to roll a mallet across it
//! - drag a striker or pickup to move it, click a striker to hit it
//! - double-click to add a striker, right-click one to remove it
//! - drag empty space to orbit, scroll to zoom
//! - hover a mode in the spectrum to see its shape and nodal lines
//!
//! Every hit, from any striker, lands on the same plate, so fast patterns and
//! rolls build up energy and drive the tension glide and cascade the way one
//! physical plate would. The voice models position as a 1-D centre-to-edge
//! coordinate, so only a point's distance from the centre (or from the middle
//! of the bar) changes the sound; its angle is free.
//!
//! The timbre map plots any two parameters as a plane. Drag the cursor to
//! morph the live sound, store snapshots as pins, drag pins to reshape them
//! and let the tour step through them bar by bar.

use std::f32::consts::{PI, TAU};
use std::ops::{Add, Mul, Sub};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Color32, ColorImage, FontId, Pos2, Rect, RichText, Sense, Stroke, TextureHandle,
    TextureOptions, Vec2,
};
use gooey::engine::{Engine, EngineOutput, Instrument};
use gooey::instruments::{
    plate_mode_shape, PlateConfig, PlateMaterial, PlateModeState, PlateVoice, PLATE_MAX_MODES,
};
use gooey::StereoFrame;

const SAMPLE_RATE: f32 = 44_100.0;
const MAX_STRIKERS: usize = 6;
const MAX_SNAPSHOTS: usize = 8;
const STEPS: usize = 16;
const HIT_QUEUE: usize = 32;
/// Hits that land on the same sample are spread 1 ms apart, loudest first,
/// because the plate has a single exciter.
const FLAM_SAMPLES: u32 = 44;
/// Index of the strike position in `PlateVoice::set_parameter_normalized`.
const STRIKE_PARAM: usize = 2;
/// Mode amplitude shown as full scale, and the range below it that is drawn.
const AMP_REF: f32 = 0.5;
const DB_RANGE: f32 = 60.0;
/// Peak surface displacement, in units of the body's radius.
const DISP_MAX: f32 = 0.14;
/// Samples of the u (centre-to-edge) axis in the displacement table.
const PROFILE_RES: usize = 128;
const SPECTROGRAM_COLS: usize = 300;
const SPECTROGRAM_ROWS: usize = 112;
const SPECTROGRAM_HZ: f32 = 30.0;
const BAR_HALF_LENGTH: f32 = 1.25;
const BAR_HALF_WIDTH: f32 = 0.3;
const ROLL_INTERVAL: Duration = Duration::from_millis(55);
const MAP_AUDITION_INTERVAL: Duration = Duration::from_millis(320);

const STRIKER_COLORS: [Color32; MAX_STRIKERS] = [
    Color32::from_rgb(255, 196, 64),
    Color32::from_rgb(255, 104, 96),
    Color32::from_rgb(120, 220, 120),
    Color32::from_rgb(196, 140, 255),
    Color32::from_rgb(255, 150, 210),
    Color32::from_rgb(240, 240, 240),
];
const PICKUP_COLORS: [Color32; 2] = [
    Color32::from_rgb(96, 210, 255),
    Color32::from_rgb(255, 120, 200),
];
const SNAPSHOT_COLORS: [Color32; MAX_SNAPSHOTS] = [
    Color32::from_rgb(255, 196, 64),
    Color32::from_rgb(96, 210, 255),
    Color32::from_rgb(255, 104, 96),
    Color32::from_rgb(120, 220, 120),
    Color32::from_rgb(196, 140, 255),
    Color32::from_rgb(255, 150, 210),
    Color32::from_rgb(255, 150, 80),
    Color32::from_rgb(160, 200, 255),
];
/// Cascade bands, low to high.
const BAND_COLORS: [Color32; 4] = [
    Color32::from_rgb(255, 190, 90),
    Color32::from_rgb(255, 120, 90),
    Color32::from_rgb(200, 110, 255),
    Color32::from_rgb(90, 170, 255),
];
const BAND_EDGES: [f32; 3] = [3.0, 8.0, 20.0];
const PANEL_BG: Color32 = Color32::from_rgb(16, 18, 22);
const DIM_TEXT: Color32 = Color32::from_rgb(150, 156, 168);

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

/// The continuous voice parameters, in the units of `PlateConfig`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Param {
    Pitch,
    Geometry,
    Hardness,
    Decay,
    HfDamping,
    Tension,
    Cascade,
    Deplete,
    Brightness,
}

impl Param {
    const ALL: [Self; 9] = [
        Self::Pitch,
        Self::Geometry,
        Self::Hardness,
        Self::Decay,
        Self::HfDamping,
        Self::Tension,
        Self::Cascade,
        Self::Deplete,
        Self::Brightness,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Pitch => "Pitch",
            Self::Geometry => "Geometry",
            Self::Hardness => "Hardness",
            Self::Decay => "Decay",
            Self::HfDamping => "HF damping",
            Self::Tension => "Tension",
            Self::Cascade => "Cascade",
            Self::Deplete => "Deplete",
            Self::Brightness => "Brightness",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::Pitch => "Fundamental frequency of the bank.",
            Self::Geometry => {
                "Stretches the mode ratios: r^(1 + geometry). Negative squeezes the \
                 partials together, positive spreads them apart."
            }
            Self::Hardness => {
                "Mallet: soft pulse (8 ms), click, then hard noise burst (0.25 ms). \
                 Harder mallets excite higher modes."
            }
            Self::Decay => "T60 of the fundamental.",
            Self::HfDamping => "How much faster the upper modes die than the fundamental.",
            Self::Tension => {
                "Energy-driven pitch glide: hard hits start sharp and slide down as \
                 the plate rings out."
            }
            Self::Cascade => {
                "Each band of modes saturates and drives the band above, so hard hits \
                 bloom into shimmer after the strike."
            }
            Self::Deplete => "Extra loss of the lower bands while they feed the cascade.",
            Self::Brightness => "Output low-pass cutoff.",
        }
    }

    fn range(self) -> (f32, f32) {
        match self {
            Self::Pitch => (30.0, 2_000.0),
            Self::Geometry => (-0.5, 0.5),
            Self::Decay => (0.05, 20.0),
            Self::HfDamping => (0.0, 8.0),
            Self::Brightness => (500.0, 20_000.0),
            Self::Hardness | Self::Tension | Self::Cascade | Self::Deplete => (0.0, 1.0),
        }
    }

    fn logarithmic(self) -> bool {
        matches!(self, Self::Pitch | Self::Decay | Self::Brightness)
    }

    fn get(self, config: &PlateConfig) -> f32 {
        match self {
            Self::Pitch => config.pitch_hz,
            Self::Geometry => config.geometry,
            Self::Hardness => config.hardness,
            Self::Decay => config.decay_seconds,
            Self::HfDamping => config.hf_damping,
            Self::Tension => config.tension,
            Self::Cascade => config.cascade,
            Self::Deplete => config.deplete,
            Self::Brightness => config.brightness_hz,
        }
    }

    fn set(self, config: &mut PlateConfig, value: f32) {
        let (min, max) = self.range();
        let value = value.clamp(min, max);
        match self {
            Self::Pitch => config.pitch_hz = value,
            Self::Geometry => config.geometry = value,
            Self::Hardness => config.hardness = value,
            Self::Decay => config.decay_seconds = value,
            Self::HfDamping => config.hf_damping = value,
            Self::Tension => config.tension = value,
            Self::Cascade => config.cascade = value,
            Self::Deplete => config.deplete = value,
            Self::Brightness => config.brightness_hz = value,
        }
    }

    fn normalized(self, config: &PlateConfig) -> f32 {
        let (min, max) = self.range();
        let value = self.get(config).clamp(min, max);
        if self.logarithmic() {
            (value / min).ln() / (max / min).ln()
        } else {
            (value - min) / (max - min)
        }
    }

    fn set_normalized(self, config: &mut PlateConfig, t: f32) {
        let (min, max) = self.range();
        let t = t.clamp(0.0, 1.0);
        let value = if self.logarithmic() {
            min * (max / min).powf(t)
        } else {
            min + t * (max - min)
        };
        self.set(config, value);
    }

    fn format(self, value: f32) -> String {
        match self {
            Self::Pitch => format!("{value:.0} Hz"),
            Self::Geometry => format!("{value:+.2}"),
            Self::Hardness => {
                let kind = if value < 1.0 / 3.0 {
                    "pulse"
                } else if value < 2.0 / 3.0 {
                    "click"
                } else {
                    "noise"
                };
                let width = 8.0 * (0.25_f32 / 8.0).powf(value);
                format!("{kind} {width:.2} ms")
            }
            Self::Decay => format!("{value:.2} s"),
            Self::HfDamping => format!("{value:.2}"),
            Self::Brightness => format!("{:.1} kHz", value / 1_000.0),
            Self::Tension | Self::Cascade | Self::Deplete => format!("{:.0}%", value * 100.0),
        }
    }
}

const PARAM_GROUPS: [(&str, &[Param]); 4] = [
    ("BODY", &[Param::Pitch, Param::Geometry]),
    ("MALLET", &[Param::Hardness]),
    ("RING", &[Param::Decay, Param::HfDamping, Param::Brightness]),
    (
        "NONLINEAR",
        &[Param::Tension, Param::Cascade, Param::Deplete],
    ),
];

#[derive(Clone, Copy)]
struct Preset {
    name: &'static str,
    config: fn() -> PlateConfig,
}

const PRESETS: [Preset; 5] = [
    Preset {
        name: "Ride",
        config: PlateConfig::ride,
    },
    Preset {
        name: "Crash",
        config: PlateConfig::crash,
    },
    Preset {
        name: "Bell",
        config: PlateConfig::bell,
    },
    Preset {
        name: "Timbale",
        config: PlateConfig::timbale,
    },
    Preset {
        name: "Gong",
        config: PlateConfig::gong,
    },
];

fn material_hint(material: PlateMaterial) -> &'static str {
    match material {
        PlateMaterial::Membrane => "circular membrane: toms, timbales, tabla",
        PlateMaterial::Plate => "free circular plate: cymbals",
        PlateMaterial::Bar => "free-free bar: marimba, glockenspiel",
        PlateMaterial::Bell => "church bell partials",
        PlateMaterial::Sheet => "dense metal sheet: gongs, thunder sheets",
    }
}

/// Maps a mode amplitude to 0-1 over the displayed dB range.
fn amplitude_level(amplitude: f32) -> f32 {
    let db = 20.0 * (amplitude.max(1.0e-9) / AMP_REF).log10();
    ((db + DB_RANGE) / DB_RANGE).clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}

impl Vec3 {
    const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    fn cross(self, other: Self) -> Self {
        Self::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    fn normalized_or(self, fallback: Self) -> Self {
        let length = self.length();
        if length > 1.0e-6 {
            self * (1.0 / length)
        } else {
            fallback
        }
    }
}

impl Add for Vec3 {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }
}

impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }
}

impl Mul<f32> for Vec3 {
    type Output = Self;
    fn mul(self, scale: f32) -> Self {
        Self::new(self.x * scale, self.y * scale, self.z * scale)
    }
}

const UP: Vec3 = Vec3::new(0.0, 0.0, 1.0);

/// A point on the body in material coordinates: `u` is the voice's 0 (centre)
/// to 1 (edge) position, `theta` the free angle around the centre.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Point {
    u: f32,
    theta: f32,
}

impl Point {
    const fn new(u: f32, theta: f32) -> Self {
        Self { u, theta }
    }
}

/// The 3-D body drawn for each material.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Body {
    Drum,
    Cymbal,
    Bar,
    Bell,
    Sheet,
}

impl Body {
    fn for_material(material: PlateMaterial) -> Self {
        match material {
            PlateMaterial::Membrane => Self::Drum,
            PlateMaterial::Plate => Self::Cymbal,
            PlateMaterial::Bar => Self::Bar,
            PlateMaterial::Bell => Self::Bell,
            PlateMaterial::Sheet => Self::Sheet,
        }
    }

    fn tint(self) -> [f32; 3] {
        match self {
            Self::Drum => [222.0, 210.0, 184.0],
            Self::Cymbal => [214.0, 172.0, 92.0],
            Self::Bar => [176.0, 186.0, 204.0],
            Self::Bell => [190.0, 128.0, 72.0],
            Self::Sheet => [160.0, 170.0, 182.0],
        }
    }

    /// Displacement scale, so curved bodies do not fold over themselves.
    fn swing(self) -> f32 {
        match self {
            Self::Bell => 0.35,
            Self::Cymbal => 0.8,
            Self::Drum | Self::Bar | Self::Sheet => 1.0,
        }
    }

    fn position_note(self) -> &'static str {
        match self {
            Self::Bar => "position = distance from the middle of the bar",
            Self::Bell => "position = distance from the crown down to the lip",
            Self::Sheet => "position = distance from the centre of the sheet",
            Self::Drum | Self::Cymbal => "position = distance from the centre",
        }
    }

    fn default_camera(self) -> Camera {
        match self {
            Self::Bell => Camera {
                yaw: 0.3,
                pitch: 0.38,
                zoom: 1.0,
            },
            Self::Sheet => Camera {
                yaw: 0.25,
                pitch: 0.18,
                zoom: 0.95,
            },
            Self::Bar => Camera {
                yaw: 0.15,
                pitch: 0.8,
                zoom: 1.1,
            },
            Self::Drum | Self::Cymbal => Camera {
                yaw: 0.35,
                pitch: 0.72,
                zoom: 1.0,
            },
        }
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Cymbal height: a shallow cone with a raised bell cup.
fn cymbal_height(u: f32) -> f32 {
    0.09 * (1.0 - u) + 0.11 * (1.0 - smoothstep(0.1, 0.26, u)) - 0.06
}

/// Bell radius and height from the crown (u = 0) to the lip (u = 1).
fn bell_profile(u: f32) -> (f32, f32) {
    if u < 0.2 {
        let w = u / 0.2;
        (0.38 * w, 0.62 - 0.04 * w * w)
    } else {
        let w = (u - 0.2) / 0.8;
        (
            0.38 + 0.12 * w + 0.48 * w * w * w,
            0.58 - 1.05 * w - 0.08 * w * w,
        )
    }
}

/// Surface point and outward normal at a material point.
fn surface_frame(body: Body, at: Point) -> (Vec3, Vec3) {
    let u = at.u.clamp(0.0, 1.0);
    let (sin, cos) = at.theta.sin_cos();
    const EPS: f32 = 1.0e-3;
    match body {
        Body::Drum => (Vec3::new(u * cos, u * sin, 0.0), UP),
        Body::Cymbal => {
            let slope = (cymbal_height((u + EPS).min(1.0)) - cymbal_height((u - EPS).max(0.0)))
                / (2.0 * EPS);
            (
                Vec3::new(u * cos, u * sin, cymbal_height(u)),
                Vec3::new(-slope * cos, -slope * sin, 1.0).normalized_or(UP),
            )
        }
        Body::Bar => {
            let side = if cos >= 0.0 { 1.0 } else { -1.0 };
            (
                Vec3::new(side * u * BAR_HALF_LENGTH, sin * BAR_HALF_WIDTH, 0.0),
                UP,
            )
        }
        Body::Bell => {
            let (r, z) = bell_profile(u);
            let (r0, z0) = bell_profile((u - EPS).max(0.0));
            let (r1, z1) = bell_profile((u + EPS).min(1.0));
            let (dr, dz) = (r1 - r0, z1 - z0);
            (
                Vec3::new(r * cos, r * sin, z - 0.05),
                Vec3::new(-dz * cos, -dz * sin, dr).normalized_or(UP),
            )
        }
        Body::Sheet => {
            let scale = cos.abs().max(sin.abs()).max(1.0e-6);
            (
                Vec3::new(u * cos / scale, 0.0, u * sin / scale),
                Vec3::new(0.0, -1.0, 0.0),
            )
        }
    }
}

/// Material point for a bar mesh coordinate (x along, s across, both -1..1).
fn bar_point(x: f32, s: f32) -> Point {
    let lateral = s.clamp(-1.0, 1.0).asin();
    let theta = if x >= 0.0 { lateral } else { PI - lateral };
    Point::new(x.abs(), theta)
}

/// A gridded surface mesh in material coordinates, rebuilt when the body
/// changes. Rows run along `a`, columns along `b`.
struct BodyMesh {
    body: Body,
    rows: usize,
    cols: usize,
    points: Vec<Point>,
    base: Vec<Vec3>,
    normals: Vec<Vec3>,
    /// Static extra triangles (drum shell), with flat normals.
    decor: Vec<([Vec3; 3], Vec3, [f32; 3])>,
    /// Static lines drawn behind the mesh (stand, strings, supports).
    lines: Vec<(Vec3, Vec3)>,
}

impl BodyMesh {
    fn new(body: Body) -> Self {
        let (rows, cols) = match body {
            Body::Bar => (73, 9),
            _ => (37, 73),
        };
        let mut points = Vec::with_capacity(rows * cols);
        for i in 0..rows {
            for j in 0..cols {
                let a = i as f32 / (rows - 1) as f32;
                let b = j as f32 / (cols - 1) as f32;
                points.push(match body {
                    Body::Bar => bar_point(2.0 * a - 1.0, 2.0 * b - 1.0),
                    _ => Point::new(a, b * TAU),
                });
            }
        }
        let (base, normals) = points
            .iter()
            .map(|point| surface_frame(body, *point))
            .unzip();
        let mut decor = Vec::new();
        let mut lines = Vec::new();
        match body {
            Body::Drum => {
                let segments = 48;
                let shell = [120.0, 36.0, 40.0];
                for k in 0..segments {
                    let t0 = k as f32 / segments as f32 * TAU;
                    let t1 = (k + 1) as f32 / segments as f32 * TAU;
                    let top0 = Vec3::new(1.01 * t0.cos(), 1.01 * t0.sin(), 0.0);
                    let top1 = Vec3::new(1.01 * t1.cos(), 1.01 * t1.sin(), 0.0);
                    let drop = Vec3::new(0.0, 0.0, -0.42);
                    let normal = Vec3::new((0.5 * (t0 + t1)).cos(), (0.5 * (t0 + t1)).sin(), 0.0);
                    decor.push(([top0, top1, top1 + drop], normal, shell));
                    decor.push(([top0, top1 + drop, top0 + drop], normal, shell));
                }
                for leg in 0..3 {
                    let angle = leg as f32 / 3.0 * TAU + 0.5;
                    let foot = Vec3::new(1.15 * angle.cos(), 1.15 * angle.sin(), -0.95);
                    lines.push((Vec3::new(angle.cos(), angle.sin(), -0.42), foot));
                }
            }
            Body::Cymbal => {
                lines.push((Vec3::new(0.0, 0.0, 0.2), Vec3::new(0.0, 0.0, -1.0)));
            }
            Body::Bar => {
                // The free-free bar's fundamental nodes, where it rests on cord.
                for side in [-1.0, 1.0] {
                    let x = side * 0.552 * BAR_HALF_LENGTH;
                    lines.push((Vec3::new(x, -0.45, -0.02), Vec3::new(x, 0.45, -0.02)));
                }
            }
            Body::Bell => {
                lines.push((Vec3::new(0.0, 0.0, 0.57), Vec3::new(0.0, 0.0, 0.85)));
                lines.push((Vec3::new(-0.45, 0.0, 0.85), Vec3::new(0.45, 0.0, 0.85)));
            }
            Body::Sheet => {
                for side in [-1.0, 1.0] {
                    lines.push((
                        Vec3::new(side * 0.95, 0.0, 1.0),
                        Vec3::new(side * 0.6, 0.0, 1.6),
                    ));
                }
            }
        }
        Self {
            body,
            rows,
            cols,
            points,
            base,
            normals,
            decor,
            lines,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Camera {
    yaw: f32,
    pitch: f32,
    zoom: f32,
}

impl Camera {
    const DISTANCE: f32 = 4.2;

    /// Screen position and depth (larger is farther away).
    fn project(&self, p: Vec3, rect: Rect) -> (Pos2, f32) {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let x1 = p.x * cy - p.y * sy;
        let y1 = p.x * sy + p.y * cy;
        let view_x = x1;
        let view_y = y1 * sp + p.z * cp;
        let view_z = -y1 * cp + p.z * sp;
        let depth = Self::DISTANCE - view_z;
        let scale = rect.width().min(rect.height()) * 1.45 * self.zoom / depth.max(0.1);
        (
            rect.center() + Vec2::new(view_x * scale, -view_y * scale),
            depth,
        )
    }

    /// Unit vector from the scene towards the camera, in world space.
    fn toward_viewer(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(-cp * sy, -cp * cy, sp)
    }
}

// ---------------------------------------------------------------------------
// Audio
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Hit {
    position: f32,
    velocity: f32,
    striker: Option<usize>,
}

/// Two copies of the same plate, one per pickup. They receive identical
/// triggers and seeds and the pickup does not feed back into the modes, so
/// the two banks ring identically and only the pickup weighting differs.
struct PlateRig {
    left: PlateVoice,
    right: PlateVoice,
    sample_rate: f32,
    striker_count: usize,
    striker_positions: [f32; MAX_STRIKERS],
    pattern: [[f32; STEPS]; MAX_STRIKERS],
    playing: bool,
    bpm: f32,
    next_step: usize,
    last_step: Option<usize>,
    samples_to_step: f64,
    bars: u32,
    queue: [Hit; HIT_QUEUE],
    queue_len: usize,
    flam_wait: u32,
    hit_serials: [u32; MAX_STRIKERS],
    volume: f32,
    peaks: [f32; 2],
    peak_release: f32,
}

impl PlateRig {
    fn new(sample_rate: f32, config: PlateConfig) -> Self {
        Self {
            left: PlateVoice::with_config(sample_rate, config),
            right: PlateVoice::with_config(sample_rate, config),
            sample_rate,
            striker_count: 0,
            striker_positions: [0.0; MAX_STRIKERS],
            pattern: [[0.0; STEPS]; MAX_STRIKERS],
            playing: false,
            bpm: 110.0,
            next_step: 0,
            last_step: None,
            samples_to_step: 0.0,
            bars: 0,
            queue: [Hit::default(); HIT_QUEUE],
            queue_len: 0,
            flam_wait: 0,
            hit_serials: [0; MAX_STRIKERS],
            volume: 0.8,
            peaks: [0.0; 2],
            peak_release: (-1.0 / (0.3 * sample_rate)).exp(),
        }
    }

    fn apply(&mut self, config: &PlateConfig, pickups: [f32; 2]) {
        self.left.set_config(PlateConfig {
            pickup_pos: pickups[0],
            ..*config
        });
        self.right.set_config(PlateConfig {
            pickup_pos: pickups[1],
            ..*config
        });
    }

    fn set_playing(&mut self, playing: bool) {
        if playing && !self.playing {
            self.next_step = 0;
            self.samples_to_step = 0.0;
        }
        if !playing {
            self.last_step = None;
        }
        self.playing = playing;
    }

    /// Queue a hit, keeping the queue ordered loudest first.
    fn queue_hit(&mut self, hit: Hit) {
        if self.queue_len == HIT_QUEUE || hit.velocity.is_nan() || hit.velocity <= 0.0 {
            return;
        }
        let at = self.queue[..self.queue_len]
            .iter()
            .position(|queued| queued.velocity < hit.velocity)
            .unwrap_or(self.queue_len);
        self.queue.copy_within(at..self.queue_len, at + 1);
        self.queue[at] = hit;
        self.queue_len += 1;
    }

    fn advance_sequencer(&mut self) {
        if !self.playing {
            return;
        }
        self.samples_to_step -= 1.0;
        if self.samples_to_step > 0.0 {
            return;
        }
        self.samples_to_step += self.sample_rate as f64 * 15.0 / self.bpm.max(1.0) as f64;
        let step = self.next_step;
        for striker in 0..self.striker_count {
            self.queue_hit(Hit {
                position: self.striker_positions[striker],
                velocity: self.pattern[striker][step],
                striker: Some(striker),
            });
        }
        self.last_step = Some(step);
        self.next_step = (step + 1) % STEPS;
        if self.next_step == 0 {
            self.bars = self.bars.wrapping_add(1);
        }
    }

    fn fire_queued(&mut self, time: f64) {
        if self.flam_wait > 0 {
            self.flam_wait -= 1;
            return;
        }
        if self.queue_len == 0 {
            return;
        }
        let hit = self.queue[0];
        self.queue.copy_within(1..self.queue_len, 0);
        self.queue_len -= 1;
        for voice in [&mut self.left, &mut self.right] {
            voice.set_parameter_normalized(STRIKE_PARAM, hit.position);
            voice.trigger_with_velocity(time, hit.velocity);
        }
        if let Some(striker) = hit.striker {
            self.hit_serials[striker] = self.hit_serials[striker].wrapping_add(1);
        }
        self.flam_wait = FLAM_SAMPLES;
    }

    fn render(&mut self, time: f64) -> StereoFrame {
        self.advance_sequencer();
        self.fire_queued(time);
        let left = self.left.tick(time) * self.volume;
        let right = self.right.tick(time) * self.volume;
        for (peak, sample) in self.peaks.iter_mut().zip([left, right]) {
            *peak = (*peak * self.peak_release).max(sample.abs());
        }
        StereoFrame { l: left, r: right }
    }
}

struct SharedRig(Arc<Mutex<PlateRig>>);

impl Instrument for SharedRig {
    fn trigger_with_velocity(&mut self, _time: f64, velocity: f32) {
        let mut rig = self.0.lock().unwrap();
        let position = rig.striker_positions[0];
        rig.queue_hit(Hit {
            position,
            velocity,
            striker: Some(0),
        });
    }

    fn tick(&mut self, time: f64) -> f32 {
        let frame = self.0.lock().unwrap().render(time);
        0.5 * (frame.l + frame.r)
    }

    fn tick_stereo(&mut self, time: f64) -> Option<StereoFrame> {
        Some(self.0.lock().unwrap().render(time))
    }

    fn is_active(&self) -> bool {
        let rig = self.0.lock().unwrap();
        rig.playing || rig.left.is_active()
    }
}

/// Drives the engine in real time when there is no audio device, so the
/// explorer still animates (for example over remote desktop).
fn spawn_silent_clock(engine: Arc<Mutex<Engine>>) {
    std::thread::spawn(move || {
        const BLOCK: u64 = 256;
        let start = Instant::now();
        let mut sample = 0_u64;
        loop {
            let due = (start.elapsed().as_secs_f64() * SAMPLE_RATE as f64) as u64;
            if sample + BLOCK <= due {
                let mut engine = engine.lock().unwrap();
                for _ in 0..BLOCK {
                    engine.tick_stereo(sample as f64 / SAMPLE_RATE as f64);
                    sample += 1;
                }
            } else {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Scene
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
struct Striker {
    at: Point,
    velocity: f32,
    pattern: [f32; STEPS],
}

#[derive(Clone, Debug, PartialEq)]
struct Scene {
    config: PlateConfig,
    strikers: Vec<Striker>,
    pickups: [Point; 2],
}

impl Scene {
    fn starter() -> Self {
        let config = PlateConfig::ride();
        let strikers = vec![
            Striker {
                at: Point::new(config.strike_pos, -1.9),
                velocity: 0.8,
                pattern: [
                    1.0, 0.0, 0.5, 0.0, 0.8, 0.0, 0.5, 0.4, 1.0, 0.0, 0.5, 0.0, 0.8, 0.0, 0.5, 0.4,
                ],
            },
            Striker {
                at: Point::new(0.86, -0.5),
                velocity: 1.0,
                pattern: [
                    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
                ],
            },
            Striker {
                at: Point::new(0.06, 2.3),
                velocity: 0.7,
                pattern: [
                    0.0, 0.0, 0.0, 0.0, 0.7, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.7, 0.0, 0.0, 0.0,
                ],
            },
        ];
        let mut scene = Self {
            config,
            strikers,
            pickups: [Point::new(0.0, 2.9), Point::new(0.0, 0.25)],
        };
        scene.place_pickups(config.pickup_pos);
        scene
    }

    /// Put the left pickup at the preset position and the right one a little
    /// further along, so the two channels hear the modes differently.
    fn place_pickups(&mut self, pickup: f32) {
        self.pickups[0].u = pickup;
        self.pickups[1].u = if pickup < 0.8 {
            pickup + 0.18
        } else {
            pickup - 0.18
        };
    }

    fn load_preset(&mut self, config: PlateConfig) {
        self.config = config;
        if let Some(first) = self.strikers.first_mut() {
            first.at.u = config.strike_pos;
        }
        self.place_pickups(config.pickup_pos);
    }

    /// The config the voice actually plays, with the first striker and left
    /// pickup written back into the position fields.
    fn effective_config(&self) -> PlateConfig {
        PlateConfig {
            strike_pos: self.strikers.first().map_or(0.5, |s| s.at.u),
            pickup_pos: self.pickups[0].u,
            ..self.config
        }
    }
}

fn config_as_rust(config: &PlateConfig) -> String {
    format!(
        "PlateConfig {{\n    pitch_hz: {:.1},\n    geometry: {:.3},\n    strike_pos: {:.3},\n    \
         pickup_pos: {:.3},\n    hardness: {:.3},\n    decay_seconds: {:.3},\n    \
         hf_damping: {:.3},\n    tension: {:.3},\n    cascade: {:.3},\n    deplete: {:.3},\n    \
         brightness_hz: {:.0},\n    material: PlateMaterial::{:?},\n    seed: 0x{:08X},\n}}",
        config.pitch_hz,
        config.geometry,
        config.strike_pos,
        config.pickup_pos,
        config.hardness,
        config.decay_seconds,
        config.hf_damping,
        config.tension,
        config.cascade,
        config.deplete,
        config.brightness_hz,
        config.material,
        config.seed,
    )
}

/// Small deterministic generator for randomise / mutate.
struct Dice(u32);

impl Dice {
    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1 << 24) as f32
    }

    fn range(&mut self, min: f32, max: f32) -> f32 {
        min + (max - min) * self.unit()
    }

    fn log_range(&mut self, min: f32, max: f32) -> f32 {
        min * (max / min).powf(self.unit())
    }
}

fn random_config(dice: &mut Dice) -> PlateConfig {
    PlateConfig {
        pitch_hz: dice.log_range(60.0, 1_200.0),
        geometry: dice.range(-0.3, 0.4),
        strike_pos: dice.unit(),
        pickup_pos: dice.unit(),
        hardness: dice.unit(),
        decay_seconds: dice.log_range(0.3, 12.0),
        hf_damping: dice.range(0.0, 3.0),
        tension: dice.range(0.0, 0.6),
        cascade: dice.unit(),
        deplete: dice.range(0.0, 0.7),
        brightness_hz: dice.log_range(3_000.0, 20_000.0),
        material: PlateMaterial::from_index((dice.unit() * 5.0) as usize),
        seed: dice.next_u32(),
    }
}

fn mutate_config(config: &mut PlateConfig, dice: &mut Dice) {
    for param in Param::ALL {
        let t = param.normalized(config) + dice.range(-0.08, 0.08);
        param.set_normalized(config, t);
    }
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Tab {
    Material,
    TimbreMap,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Marker {
    Striker(usize),
    Pickup(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ViewDrag {
    Marker(Marker),
    Orbit,
    Roll,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum MapDrag {
    Cursor,
    Snapshot(usize),
}

/// An expanding ring where a hit landed.
struct Ripple {
    at: Point,
    born: Instant,
    color: Color32,
}

struct Explorer {
    rig: Arc<Mutex<PlateRig>>,
    _engine: Arc<Mutex<Engine>>,
    _output: Option<EngineOutput>,
    audio_status: String,
    scene: Scene,
    snapshots: Vec<Scene>,
    tab: Tab,
    camera: Camera,
    mesh: BodyMesh,
    playing: bool,
    bpm: f32,
    volume: f32,
    tap_velocity: f32,
    motion_speed: f32,
    audition: bool,
    tour: bool,
    tour_index: usize,
    seen_bars: u32,
    pending_hits: Vec<Hit>,
    dice: Dice,
    // Live readings from the voice, refreshed every frame.
    modes: [PlateModeState; PLATE_MAX_MODES],
    phases: [f32; PLATE_MAX_MODES],
    energy: f32,
    cascade_level: f32,
    peaks: [f32; 2],
    current_step: Option<usize>,
    seen_serials: [u32; MAX_STRIKERS],
    flashes: [Option<Instant>; MAX_STRIKERS],
    ripples: Vec<Ripple>,
    hovered_mode: Option<usize>,
    view_drag: Option<ViewDrag>,
    last_roll: Instant,
    map_axes: [Param; 2],
    map_drag: Option<MapDrag>,
    map_trail: Vec<Pos2>,
    last_map_audition: Instant,
    spectrogram: ColorImage,
    spectrogram_texture: Option<TextureHandle>,
    last_spectrogram: Instant,
    shape_table: Vec<[f32; PROFILE_RES]>,
    toast: Option<(String, Instant)>,
}

impl Explorer {
    fn new(
        rig: Arc<Mutex<PlateRig>>,
        engine: Arc<Mutex<Engine>>,
        output: Option<EngineOutput>,
        audio_status: String,
    ) -> Self {
        let scene = Scene::starter();
        let body = Body::for_material(scene.config.material);
        let shape_table = (0..PLATE_MAX_MODES)
            .map(|k| {
                std::array::from_fn(|i| plate_mode_shape(k, i as f32 / (PROFILE_RES - 1) as f32))
            })
            .collect();
        Self {
            rig,
            _engine: engine,
            _output: output,
            audio_status,
            scene,
            snapshots: Vec::new(),
            tab: Tab::Material,
            camera: body.default_camera(),
            mesh: BodyMesh::new(body),
            playing: false,
            bpm: 110.0,
            volume: 0.8,
            tap_velocity: 0.8,
            motion_speed: 1.0,
            audition: true,
            tour: false,
            tour_index: 0,
            seen_bars: 0,
            pending_hits: Vec::new(),
            dice: Dice(0x1234_5678),
            modes: [PlateModeState::default(); PLATE_MAX_MODES],
            phases: std::array::from_fn(|k| k as f32 * 2.399),
            energy: 0.0,
            cascade_level: 0.0,
            peaks: [0.0; 2],
            current_step: None,
            seen_serials: [0; MAX_STRIKERS],
            flashes: [None; MAX_STRIKERS],
            ripples: Vec::new(),
            hovered_mode: None,
            view_drag: None,
            last_roll: Instant::now(),
            map_axes: [Param::Cascade, Param::Tension],
            map_drag: None,
            map_trail: Vec::new(),
            last_map_audition: Instant::now(),
            spectrogram: ColorImage::new([SPECTROGRAM_COLS, SPECTROGRAM_ROWS], PANEL_BG),
            spectrogram_texture: None,
            last_spectrogram: Instant::now(),
            shape_table,
            toast: None,
        }
    }

    // --- actions ----------------------------------------------------------

    fn hit_striker(&mut self, index: usize) {
        if let Some(striker) = self.scene.strikers.get(index) {
            self.pending_hits.push(Hit {
                position: striker.at.u,
                velocity: striker.velocity,
                striker: Some(index),
            });
        }
    }

    fn tap(&mut self, at: Point, velocity: f32) {
        self.pending_hits.push(Hit {
            position: at.u,
            velocity,
            striker: None,
        });
        self.ripples.push(Ripple {
            at,
            born: Instant::now(),
            color: Color32::WHITE,
        });
    }

    /// A hit to hear an edit, unless the pattern is already playing it.
    fn audition(&mut self) {
        if self.audition && !self.playing {
            self.hit_striker(0);
        }
    }

    fn add_striker(&mut self, at: Point) {
        if self.scene.strikers.len() < MAX_STRIKERS {
            self.scene.strikers.push(Striker {
                at,
                velocity: 0.8,
                pattern: [0.0; STEPS],
            });
        }
    }

    fn remove_striker(&mut self, index: usize) {
        if self.scene.strikers.len() > 1 && index < self.scene.strikers.len() {
            self.scene.strikers.remove(index);
            // Serials are per slot, so re-sync them to avoid stale flashes.
            self.seen_serials = self.rig.lock().unwrap().hit_serials;
        }
    }

    fn recall(&mut self, scene: Scene) {
        let body_changed = Body::for_material(scene.config.material)
            != Body::for_material(self.scene.config.material);
        self.scene = scene;
        if body_changed {
            self.camera = Body::for_material(self.scene.config.material).default_camera();
        }
    }

    fn store_snapshot(&mut self) {
        if self.snapshots.len() < MAX_SNAPSHOTS {
            self.snapshots.push(self.scene.clone());
            self.notify(format!(
                "Stored snapshot {}",
                snapshot_letter(self.snapshots.len() - 1)
            ));
        }
    }

    fn set_material(&mut self, material: PlateMaterial) {
        if self.scene.config.material != material {
            self.scene.config.material = material;
            self.camera = Body::for_material(material).default_camera();
            self.audition();
        }
    }

    fn notify(&mut self, message: String) {
        self.toast = Some((message, Instant::now()));
    }

    /// Push the scene to the audio thread and read back the voice state.
    fn sync(&mut self) {
        let config = self.scene.effective_config();
        let mut rig = self.rig.lock().unwrap();
        rig.apply(&config, [self.scene.pickups[0].u, self.scene.pickups[1].u]);
        rig.striker_count = self.scene.strikers.len();
        for (index, striker) in self.scene.strikers.iter().enumerate() {
            rig.striker_positions[index] = striker.at.u;
            rig.pattern[index] = striker.pattern;
        }
        rig.bpm = self.bpm;
        rig.volume = self.volume;
        rig.set_playing(self.playing);
        for hit in self.pending_hits.drain(..) {
            rig.queue_hit(hit);
        }
        self.modes = rig.left.mode_states();
        self.energy = rig.left.energy();
        self.cascade_level = rig.left.cascade_level();
        self.peaks = rig.peaks;
        self.current_step = rig.last_step;
        let serials = rig.hit_serials;
        let bars = rig.bars;
        drop(rig);

        let now = Instant::now();
        for (index, serial) in serials.iter().enumerate() {
            if *serial != self.seen_serials[index] {
                self.seen_serials[index] = *serial;
                self.flashes[index] = Some(now);
                if let Some(striker) = self.scene.strikers.get(index) {
                    self.ripples.push(Ripple {
                        at: striker.at,
                        born: now,
                        color: STRIKER_COLORS[index],
                    });
                }
            }
        }
        self.ripples
            .retain(|ripple| ripple.born.elapsed() < Duration::from_millis(600));
        if self.ripples.len() > 48 {
            self.ripples.drain(..self.ripples.len() - 48);
        }

        if bars != self.seen_bars {
            self.seen_bars = bars;
            if self.tour && self.playing && !self.snapshots.is_empty() {
                self.tour_index = (self.tour_index + 1) % self.snapshots.len();
                self.recall(self.snapshots[self.tour_index].clone());
            }
        }
    }

    fn advance_phases(&mut self, dt: f32) {
        for (phase, mode) in self.phases.iter_mut().zip(&self.modes) {
            let rate = 0.9 * mode.ratio.max(0.1).sqrt() * self.motion_speed;
            *phase = (*phase + dt * TAU * rate) % TAU;
        }
    }

    /// Surface displacement along u. Shows one mode's shape while it is
    /// hovered in the spectrum, otherwise the live sum of all modes.
    fn displacement_profile(&self) -> [f32; PROFILE_RES] {
        let mut profile = [0.0; PROFILE_RES];
        if let Some(k) = self.hovered_mode {
            let level = amplitude_level(self.modes[k].amplitude).max(0.6);
            let swing = self.phases[k].cos();
            for (value, shape) in profile.iter_mut().zip(&self.shape_table[k]) {
                *value = DISP_MAX * level * shape * swing;
            }
            return profile;
        }
        let levels: [f32; PLATE_MAX_MODES] = std::array::from_fn(|k| {
            if self.modes[k].gain > 0.0 {
                amplitude_level(self.modes[k].amplitude)
            } else {
                0.0
            }
        });
        let loudest = levels.iter().fold(0.0_f32, |max, level| max.max(*level));
        if loudest <= 0.0 {
            return profile;
        }
        let norm = 1.5 * levels.iter().map(|l| l * l).sum::<f32>().sqrt();
        for (k, level) in levels.iter().enumerate() {
            if *level <= 0.0 {
                continue;
            }
            let weight = level * self.phases[k].cos() / norm;
            for (value, shape) in profile.iter_mut().zip(&self.shape_table[k]) {
                *value += weight * shape;
            }
        }
        for value in &mut profile {
            *value = (*value).clamp(-1.0, 1.0) * DISP_MAX * loudest;
        }
        profile
    }

    fn update_spectrogram(&mut self, ctx: &egui::Context) {
        if self.last_spectrogram.elapsed().as_secs_f32() >= 1.0 / SPECTROGRAM_HZ {
            self.last_spectrogram = Instant::now();
            let [width, height] = self.spectrogram.size;
            for row in self.spectrogram.pixels.chunks_mut(width) {
                row.copy_within(1.., 0);
                row[width - 1] = PANEL_BG;
            }
            for mode in &self.modes {
                if mode.gain <= 0.0 || mode.frequency_hz <= 0.0 {
                    continue;
                }
                let level = amplitude_level(mode.amplitude);
                if level <= 0.0 {
                    continue;
                }
                let y = ((1.0 - log_freq_position(mode.frequency_hz)) * (height - 1) as f32)
                    .round()
                    .clamp(0.0, (height - 1) as f32) as usize;
                let color = lerp_color(PANEL_BG, BAND_COLORS[mode.band as usize], level);
                let pixel = &mut self.spectrogram.pixels[y * width + width - 1];
                *pixel = brighter(*pixel, color);
            }
        }
        match &mut self.spectrogram_texture {
            Some(texture) => texture.set(self.spectrogram.clone(), TextureOptions::LINEAR),
            None => {
                self.spectrogram_texture = Some(ctx.load_texture(
                    "plate-spectrogram",
                    self.spectrogram.clone(),
                    TextureOptions::LINEAR,
                ));
            }
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let keys = [
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
            egui::Key::Num6,
        ];
        let (space, digits) = ctx.input(|input| {
            (
                input.key_pressed(egui::Key::Space),
                keys.map(|key| input.key_pressed(key)),
            )
        });
        if space {
            self.playing = !self.playing;
        }
        for (index, pressed) in digits.iter().enumerate() {
            if *pressed {
                self.hit_striker(index);
            }
        }
    }

    // --- panels -----------------------------------------------------------

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("PLATE EXPLORER").strong().size(16.0));
            ui.separator();
            let label = if self.playing { "⏹ Stop" } else { "▶ Play" };
            if ui
                .add(
                    egui::Button::new(RichText::new(label).strong()).min_size(Vec2::new(72.0, 0.0)),
                )
                .on_hover_text("Space")
                .clicked()
            {
                self.playing = !self.playing;
            }
            ui.add(
                egui::DragValue::new(&mut self.bpm)
                    .range(40.0..=220.0)
                    .speed(0.5)
                    .suffix(" bpm"),
            );
            ui.separator();
            ui.label("Volume");
            ui.add(egui::Slider::new(&mut self.volume, 0.0..=1.0).show_value(false));
            ui.separator();
            ui.selectable_value(&mut self.tab, Tab::Material, "Material");
            ui.selectable_value(&mut self.tab, Tab::TimbreMap, "Timbre map");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(&self.audio_status).color(DIM_TEXT).small());
            });
        });
    }

    fn left_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        section_heading(ui, "PRESETS");
        ui.horizontal_wrapped(|ui| {
            for preset in PRESETS {
                if ui.button(preset.name).clicked() {
                    let config = (preset.config)();
                    if Body::for_material(config.material)
                        != Body::for_material(self.scene.config.material)
                    {
                        self.camera = Body::for_material(config.material).default_camera();
                    }
                    self.scene.load_preset(config);
                    self.audition();
                }
            }
        });
        ui.add_space(6.0);
        section_heading(ui, "MATERIAL");
        ui.horizontal_wrapped(|ui| {
            for material in PlateMaterial::ALL {
                let selected = self.scene.config.material == material;
                if ui
                    .selectable_label(selected, material.name())
                    .on_hover_text(material_hint(material))
                    .clicked()
                {
                    self.set_material(material);
                }
            }
        });
        ui.label(
            RichText::new(material_hint(self.scene.config.material))
                .color(DIM_TEXT)
                .small(),
        );
        ui.add_space(4.0);

        let mut audition = false;
        for (heading, params) in PARAM_GROUPS {
            ui.add_space(4.0);
            section_heading(ui, heading);
            for param in params {
                audition |= param_slider(ui, &mut self.scene.config, *param);
            }
        }
        if audition {
            self.audition();
        }

        ui.add_space(6.0);
        section_heading(ui, "SEED");
        ui.horizontal(|ui| {
            ui.label(format!("0x{:08X}", self.scene.config.seed));
            if ui
                .button("🎲 Reroll")
                .on_hover_text("New jitter for the Plate and Sheet mode tails")
                .clicked()
            {
                self.scene.config.seed = self.dice.next_u32();
                self.audition();
            }
        });
        ui.add_space(6.0);
        section_heading(ui, "EXPLORE");
        ui.horizontal(|ui| {
            if ui
                .button("Randomise")
                .on_hover_text("Random material and parameters")
                .clicked()
            {
                let config = random_config(&mut self.dice);
                if Body::for_material(config.material)
                    != Body::for_material(self.scene.config.material)
                {
                    self.camera = Body::for_material(config.material).default_camera();
                }
                self.scene.config = config;
                self.audition();
            }
            if ui
                .button("Mutate")
                .on_hover_text("Nudge every parameter a little")
                .clicked()
            {
                mutate_config(&mut self.scene.config, &mut self.dice);
                self.audition();
            }
            if ui
                .button("Copy as Rust")
                .on_hover_text("Copy a PlateConfig literal for this sound")
                .clicked()
            {
                ui.ctx()
                    .copy_text(config_as_rust(&self.scene.effective_config()));
                self.notify("Copied PlateConfig to the clipboard".into());
            }
        });
        ui.checkbox(&mut self.audition, "Hit on every edit")
            .on_hover_text("Strike striker 1 when a control changes (while stopped)");
        labeled_slider(ui, "Tap velocity", &mut self.tap_velocity, 0.05..=1.0);
        labeled_slider(ui, "Motion speed", &mut self.motion_speed, 0.1..=3.0)
            .on_hover_text("Speed of the slowed-down surface animation");
    }

    fn right_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        section_heading(ui, "STRIKERS");
        let mut remove = None;
        let mut hit = None;
        for (index, striker) in self.scene.strikers.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
                ui.painter()
                    .circle_filled(rect.center(), 8.0, STRIKER_COLORS[index]);
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    format!("{}", index + 1),
                    FontId::proportional(11.0),
                    Color32::BLACK,
                );
                if response
                    .on_hover_text("Hit (or press its number key)")
                    .clicked()
                {
                    hit = Some(index);
                }
                ui.label(
                    RichText::new(format!("pos {:.2}", striker.at.u))
                        .monospace()
                        .color(DIM_TEXT),
                );
                ui.add(
                    egui::Slider::new(&mut striker.velocity, 0.05..=1.0)
                        .show_value(false)
                        .text("vel"),
                );
                if ui.small_button("🗙").on_hover_text("Remove").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = hit {
            self.hit_striker(index);
        }
        if let Some(index) = remove {
            self.remove_striker(index);
        }
        if self.scene.strikers.len() < MAX_STRIKERS && ui.button("+ Add striker").clicked() {
            let angle = self.scene.strikers.len() as f32 * 1.7;
            self.add_striker(Point::new(0.5, angle));
        }

        ui.add_space(8.0);
        section_heading(ui, "PICKUPS");
        for (index, name) in ["L", "R"].iter().enumerate() {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                ui.painter()
                    .rect_filled(rect.shrink(2.0), 3.0, PICKUP_COLORS[index]);
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    *name,
                    FontId::proportional(11.0),
                    Color32::BLACK,
                );
                ui.add(
                    egui::Slider::new(&mut self.scene.pickups[index].u, 0.0..=1.0)
                        .fixed_decimals(2)
                        .text("pos"),
                );
            });
        }
        if ui
            .button("Mono (R = L)")
            .on_hover_text("Move the right pickup onto the left one")
            .clicked()
        {
            self.scene.pickups[1] = self.scene.pickups[0];
        }

        ui.add_space(8.0);
        section_heading(ui, "RESPONSE");
        meter(
            ui,
            "Energy",
            self.energy,
            Color32::from_rgb(255, 170, 80),
            "Drives the tension glide and deplete",
        );
        meter(
            ui,
            "Cascade",
            (self.cascade_level * 2.0).min(1.0),
            Color32::from_rgb(200, 110, 255),
            "Shimmer being pushed into the upper bands",
        );
        meter(ui, "Out L", self.peaks[0], PICKUP_COLORS[0], "Left peak");
        meter(ui, "Out R", self.peaks[1], PICKUP_COLORS[1], "Right peak");
        let f0 = self.modes[0].frequency_hz / self.modes[0].ratio.max(1.0e-3);
        ui.label(
            RichText::new(format!(
                "f0 now {:.1} Hz ({:+.1} cents)",
                f0,
                1_200.0 * (f0 / self.scene.config.pitch_hz.max(1.0)).log2()
            ))
            .monospace()
            .color(DIM_TEXT),
        )
        .on_hover_text("Live fundamental, including the tension glide");

        ui.add_space(8.0);
        section_heading(ui, "SNAPSHOTS");
        let mut recall = None;
        let mut delete = None;
        ui.horizontal_wrapped(|ui| {
            for (index, snapshot) in self.snapshots.iter().enumerate() {
                let text = RichText::new(snapshot_letter(index))
                    .strong()
                    .color(Color32::BLACK);
                let response = ui
                    .add(egui::Button::new(text).fill(SNAPSHOT_COLORS[index]))
                    .on_hover_text(format!(
                        "{}: click to recall, right-click to delete",
                        snapshot.config.material.name()
                    ));
                if response.clicked() {
                    recall = Some(index);
                }
                if response.secondary_clicked() {
                    delete = Some(index);
                }
            }
        });
        if let Some(index) = recall {
            self.tour_index = index;
            self.recall(self.snapshots[index].clone());
            self.audition();
        }
        if let Some(index) = delete {
            self.snapshots.remove(index);
        }
        ui.horizontal(|ui| {
            let can_store = self.snapshots.len() < MAX_SNAPSHOTS;
            if ui
                .add_enabled(can_store, egui::Button::new("+ Store current"))
                .clicked()
            {
                self.store_snapshot();
            }
            ui.checkbox(&mut self.tour, "Tour")
                .on_hover_text("While playing, step to the next snapshot every bar");
        });
    }

    fn bottom_panel(&mut self, ui: &mut egui::Ui) {
        let full = ui.available_rect_before_wrap();
        let split = (full.width() * 0.42).clamp(360.0, 620.0);
        let sequencer_rect = Rect::from_min_size(full.min, Vec2::new(split - 8.0, full.height()));
        let analysis_rect = Rect::from_min_max(Pos2::new(full.min.x + split, full.min.y), full.max);
        ui.allocate_rect(full, Sense::hover());
        self.sequencer(ui, sequencer_rect);
        let spectrum_height = analysis_rect.height() * 0.48;
        let spectrum_rect = Rect::from_min_size(
            analysis_rect.min,
            Vec2::new(analysis_rect.width(), spectrum_height),
        );
        let spectrogram_rect = Rect::from_min_max(
            Pos2::new(analysis_rect.min.x, spectrum_rect.max.y + 8.0),
            analysis_rect.max,
        );
        self.spectrum(ui, spectrum_rect);
        self.modal_spectrogram(ui, spectrogram_rect);
    }

    fn sequencer(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let painter = ui.painter_at(rect);
        painter.text(
            rect.min,
            Align2::LEFT_TOP,
            "PATTERN  ·  click a step to cycle: off, 100%, 60%, 30%",
            FontId::proportional(11.0),
            DIM_TEXT,
        );
        let grid = Rect::from_min_max(rect.min + Vec2::new(30.0, 20.0), rect.max);
        let rows = self.scene.strikers.len().max(1);
        let cell_w = grid.width() / STEPS as f32;
        let cell_h = (grid.height() / rows as f32).min(34.0);
        for (row, striker) in self.scene.strikers.iter_mut().enumerate() {
            let y = grid.min.y + row as f32 * cell_h;
            let label_center = Pos2::new(rect.min.x + 12.0, y + cell_h * 0.5);
            painter.circle_filled(label_center, 8.0, STRIKER_COLORS[row]);
            painter.text(
                label_center,
                Align2::CENTER_CENTER,
                format!("{}", row + 1),
                FontId::proportional(11.0),
                Color32::BLACK,
            );
            for step in 0..STEPS {
                let cell = Rect::from_min_size(
                    Pos2::new(grid.min.x + step as f32 * cell_w, y),
                    Vec2::new(cell_w, cell_h),
                )
                .shrink(2.0);
                let response = ui.interact(cell, ui.id().with(("step", row, step)), Sense::click());
                if response.clicked() {
                    let value = &mut striker.pattern[step];
                    *value = if *value <= 0.0 {
                        1.0
                    } else if *value > 0.8 {
                        0.6
                    } else if *value > 0.45 {
                        0.3
                    } else {
                        0.0
                    };
                }
                let beat = step % 4 == 0;
                let base = if beat {
                    Color32::from_rgb(40, 44, 52)
                } else {
                    Color32::from_rgb(30, 33, 40)
                };
                painter.rect_filled(cell, 3.0, base);
                let velocity = striker.pattern[step];
                if velocity > 0.0 {
                    let fill = Rect::from_min_max(
                        Pos2::new(cell.min.x, cell.max.y - cell.height() * velocity),
                        cell.max,
                    );
                    painter.rect_filled(fill, 3.0, STRIKER_COLORS[row]);
                }
                if self.current_step == Some(step) {
                    painter.rect_stroke(cell, 3.0, Stroke::new(1.5_f32, Color32::WHITE));
                } else if response.hovered() {
                    painter.rect_stroke(cell, 3.0, Stroke::new(1.0_f32, DIM_TEXT));
                }
            }
        }
    }

    fn spectrum(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, PANEL_BG);
        let plot = rect.shrink2(Vec2::new(8.0, 18.0));
        painter.text(
            rect.min + Vec2::new(8.0, 3.0),
            Align2::LEFT_TOP,
            "MODES  ·  height = level, colour = cascade band  ·  hover one to see its shape",
            FontId::proportional(11.0),
            DIM_TEXT,
        );
        for (freq, label) in [(100.0, "100"), (1_000.0, "1k"), (10_000.0, "10k")] {
            let x = plot.min.x + log_freq_position(freq) * plot.width();
            painter.line_segment(
                [Pos2::new(x, plot.min.y), Pos2::new(x, plot.max.y)],
                Stroke::new(1.0_f32, Color32::from_gray(40)),
            );
            painter.text(
                Pos2::new(x + 3.0, plot.max.y + 2.0),
                Align2::LEFT_TOP,
                label,
                FontId::proportional(10.0),
                DIM_TEXT,
            );
        }
        let f0 = self.modes[0].frequency_hz / self.modes[0].ratio.max(1.0e-3);
        for (band, edge) in BAND_EDGES.iter().enumerate() {
            let x = plot.min.x + log_freq_position(f0 * edge) * plot.width();
            if x > plot.min.x && x < plot.max.x {
                painter.line_segment(
                    [Pos2::new(x, plot.min.y), Pos2::new(x, plot.max.y)],
                    Stroke::new(1.0_f32, BAND_COLORS[band + 1].gamma_multiply(0.35)),
                );
            }
        }

        let pointer = ui
            .ctx()
            .pointer_hover_pos()
            .filter(|pos| plot.expand(4.0).contains(*pos));
        let mut nearest: Option<(usize, f32)> = None;
        for (k, mode) in self.modes.iter().enumerate() {
            if mode.gain <= 0.0 || mode.frequency_hz <= 0.0 {
                continue;
            }
            let x = plot.min.x + log_freq_position(mode.frequency_hz) * plot.width();
            if let Some(pointer) = pointer {
                let distance = (pointer.x - x).abs();
                if distance < 6.0 && nearest.is_none_or(|(_, best)| distance < best) {
                    nearest = Some((k, distance));
                }
            }
            let level = amplitude_level(mode.amplitude);
            let color = BAND_COLORS[mode.band as usize];
            painter.line_segment(
                [Pos2::new(x, plot.max.y), Pos2::new(x, plot.min.y)],
                Stroke::new(1.0_f32, color.gamma_multiply(0.12)),
            );
            if level > 0.0 {
                painter.line_segment(
                    [
                        Pos2::new(x, plot.max.y),
                        Pos2::new(x, plot.max.y - level * plot.height()),
                    ],
                    Stroke::new(3.0_f32, color),
                );
            }
        }
        self.hovered_mode = nearest.map(|(k, _)| k);
        if let Some(k) = self.hovered_mode {
            let mode = self.modes[k];
            let x = plot.min.x + log_freq_position(mode.frequency_hz) * plot.width();
            painter.line_segment(
                [Pos2::new(x, plot.min.y), Pos2::new(x, plot.max.y)],
                Stroke::new(1.0_f32, Color32::WHITE),
            );
            let db = 20.0 * (mode.amplitude.max(1.0e-9) / AMP_REF).log10();
            let text = format!(
                "mode {}  {:.0} Hz  ×{:.2}  band {}  {:.0} dB  strike {:+.2}  pickup {:+.2}",
                k + 1,
                mode.frequency_hz,
                mode.ratio,
                mode.band,
                db.max(-99.0),
                mode.strike_weight,
                mode.pickup_weight
            );
            let galley = painter.layout_no_wrap(text, FontId::monospace(11.0), Color32::WHITE);
            let label = Rect::from_min_size(plot.min, galley.size()).expand(3.0);
            painter.rect_filled(label, 3.0, Color32::from_black_alpha(220));
            painter.galley(plot.min, galley, Color32::WHITE);
        }
    }

    fn modal_spectrogram(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, PANEL_BG);
        if let Some(texture) = &self.spectrogram_texture {
            let image_rect = Rect::from_min_max(
                rect.min + Vec2::new(8.0, 20.0),
                rect.max - Vec2::new(8.0, 4.0),
            );
            painter.image(
                texture.id(),
                image_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        painter.text(
            rect.min + Vec2::new(10.0, 4.0),
            Align2::LEFT_TOP,
            "MODE HISTORY  ·  last 10 s, log frequency  ·  glides bend, cascades bloom upward",
            FontId::proportional(11.0),
            DIM_TEXT,
        );
    }

    // --- 3-D material view --------------------------------------------------

    fn material_view(&mut self, ui: &mut egui::Ui) {
        let body = Body::for_material(self.scene.config.material);
        if self.mesh.body != body {
            self.mesh = BodyMesh::new(body);
        }
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, Color32::from_rgb(12, 13, 17));

        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.camera.zoom = (self.camera.zoom * (1.0 + scroll * 0.002)).clamp(0.4, 3.0);
            }
        }

        let profile = self.displacement_profile();
        let displace = |u: f32| sample_profile(&profile, u) * body.swing();
        let camera = self.camera;
        let marker_screen = |at: Point| -> (Pos2, Vec3, Vec3) {
            let (base, normal) = surface_frame(body, at);
            let point = base + normal * displace(at.u);
            (camera.project(point, rect).0, point, normal)
        };

        // Interaction.
        let pointer = response.interact_pointer_pos().or(response.hover_pos());
        let hovered_marker = pointer.and_then(|pos| {
            let mut best: Option<(Marker, f32)> = None;
            let markers = self
                .scene
                .strikers
                .iter()
                .enumerate()
                .map(|(i, s)| (Marker::Striker(i), s.at))
                .chain(
                    self.scene
                        .pickups
                        .iter()
                        .enumerate()
                        .map(|(i, p)| (Marker::Pickup(i), *p)),
                );
            for (marker, at) in markers {
                let distance = marker_screen(at).0.distance(pos);
                if distance < 13.0 && best.is_none_or(|(_, d)| distance < d) {
                    best = Some((marker, distance));
                }
            }
            best.map(|(marker, _)| marker)
        });

        if response.drag_started() {
            let shift = ui.input(|input| input.modifiers.shift);
            self.view_drag = Some(match hovered_marker {
                Some(marker) => ViewDrag::Marker(marker),
                None if shift => ViewDrag::Roll,
                None => ViewDrag::Orbit,
            });
        }
        if response.dragged() {
            match self.view_drag {
                Some(ViewDrag::Marker(marker)) => {
                    if let Some(at) =
                        pointer.and_then(|pos| pick_surface(&self.mesh, &camera, rect, pos))
                    {
                        match marker {
                            Marker::Striker(i) => {
                                if let Some(striker) = self.scene.strikers.get_mut(i) {
                                    striker.at = at;
                                }
                            }
                            Marker::Pickup(i) => self.scene.pickups[i] = at,
                        }
                    }
                }
                Some(ViewDrag::Orbit) => {
                    let delta = response.drag_delta();
                    self.camera.yaw -= delta.x * 0.008;
                    self.camera.pitch = (self.camera.pitch + delta.y * 0.008).clamp(-0.2, 1.5);
                }
                Some(ViewDrag::Roll) if self.last_roll.elapsed() >= ROLL_INTERVAL => {
                    if let Some(at) =
                        pointer.and_then(|pos| pick_surface(&self.mesh, &camera, rect, pos))
                    {
                        self.last_roll = Instant::now();
                        let jitter = self.dice.range(0.75, 1.0);
                        self.tap(at, self.tap_velocity * jitter);
                    }
                }
                Some(ViewDrag::Roll) => {}
                None => {}
            }
        }
        if response.drag_stopped() {
            if let Some(ViewDrag::Marker(Marker::Striker(i))) = self.view_drag {
                if self.audition && !self.playing {
                    self.hit_striker(i);
                }
            }
            self.view_drag = None;
        }
        if response.clicked() {
            match hovered_marker {
                Some(Marker::Striker(i)) => self.hit_striker(i),
                Some(Marker::Pickup(_)) => {}
                None => {
                    if let Some(at) =
                        pointer.and_then(|pos| pick_surface(&self.mesh, &camera, rect, pos))
                    {
                        self.tap(at, self.tap_velocity);
                    }
                }
            }
        }
        if response.double_clicked() && hovered_marker.is_none() {
            if let Some(at) = pointer.and_then(|pos| pick_surface(&self.mesh, &camera, rect, pos)) {
                self.add_striker(at);
            }
        }
        if response.secondary_clicked() {
            if let Some(Marker::Striker(i)) = hovered_marker {
                self.remove_striker(i);
            }
        }

        // Static decorations behind the body.
        for (a, b) in &self.mesh.lines {
            painter.line_segment(
                [camera.project(*a, rect).0, camera.project(*b, rect).0],
                Stroke::new(2.0_f32, Color32::from_gray(70)),
            );
        }

        // Body mesh, depth-sorted back to front.
        let mesh = &self.mesh;
        let light = (camera.toward_viewer() + Vec3::new(-0.35, 0.25, 0.55)).normalized_or(UP);
        let tint = body.tint();
        let count = mesh.rows * mesh.cols;
        let mut positions = Vec::with_capacity(count);
        let mut disps = Vec::with_capacity(count);
        for index in 0..count {
            let d = displace(mesh.points[index].u);
            disps.push(d);
            positions.push(mesh.base[index] + mesh.normals[index] * d);
        }
        let mut out = egui::Mesh::default();
        let mut depths = Vec::with_capacity(count);
        for row in 0..mesh.rows {
            for col in 0..mesh.cols {
                let index = row * mesh.cols + col;
                let (screen, depth) = camera.project(positions[index], rect);
                depths.push(depth);
                let along = positions[(row + 1).min(mesh.rows - 1) * mesh.cols + col]
                    - positions[row.saturating_sub(1) * mesh.cols + col];
                let across = positions[row * mesh.cols + (col + 1).min(mesh.cols - 1)]
                    - positions[row * mesh.cols + col.saturating_sub(1)];
                let normal = along.cross(across).normalized_or(mesh.normals[index]);
                let shade = 0.28 + 0.72 * normal.dot(light).abs();
                let swing = (disps[index] / (DISP_MAX * body.swing())).clamp(-1.0, 1.0);
                out.colored_vertex(screen, surface_color(tint, shade, swing));
            }
        }
        let mut triangles: Vec<([u32; 3], f32)> =
            Vec::with_capacity((mesh.rows - 1) * (mesh.cols - 1) * 2);
        for row in 0..mesh.rows - 1 {
            for col in 0..mesh.cols - 1 {
                let a = (row * mesh.cols + col) as u32;
                let b = a + 1;
                let c = a + mesh.cols as u32;
                let d = c + 1;
                let depth = |indices: [u32; 3]| {
                    indices.iter().map(|i| depths[*i as usize]).sum::<f32>() / 3.0
                };
                triangles.push(([a, c, d], depth([a, c, d])));
                triangles.push(([a, d, b], depth([a, d, b])));
            }
        }
        for (corners, normal, color) in &mesh.decor {
            let base = out.vertices.len() as u32;
            let shade = 0.25 + 0.6 * normal.dot(light).abs();
            let color = Color32::from_rgb(
                (color[0] * shade) as u8,
                (color[1] * shade) as u8,
                (color[2] * shade) as u8,
            );
            let mut depth = 0.0;
            for corner in corners {
                let (screen, z) = camera.project(*corner, rect);
                depth += z / 3.0;
                out.colored_vertex(screen, color);
            }
            triangles.push(([base, base + 1, base + 2], depth));
        }
        triangles.sort_by(|a, b| b.1.total_cmp(&a.1));
        out.indices = triangles.iter().flat_map(|(t, _)| *t).collect();
        painter.add(egui::Shape::mesh(out));

        // Nodal lines of the hovered mode.
        if let Some(k) = self.hovered_mode {
            for u in nodal_positions(k) {
                let mut points = Vec::new();
                let mut previous: Option<Pos2> = None;
                for step in 0..=96 {
                    let theta = step as f32 / 96.0 * TAU;
                    let (screen, _, _) = marker_screen(Point::new(u, theta));
                    if previous.is_some_and(|p| p.distance(screen) > rect.width() * 0.2) {
                        painter.extend(egui::Shape::dashed_line(
                            &points,
                            Stroke::new(1.5_f32, Color32::WHITE),
                            6.0,
                            4.0,
                        ));
                        points.clear();
                    }
                    points.push(screen);
                    previous = Some(screen);
                }
                painter.extend(egui::Shape::dashed_line(
                    &points,
                    Stroke::new(1.5_f32, Color32::WHITE),
                    6.0,
                    4.0,
                ));
            }
        }

        // Hit ripples.
        for ripple in &self.ripples {
            let age = ripple.born.elapsed().as_secs_f32() / 0.6;
            let (center, _, _) = marker_screen(ripple.at);
            painter.circle_stroke(
                center,
                6.0 + 34.0 * age,
                Stroke::new(2.0_f32, ripple.color.gamma_multiply((1.0 - age).max(0.0))),
            );
        }

        // Pickups and strikers.
        for (index, pickup) in self.scene.pickups.iter().enumerate() {
            let (screen, point, normal) = marker_screen(*pickup);
            let top = camera.project(point + normal * 0.16, rect).0;
            painter.line_segment([screen, top], Stroke::new(2.0_f32, PICKUP_COLORS[index]));
            let size = if hovered_marker == Some(Marker::Pickup(index)) {
                9.0
            } else {
                7.0
            };
            painter.rect_filled(
                Rect::from_center_size(top, Vec2::splat(size * 2.0)),
                3.0,
                PICKUP_COLORS[index],
            );
            painter.text(
                top,
                Align2::CENTER_CENTER,
                ["L", "R"][index],
                FontId::proportional(11.0),
                Color32::BLACK,
            );
            painter.circle_filled(screen, 3.0, PICKUP_COLORS[index]);
        }
        for (index, striker) in self.scene.strikers.iter().enumerate() {
            let (screen, point, normal) = marker_screen(striker.at);
            let flash = self.flashes[index]
                .map(|time| 1.0 - (time.elapsed().as_secs_f32() / 0.25).min(1.0))
                .unwrap_or(0.0);
            // The mallet dips towards the surface when it hits.
            let lift = 0.32 - 0.18 * flash;
            let head = camera.project(point + normal * lift, rect).0;
            painter.line_segment(
                [screen, head],
                Stroke::new(1.5_f32, STRIKER_COLORS[index].gamma_multiply(0.7)),
            );
            let hovered = hovered_marker == Some(Marker::Striker(index));
            let radius = if hovered { 10.0 } else { 8.0 } + 3.0 * flash;
            painter.circle(
                head,
                radius,
                STRIKER_COLORS[index],
                Stroke::new(if hovered { 2.0_f32 } else { 1.0 }, Color32::BLACK),
            );
            painter.text(
                head,
                Align2::CENTER_CENTER,
                format!("{}", index + 1),
                FontId::proportional(11.0),
                Color32::BLACK,
            );
            painter.circle_filled(screen, 3.0, STRIKER_COLORS[index]);
        }

        // Labels.
        let material = self.scene.config.material;
        painter.text(
            rect.min + Vec2::new(14.0, 12.0),
            Align2::LEFT_TOP,
            format!(
                "{}  ·  {}",
                material.name().to_uppercase(),
                material_hint(material)
            ),
            FontId::proportional(15.0),
            Color32::WHITE,
        );
        painter.text(
            rect.min + Vec2::new(14.0, 32.0),
            Align2::LEFT_TOP,
            format!("{} (angle is free)", body.position_note()),
            FontId::proportional(12.0),
            DIM_TEXT,
        );
        if let Some(k) = self.hovered_mode {
            painter.text(
                rect.min + Vec2::new(14.0, 50.0),
                Align2::LEFT_TOP,
                format!(
                    "showing mode {} alone ({:.0} Hz); dashed rings are its nodes",
                    k + 1,
                    self.modes[k].frequency_hz
                ),
                FontId::proportional(12.0),
                Color32::from_rgb(255, 220, 140),
            );
        }
        if let Some(marker) = hovered_marker {
            let (name, u) = match marker {
                Marker::Striker(i) => (format!("Striker {}", i + 1), self.scene.strikers[i].at.u),
                Marker::Pickup(i) => (format!("Pickup {}", ["L", "R"][i]), self.scene.pickups[i].u),
            };
            if let Some(pos) = pointer {
                painter.text(
                    pos + Vec2::new(16.0, 10.0),
                    Align2::LEFT_TOP,
                    format!("{name}  ·  position {u:.2}"),
                    FontId::proportional(12.0),
                    Color32::WHITE,
                );
            }
        }
        painter.text(
            Pos2::new(rect.min.x + 14.0, rect.max.y - 10.0),
            Align2::LEFT_BOTTOM,
            "click: tap  ·  shift-drag: roll  ·  drag a point: move it  ·  double-click: add \
             striker  ·  right-click striker: remove  ·  drag: orbit  ·  scroll: zoom  ·  1-6: hit",
            FontId::proportional(11.0),
            DIM_TEXT,
        );
        if ui
            .put(
                Rect::from_min_size(
                    Pos2::new(rect.max.x - 96.0, rect.min.y + 10.0),
                    Vec2::new(84.0, 20.0),
                ),
                egui::Button::new("Reset view"),
            )
            .clicked()
        {
            self.camera = body.default_camera();
        }
    }

    // --- timbre map ---------------------------------------------------------

    fn timbre_map(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("X");
            let before = self.map_axes;
            axis_combo(ui, "map-x", &mut self.map_axes[0]);
            ui.label("Y");
            axis_combo(ui, "map-y", &mut self.map_axes[1]);
            if before != self.map_axes {
                self.map_trail.clear();
            }
            if ui.button("Clear trail").clicked() {
                self.map_trail.clear();
            }
            ui.label(
                RichText::new(
                    "drag anywhere to morph  ·  drag a pin to reshape it  ·  click a pin or \
                     preset to recall",
                )
                .color(DIM_TEXT)
                .small(),
            );
        });
        let available = ui.available_rect_before_wrap();
        let (rect, response) = ui.allocate_exact_size(available.size(), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, Color32::from_rgb(12, 13, 17));
        let plot = rect.shrink2(Vec2::new(70.0, 34.0));
        let [x_param, y_param] = self.map_axes;
        let to_screen = |config: &PlateConfig| {
            Pos2::new(
                plot.min.x + x_param.normalized(config) * plot.width(),
                plot.max.y - y_param.normalized(config) * plot.height(),
            )
        };
        let from_screen = |pos: Pos2| {
            (
                ((pos.x - plot.min.x) / plot.width()).clamp(0.0, 1.0),
                ((plot.max.y - pos.y) / plot.height()).clamp(0.0, 1.0),
            )
        };

        // Grid and axes.
        for i in 0..=4 {
            let t = i as f32 / 4.0;
            let x = plot.min.x + t * plot.width();
            let y = plot.max.y - t * plot.height();
            let stroke = Stroke::new(
                1.0_f32,
                Color32::from_gray(if i % 2 == 0 { 42 } else { 30 }),
            );
            painter.line_segment([Pos2::new(x, plot.min.y), Pos2::new(x, plot.max.y)], stroke);
            painter.line_segment([Pos2::new(plot.min.x, y), Pos2::new(plot.max.x, y)], stroke);
            let mut probe = self.scene.config;
            x_param.set_normalized(&mut probe, t);
            painter.text(
                Pos2::new(x, plot.max.y + 6.0),
                Align2::CENTER_TOP,
                x_param.format(x_param.get(&probe)),
                FontId::proportional(10.0),
                DIM_TEXT,
            );
            y_param.set_normalized(&mut probe, t);
            painter.text(
                Pos2::new(plot.min.x - 6.0, y),
                Align2::RIGHT_CENTER,
                y_param.format(y_param.get(&probe)),
                FontId::proportional(10.0),
                DIM_TEXT,
            );
        }
        painter.text(
            Pos2::new(plot.center().x, rect.max.y - 4.0),
            Align2::CENTER_BOTTOM,
            x_param.label(),
            FontId::proportional(13.0),
            Color32::WHITE,
        );
        painter.text(
            Pos2::new(rect.min.x + 8.0, rect.min.y + 6.0),
            Align2::LEFT_TOP,
            y_param.label(),
            FontId::proportional(13.0),
            Color32::WHITE,
        );

        // Factory presets for orientation.
        let mut preset_spots = Vec::new();
        for preset in PRESETS {
            let config = (preset.config)();
            let spot = to_screen(&config);
            painter.circle_stroke(spot, 5.0, Stroke::new(1.0_f32, Color32::from_gray(110)));
            painter.text(
                spot + Vec2::new(8.0, 0.0),
                Align2::LEFT_CENTER,
                preset.name,
                FontId::proportional(11.0),
                Color32::from_gray(130),
            );
            preset_spots.push((spot, config));
        }

        // Trail of where the cursor has been, in normalised coordinates.
        if self.map_trail.len() > 1 {
            let points: Vec<Pos2> = self
                .map_trail
                .iter()
                .map(|p| {
                    Pos2::new(
                        plot.min.x + p.x * plot.width(),
                        plot.max.y - p.y * plot.height(),
                    )
                })
                .collect();
            painter.add(egui::Shape::line(
                points,
                Stroke::new(1.5_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 50)),
            ));
        }

        let pointer = response.interact_pointer_pos().or(response.hover_pos());
        let near_snapshot = pointer.and_then(|pos| {
            self.snapshots
                .iter()
                .enumerate()
                .map(|(i, scene)| (i, to_screen(&scene.config).distance(pos)))
                .filter(|(_, d)| *d < 14.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i)
        });
        let near_preset = pointer.and_then(|pos| {
            preset_spots
                .iter()
                .find(|(spot, _)| spot.distance(pos) < 8.0)
                .map(|(_, config)| *config)
        });

        if response.drag_started() {
            self.map_drag = Some(match near_snapshot {
                Some(index) => {
                    self.recall(self.snapshots[index].clone());
                    MapDrag::Snapshot(index)
                }
                None => MapDrag::Cursor,
            });
        }
        let mut moved = false;
        if response.dragged()
            || (response.clicked() && near_snapshot.is_none() && near_preset.is_none())
        {
            if let Some(pos) = pointer {
                let (tx, ty) = from_screen(pos);
                x_param.set_normalized(&mut self.scene.config, tx);
                y_param.set_normalized(&mut self.scene.config, ty);
                if let Some(MapDrag::Snapshot(index)) = self.map_drag {
                    let snapshot = &mut self.snapshots[index].config;
                    x_param.set_normalized(snapshot, tx);
                    y_param.set_normalized(snapshot, ty);
                }
                self.map_trail.push(Pos2::new(tx, ty));
                if self.map_trail.len() > 600 {
                    self.map_trail.remove(0);
                }
                moved = true;
            }
        }
        if response.clicked() {
            if let Some(index) = near_snapshot {
                self.tour_index = index;
                self.recall(self.snapshots[index].clone());
            } else if let Some(config) = near_preset {
                self.scene.load_preset(config);
            }
            self.audition();
        } else if moved
            && self.audition
            && !self.playing
            && self.last_map_audition.elapsed() >= MAP_AUDITION_INTERVAL
        {
            self.last_map_audition = Instant::now();
            self.hit_striker(0);
        }
        if response.drag_stopped() {
            self.map_drag = None;
        }

        // Snapshot pins.
        for (index, scene) in self.snapshots.iter().enumerate() {
            let spot = to_screen(&scene.config);
            let hovered = near_snapshot == Some(index);
            painter.circle(
                spot,
                if hovered { 12.0 } else { 10.0 },
                SNAPSHOT_COLORS[index],
                Stroke::new(1.0_f32, Color32::BLACK),
            );
            painter.text(
                spot,
                Align2::CENTER_CENTER,
                snapshot_letter(index),
                FontId::proportional(12.0),
                Color32::BLACK,
            );
        }

        // Live cursor.
        let cursor = to_screen(&self.scene.config);
        let pulse = 1.0 + 0.4 * self.energy.min(1.0);
        painter.circle_stroke(cursor, 14.0 * pulse, Stroke::new(2.0_f32, Color32::WHITE));
        painter.circle_filled(cursor, 3.0, Color32::WHITE);
        painter.text(
            cursor + Vec2::new(18.0, -16.0),
            Align2::LEFT_BOTTOM,
            format!(
                "{} · {}",
                x_param.format(x_param.get(&self.scene.config)),
                y_param.format(y_param.get(&self.scene.config))
            ),
            FontId::monospace(11.0),
            Color32::WHITE,
        );
    }
}

impl eframe::App for Explorer {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dt = ctx.input(|input| input.stable_dt).min(0.1);
        self.handle_keys(ctx);
        self.sync();
        self.advance_phases(dt);
        self.update_spectrogram(ctx);

        egui::TopBottomPanel::top("top")
            .frame(egui::Frame::side_top_panel(&ctx.style()).inner_margin(8.0))
            .show(ctx, |ui| self.top_bar(ui));
        egui::SidePanel::left("material")
            .exact_width(300.0)
            .resizable(false)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.left_panel(ui));
            });
        egui::SidePanel::right("scene")
            .exact_width(300.0)
            .resizable(false)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.right_panel(ui));
            });
        egui::TopBottomPanel::bottom("analysis")
            .exact_height(250.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_space(6.0);
                self.bottom_panel(ui);
            });
        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Material => self.material_view(ui),
            Tab::TimbreMap => self.timbre_map(ui),
        });

        if let Some((message, time)) = &self.toast {
            if time.elapsed() < Duration::from_secs(2) {
                egui::Area::new(egui::Id::new("toast"))
                    .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 56.0))
                    .show(ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| ui.label(message.as_str()));
                    });
            } else {
                self.toast = None;
            }
        }
        ctx.request_repaint();
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn snapshot_letter(index: usize) -> String {
    char::from(b'A' + index as u8).to_string()
}

/// 0-1 position of a frequency on a 20 Hz to 20 kHz log axis.
fn log_freq_position(freq: f32) -> f32 {
    ((freq.max(20.0) / 20.0).log10() / 3.0).clamp(0.0, 1.0)
}

fn sample_profile(profile: &[f32; PROFILE_RES], u: f32) -> f32 {
    let x = u.clamp(0.0, 1.0) * (PROFILE_RES - 1) as f32;
    let i = (x as usize).min(PROFILE_RES - 2);
    let frac = x - i as f32;
    profile[i] + (profile[i + 1] - profile[i]) * frac
}

/// Positions in 0-1 where `plate_mode_shape(mode, u)` crosses zero.
fn nodal_positions(mode: usize) -> Vec<f32> {
    let n = (mode + 1) as f32;
    // p = 0.5 - 0.47 u runs from 0.5 (centre) to 0.03 (edge); nodes sit at p = j / n.
    (1..=mode + 1)
        .map(|j| (0.5 - j as f32 / n) / 0.47)
        .filter(|u| (0.0..=1.0).contains(u))
        .collect()
}

/// Nearest point on the (undisplaced) surface under the pointer, refined
/// around the closest mesh vertex. Ties prefer the side facing the camera.
fn pick_surface(mesh: &BodyMesh, camera: &Camera, rect: Rect, pointer: Pos2) -> Option<Point> {
    let mut candidates: Vec<(usize, f32, f32)> = mesh
        .base
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (screen, depth) = camera.project(*p, rect);
            (i, screen.distance(pointer), depth)
        })
        .collect();
    let closest = candidates.iter().map(|c| c.1).fold(f32::INFINITY, f32::min);
    if closest > 40.0 {
        return None;
    }
    candidates.retain(|c| c.1 < closest + 6.0);
    let (index, _, _) = *candidates.iter().min_by(|a, b| a.2.total_cmp(&b.2))?;
    let mut best = mesh.points[index];
    let mut best_distance = camera
        .project(surface_frame(mesh.body, best).0, rect)
        .0
        .distance(pointer);
    let (mut du, mut dt) = match mesh.body {
        Body::Bar => (1.0 / (mesh.rows - 1) as f32, 0.25),
        _ => (1.0 / (mesh.rows - 1) as f32, TAU / (mesh.cols - 1) as f32),
    };
    for _ in 0..4 {
        let center = best;
        for i in -2..=2 {
            for j in -2..=2 {
                let candidate = Point::new(
                    (center.u + i as f32 * du * 0.5).clamp(0.0, 1.0),
                    center.theta + j as f32 * dt * 0.5,
                );
                let distance = camera
                    .project(surface_frame(mesh.body, candidate).0, rect)
                    .0
                    .distance(pointer);
                if distance < best_distance {
                    best = candidate;
                    best_distance = distance;
                }
            }
        }
        du *= 0.5;
        dt *= 0.5;
    }
    Some(best)
}

fn surface_color(tint: [f32; 3], shade: f32, swing: f32) -> Color32 {
    // Crests glow warm, troughs cool, so the mode shape reads at a glance.
    let accent = if swing >= 0.0 {
        [255.0, 150.0, 60.0]
    } else {
        [70.0, 160.0, 255.0]
    };
    let t = 0.85 * swing.abs();
    let mix = |i: usize| {
        let base = tint[i] * shade;
        (base + (accent[i] * (0.45 + 0.55 * shade) - base) * t).clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(mix(0), mix(1), mix(2))
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)) as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

fn brighter(a: Color32, b: Color32) -> Color32 {
    Color32::from_rgb(a.r().max(b.r()), a.g().max(b.g()), a.b().max(b.b()))
}

fn section_heading(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).small().strong().color(DIM_TEXT));
}

/// A normalised slider with the value in the parameter's units. Returns true
/// when an edit has finished (to audition it).
fn param_slider(ui: &mut egui::Ui, config: &mut PlateConfig, param: Param) -> bool {
    let mut t = param.normalized(config);
    let mut finished = false;
    ui.horizontal(|ui| {
        ui.add_sized(Vec2::new(78.0, 18.0), egui::Label::new(param.label()))
            .on_hover_text(param.hint());
        let response = ui
            .add(egui::Slider::new(&mut t, 0.0..=1.0).show_value(false))
            .on_hover_text(param.hint());
        if response.changed() {
            param.set_normalized(config, t);
        }
        finished = response.drag_stopped() || (response.changed() && !response.dragged());
        ui.label(
            RichText::new(param.format(param.get(config)))
                .monospace()
                .size(11.0),
        );
    });
    finished
}

fn labeled_slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) -> egui::Response {
    ui.horizontal(|ui| {
        ui.add_sized(Vec2::new(78.0, 18.0), egui::Label::new(label));
        ui.add(egui::Slider::new(value, range).fixed_decimals(2))
    })
    .inner
}

fn meter(ui: &mut egui::Ui, label: &str, value: f32, color: Color32, hint: &str) {
    ui.horizontal(|ui| {
        ui.add_sized(
            Vec2::new(56.0, 16.0),
            egui::Label::new(RichText::new(label).small()),
        );
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width() - 8.0, 10.0), Sense::hover());
        ui.painter().rect_filled(rect, 3.0, Color32::from_gray(30));
        let fill = Rect::from_min_size(
            rect.min,
            Vec2::new(rect.width() * value.clamp(0.0, 1.0), rect.height()),
        );
        ui.painter().rect_filled(fill, 3.0, color);
        response.on_hover_text(hint);
    });
}

fn axis_combo(ui: &mut egui::Ui, id: &str, axis: &mut Param) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(axis.label())
        .show_ui(ui, |ui| {
            for param in Param::ALL {
                ui.selectable_value(axis, param, param.label());
            }
        });
}

fn main() -> anyhow::Result<()> {
    let scene = Scene::starter();
    let rig = Arc::new(Mutex::new(PlateRig::new(
        SAMPLE_RATE,
        scene.effective_config(),
    )));
    let mut engine = Engine::new(SAMPLE_RATE);
    engine.set_master_gain(1.0);
    engine.add_instrument("plate", Box::new(SharedRig(Arc::clone(&rig))));
    let engine = Arc::new(Mutex::new(engine));

    let mut output = EngineOutput::new();
    let started = output
        .initialize(SAMPLE_RATE)
        .and_then(|_| output.create_stream_with_engine(Arc::clone(&engine)))
        .and_then(|_| output.start());
    let (output, audio_status) = match started {
        Ok(()) => (
            Some(output),
            format!("audio {:.1} kHz", SAMPLE_RATE / 1_000.0),
        ),
        Err(error) => {
            eprintln!("No audio output ({error}); running silently.");
            spawn_silent_clock(Arc::clone(&engine));
            (None, "no audio device: running silently".to_string())
        }
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1_480.0, 940.0])
            .with_min_inner_size([1_180.0, 760.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Plate Explorer",
        options,
        Box::new(move |context| {
            context.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(Explorer::new(rig, engine, output, audio_status)))
        }),
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rig() -> PlateRig {
        PlateRig::new(SAMPLE_RATE, PlateConfig::ride())
    }

    #[test]
    fn params_round_trip_through_normalised_values() {
        let mut config = PlateConfig::crash();
        for param in Param::ALL {
            for t in [0.0, 0.25, 0.5, 0.9, 1.0] {
                param.set_normalized(&mut config, t);
                assert!(
                    (param.normalized(&config) - t).abs() < 1.0e-4,
                    "{param:?} at {t}"
                );
            }
        }
    }

    #[test]
    fn surface_points_keep_the_material_position() {
        for body in [Body::Drum, Body::Cymbal, Body::Bell, Body::Sheet] {
            let mesh = BodyMesh::new(body);
            assert_eq!(mesh.points.len(), mesh.rows * mesh.cols);
            assert!(mesh.base.iter().all(|p| p.length().is_finite()));
        }
        // The bar folds its two halves onto the same 0-1 position.
        for (x, s) in [(0.6, 0.0), (-0.6, 0.5), (-0.2, -0.9)] {
            let point = bar_point(x, s);
            assert!((point.u - x.abs()).abs() < 1.0e-6);
            let (p, _) = surface_frame(Body::Bar, point);
            assert!((p.x - x * BAR_HALF_LENGTH).abs() < 1.0e-4, "x {x} s {s}");
            assert!((p.y - s * BAR_HALF_WIDTH).abs() < 1.0e-4);
        }
    }

    #[test]
    fn nodal_positions_are_zeros_of_the_mode_shape() {
        for mode in 0..PLATE_MAX_MODES {
            for u in nodal_positions(mode) {
                assert!(
                    plate_mode_shape(mode, u).abs() < 1.0e-3,
                    "mode {mode} u {u}"
                );
            }
        }
        // Even-numbered shapes have a node at the centre.
        assert!(nodal_positions(1).iter().any(|u| u.abs() < 1.0e-6));
    }

    #[test]
    fn simultaneous_hits_flam_loudest_first() {
        let mut rig = rig();
        rig.striker_count = 2;
        rig.queue_hit(Hit {
            position: 0.2,
            velocity: 0.3,
            striker: Some(0),
        });
        rig.queue_hit(Hit {
            position: 0.8,
            velocity: 0.9,
            striker: Some(1),
        });
        rig.render(0.0);
        assert_eq!(rig.hit_serials, [0, 1, 0, 0, 0, 0]);
        for i in 1..FLAM_SAMPLES {
            rig.render(i as f64 / SAMPLE_RATE as f64);
            assert_eq!(rig.hit_serials[0], 0);
        }
        rig.render(FLAM_SAMPLES as f64 / SAMPLE_RATE as f64);
        rig.render((FLAM_SAMPLES + 1) as f64 / SAMPLE_RATE as f64);
        assert_eq!(rig.hit_serials[0], 1);
    }

    #[test]
    fn sequencer_plays_steps_on_the_sixteenth_grid() {
        let mut rig = rig();
        rig.bpm = 120.0;
        rig.striker_count = 1;
        rig.pattern[0][0] = 1.0;
        rig.pattern[0][4] = 1.0;
        rig.set_playing(true);
        let step = (SAMPLE_RATE * 15.0 / 120.0) as usize;
        let mut hits = Vec::new();
        for i in 0..step * 16 {
            let before = rig.hit_serials[0];
            rig.render(i as f64 / SAMPLE_RATE as f64);
            if rig.hit_serials[0] != before {
                hits.push(i);
            }
        }
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0], 0);
        assert!((hits[1] as i64 - 4 * step as i64).abs() <= 1);
        assert_eq!(rig.bars, 1);
    }

    #[test]
    fn left_and_right_banks_ring_identically() {
        let mut rig = rig();
        rig.apply(&PlateConfig::crash(), [0.2, 0.2]);
        rig.queue_hit(Hit {
            position: 0.4,
            velocity: 1.0,
            striker: None,
        });
        for i in 0..4_800 {
            let frame = rig.render(i as f64 / SAMPLE_RATE as f64);
            assert_eq!(frame.l, frame.r);
        }
        rig.apply(&PlateConfig::crash(), [0.2, 0.9]);
        let mut differs = false;
        for i in 4_800..9_600 {
            let frame = rig.render(i as f64 / SAMPLE_RATE as f64);
            differs |= frame.l != frame.r;
        }
        assert!(differs, "moving one pickup should change only that channel");
    }

    #[test]
    fn displacement_follows_the_voice() {
        let rig = Arc::new(Mutex::new(rig()));
        let engine = Arc::new(Mutex::new(Engine::new(SAMPLE_RATE)));
        let mut explorer = Explorer::new(Arc::clone(&rig), engine, None, String::new());
        explorer.sync();
        assert!(explorer.displacement_profile().iter().all(|d| *d == 0.0));
        explorer.hit_striker(0);
        explorer.sync();
        {
            let mut rig = rig.lock().unwrap();
            for i in 0..2_000 {
                rig.render(i as f64 / SAMPLE_RATE as f64);
            }
        }
        explorer.sync();
        let profile = explorer.displacement_profile();
        assert!(profile.iter().any(|d| d.abs() > 1.0e-3));
        assert!(profile.iter().all(|d| d.abs() <= DISP_MAX + 1.0e-6));
    }

    #[test]
    fn rust_export_names_the_material() {
        let text = config_as_rust(&PlateConfig::gong());
        assert!(text.contains("material: PlateMaterial::Sheet"));
        assert!(text.contains("pitch_hz: 70.0"));
    }
}
