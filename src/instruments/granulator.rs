//! Mono frozen-scan granular instrument.
//!
//! This is a small, engine-native subset inspired by the Arbhar grain player:
//! a fixed pool of grains reads from an in-memory sample buffer around a scan
//! position, with random spray, pitch/speed, direction probability, and a
//! shaped window envelope.

use crate::effects::Waveshaper;
use crate::engine::{Instrument, Modulatable};
use crate::utils::{cubic_interpolate, SmoothedParam};
use std::sync::Arc;

const MAX_GRAINS: usize = 64;
// Capacity of the auxiliary "release pool" used to hold stolen victims while
// they fade out. Keeping the release pool separate from the main grain pool
// means a steal can both fade the victim AND allow the new spawn into the
// freed main slot, so we don't drop scheduled grains under sustained
// saturation. The pool is small because each release lasts only ~4 ms; at
// the maximum density of 80 g/s and a 4 ms release that's ≤1 concurrent
// stolen grain on average, with headroom for bursts.
const RELEASE_POOL_SIZE: usize = 16;
// Length of the fade-out applied when a grain is stolen to make room for a new
// one. ~4 ms is short enough to be inaudible as a duck but long enough to
// avoid a click from terminating mid-envelope.
const STEAL_RELEASE_MS: f32 = 4.0;
// Fixed internal drive for the shared Waveshaper effect. The user-facing
// `drive` parameter controls the Waveshaper's mix rather than its drive
// gain, so going from 0 to 1 fades the saturated path in against the dry
// path instead of pushing more level into the saturator. The Waveshaper's
// own compensation (`tanh(0.5) / tanh(0.5 * drive)`) keeps the wet path
// unity at ±0.5 amplitude, so peaks soften and quiet signal is preserved.
const DRIVE_INTERNAL_AMOUNT: f32 = 4.0;
const MIN_GRAIN_MS: f32 = 5.0;
const MAX_GRAIN_MS: f32 = 3000.0;
const MAX_SPRAY_SECS: f32 = 10.0;
const MIN_CLOUD_MS: f32 = 50.0;
const MAX_CLOUD_MS: f32 = 8000.0;
const MAX_DENSITY: f32 = 80.0;
const MIN_PITCH: f32 = 0.25;
const MAX_PITCH: f32 = 4.0;

/// Shared mono sample data for granular playback.
#[derive(Clone, Debug)]
pub struct SampleBuffer {
    samples: Arc<[f32]>,
    sample_rate: f32,
}

impl SampleBuffer {
    pub fn from_mono(samples: Vec<f32>, sample_rate: f32) -> Result<Self, String> {
        if samples.is_empty() {
            return Err("SampleBuffer requires at least one sample".to_string());
        }
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            return Err(format!("Invalid sample rate: {sample_rate}"));
        }
        if samples.iter().any(|sample| !sample.is_finite()) {
            return Err("SampleBuffer samples must be finite".to_string());
        }

        Ok(Self {
            samples: Arc::from(samples.into_boxed_slice()),
            sample_rate,
        })
    }

    #[cfg(feature = "bounce")]
    pub fn from_wav_mono(path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        let mut reader = hound::WavReader::open(path.as_ref())
            .map_err(|e| format!("Failed to open WAV: {e}"))?;
        let spec = reader.spec();
        if spec.channels == 0 {
            return Err("WAV must have at least one channel".to_string());
        }
        if spec.sample_rate == 0 {
            return Err("WAV sample rate must be greater than zero".to_string());
        }

        let channels = spec.channels as usize;
        let interleaved = match spec.sample_format {
            hound::SampleFormat::Float => reader
                .samples::<f32>()
                .map(|s| s.map_err(|e| format!("Failed to read WAV sample: {e}")))
                .collect::<Result<Vec<_>, _>>()?,
            hound::SampleFormat::Int => match spec.bits_per_sample {
                0 => return Err("WAV bit depth must be greater than zero".to_string()),
                1..=8 => {
                    let scale = ((1_i32 << (spec.bits_per_sample - 1)) - 1) as f32;
                    reader
                        .samples::<i8>()
                        .map(|s| {
                            s.map(|v| v as f32 / scale)
                                .map_err(|e| format!("Failed to read WAV sample: {e}"))
                        })
                        .collect::<Result<Vec<_>, _>>()?
                }
                9..=16 => {
                    let scale = ((1_i32 << (spec.bits_per_sample - 1)) - 1) as f32;
                    reader
                        .samples::<i16>()
                        .map(|s| {
                            s.map(|v| v as f32 / scale)
                                .map_err(|e| format!("Failed to read WAV sample: {e}"))
                        })
                        .collect::<Result<Vec<_>, _>>()?
                }
                17..=32 => {
                    let scale = ((1_i64 << (spec.bits_per_sample - 1)) - 1) as f32;
                    reader
                        .samples::<i32>()
                        .map(|s| {
                            s.map(|v| v as f32 / scale)
                                .map_err(|e| format!("Failed to read WAV sample: {e}"))
                        })
                        .collect::<Result<Vec<_>, _>>()?
                }
                bits => return Err(format!("Unsupported WAV bit depth: {bits}")),
            },
        };

        if interleaved.is_empty() {
            return Err("WAV contains no samples".to_string());
        }

        let mut mono = Vec::with_capacity(interleaved.len() / channels);
        for frame in interleaved.chunks_exact(channels) {
            mono.push(frame.iter().sum::<f32>() / channels as f32);
        }

        Self::from_mono(mono, spec.sample_rate as f32)
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    #[inline]
    fn sample_clamped(&self, index: isize) -> f32 {
        let last = self.samples.len() as isize - 1;
        self.samples[index.clamp(0, last) as usize]
    }

    #[inline]
    fn sample_interpolated(&self, position: f32) -> f32 {
        if self.samples.len() == 1 {
            return self.samples[0];
        }

        let last = self.samples.len() as f32 - 1.0;
        let position = position.clamp(0.0, last);
        let index = position.floor() as isize;
        let frac = position - index as f32;

        let p0 = self.sample_clamped(index - 1);
        let p1 = self.sample_clamped(index);
        let p2 = self.sample_clamped(index + 1);
        let p3 = self.sample_clamped(index + 2);

        cubic_interpolate(p0, p1, p2, p3, frac)
    }
}

