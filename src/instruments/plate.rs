//! A modal plate / membrane percussion voice.
//!
//! A bank of pitch-tracked resonant modes is struck by a short exciter. Mode
//! ratios come from one of five material tables (membrane, free plate, bar,
//! bell, metal sheet). Two cheap nonlinearities make the voice respond to how
//! hard it is hit:
//!
//! - **Tension glide**: a short-time estimate of the total modal energy raises
//!   every mode frequency, so hard hits start sharp and glide down as they
//!   decay (after Marogna, Avanzini and Bank, DAFx 2010).
//! - **Mode cascade**: the modes are grouped into four bands relative to the
//!   fundamental. Each band passes through a bounded cubic and the result
//!   drives the modes one band up, approximating the von Karman energy
//!   cascade that makes a crash or gong shimmer and bloom after the strike.
//!
//! Mode frequencies and decay times are recomputed at control rate (every
//! [`CONTROL_BLOCK`] samples) because `Resonator::set_frequency` costs a
//! `tan()`. Everything is fixed-size, so triggering and ticking never allocate.

use std::f32::consts::{PI, TAU};

use crate::engine::{Instrument, Modulatable};
use crate::filters::Resonator;
use crate::gen::{Exciter, ExciterKind};
use crate::utils::{tuning_to_multiplier, Oversampler2x, SmoothedParam, XorShift32};

/// Number of modes in the bank. The `plate-hq` feature doubles it.
pub const PLATE_MAX_MODES: usize = if cfg!(feature = "plate-hq") { 64 } else { 32 };
/// Number of indexed parameters, matching the future `PLATE_PARAM_*` 0-11.
pub const PLATE_PARAM_COUNT: usize = 12;
/// Index of the discrete material parameter.
pub const PLATE_PARAM_MATERIAL_INDEX: usize = 11;

const SMOOTH_MS: f32 = 15.0;
/// Samples between mode frequency and decay updates.
const CONTROL_BLOCK: u32 = 32;
/// Samples between end-of-ring checks.
const QUIET_CHECK_BLOCK: u32 = 64;
/// Upper edges of the cascade bands, in multiples of f0. Band 3 is open-ended.
const BAND_EDGES: [f32; 3] = [3.0, 8.0, 20.0];
const CASCADE_BANDS: usize = 4;
/// Pre-gain into the cascade cubic. Quiet bands stay nearly linear (so the
/// cubic term is negligible); loud bands saturate.
const CASCADE_DRIVE: f32 = 2.0;
/// Scale of the cubic products injected into the next band's modes.
const CASCADE_INJECT: f32 = 0.02;
/// Scale of the noise injected into the next band's modes, enveloped by the
/// cubic products. High-Q modes ignore products that miss their frequency,
/// so this broadband part is what lets the upper bands bloom.
const CASCADE_NOISE: f32 = 3.0;
/// Envelope follower time constant for the noise injection.
const CASCADE_ENV_MS: f32 = 5.0;
/// Extra damping of lower bands as they feed upper ones, at full energy.
const DEPLETE_RATE: f32 = 3.0;
/// Energy follower time constant.
const ENERGY_MS: f32 = 10.0;
/// Modal energy that maps to E = 1 (calibrated so a full-velocity ride peaks
/// near 1).
const ENERGY_REF: f32 = 1.2;
/// Mode-sum gain before the brightness filter and soft clip.
const OUTPUT_GAIN: f32 = 0.35;
/// Peak exciter amplitude at full velocity.
const EXCITER_AMPLITUDE: f32 = 0.6;
/// Fades modes out between these fractions of the sample rate.
const NYQUIST_FADE_START: f32 = 0.40;
const NYQUIST_FADE_END: f32 = 0.45;

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

/// The mode-ratio table the plate rings with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlateMaterial {
    /// Ideal circular membrane (Bessel zeros): toms, timbales, tabla-like.
    Membrane,
    /// Free circular plate with a dense inharmonic tail: cymbals.
    Plate,
    /// Free-free bar: marimba, glockenspiel, metal bars.
    Bar,
    /// Church bell partials: hum, prime, tierce, quint, nominal and up.
    Bell,
    /// Dense, almost harmonic metal sheet: gongs and thunder sheets.
    Sheet,
}

