use crate::envelope::{ADSRConfig, Envelope, EnvelopeCurve};
use crate::filters::state_variable::StateVariableFilter;
use crate::gen::pink_noise::PinkNoise;
use crate::utils::smoother::{SmoothedParam, DEFAULT_SMOOTH_TIME_MS};

/// Static filter bank data from Max patch (preset 1)
const FILTER_FREQUENCIES: [f32; 13] = [
    165.0, 228.0, 294.0, 320.0, 326.0, 356.0, 358.0, 419.0, 481.0, 549.0, 606.0, 724.0, 888.0,
];

/// Max patch "Q" values are actually bandwidths in Hz, not resonance Q factors
/// We store them here and convert to actual Q (frequency/bandwidth) at runtime
const FILTER_BANDWIDTHS: [f32; 13] = [
    275.0, 220.0, 79.0, 65.0, 57.0, 86.0, 100.0, 58.0, 72.0, 86.0, 88.0, 87.0, 81.0,
];

/// Normalized gains from Max (original: 376, 205, 143, 129, 141, 99, 119, 80, 60, 66, 85, 66, 35)
const FILTER_GAINS: [f32; 13] = [
    1.0, 0.545, 0.380, 0.343, 0.375, 0.263, 0.316, 0.213, 0.160, 0.176, 0.226, 0.176, 0.093,
];

/// Soft clipper using tanh - prevents hard clipping artifacts
#[inline]
fn soft_clip(x: f32) -> f32 {
    // tanh naturally limits output to [-1, 1] with smooth saturation
    x.tanh()
}

/// Parameter ranges for normalization
pub(crate) mod ranges {
    // Pitch: MIDI note number 36-84 (C2-C6)
    pub const PITCH_MIN: f32 = 36.0;
    pub const PITCH_MAX: f32 = 84.0;

    // Tone: 200-8000 Hz lowpass cutoff
    pub const TONE_CUTOFF_MIN: f32 = 200.0;
    pub const TONE_CUTOFF_MAX: f32 = 8000.0;

    // Decay: 0.03-0.8 seconds (toms are punchy, not sustained)
    pub const DECAY_MIN: f32 = 0.03;
    pub const DECAY_MAX: f32 = 0.8;

    // Decay curve: 0.2-1.5 exponential
    // < 1.0 = punchy (fast initial decay), > 1.0 = soft (slow initial decay)
    pub const DECAY_CURVE_MIN: f32 = 0.2;
    pub const DECAY_CURVE_MAX: f32 = 1.5;

    /// Denormalize a 0-1 value to a range
    #[inline]
    pub fn denormalize(normalized: f32, min: f32, max: f32) -> f32 {
        min + normalized * (max - min)
    }
}

/// 13-band parallel bandpass filter bank for tom synthesis
struct TomFilterBank {
    filters: [StateVariableFilter; 13],
    base_gains: [f32; 13],
    current_gains: [f32; 13],
}

impl TomFilterBank {
    fn new(sample_rate: f32) -> Self {
        // Initialize 13 SVF filters with preset data
        // Convert Max bandwidth values to proper Q factors: Q = frequency / bandwidth
        let filters = core::array::from_fn(|i| {
            let freq = FILTER_FREQUENCIES[i];
            let q = freq / FILTER_BANDWIDTHS[i];
            StateVariableFilter::new(sample_rate, freq, q)
        });

        Self {
            filters,
            base_gains: FILTER_GAINS,
            current_gains: FILTER_GAINS,
        }
    }