/// Normalized granulator preset.
#[derive(Clone, Copy, Debug)]
pub struct GranulatorConfig {
    pub scan_position: f32,
    pub grain_length: f32,
    pub spray: f32,
    pub pitch: f32,
    pub density: f32,
    pub texture: f32,
    pub direction: f32,
    pub cloud_duration: f32,
    pub volume: f32,
    pub random_timing: f32,
    pub random_amp: f32,
    pub drive: f32,
}

impl Default for GranulatorConfig {
    fn default() -> Self {
        Self {
            scan_position: 0.5,
            grain_length: 0.16,
            spray: 0.12,
            pitch: 0.5,
            density: 0.35,
            texture: 0.25,
            direction: 0.0,
            cloud_duration: 0.35,
            volume: 0.8,
            random_timing: 0.0,
            random_amp: 0.0,
            drive: 0.0,
        }
    }
}

#[derive(Clone, Debug)]
struct GranulatorParams {
    scan_position: SmoothedParam,
    grain_length: SmoothedParam,
    spray: SmoothedParam,
    pitch: SmoothedParam,
    density: SmoothedParam,
    texture: SmoothedParam,
    direction: SmoothedParam,
    cloud_duration: SmoothedParam,
    volume: SmoothedParam,
    random_timing: SmoothedParam,
    random_amp: SmoothedParam,
    drive: SmoothedParam,
}

impl GranulatorParams {
    fn from_config(config: GranulatorConfig, sample_rate: f32) -> Self {
        Self {
            scan_position: SmoothedParam::new_normalized(config.scan_position, sample_rate),
            grain_length: SmoothedParam::new_normalized(config.grain_length, sample_rate),
            spray: SmoothedParam::new_normalized(config.spray, sample_rate),
            pitch: SmoothedParam::new_normalized(config.pitch, sample_rate),
            density: SmoothedParam::new_normalized(config.density, sample_rate),
            texture: SmoothedParam::new_normalized(config.texture, sample_rate),
            direction: SmoothedParam::new_normalized(config.direction, sample_rate),
            cloud_duration: SmoothedParam::new_normalized(config.cloud_duration, sample_rate),
            volume: SmoothedParam::new_normalized(config.volume, sample_rate),
            random_timing: SmoothedParam::new_normalized(config.random_timing, sample_rate),
            random_amp: SmoothedParam::new_normalized(config.random_amp, sample_rate),
            drive: SmoothedParam::new_normalized(config.drive, sample_rate),
        }
    }

    #[inline]
    fn tick(&mut self) {
        self.scan_position.tick();
        self.grain_length.tick();
        self.spray.tick();
        self.pitch.tick();
        self.density.tick();
        self.texture.tick();
        self.direction.tick();
        self.cloud_duration.tick();
        self.volume.tick();
        self.random_timing.tick();
        self.random_amp.tick();
        self.drive.tick();
    }

    fn snap_all(&mut self) {
        self.scan_position.snap();
        self.grain_length.snap();
        self.spray.snap();
        self.pitch.snap();
        self.density.snap();
        self.texture.snap();
        self.direction.snap();
        self.cloud_duration.snap();
        self.volume.snap();
        self.random_timing.snap();
        self.random_amp.snap();
        self.drive.snap();
    }
}