/// Circular membrane mode ratios j(m,n) / j(0,1), sorted.
const MEMBRANE_RATIOS: [f32; 32] = [
    1.000, 1.593, 2.136, 2.295, 2.653, 2.917, 3.155, 3.500, 3.598, 3.647, 4.059, 4.132, 4.230,
    4.601, 4.610, 4.832, 4.903, 5.084, 5.131, 5.412, 5.540, 5.553, 5.651, 5.977, 6.019, 6.153,
    6.163, 6.209, 6.483, 6.529, 6.669, 6.746,
];
/// Free circular plate (Fletcher and Rossing), before the dense tail.
const PLATE_RATIOS: [f32; 12] = [
    1.0, 1.73, 2.33, 3.91, 4.11, 6.30, 6.92, 7.36, 8.95, 10.2, 11.8, 12.1,
];
/// Free-free bar, then ((2n + 1) / 3.011)^2.
const BAR_RATIOS: [f32; 4] = [1.0, 2.756, 5.404, 8.933];
/// Church bell partials relative to the prime.
const BELL_RATIOS: [f32; 20] = [
    0.5, 1.0, 1.19, 1.5, 2.0, 2.51, 2.66, 3.01, 3.37, 4.0, 4.24, 4.52, 5.0, 5.34, 5.95, 6.32, 6.72,
    7.13, 7.55, 8.0,
];

impl PlateMaterial {
    pub const ALL: [Self; 5] = [
        Self::Membrane,
        Self::Plate,
        Self::Bar,
        Self::Bell,
        Self::Sheet,
    ];

    pub fn index(self) -> usize {
        match self {
            Self::Membrane => 0,
            Self::Plate => 1,
            Self::Bar => 2,
            Self::Bell => 3,
            Self::Sheet => 4,
        }
    }

    /// Clamp and map a 0-4 index to a material.
    pub fn from_index(index: usize) -> Self {
        Self::ALL[index.min(Self::ALL.len() - 1)]
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Membrane => "Membrane",
            Self::Plate => "Plate",
            Self::Bar => "Bar",
            Self::Bell => "Bell",
            Self::Sheet => "Sheet",
        }
    }

    /// Mode frequency ratios relative to f0, strictly increasing. `seed`
    /// jitters the statistical tails of Plate and Sheet.
    pub fn ratios(self, seed: u32) -> [f32; PLATE_MAX_MODES] {
        let mut rng = XorShift32::new(seed ^ 0x5EED_0D35);
        let mut out = [1.0; PLATE_MAX_MODES];
        for (k, ratio) in out.iter_mut().enumerate() {
            let n = (k + 1) as f32;
            *ratio = match self {
                Self::Membrane => MEMBRANE_RATIOS
                    .get(k)
                    .copied()
                    .unwrap_or_else(|| MEMBRANE_RATIOS[31] + 0.2 * (n - 32.0)),
                Self::Plate => PLATE_RATIOS.get(k).copied().unwrap_or_else(|| {
                    let jitter = 1.0 + 0.03 * rng.next_bipolar();
                    PLATE_RATIOS[11] * (n / 12.0).powf(1.12) * jitter
                }),
                Self::Bar => BAR_RATIOS
                    .get(k)
                    .copied()
                    .unwrap_or_else(|| ((2.0 * n + 1.0) / 3.011).powi(2)),
                Self::Bell => BELL_RATIOS
                    .get(k)
                    .copied()
                    .unwrap_or_else(|| BELL_RATIOS[19] * 1.06_f32.powf(n - 20.0)),
                Self::Sheet => {
                    if k == 0 {
                        1.0
                    } else {
                        n.powf(0.9) * (1.0 + 0.04 * rng.next_bipolar())
                    }
                }
            };
        }
        // Jitter can reorder neighbours; keep the table strictly increasing.
        for k in 1..PLATE_MAX_MODES {
            if out[k] <= out[k - 1] {
                out[k] = out[k - 1] * 1.002;
            }
        }
        out
    }
}

