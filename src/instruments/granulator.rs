//! Mono frozen-scan granular instrument.
//!
//! This is a small, engine-native subset inspired by the Arbhar grain player:
//! a fixed pool of grains reads from an in-memory sample buffer around a scan
//! position, with random spray, pitch/speed, direction probability, and a
//! shaped window envelope.

use crate::engine::{Instrument, Modulatable};
use crate::utils::SmoothedParam;
use std::sync::Arc;

const MAX_GRAINS: usize = 64;
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
        }
    }
}

/// Mono granular instrument with a frozen scan position.
pub struct Granulator {
    sample_rate: f32,
    buffer: SampleBuffer,
    params: GranulatorParams,
    grains: [Grain; MAX_GRAINS],
    gain_compensation: SmoothedParam,
    cloud_active: bool,
    cloud_end_time: f64,
    next_grain_time: f64,
    current_velocity: f32,
    rng: XorShift32,
}

impl Granulator {
    pub fn new(sample_rate: f32, buffer: SampleBuffer) -> Self {
        Self::with_config(sample_rate, buffer, GranulatorConfig::default())
    }

    pub fn with_config(sample_rate: f32, buffer: SampleBuffer, config: GranulatorConfig) -> Self {
        Self {
            sample_rate,
            buffer,
            params: GranulatorParams::from_config(config, sample_rate),
            grains: [Grain::default(); MAX_GRAINS],
            gain_compensation: SmoothedParam::new(1.0, 0.0, 1.0, sample_rate, 10.0),
            cloud_active: false,
            cloud_end_time: 0.0,
            next_grain_time: 0.0,
            current_velocity: 1.0,
            rng: XorShift32::new(0x1234_abcd),
        }
    }

    pub fn set_buffer(&mut self, buffer: SampleBuffer) {
        self.buffer = buffer;
        self.kill_all_grains();
    }

    pub fn set_seed(&mut self, seed: u32) {
        self.rng = XorShift32::new(seed);
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

    pub fn active_grain_count(&self) -> usize {
        self.grains.iter().filter(|grain| grain.active).count()
    }

    fn kill_all_grains(&mut self) {
        for grain in &mut self.grains {
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
        let mut guard = 0;
        while self.cloud_active && current_time + 1e-12 >= self.next_grain_time && guard < 8 {
            self.spawn_grain();
            self.next_grain_time += interval;
            if self.next_grain_time > self.cloud_end_time {
                self.cloud_active = false;
            }
            guard += 1;
        }
    }

    fn spawn_grain(&mut self) {
        let Some(slot) = self.grains.iter().position(|grain| !grain.active) else {
            return;
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

        self.grains[slot] = Grain {
            active: true,
            source_pos,
            age_samples: 0.0,
            duration_samples,
            speed,
            direction,
            window_shape,
            velocity: self.current_velocity,
        };
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
        for grain in &mut self.grains {
            if !grain.active {
                continue;
            }

            if grain.age_samples >= grain.duration_samples {
                grain.active = false;
                continue;
            }

            let phase = (grain.age_samples / grain.duration_samples).clamp(0.0, 1.0);
            let window = raised_sine_window(phase, grain.window_shape);
            let sample = self.buffer.sample_interpolated(grain.source_pos);
            output += sample * window * grain.velocity * gain_comp;

            grain.source_pos += grain.speed * grain.direction;
            grain.age_samples += 1.0;
        }

        output
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
        self.tick_grains() * self.params.volume.get()
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
            "volume" => self.params.volume.set_bipolar(value),
            _ => return Err(format!("Unknown granulator parameter: {parameter}")),
        }
        Ok(())
    }

    fn parameter_range(&self, parameter: &str) -> Option<(f32, f32)> {
        match parameter {
            "scan_position" | "grain_length" | "spray" | "pitch" | "density" | "texture"
            | "direction" | "volume" => Some((0.0, 1.0)),
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

#[inline]
fn cubic_interpolate(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let a0 = -0.5 * p0 + 1.5 * p1 - 1.5 * p2 + 0.5 * p3;
    let a1 = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
    let a2 = -0.5 * p0 + 0.5 * p2;
    let a3 = p1;
    ((a0 * t + a1) * t + a2) * t + a3
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