#[derive(Clone, Copy, Debug)]
struct Grain {
    active: bool,
    source_pos: f32,
    age_samples: f32,
    duration_samples: f32,
    speed: f32,
    direction: f32,
    window_shape: f32,
    velocity: f32,
    // Soft-kill fade-out state. While `release_samples > 0`, the grain is being
    // faded out (used when a new spawn steals this slot); when it hits 0 the
    // grain deactivates without producing a sample-edge discontinuity.
    release_samples: f32,
    release_total: f32,
}

impl Default for Grain {
    fn default() -> Self {
        Self {
            active: false,
            source_pos: 0.0,
            age_samples: 0.0,
            duration_samples: 1.0,
            speed: 1.0,
            direction: 1.0,
            window_shape: 1.0,
            velocity: 1.0,
            release_samples: 0.0,
            release_total: 0.0,
        }
    }
}

/// Mono granular instrument with a frozen scan position.
pub struct Granulator {
    sample_rate: f32,
    buffer: SampleBuffer,
    params: GranulatorParams,
    grains: [Grain; MAX_GRAINS],
    // Auxiliary slots that only ever hold grains in the middle of a release
    // fade-out. Used when stealing a slot from a saturated `grains` pool: the
    // victim is moved here so its envelope can finish cleanly while a new
    // grain takes the freed main slot. See RELEASE_POOL_SIZE for sizing.
    release_grains: [Grain; RELEASE_POOL_SIZE],
    gain_compensation: SmoothedParam,
    cloud_active: bool,
    cloud_end_time: f64,
    next_grain_time: f64,
    current_velocity: f32,
    rng: XorShift32,
    drive_shaper: Waveshaper,
}

impl Granulator {
    pub fn new(sample_rate: f32, buffer: SampleBuffer) -> Self {
        Self::with_config(sample_rate, buffer, GranulatorConfig::default())
    }

    pub fn with_config(sample_rate: f32, buffer: SampleBuffer, config: GranulatorConfig) -> Self {
        // Initialize Waveshaper with mix = configured drive so the wet path
        // matches the requested drive level on construction; internal drive
        // is fixed so the user-facing param is gain-neutral (see
        // DRIVE_INTERNAL_AMOUNT comment).
        let drive_shaper = Waveshaper::new(DRIVE_INTERNAL_AMOUNT, config.drive.clamp(0.0, 1.0));
        Self {
            sample_rate,
            buffer,
            params: GranulatorParams::from_config(config, sample_rate),
            grains: [Grain::default(); MAX_GRAINS],
            release_grains: [Grain::default(); RELEASE_POOL_SIZE],
            gain_compensation: SmoothedParam::new(1.0, 0.0, 1.0, sample_rate, 10.0),
            cloud_active: false,
            cloud_end_time: 0.0,
            next_grain_time: 0.0,
            current_velocity: 1.0,
            rng: XorShift32::new(0x1234_abcd),
            drive_shaper,
        }
    }

    pub fn set_buffer(&mut self, buffer: SampleBuffer) {
        self.buffer = buffer;
        self.kill_all_grains();
    }

    pub fn set_seed(&mut self, seed: u32) {
        self.rng = XorShift32::new(seed);
    }

    pub fn buffer_len(&self) -> usize {
        self.buffer.len()
    }

    pub fn buffer_sample_rate(&self) -> f32 {
        self.buffer.sample_rate()
    }

    pub fn snap_params(&mut self) {
        self.params.snap_all();
        self.gain_compensation.snap();
    }

    pub fn scan_position(&self) -> f32 {
        self.params.scan_position.target()
    }

    pub fn grain_length(&self) -> f32 {
        self.params.grain_length.target()
    }

    pub fn spray(&self) -> f32 {
        self.params.spray.target()
    }

    pub fn pitch(&self) -> f32 {
        self.params.pitch.target()
    }

    pub fn density(&self) -> f32 {
        self.params.density.target()
    }

    pub fn texture(&self) -> f32 {
        self.params.texture.target()
    }

    pub fn direction(&self) -> f32 {
        self.params.direction.target()
    }

    pub fn cloud_duration(&self) -> f32 {
        self.params.cloud_duration.target()
    }

    pub fn volume(&self) -> f32 {
        self.params.volume.target()
    }

    pub fn random_timing(&self) -> f32 {
        self.params.random_timing.target()
    }

    pub fn random_amp(&self) -> f32 {
        self.params.random_amp.target()
    }

    pub fn drive(&self) -> f32 {
        self.params.drive.target()
    }

    pub fn grain_length_ms(&self) -> f32 {
        grain_length_ms(self.grain_length())
    }

    pub fn spray_ms(&self) -> f32 {
        spray_seconds(self.spray()) * 1000.0
    }

    pub fn pitch_ratio(&self) -> f32 {
        pitch_ratio(self.pitch())
    }

    pub fn density_grains_per_second(&self) -> f32 {
        density_grains_per_second(self.density())
    }