/// Static settings for a [`PlateVoice`]. Units are physical, not normalized.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlateConfig {
    /// Fundamental in Hz (30-2000).
    pub pitch_hz: f32,
    /// Ratio stretch exponent: ratios become `r^(1 + geometry)` (-0.5..0.5).
    pub geometry: f32,
    /// 0 = centre, 1 = edge. Latched on trigger.
    pub strike_pos: f32,
    /// 0 = centre, 1 = edge.
    pub pickup_pos: f32,
    /// 0 = soft mallet (8 ms pulse) to 1 = hard stick (0.25 ms noise burst).
    pub hardness: f32,
    /// Fundamental T60 in seconds (0.05-20).
    pub decay_seconds: f32,
    /// How much faster high modes die (0-8).
    pub hf_damping: f32,
    /// Pitch glide depth (0 = linear, 1 = about an octave at full energy).
    pub tension: f32,
    /// Cubic cascade gain (0-1).
    pub cascade: f32,
    /// Extra loss of lower bands while they feed upper ones (0-1).
    pub deplete: f32,
    /// Output low-pass cutoff in Hz (500-20000).
    pub brightness_hz: f32,
    pub material: PlateMaterial,
    pub seed: u32,
}

impl Default for PlateConfig {
    fn default() -> Self {
        Self::ride()
    }
}

impl PlateConfig {
    pub fn ride() -> Self {
        Self {
            pitch_hz: 380.0,
            geometry: 0.1,
            strike_pos: 0.35,
            pickup_pos: 0.6,
            hardness: 0.55,
            decay_seconds: 5.0,
            hf_damping: 0.5,
            tension: 0.08,
            cascade: 0.5,
            deplete: 0.2,
            brightness_hz: 12_000.0,
            material: PlateMaterial::Plate,
            seed: 0x0052_1DE0,
        }
    }

    pub fn crash() -> Self {
        Self {
            pitch_hz: 260.0,
            geometry: 0.25,
            strike_pos: 0.75,
            pickup_pos: 0.7,
            hardness: 0.85,
            decay_seconds: 3.5,
            hf_damping: 0.3,
            tension: 0.15,
            cascade: 0.85,
            deplete: 0.5,
            brightness_hz: 16_000.0,
            material: PlateMaterial::Plate,
            seed: 0x00C2_A54E,
        }
    }

    pub fn bell() -> Self {
        Self {
            pitch_hz: 600.0,
            geometry: 0.0,
            strike_pos: 0.2,
            pickup_pos: 0.3,
            hardness: 0.4,
            decay_seconds: 7.0,
            hf_damping: 0.3,
            tension: 0.0,
            cascade: 0.1,
            deplete: 0.0,
            brightness_hz: 14_000.0,
            material: PlateMaterial::Bell,
            seed: 0x00BE_11ED,
        }
    }

    pub fn timbale() -> Self {
        Self {
            pitch_hz: 330.0,
            geometry: 0.0,
            strike_pos: 0.65,
            pickup_pos: 0.4,
            hardness: 0.6,
            decay_seconds: 0.6,
            hf_damping: 1.5,
            tension: 0.2,
            cascade: 0.15,
            deplete: 0.1,
            brightness_hz: 9_000.0,
            material: PlateMaterial::Membrane,
            seed: 0x0071_BA1E,
        }
    }

    pub fn gong() -> Self {
        Self {
            pitch_hz: 70.0,
            geometry: 0.15,
            strike_pos: 0.25,
            pickup_pos: 0.5,
            hardness: 0.1,
            decay_seconds: 14.0,
            hf_damping: 0.8,
            tension: 0.35,
            cascade: 0.55,
            deplete: 0.4,
            brightness_hz: 8_000.0,
            material: PlateMaterial::Sheet,
            seed: 0x0060_0606,
        }
    }
}