    /// Process input through all 13 bandpass filters and sum
    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let mut output = 0.0;
        for i in 0..13 {
            // Use bandpass mode (mode 1)
            output += self.filters[i].process_mode(input, 1) * self.current_gains[i];
        }
        // Normalize output - reduced from 0.15 to account for resonant filter gain
        // With Q values up to ~11, filters have significant gain at resonance
        // Using 0.05 keeps worst-case peaks under 1.0
        let normalized = output * 0.05;
        // Soft clip as safety for any remaining peaks
        soft_clip(normalized)
    }

    /// Reset all filter states
    fn reset(&mut self) {
        for filter in &mut self.filters {
            filter.reset();
        }
    }

    /// Set pitch ratio to shift all filter frequencies
    fn set_pitch_ratio(&mut self, ratio: f32) {
        for i in 0..13 {
            self.filters[i].set_cutoff_freq(FILTER_FREQUENCIES[i] * ratio);
        }
    }

    /// Apply color parameter to adjust gain distribution
    /// color: 0-1 where 0.5 is neutral
    fn apply_color(&mut self, color: f32) {
        let normalized = (color - 0.5) * 2.0; // -1 to +1
        for i in 0..13 {
            let position = i as f32 / 12.0; // 0 to 1 (low to high)
            let curve = if normalized > 0.0 {
                // Emphasize high frequencies
                1.0 + normalized * position
            } else {
                // Emphasize low frequencies
                1.0 + normalized * (1.0 - position)
            };
            self.current_gains[i] = self.base_gains[i] * curve;
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TomConfig {
    pub pitch: f32,       // 0-1 → MIDI 36-84
    pub color: f32,       // 0-1 (0.5 neutral)
    pub tone: f32,        // 0-1 → 200-8000 Hz lowpass cutoff
    pub bend: f32,        // 0-1 pitch envelope amount
    pub decay: f32,       // 0-1 → 0.05-3.0s
    pub decay_curve: f32, // 0-1 → 0.1-10.0 exponential
    pub volume: f32,      // 0-1
}

impl TomConfig {
    pub fn new(
        pitch: f32,
        color: f32,
        tone: f32,
        bend: f32,
        decay: f32,
        decay_curve: f32,
        volume: f32,
    ) -> Self {
        Self {
            pitch: pitch.clamp(0.0, 1.0),
            color: color.clamp(0.0, 1.0),
            tone: tone.clamp(0.0, 1.0),
            bend: bend.clamp(0.0, 1.0),
            decay: decay.clamp(0.0, 1.0),
            decay_curve: decay_curve.clamp(0.0, 1.0),
            volume: volume.clamp(0.0, 1.0),
        }
    }

    pub fn default() -> Self {
        Self::mid_tom()
    }

    /// DS Tom preset - matches Max patch parameters
    /// pitch: 57, color: 43, tone: 100, bend: 35%
    pub fn ds_tom() -> Self {
        Self::new(
            (57.0 - ranges::PITCH_MIN) / (ranges::PITCH_MAX - ranges::PITCH_MIN), // MIDI 57
            0.43,  // color 43%
            1.0,   // tone 100%
            0.35,  // bend 35%
            0.35,  // decay ~300ms
            0.3,   // decay curve (punchy exponential)
            0.85,  // volume
        )
    }

    pub fn high_tom() -> Self {
        Self::new(
            0.75, // ~MIDI 72
            0.6,  // brighter
            0.7,
            0.4,
            0.25, // decay ~220ms
            0.35, // punchy curve
            0.85,
        )
    }

    pub fn mid_tom() -> Self {
        Self::new(
            0.5,  // ~MIDI 60
            0.5,  // neutral color
            0.5,
            0.3,
            0.4,  // decay ~340ms
            0.3,  // punchy curve
            0.85,
        )
    }

    pub fn low_tom() -> Self {
        Self::new(
            0.35, // ~MIDI 53
            0.4,  // darker
            0.4,
            0.25,
            0.55, // decay ~450ms
            0.25, // slightly less punchy
            0.85,
        )
    }

    pub fn floor_tom() -> Self {
        Self::new(
            0.25, // ~MIDI 48
            0.35, // darker
            0.3,
            0.2,
            0.7,  // decay ~570ms
            0.2,  // rounder curve for floor tom
            0.9,
        )
    }
}

/// Smoothed parameters for real-time control of the tom drum
pub struct TomParams {
    pub pitch: SmoothedParam,
    pub color: SmoothedParam,
    pub tone: SmoothedParam,
    pub bend: SmoothedParam,
    pub decay: SmoothedParam,
    pub decay_curve: SmoothedParam,
    pub volume: SmoothedParam,
}

impl TomParams {
    pub fn from_config(config: &TomConfig, sample_rate: f32) -> Self {
        Self {
            pitch: SmoothedParam::new(config.pitch, 0.0, 1.0, sample_rate, DEFAULT_SMOOTH_TIME_MS),
            color: SmoothedParam::new(config.color, 0.0, 1.0, sample_rate, DEFAULT_SMOOTH_TIME_MS),
            tone: SmoothedParam::new(config.tone, 0.0, 1.0, sample_rate, DEFAULT_SMOOTH_TIME_MS),
            bend: SmoothedParam::new(config.bend, 0.0, 1.0, sample_rate, DEFAULT_SMOOTH_TIME_MS),
            decay: SmoothedParam::new(config.decay, 0.0, 1.0, sample_rate, DEFAULT_SMOOTH_TIME_MS),
            decay_curve: SmoothedParam::new(
                config.decay_curve,
                0.0,
                1.0,
                sample_rate,
                DEFAULT_SMOOTH_TIME_MS,
            ),
            volume: SmoothedParam::new(config.volume, 0.0, 1.0, sample_rate, DEFAULT_SMOOTH_TIME_MS),
        }
    }

    #[inline]
    pub fn tick(&mut self) {
        self.pitch.tick();
        self.color.tick();
        self.tone.tick();
        self.bend.tick();
        self.decay.tick();
        self.decay_curve.tick();
        self.volume.tick();
    }
}

pub struct TomDrum {
    pub sample_rate: f32,
    pub config: TomConfig,
    pub params: TomParams,

    // DS synthesis components
    pink_noise: PinkNoise,
    filter_bank: TomFilterBank,
    tone_filter: StateVariableFilter,

    // Envelopes
    amplitude_envelope: Envelope,
    pitch_envelope: Envelope,

    // Trigger state
    triggered_pitch_ratio: f32,
    current_velocity: f32,
    pub is_active: bool,
}

impl TomDrum {
    pub fn new(sample_rate: f32) -> Self {
        let config = TomConfig::default();
        Self::with_config(sample_rate, config)
    }

    pub fn with_config(sample_rate: f32, config: TomConfig) -> Self {
        let params = TomParams::from_config(&config, sample_rate);
        let tone_cutoff = ranges::denormalize(
            config.tone,
            ranges::TONE_CUTOFF_MIN,
            ranges::TONE_CUTOFF_MAX,
        );

        Self {
            sample_rate,
            config,
            params,
            pink_noise: PinkNoise::new(),
            filter_bank: TomFilterBank::new(sample_rate),
            tone_filter: StateVariableFilter::new(sample_rate, tone_cutoff, 0.7),
            amplitude_envelope: Envelope::new(),
            pitch_envelope: Envelope::new(),
            triggered_pitch_ratio: 1.0,
            current_velocity: 1.0,
            is_active: false,
        }
    }

    pub fn set_config(&mut self, config: TomConfig) {
        self.config = config;
        self.params.pitch.set_target(config.pitch);
        self.params.color.set_target(config.color);
        self.params.tone.set_target(config.tone);
        self.params.bend.set_target(config.bend);
        self.params.decay.set_target(config.decay);
        self.params.decay_curve.set_target(config.decay_curve);
        self.params.volume.set_target(config.volume);
    }

    pub fn trigger(&mut self, time: f32) {
        self.trigger_with_velocity(time, 1.0);
    }

    pub fn trigger_with_velocity(&mut self, time: f32, velocity: f32) {
        self.current_velocity = velocity.clamp(0.0, 1.0);
        self.is_active = true;

        // Snapshot pitch ratio at trigger time
        let pitch_midi = ranges::denormalize(
            self.params.pitch.get(),
            ranges::PITCH_MIN,
            ranges::PITCH_MAX,
        );
        // MIDI 57 (A3, ~220 Hz) is the reference point
        self.triggered_pitch_ratio = 2.0_f32.powf((pitch_midi - 57.0) / 12.0);

        // Configure amplitude envelope with decay curve
        let decay_secs = ranges::denormalize(
            self.params.decay.get(),
            ranges::DECAY_MIN,
            ranges::DECAY_MAX,
        );
        let decay_curve_val = ranges::denormalize(
            self.params.decay_curve.get(),
            ranges::DECAY_CURVE_MIN,
            ranges::DECAY_CURVE_MAX,
        );

        let mut amp_config = ADSRConfig::new(0.001, decay_secs, 0.0, decay_secs * 0.2);
        amp_config.decay_curve = EnvelopeCurve::Exponential(decay_curve_val);
        self.amplitude_envelope.set_config(amp_config);
        self.amplitude_envelope.trigger(time);

        // Configure pitch envelope for bend
        let pitch_decay = decay_secs * 0.5; // Pitch settles faster than amplitude
        self.pitch_envelope.set_config(ADSRConfig::new(
            0.001,
            pitch_decay,
            0.0,
            pitch_decay * 0.1,
        ));
        self.pitch_envelope.trigger(time);

        // Reset filter states and noise generator
        self.filter_bank.reset();
        self.tone_filter.reset();
        self.pink_noise.reset();
    }

    pub fn release(&mut self, time: f32) {
        if self.is_active {
            self.amplitude_envelope.release(time);
            self.pitch_envelope.release(time);
        }
    }

    pub fn tick(&mut self, current_time: f32) -> f32 {
        self.params.tick();

        if !self.is_active {
            return 0.0;
        }

        // Get pitch modulation from bend envelope
        let pitch_env = self.pitch_envelope.get_amplitude(current_time);
        let bend_amount = self.params.bend.get();
        // Bend creates pitch drop: starts high, falls to base
        let pitch_mod = self.triggered_pitch_ratio * (1.0 + bend_amount * pitch_env);

        // Update filter bank frequencies with pitch modulation
        self.filter_bank.set_pitch_ratio(pitch_mod);

        // Update color
        self.filter_bank.apply_color(self.params.color.get());

        // Generate noise and process through filter bank
        let noise = self.pink_noise.tick();
        let filtered = self.filter_bank.process(noise);

        // Apply tone filter (lowpass)
        let tone_hz = ranges::denormalize(
            self.params.tone.get(),
            ranges::TONE_CUTOFF_MIN,
            ranges::TONE_CUTOFF_MAX,
        );
        self.tone_filter.set_cutoff_freq(tone_hz);
        let toned = self.tone_filter.process_mode(filtered, 0); // LP mode

        // Apply amplitude envelope
        let amp_env = self.amplitude_envelope.get_amplitude(current_time);
        let velocity_scale = self.current_velocity.sqrt();
        let output = toned * amp_env * velocity_scale * self.params.volume.get();

        // Check if still active
        if !self.amplitude_envelope.is_active {
            self.is_active = false;
        }

        output
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    // Parameter setters
    pub fn set_pitch(&mut self, pitch: f32) {
        self.params.pitch.set_target(pitch.clamp(0.0, 1.0));
        self.config.pitch = pitch.clamp(0.0, 1.0);
    }

    pub fn set_color(&mut self, color: f32) {
        self.params.color.set_target(color.clamp(0.0, 1.0));
        self.config.color = color.clamp(0.0, 1.0);
    }

    pub fn set_tone(&mut self, tone: f32) {
        self.params.tone.set_target(tone.clamp(0.0, 1.0));
        self.config.tone = tone.clamp(0.0, 1.0);
    }

    pub fn set_bend(&mut self, bend: f32) {
        self.params.bend.set_target(bend.clamp(0.0, 1.0));
        self.config.bend = bend.clamp(0.0, 1.0);
    }

    pub fn set_decay(&mut self, decay: f32) {
        self.params.decay.set_target(decay.clamp(0.0, 1.0));
        self.config.decay = decay.clamp(0.0, 1.0);
    }

    pub fn set_decay_curve(&mut self, decay_curve: f32) {
        self.params.decay_curve.set_target(decay_curve.clamp(0.0, 1.0));
        self.config.decay_curve = decay_curve.clamp(0.0, 1.0);
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.params.volume.set_target(volume.clamp(0.0, 1.0));
        self.config.volume = volume.clamp(0.0, 1.0);
    }
}

impl crate::engine::Instrument for TomDrum {
    fn trigger_with_velocity(&mut self, time: f32, velocity: f32) {
        TomDrum::trigger_with_velocity(self, time, velocity);
    }

    fn tick(&mut self, current_time: f32) -> f32 {
        self.tick(current_time)
    }

    fn is_active(&self) -> bool {
        self.is_active()
    }

    fn as_modulatable(&mut self) -> Option<&mut dyn crate::engine::Modulatable> {
        Some(self)
    }
}

impl crate::engine::Modulatable for TomDrum {
    fn modulatable_parameters(&self) -> Vec<&'static str> {
        vec!["pitch", "color", "tone", "bend", "decay", "decay_curve", "volume"]
    }

    fn apply_modulation(&mut self, parameter: &str, value: f32) -> Result<(), String> {
        match parameter {
            "pitch" => {
                self.params.pitch.set_bipolar(value);
                Ok(())
            }
            "color" => {
                self.params.color.set_bipolar(value);
                Ok(())
            }
            "tone" => {
                self.params.tone.set_bipolar(value);
                Ok(())
            }
            "bend" => {
                self.params.bend.set_bipolar(value);
                Ok(())
            }
            "decay" => {
                self.params.decay.set_bipolar(value);
                Ok(())
            }
            "decay_curve" => {
                self.params.decay_curve.set_bipolar(value);
                Ok(())
            }
            "volume" => {
                self.params.volume.set_bipolar(value);
                Ok(())
            }
            _ => Err(format!("Unknown parameter: {}", parameter)),
        }
    }

    fn parameter_range(&self, parameter: &str) -> Option<(f32, f32)> {
        match parameter {
            "pitch" | "color" | "tone" | "bend" | "decay" | "decay_curve" | "volume" => {
                Some((0.0, 1.0))
            }
            _ => None,
        }
    }
}