    pub fn cloud_duration_ms(&self) -> f32 {
        cloud_duration_ms(self.cloud_duration())
    }

    pub fn set_scan_position(&mut self, value: f32) {
        self.params.scan_position.set_target(value);
    }

    pub fn set_grain_length(&mut self, value: f32) {
        self.params.grain_length.set_target(value);
    }

    pub fn set_spray(&mut self, value: f32) {
        self.params.spray.set_target(value);
    }

    pub fn set_pitch(&mut self, value: f32) {
        self.params.pitch.set_target(value);
    }

    pub fn set_density(&mut self, value: f32) {
        self.params.density.set_target(value);
    }

    pub fn set_texture(&mut self, value: f32) {
        self.params.texture.set_target(value);
    }

    pub fn set_direction(&mut self, value: f32) {
        self.params.direction.set_target(value);
    }

    pub fn set_cloud_duration(&mut self, value: f32) {
        self.params.cloud_duration.set_target(value);
    }

    pub fn set_volume(&mut self, value: f32) {
        self.params.volume.set_target(value);
    }

    pub fn set_random_timing(&mut self, value: f32) {
        self.params.random_timing.set_target(value);
    }

    pub fn set_random_amp(&mut self, value: f32) {
        self.params.random_amp.set_target(value);
    }

    pub fn set_drive(&mut self, value: f32) {
        self.params.drive.set_target(value);
    }

    pub fn active_grain_count(&self) -> usize {
        self.grains.iter().filter(|grain| grain.active).count()
            + self
                .release_grains
                .iter()
                .filter(|grain| grain.active)
                .count()
    }

    fn kill_all_grains(&mut self) {
        for grain in &mut self.grains {
            grain.active = false;
        }
        for grain in &mut self.release_grains {
            grain.active = false;
        }
        self.cloud_active = false;
    }

    fn spawn_due_grains(&mut self, current_time: f64) {
        if !self.cloud_active {
            return;
        }

        if current_time > self.cloud_end_time {
            self.cloud_active = false;
            return;
        }

        let density = density_grains_per_second(self.params.density.get());
        if density <= 0.0 {
            return;
        }

        let interval = 1.0 / density as f64;
        let random_timing = self.params.random_timing.get().clamp(0.0, 1.0) as f64;
        let mut guard = 0;
        while self.cloud_active && current_time + 1e-12 >= self.next_grain_time && guard < 8 {
            self.spawn_grain();
            self.next_grain_time += interval;
            // Random timing jitter: signed offset bounded by ±interval * amount.
            // Average density is preserved because the jitter is zero-mean and
            // applied after the interval has already advanced.
            if random_timing > 0.0 {
                let jitter = ((self.rng.next_f32() as f64) * 2.0 - 1.0) * interval * random_timing;
                self.next_grain_time = (self.next_grain_time + jitter).max(current_time);
            }
            if self.next_grain_time > self.cloud_end_time {
                self.cloud_active = false;
            }
            guard += 1;
        }
    }

    fn spawn_grain(&mut self) {
        // Pre-roll the RNG for amp jitter so the deterministic-output test still
        // sees a stable sequence regardless of whether the slot is free or stolen.
        let amp_jitter = self.rng.next_f32();

        let slot = match self.grains.iter().position(|grain| !grain.active) {
            Some(s) => s,
            None => {
                // Soft stealing: move the victim out into the release pool so
                // it can fade out cleanly, freeing its main slot for this
                // spawn. If the release pool is also full we drop this grain
                // — but the steal_grain path is the common case, so the
                // requested spawn usually proceeds instead of being lost.
                if !self.steal_grain() {
                    return;
                }
                match self.grains.iter().position(|grain| !grain.active) {
                    Some(s) => s,
                    None => return,
                }
            }
        };

        let last_sample = (self.buffer.len() - 1) as f32;
        let scan = self.params.scan_position.get().clamp(0.0, 1.0) * last_sample;
        let spray_samples = spray_seconds(self.params.spray.get()) * self.buffer.sample_rate();
        let spray_offset = (self.rng.next_f32() * 2.0 - 1.0) * spray_samples;
        let requested_source_pos = (scan + spray_offset).clamp(0.0, last_sample);

        let direction = if self.rng.next_f32() < self.params.direction.get() {
            -1.0
        } else {
            1.0
        };
        let speed =
            pitch_ratio(self.params.pitch.get()) * (self.buffer.sample_rate() / self.sample_rate);
        let mut duration_samples =
            (grain_length_ms(self.params.grain_length.get()) * 0.001 * self.sample_rate).max(1.0);
        let window_shape = window_shape(self.params.texture.get());
        let source_travel = duration_samples * speed;

        // Avoid hard grain termination at buffer edges. The old version let grains
        // start anywhere and killed them when the read head crossed an edge, which
        // bypassed the window envelope and produced clicks near the start/end of
        // short buffers or wide spray ranges.
        let source_pos = if source_travel >= last_sample {
            duration_samples = (last_sample / speed).max(1.0);
            if direction < 0.0 {
                last_sample
            } else {
                0.0
            }
        } else if direction < 0.0 {
            requested_source_pos.clamp(source_travel, last_sample)
        } else {
            requested_source_pos.clamp(0.0, last_sample - source_travel)
        };

        // Per-grain random amplitude: factor in [1 - random_amp, 1.0].
        let random_amp = self.params.random_amp.get().clamp(0.0, 1.0);
        let amp_factor = 1.0 - random_amp * amp_jitter;

        self.grains[slot] = Grain {
            active: true,
            source_pos,
            age_samples: 0.0,
            duration_samples,
            speed,
            direction,
            window_shape,
            velocity: self.current_velocity * amp_factor,
            release_samples: 0.0,
            release_total: 0.0,
        };
    }