/// Weight of each mode at a 0 (centre) to 1 (edge) position, using the 1-D
/// stand-in `sin(pi * p * (k + 1))`, normalised to unit RMS over the bank.
///
/// Centre maps to p = 0.5, which silences every even-numbered shape for a
/// hollow centre hit. The edge maps to p = 0.03 rather than 0, which would
/// silence every mode.
fn position_weights(pos: f32) -> [f32; PLATE_MAX_MODES] {
    let p = 0.5 - 0.47 * pos.clamp(0.0, 1.0);
    let mut weights: [f32; PLATE_MAX_MODES] =
        std::array::from_fn(|k| (PI * p * (k + 1) as f32).sin());
    let mean_square = weights.iter().map(|w| w * w).sum::<f32>() / PLATE_MAX_MODES as f32;
    let norm = 1.0 / mean_square.sqrt().max(1.0e-3);
    for w in &mut weights {
        *w *= norm;
    }
    weights
}

/// Bounded, C1-smooth cubic: `u - u^3 / 3` saturating at +-2/3, rescaled to
/// +-1 and cubed so quiet bands contribute almost nothing.
#[inline]
fn cascade_cubic(x: f32) -> f32 {
    let u = x.clamp(-1.0, 1.0);
    let y = 1.5 * (u - u * u * u / 3.0);
    y * y * y
}

pub struct PlateVoice {
    sample_rate: f32,
    modes: [Resonator; PLATE_MAX_MODES],
    /// Material table, latched on trigger.
    ratios: [f32; PLATE_MAX_MODES],
    /// Strike weights, latched on trigger.
    w_strike: [f32; PLATE_MAX_MODES],
    /// Pickup weights, refreshed at control rate.
    w_pickup: [f32; PLATE_MAX_MODES],
    /// Output gain per mode: roll-off times the Nyquist fade.
    mode_gain: [f32; PLATE_MAX_MODES],
    /// `1 / omega` so a unit-area input gives roughly unit mode amplitude.
    input_norm: [f32; PLATE_MAX_MODES],
    /// `sqrt(4 zeta omega)` so unit white noise gives roughly unit mode RMS,
    /// whatever the mode's decay time.
    noise_norm: [f32; PLATE_MAX_MODES],
    mode_band: [u8; PLATE_MAX_MODES],
    mode_enabled: [bool; PLATE_MAX_MODES],
    exciter: Exciter,
    /// Scales the exciter so its area is close to its amplitude.
    exciter_norm: f32,
    params: [SmoothedParam; PLATE_PARAM_COUNT - 1],
    /// Channel pitch offset (+-12 st); independent of the pitch parameter.
    channel_tuning: SmoothedParam,
    pending_material: PlateMaterial,
    active_material: PlateMaterial,
    latched_seed: u32,
    energy: f32,
    energy_coeff: f32,
    cascade_os: [Oversampler2x; CASCADE_BANDS - 1],
    /// Last cubic products, injected into the next band on the next sample.
    cascade_drive: [f32; CASCADE_BANDS - 1],
    /// Last enveloped noise, injected into the next band on the next sample.
    cascade_noise: [f32; CASCADE_BANDS - 1],
    cascade_env: [f32; CASCADE_BANDS - 1],
    cascade_env_coeff: f32,
    noise_rng: XorShift32,
    lp_state: f32,
    lp_coeff: f32,
    control_counter: u32,
    quiet_counter: u32,
    seed: u32,
    hit_counter: u32,
    velocity: f32,
    trigger_time: f64,
    active: bool,
    midi_note: Option<u8>,
    ring_limit_secs: Option<f32>,
}

impl PlateVoice {
    pub fn new(sample_rate: f32) -> Self {
        Self::with_config(sample_rate, PlateConfig::default())
    }