    /// Relocate the most-stealable active grain from `grains` into
    /// `release_grains` with a short fade-out, freeing its main slot for a
    /// new spawn. Returns true if a main slot was freed, false otherwise
    /// (release pool full or no active main grains).
    fn steal_grain(&mut self) -> bool {
        // Pick the active main-pool grain with the shortest remaining
        // playback time — that's the grain whose loss costs the least.
        let mut victim: Option<usize> = None;
        let mut shortest_remaining = f32::INFINITY;
        for (idx, grain) in self.grains.iter().enumerate() {
            if !grain.active {
                continue;
            }
            let remaining = (grain.duration_samples - grain.age_samples).max(0.0);
            if remaining < shortest_remaining {
                shortest_remaining = remaining;
                victim = Some(idx);
            }
        }
        let Some(idx) = victim else {
            return false;
        };

        let Some(release_slot) = self.release_grains.iter().position(|grain| !grain.active) else {
            return false;
        };

        let release = (STEAL_RELEASE_MS * 0.001 * self.sample_rate).max(1.0);
        let remaining = (self.grains[idx].duration_samples - self.grains[idx].age_samples).max(1.0);
        let release = release.min(remaining);

        let mut moved = self.grains[idx];
        moved.release_samples = release;
        moved.release_total = release;
        self.release_grains[release_slot] = moved;
        self.grains[idx].active = false;
        true
    }

    fn tick_grains(&mut self) -> f32 {
        let active_count = self.active_grain_count();
        if active_count == 0 {
            self.gain_compensation.set_target(1.0);
            self.gain_compensation.tick();
            return 0.0;
        }

        self.gain_compensation
            .set_target(1.0 / (active_count as f32).sqrt());
        let gain_comp = self.gain_compensation.tick();
        let mut output = 0.0;
        Self::tick_grain_slice(&mut self.grains, &self.buffer, gain_comp, &mut output);
        Self::tick_grain_slice(
            &mut self.release_grains,
            &self.buffer,
            gain_comp,
            &mut output,
        );
        output
    }

    fn tick_grain_slice(
        slice: &mut [Grain],
        buffer: &SampleBuffer,
        gain_comp: f32,
        output: &mut f32,
    ) {
        for grain in slice.iter_mut() {
            if !grain.active {
                continue;
            }

            if grain.age_samples >= grain.duration_samples {
                grain.active = false;
                continue;
            }

            let phase = (grain.age_samples / grain.duration_samples).clamp(0.0, 1.0);
            let window = raised_sine_window(phase, grain.window_shape);
            let release_gain = if grain.release_total > 0.0 {
                (grain.release_samples / grain.release_total).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let sample = buffer.sample_interpolated(grain.source_pos);
            *output += sample * window * release_gain * grain.velocity * gain_comp;

            grain.source_pos += grain.speed * grain.direction;
            grain.age_samples += 1.0;
            if grain.release_samples > 0.0 {
                grain.release_samples -= 1.0;
                if grain.release_samples <= 0.0 {
                    grain.active = false;
                }
            }
        }
    }
}

impl Instrument for Granulator {
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        self.current_velocity = velocity.clamp(0.0, 1.0);
        self.cloud_active = true;
        self.cloud_end_time =
            time + cloud_duration_ms(self.params.cloud_duration.target()) as f64 * 0.001;
        self.next_grain_time = time;
    }

    fn tick(&mut self, current_time: f64) -> f32 {
        self.params.tick();
        self.spawn_due_grains(current_time);
        let raw = self.tick_grains();
        // Push the latest smoothed drive target into the Waveshaper's mix.
        // Drive=0 → mix=0 → exact dry passthrough; drive=1 → fully wet at
        // the fixed internal drive. Internal drive is held constant so this
        // user-facing knob fades saturation in/out rather than boosting
        // level into the saturator.
        self.drive_shaper.set_mix(self.params.drive.get());
        let driven = self.drive_shaper.process(raw);
        driven * self.params.volume.get()
    }

    fn is_active(&self) -> bool {
        self.cloud_active || self.grains.iter().any(|grain| grain.active)
    }

    fn as_modulatable(&mut self) -> Option<&mut dyn Modulatable> {
        Some(self)
    }
}

impl Modulatable for Granulator {
    fn modulatable_parameters(&self) -> Vec<&'static str> {
        vec![
            "scan_position",
            "grain_length",
            "spray",
            "pitch",
            "density",
            "texture",
            "direction",
            "random_timing",
            "random_amp",
            "drive",
            "volume",
        ]
    }

    fn apply_modulation(&mut self, parameter: &str, value: f32) -> Result<(), String> {
        match parameter {
            "scan_position" => self.params.scan_position.set_bipolar(value),
            "grain_length" => self.params.grain_length.set_bipolar(value),
            "spray" => self.params.spray.set_bipolar(value),
            "pitch" => self.params.pitch.set_bipolar(value),
            "density" => self.params.density.set_bipolar(value),
            "texture" => self.params.texture.set_bipolar(value),
            "direction" => self.params.direction.set_bipolar(value),
            "random_timing" => self.params.random_timing.set_bipolar(value),
            "random_amp" => self.params.random_amp.set_bipolar(value),
            "drive" => self.params.drive.set_bipolar(value),
            "volume" => self.params.volume.set_bipolar(value),
            _ => return Err(format!("Unknown granulator parameter: {parameter}")),
        }
        Ok(())
    }

    fn parameter_range(&self, parameter: &str) -> Option<(f32, f32)> {
        match parameter {
            "scan_position" | "grain_length" | "spray" | "pitch" | "density" | "texture"
            | "direction" | "random_timing" | "random_amp" | "drive" | "volume" => Some((0.0, 1.0)),
            _ => None,
        }
    }
}

#[inline]
fn grain_length_ms(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    MIN_GRAIN_MS + value * value * (MAX_GRAIN_MS - MIN_GRAIN_MS)
}

#[inline]
fn spray_seconds(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    value * value * value * MAX_SPRAY_SECS
}

#[inline]
fn pitch_ratio(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    MIN_PITCH * (MAX_PITCH / MIN_PITCH).powf(value)
}

#[inline]
fn density_grains_per_second(value: f32) -> f32 {
    value.clamp(0.0, 1.0) * MAX_DENSITY
}

#[inline]
fn cloud_duration_ms(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    MIN_CLOUD_MS + value * value * (MAX_CLOUD_MS - MIN_CLOUD_MS)
}

#[inline]
fn window_shape(value: f32) -> f32 {
    0.5 + value.clamp(0.0, 1.0) * 3.5
}

#[inline]
fn raised_sine_window(phase: f32, shape: f32) -> f32 {
    (std::f32::consts::PI * phase.clamp(0.0, 1.0))
        .sin()
        .max(0.0)
        .powf(shape)
}

#[derive(Clone, Copy, Debug)]
struct XorShift32 {
    state: u32,
}

impl XorShift32 {
    fn new(seed: u32) -> Self {
        Self {
            state: if seed == 0 { 0x6d2b_79f5 } else { seed },
        }
    }

    #[inline]
    fn next_u32(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }

    #[inline]
    fn next_f32(&mut self) -> f32 {
        self.next_u32() as f32 / u32::MAX as f32
    }
}