    pub fn with_config(sample_rate: f32, config: PlateConfig) -> Self {
        let sr = sample_rate.max(1.0);
        let mut voice = Self {
            sample_rate: sr,
            modes: std::array::from_fn(|_| Resonator::new(sr)),
            ratios: config.material.ratios(config.seed),
            w_strike: position_weights(config.strike_pos),
            w_pickup: position_weights(config.pickup_pos),
            mode_gain: [0.0; PLATE_MAX_MODES],
            input_norm: [0.0; PLATE_MAX_MODES],
            noise_norm: [0.0; PLATE_MAX_MODES],
            mode_band: [0; PLATE_MAX_MODES],
            mode_enabled: [false; PLATE_MAX_MODES],
            exciter: Exciter::new(sr),
            exciter_norm: 1.0,
            params: [
                SmoothedParam::new(config.pitch_hz, 30.0, 2_000.0, sr, SMOOTH_MS),
                SmoothedParam::new(config.geometry, -0.5, 0.5, sr, SMOOTH_MS),
                SmoothedParam::new_normalized(config.strike_pos, sr),
                SmoothedParam::new_normalized(config.pickup_pos, sr),
                SmoothedParam::new_normalized(config.hardness, sr),
                SmoothedParam::new(config.decay_seconds, 0.05, 20.0, sr, SMOOTH_MS),
                SmoothedParam::new(config.hf_damping, 0.0, 8.0, sr, SMOOTH_MS),
                SmoothedParam::new_normalized(config.tension, sr),
                SmoothedParam::new_normalized(config.cascade, sr),
                SmoothedParam::new_normalized(config.deplete, sr),
                SmoothedParam::new(config.brightness_hz, 500.0, 20_000.0, sr, SMOOTH_MS),
            ],
            channel_tuning: SmoothedParam::new_normalized(0.5, sr),
            pending_material: config.material,
            active_material: config.material,
            latched_seed: config.seed,
            energy: 0.0,
            energy_coeff: 1.0 - (-1.0 / (ENERGY_MS * 0.001 * sr)).exp(),
            cascade_os: std::array::from_fn(|_| Oversampler2x::new()),
            cascade_drive: [0.0; CASCADE_BANDS - 1],
            cascade_noise: [0.0; CASCADE_BANDS - 1],
            cascade_env: [0.0; CASCADE_BANDS - 1],
            cascade_env_coeff: 1.0 - (-1.0 / (CASCADE_ENV_MS * 0.001 * sr)).exp(),
            noise_rng: XorShift32::new(config.seed),
            lp_state: 0.0,
            lp_coeff: 1.0,
            control_counter: 0,
            quiet_counter: 0,
            seed: config.seed,
            hit_counter: 0,
            velocity: 1.0,
            trigger_time: 0.0,
            active: false,
            midi_note: None,
            ring_limit_secs: None,
        };
        voice.update_control();
        voice
    }

    pub fn config_targets(&self) -> PlateConfig {
        PlateConfig {
            pitch_hz: self.params[0].target(),
            geometry: self.params[1].target(),
            strike_pos: self.params[2].target(),
            pickup_pos: self.params[3].target(),
            hardness: self.params[4].target(),
            decay_seconds: self.params[5].target(),
            hf_damping: self.params[6].target(),
            tension: self.params[7].target(),
            cascade: self.params[8].target(),
            deplete: self.params[9].target(),
            brightness_hz: self.params[10].target(),
            material: self.pending_material,
            seed: self.seed,
        }
    }

    pub fn set_config(&mut self, config: PlateConfig) {
        let values = [
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
        ];
        for (param, value) in self.params.iter_mut().zip(values) {
            param.set_target(finite(value, param.target()));
        }
        self.pending_material = config.material;
        self.seed = config.seed;
    }

    pub fn parameter_normalized(&self, index: usize) -> Option<f32> {
        if index == PLATE_PARAM_MATERIAL_INDEX {
            return Some(self.pending_material.index() as f32 / 4.0);
        }
        let p = self.params.get(index)?;
        Some(if Self::exponential_parameter(index) {
            (p.target() / p.min).ln() / (p.max / p.min).ln()
        } else {
            (p.target() - p.min) / (p.max - p.min)
        })
    }

    pub fn set_parameter_normalized(&mut self, index: usize, value: f32) {
        if !value.is_finite() {
            return;
        }
        let normalized = value.clamp(0.0, 1.0);
        if index == PLATE_PARAM_MATERIAL_INDEX {
            self.set_material(PlateMaterial::from_index(
                (normalized * 4.0).round() as usize
            ));
            return;
        }
        if let Some(param) = self.params.get_mut(index) {
            if Self::exponential_parameter(index) {
                param.set_target(param.min * (param.max / param.min).powf(normalized));
            } else {
                param.set_normalized(normalized);
            }
        }
    }

    /// Pitch, decay and brightness use logarithmic slider spacing.
    fn exponential_parameter(index: usize) -> bool {
        matches!(index, 0 | 5 | 10)
    }

    /// 0.5 is neutral; endpoints shift the pitch by 12 semitones.
    pub fn set_channel_tuning(&mut self, value: f32) {
        if value.is_finite() {
            self.channel_tuning.set_target(value);
        }
    }

    pub fn channel_tuning(&self) -> f32 {
        self.channel_tuning.target()
    }

    /// Takes effect on the next trigger so a ringing bank is not retuned.
    pub fn set_material(&mut self, material: PlateMaterial) {
        self.pending_material = material;
    }

    pub fn material(&self) -> PlateMaterial {
        self.pending_material
    }

    pub fn set_ring_limit_secs(&mut self, limit: Option<f32>) {
        self.ring_limit_secs = limit.map(|v| finite(v, 0.001).max(0.001));
    }

    pub fn set_seed(&mut self, seed: u32) {
        self.seed = seed;
    }

    /// Short-time modal energy, normalised so a full-velocity ride peaks
    /// near 1. Exposed for metering and tests.
    pub fn energy(&self) -> f32 {
        self.energy / ENERGY_REF
    }

    /// Largest cascade envelope across bands (0-1). Exposed for metering.
    pub fn cascade_level(&self) -> f32 {
        self.cascade_env.iter().fold(0.0, |max, env| max.max(*env))
    }

    pub fn reset(&mut self) {
        for mode in &mut self.modes {
            mode.reset();
        }
        for os in &mut self.cascade_os {
            os.reset();
        }
        self.exciter.reset();
        self.cascade_drive = [0.0; CASCADE_BANDS - 1];
        self.cascade_noise = [0.0; CASCADE_BANDS - 1];
        self.cascade_env = [0.0; CASCADE_BANDS - 1];
        self.energy = 0.0;
        self.lp_state = 0.0;
        self.active = false;
    }

    fn base_pitch(&self) -> f32 {
        let pitch = self
            .midi_note
            .map(|note| 440.0 * 2.0_f32.powf((note as f32 - 69.0) / 12.0))
            .unwrap_or_else(|| self.params[0].get());
        pitch * tuning_to_multiplier(self.channel_tuning.get())
    }