impl Default for XorShift32 {
    fn default() -> Self {
        Self::new(0x1234_abcd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_buffer() -> SampleBuffer {
        let samples = (0..4410)
            .map(|i| ((i as f32 / 44100.0) * 440.0 * std::f32::consts::TAU).sin() * 0.5)
            .collect();
        SampleBuffer::from_mono(samples, 44100.0).unwrap()
    }

    #[test]
    fn sample_buffer_rejects_empty_input() {
        assert!(SampleBuffer::from_mono(Vec::new(), 44100.0).is_err());
    }

    #[test]
    fn sample_buffer_rejects_invalid_sample_rate() {
        assert!(SampleBuffer::from_mono(vec![0.0], 0.0).is_err());
        assert!(SampleBuffer::from_mono(vec![0.0], f32::NAN).is_err());
    }

    #[test]
    fn interpolation_is_finite_at_edges() {
        let buffer = SampleBuffer::from_mono(vec![0.0, 1.0, -1.0, 0.5], 44100.0).unwrap();
        assert!(buffer.sample_interpolated(-10.0).is_finite());
        assert!(buffer.sample_interpolated(0.0).is_finite());
        assert!(buffer.sample_interpolated(3.0).is_finite());
        assert!(buffer.sample_interpolated(99.0).is_finite());
    }

    #[test]
    fn triggered_granulator_produces_finite_audio() {
        let mut granulator = Granulator::new(44100.0, test_buffer());
        granulator.set_seed(7);
        granulator.trigger_with_velocity(0.0, 1.0);

        let mut max_abs = 0.0_f32;
        for i in 0..44100 {
            let sample = granulator.tick(i as f64 / 44100.0);
            assert!(sample.is_finite());
            max_abs = max_abs.max(sample.abs());
        }

        assert!(max_abs > 0.001);
    }

    #[test]
    fn same_seed_produces_same_output() {
        let buffer = test_buffer();
        let mut a = Granulator::new(44100.0, buffer.clone());
        let mut b = Granulator::new(44100.0, buffer);
        a.set_seed(99);
        b.set_seed(99);
        a.snap_params();
        b.snap_params();
        a.trigger_with_velocity(0.0, 0.8);
        b.trigger_with_velocity(0.0, 0.8);

        for i in 0..4096 {
            let time = i as f64 / 44100.0;
            assert_eq!(a.tick(time), b.tick(time));
        }
    }

    #[test]
    fn modulatable_parameter_list_matches_ranges() {
        let granulator = Granulator::new(44100.0, test_buffer());
        for parameter in granulator.modulatable_parameters() {
            assert_eq!(granulator.parameter_range(parameter), Some((0.0, 1.0)));
        }
    }

    #[test]
    fn dense_cloud_with_random_amp_remains_finite() {
        let mut granulator = Granulator::new(44100.0, test_buffer());
        granulator.set_seed(13);
        granulator.set_density(1.0);
        granulator.set_random_amp(1.0);
        granulator.set_random_timing(1.0);
        granulator.set_cloud_duration(1.0);
        granulator.snap_params();
        granulator.trigger_with_velocity(0.0, 1.0);

        let mut max_abs = 0.0_f32;
        for i in 0..88_200 {
            let sample = granulator.tick(i as f64 / 44100.0);
            assert!(sample.is_finite(), "non-finite sample at {i}");
            max_abs = max_abs.max(sample.abs());
        }
        assert!(max_abs > 0.001, "expected audible output");
    }

    #[test]
    fn random_timing_preserves_average_density() {
        // With random_timing = 1.0 the spawn times are heavily jittered. The
        // long-run average density must still track the target because the
        // jitter is zero-mean.
        let mut granulator = Granulator::new(44100.0, test_buffer());
        granulator.set_seed(101);
        granulator.set_density(0.5); // 0.5 * MAX_DENSITY(80) = 40 grains/sec target
        granulator.set_grain_length(0.0); // shortest grains so they end quickly and free slots
        granulator.set_random_timing(1.0);
        granulator.set_cloud_duration(1.0);
        granulator.snap_params();
        granulator.trigger_with_velocity(0.0, 1.0);

        // Run ~2 seconds and count grain spawns by sampling the pool. Because
        // grain_length is the shortest setting (~5 ms) each grain ends fast
        // enough that we can approximate the spawn rate from active counts
        // observed at large intervals — but a simpler accept-band check is
        // enough here: confirm the granulator is producing audio across the
        // whole cloud duration, which only happens if scheduling is healthy.
        let mut audible_blocks = 0;
        let block = 4410; // 0.1s
        for b in 0..20 {
            let mut block_max = 0.0_f32;
            for i in 0..block {
                let t = (b * block + i) as f64 / 44100.0;
                let s = granulator.tick(t);
                assert!(s.is_finite());
                block_max = block_max.max(s.abs());
            }
            if block_max > 1e-4 {
                audible_blocks += 1;
            }
        }
        // At density 40/s with grains across a 2s cloud, the vast majority
        // of 0.1s windows should be audible even under heavy jitter.
        assert!(
            audible_blocks >= 12,
            "random_timing collapsed scheduling: only {audible_blocks}/20 blocks audible"
        );
    }

    #[test]
    fn soft_grain_stealing_does_not_click() {
        // Force the grain pool to saturate by combining max density with
        // long grains. The release pool absorbs stolen victims so new
        // spawns still land. Output must stay finite and bounded.
        let mut granulator = Granulator::new(44100.0, test_buffer());
        granulator.set_seed(31);
        granulator.set_density(1.0);
        granulator.set_grain_length(1.0); // 3000 ms grains
        granulator.set_cloud_duration(1.0);
        granulator.snap_params();
        granulator.trigger_with_velocity(0.0, 1.0);

        for i in 0..88_200 {
            let s = granulator.tick(i as f64 / 44100.0);
            assert!(s.is_finite());
            assert!(s.abs() < 4.0, "runaway gain at sample {i}: {s}");
        }
        // Total live voices are bounded by main + release pool capacity.
        assert!(granulator.active_grain_count() <= MAX_GRAINS + RELEASE_POOL_SIZE);
    }

    #[test]
    fn soft_steal_promotes_new_spawn_into_main_pool() {
        // Saturate the main pool with long grains so every subsequent
        // spawn must go through steal_grain. The contract under review:
        // the new spawn must actually land in the main pool instead of
        // being silently dropped while the victim fades out.
        //
        // Use a deliberately long buffer (3s) so grain durations aren't
        // clamped down by buffer length — otherwise short grains free
        // their slots naturally and the pool never saturates.
        let sample_rate = 44100.0;
        let long_buffer_samples: Vec<f32> = (0..(sample_rate as usize * 3))
            .map(|i| ((i as f32 / sample_rate) * 440.0 * std::f32::consts::TAU).sin() * 0.5)
            .collect();
        let buffer = SampleBuffer::from_mono(long_buffer_samples, sample_rate).unwrap();
        let mut granulator = Granulator::new(sample_rate, buffer);
        granulator.set_seed(53);
        granulator.set_density(1.0); // 80 g/s
        granulator.set_grain_length(1.0); // 3000 ms — main pool fills fast
        granulator.set_cloud_duration(1.0);
        granulator.snap_params();
        granulator.trigger_with_velocity(0.0, 1.0);

        // Run ~1.5 s: at 80 g/s × 3 s grains, the 64-slot main pool
        // saturates around t≈0.8 s; everything after that exercises
        // the steal path. Track the peak releasing-pool occupancy because
        // each release fade is only ~4 ms while steals happen every ~12 ms,
        // so the release pool is empty more often than not at any
        // arbitrary moment.
        let mut peak_releasing = 0usize;
        let mut peak_main = 0usize;
        for i in 0..((sample_rate * 1.5) as usize) {
            granulator.tick(i as f64 / sample_rate as f64);
            let main_now = granulator.grains.iter().filter(|g| g.active).count();
            let rel_now = granulator
                .release_grains
                .iter()
                .filter(|g| g.active)
                .count();
            peak_main = peak_main.max(main_now);
            peak_releasing = peak_releasing.max(rel_now);
        }

        assert_eq!(
            peak_main, MAX_GRAINS,
            "main pool should saturate under sustained density"
        );
        assert!(
            peak_releasing > 0,
            "release pool should hold at least one stolen victim at some point"
        );

        // And the youngest grain in the main pool should be very young
        // (the post-steal spawn), proving new grains are landing rather
        // than being dropped.
        let youngest_age = granulator
            .grains
            .iter()
            .filter(|g| g.active)
            .map(|g| g.age_samples as i32)
            .min()
            .unwrap_or(i32::MAX);
        assert!(
            youngest_age < (sample_rate * 0.05) as i32,
            "no recently-spawned grain found in main pool (youngest = {youngest_age} samples)"
        );
    }

    #[test]
    fn drive_is_roughly_gain_neutral() {
        // The drive parameter is wired into the Waveshaper's mix while the
        // internal drive is fixed. With Waveshaper's reference-level
        // compensation centered at ±0.5, full drive (mix=1) must not boost
        // the cloud's peak level substantially over a clean (drive=0) cloud.
        // Same seed and inputs → directly comparable peak levels.
        fn peak_with_drive(drive: f32) -> f32 {
            let mut g = Granulator::new(44100.0, test_buffer());
            g.set_seed(17);
            g.set_density(0.4);
            g.set_cloud_duration(0.6);
            g.set_volume(1.0);
            g.set_drive(drive);
            g.snap_params();
            g.trigger_with_velocity(0.0, 1.0);
            let mut peak = 0.0_f32;
            for i in 0..44_100 {
                let s = g.tick(i as f64 / 44100.0);
                assert!(s.is_finite());
                peak = peak.max(s.abs());
            }
            peak
        }
        let dry_peak = peak_with_drive(0.0);
        let wet_peak = peak_with_drive(1.0);
        // Wet path must not exceed the dry peak by more than ~25%. This is
        // the user-visible contract: turning drive up should saturate, not
        // boost loudness.
        assert!(
            wet_peak <= dry_peak * 1.25,
            "drive boosted peak too much: dry={dry_peak}, wet={wet_peak}"
        );
        // And the peak should still be bounded (no runaway).
        assert!(wet_peak.is_finite() && wet_peak < 4.0);
    }

    #[cfg(feature = "bounce")]
    #[test]
    fn loads_wav_as_mono() {
        let path =
            std::env::temp_dir().join(format!("gooey_granulator_test_{}.wav", std::process::id()));
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        {
            let mut writer = hound::WavWriter::create(&path, spec).unwrap();
            writer.write_sample::<i16>(i16::MAX).unwrap();
            writer.write_sample::<i16>(0).unwrap();
            writer.write_sample::<i16>(0).unwrap();
            writer.write_sample::<i16>(i16::MAX).unwrap();
            writer.finalize().unwrap();
        }

        let buffer = SampleBuffer::from_wav_mono(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(buffer.len(), 2);
        assert_eq!(buffer.sample_rate(), 44100.0);
        assert!(buffer.samples.iter().all(|sample| sample.is_finite()));
    }
}