    /// Recompute mode frequencies, decays, gains and pickup weights from the
    /// current smoothed parameter values and energy.
    fn update_control(&mut self) {
        let sr = self.sample_rate;
        let f0 = self.base_pitch();
        let geometry = self.params[1].get();
        let decay = self.params[5].get();
        let hf_damping = self.params[6].get();
        let tension = self.params[7].get();
        let cascade = self.params[8].get();
        let deplete = self.params[9].get();
        let e = self.energy().clamp(0.0, 1.0);
        let glide = (1.0 + 2.0 * tension * e).sqrt();
        let deplete_scale = 1.0 + DEPLETE_RATE * deplete * cascade * e;
        let stretch = 1.0 + geometry;
        let fade_start = NYQUIST_FADE_START * sr;
        let fade_end = NYQUIST_FADE_END * sr;
        self.w_pickup = position_weights(self.params[3].get());

        for k in 0..PLATE_MAX_MODES {
            let ratio = self.ratios[k].powf(stretch);
            let freq = f0 * ratio * glide;
            let fade = ((fade_end - freq) / (fade_end - fade_start)).clamp(0.0, 1.0);
            if fade <= 0.0 {
                if self.mode_enabled[k] {
                    self.modes[k].reset();
                }
                self.mode_enabled[k] = false;
                self.mode_gain[k] = 0.0;
                continue;
            }
            self.mode_enabled[k] = true;
            let band = BAND_EDGES.iter().filter(|edge| ratio >= **edge).count();
            self.mode_band[k] = band as u8;
            let mode = &mut self.modes[k];
            mode.set_frequency(freq);
            let mut t60 = decay / (1.0 + hf_damping * (ratio - 1.0).max(0.0));
            if band < CASCADE_BANDS - 1 {
                t60 /= deplete_scale;
            }
            mode.set_decay_time(t60);
            self.mode_gain[k] = fade / ratio.sqrt();
            self.input_norm[k] = 1.0 / (TAU * freq / sr).max(1.0e-3);
            // 4 zeta omega = 4 * 6.91 / T60 in samples.
            self.noise_norm[k] = (27.6 / (t60.max(0.01) * sr)).sqrt();
        }

        let brightness = self.params[10].get().min(sr * 0.45);
        self.lp_coeff = 1.0 - (-TAU * brightness / sr).exp();
    }
}

impl Instrument for PlateVoice {
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        let velocity = finite(velocity, 0.0).clamp(0.0, 1.0);
        if velocity == 0.0 {
            return;
        }
        if !self.active {
            // A silent bank has nothing to glide from, so jump straight to
            // the latest targets instead of sweeping the first hit.
            for param in &mut self.params {
                param.snap();
            }
            self.channel_tuning.snap();
        }
        if self.pending_material != self.active_material || self.seed != self.latched_seed {
            self.active_material = self.pending_material;
            self.latched_seed = self.seed;
            self.ratios = self.active_material.ratios(self.seed);
        }
        self.w_strike = position_weights(self.params[2].target());

        let hardness = self.params[4].target();
        let kind = if hardness < 1.0 / 3.0 {
            ExciterKind::Pulse
        } else if hardness < 2.0 / 3.0 {
            ExciterKind::ClickTable
        } else {
            ExciterKind::NoiseBurst
        };
        let width_ms = 8.0 * (0.25_f32 / 8.0).powf(hardness) * (1.0 - 0.3 * velocity);
        self.exciter.set_kind(kind);
        self.exciter.set_width_ms(width_ms);
        let width_samples = (self.exciter.width_ms() * 0.001 * self.sample_rate).max(1.0);
        // Normalise each kind so its area (and so its low-frequency drive)
        // is close to its amplitude.
        self.exciter_norm = match kind {
            ExciterKind::Pulse => 2.0 / width_samples,
            ExciterKind::ClickTable => 0.12,
            ExciterKind::NoiseBurst => 6.0 / width_samples.sqrt(),
        };
        self.hit_counter = self.hit_counter.wrapping_add(1);
        self.exciter.trigger(
            velocity.sqrt() * EXCITER_AMPLITUDE,
            self.seed ^ self.hit_counter.wrapping_mul(0x9E37_79B9),
        );

        self.velocity = velocity;
        self.trigger_time = time;
        self.active = true;
        self.quiet_counter = 0;
        self.control_counter = 0;
    }

    fn tick(&mut self, time: f64) -> f32 {
        if !self.active {
            return 0.0;
        }
        let elapsed = (time - self.trigger_time).max(0.0) as f32;
        if self.ring_limit_secs.is_some_and(|limit| elapsed >= limit) {
            self.reset();
            return 0.0;
        }

        for param in &mut self.params {
            param.tick();
        }
        self.channel_tuning.tick();
        if self.control_counter == 0 {
            self.update_control();
        }
        self.control_counter = (self.control_counter + 1) % CONTROL_BLOCK;

        let excitation = self.exciter.tick() * self.exciter_norm;
        let inject = self.cascade_drive;
        let inject_noise = self.cascade_noise;
        let mut sum = 0.0;
        let mut modal_energy = 0.0;
        let mut bands = [0.0_f32; CASCADE_BANDS];
        for k in 0..PLATE_MAX_MODES {
            if !self.mode_enabled[k] {
                continue;
            }
            let band = self.mode_band[k] as usize;
            let mut drive = excitation * self.w_strike[k] * self.input_norm[k];
            if band > 0 {
                drive += inject[band - 1] * self.input_norm[k]
                    + inject_noise[band - 1] * self.noise_norm[k];
            }
            let y = self.modes[k].process(drive);
            let weighted = y * self.mode_gain[k];
            sum += weighted * self.w_pickup[k];
            bands[band] += weighted;
            modal_energy += y * y;
        }
        self.energy += self.energy_coeff * (modal_energy - self.energy);

        let cascade = self.params[8].get();
        if cascade > 0.0 {
            let noise = self.noise_rng.next_bipolar();
            for (i, os) in self.cascade_os.iter_mut().enumerate() {
                let products = os.process(bands[i] * CASCADE_DRIVE, cascade_cubic);
                let env = &mut self.cascade_env[i];
                *env += self.cascade_env_coeff * (products.abs() - *env);
                self.cascade_drive[i] = cascade * products * CASCADE_INJECT;
                self.cascade_noise[i] = cascade * *env * noise * CASCADE_NOISE;
            }
        } else {
            self.cascade_drive = [0.0; CASCADE_BANDS - 1];
            self.cascade_noise = [0.0; CASCADE_BANDS - 1];
            self.cascade_env = [0.0; CASCADE_BANDS - 1];
        }

        self.lp_state += self.lp_coeff * (sum * OUTPUT_GAIN - self.lp_state);
        let output = self.lp_state.tanh();

        self.quiet_counter += 1;
        if self.quiet_counter >= QUIET_CHECK_BLOCK {
            self.quiet_counter = 0;
            if !self.exciter.is_active()
                && self.lp_state.abs() < 1.0e-5
                && self.modes.iter().all(Resonator::is_quiet)
            {
                self.reset();
            }
        }

        if output.is_finite() {
            output
        } else {
            self.reset();
            0.0
        }
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn set_midi_note(&mut self, note: u8) {
        self.midi_note = Some(note);
    }

    fn as_modulatable(&mut self) -> Option<&mut dyn Modulatable> {
        Some(self)
    }
}

impl Modulatable for PlateVoice {
    fn modulatable_parameters(&self) -> Vec<&'static str> {
        vec![
            "pitch",
            "geometry",
            "strike_pos",
            "pickup_pos",
            "hardness",
            "decay",
            "hf_damping",
            "tension",
            "cascade",
            "deplete",
            "brightness",
            "material",
        ]
    }

    fn apply_modulation(&mut self, parameter: &str, value: f32) -> Result<(), String> {
        if let Some(index) = self
            .modulatable_parameters()
            .iter()
            .position(|candidate| *candidate == parameter)
        {
            self.set_parameter_normalized(index, value);
            Ok(())
        } else {
            Err(format!("Unknown PlateVoice parameter: {parameter}"))
        }
    }

    fn parameter_range(&self, parameter: &str) -> Option<(f32, f32)> {
        self.modulatable_parameters()
            .iter()
            .position(|candidate| *candidate == parameter)
            .map(|_| (0.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_latches_on_the_next_trigger() {
        let mut voice = PlateVoice::with_config(48_000.0, PlateConfig::ride());
        voice.set_material(PlateMaterial::Bell);
        assert_eq!(voice.active_material, PlateMaterial::Plate);
        voice.trigger_with_velocity(0.0, 1.0);
        assert_eq!(voice.active_material, PlateMaterial::Bell);
        assert_eq!(voice.ratios[0], 0.5);
    }

    #[test]
    fn edge_strike_still_excites_modes() {
        let weights = position_weights(1.0);
        assert!(weights.iter().all(|w| w.is_finite()));
        assert!(weights.iter().any(|w| w.abs() > 0.5));
    }
}
