use crate::envelope::{ADSRConfig, Envelope};
use crate::gen::polyblep;
use crate::gen::waveform::Waveform;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub struct Oscillator {
    pub sample_rate: f32,
    pub waveform: Waveform,
    pub current_sample_index: f32,
    pub frequency_hz: f32,
    pub envelope: Envelope,
    pub volume: f32,
    pub modulator_frequency_hz: f32,
    pub enabled: bool,
}

impl Oscillator {
    pub fn new(sample_rate: f32, frequency_hz: f32) -> Self {
        Self {
            sample_rate,
            waveform: Waveform::Square,
            current_sample_index: 0.0,
            frequency_hz,
            envelope: Envelope::new(),
            volume: 1.0,
            modulator_frequency_hz: frequency_hz * 0.5, // Default modulator at half carrier frequency
            enabled: true,
        }
    }

    // fn advance_sample(&mut self) {
    //     self.current_sample_index = (self.current_sample_index + 1.0) % self.sample_rate;
    // }

    fn calculate_sine_output_from_freq(&self, freq: f32) -> f32 {
        let two_pi = 2.0 * std::f32::consts::PI;
        // current_sample_index is now in samples, so use the original calculation
        (self.current_sample_index * freq * two_pi / self.sample_rate).sin()
    }

    fn is_multiple_of_freq_above_nyquist(&self, multiple: f32) -> bool {
        self.frequency_hz * multiple > self.sample_rate / 2.0
    }

    // fn sine_wave(&mut self) -> f32 {
    //     self.advance_sample();
    //     self.calculate_sine_output_from_freq(self.frequency_hz)
    // }

    // fn generative_waveform(&mut self, harmonic_index_increment: i32, gain_exponent: f32) -> f32 {
    //     self.advance_sample();
    //     let mut output = 0.0;
    //     let mut i = 1;
    //     while !self.is_multiple_of_freq_above_nyquist(i as f32) {
    //         let gain = 1.0 / (i as f32).powf(gain_exponent);
    //         output += gain * self.calculate_sine_output_from_freq(self.frequency_hz * i as f32);
    //         i += harmonic_index_increment;
    //     }
    //     output
    // }

    // fn square_wave(&mut self) -> f32 {
    //     self.generative_waveform(2, 1.0)
    // }

    // fn saw_wave(&mut self) -> f32 {
    //     self.generative_waveform(1, 1.0)
    // }

    // fn triangle_wave(&mut self) -> f32 {
    //     self.generative_waveform(2, 2.0)
    // }

    // fn ring_mod_wave(&mut self) -> f32 {
    //     self.advance_sample();
    //     let carrier = self.calculate_sine_output_from_freq(self.frequency_hz);
    //     let modulator = self.calculate_sine_output_from_freq(self.modulator_frequency_hz);
    //     carrier * modulator
    // }

    // fn noise_wave(&mut self) -> f32 {
    //     self.advance_sample();

    //     // Use current sample index to generate pseudo-random noise
    //     let mut hasher = DefaultHasher::new();
    //     (self.current_sample_index as u64).hash(&mut hasher);
    //     let hash = hasher.finish();

    //     // Convert hash to float in range [-1.0, 1.0]
    //     let normalized = (hash as f32) / (u64::MAX as f32);
    //     (normalized * 2.0) - 1.0
    // }

    // Time-based waveform methods that don't use advance_sample()
    fn sine_wave_time_based(&self) -> f32 {
        self.calculate_sine_output_from_freq(self.frequency_hz)
    }

    fn generative_waveform_time_based(
        &self,
        harmonic_index_increment: i32,
        gain_exponent: f32,
    ) -> f32 {
        let mut output = 0.0;
        let mut i = 1;
        let nyquist = self.sample_rate / 2.0;
        let max_harmonics = (nyquist / self.frequency_hz) as i32;

        while i <= max_harmonics && !self.is_multiple_of_freq_above_nyquist(i as f32) {
            let gain = 1.0 / (i as f32).powf(gain_exponent);
            // Gibbs taper: smooth rolloff for harmonics in the top 25% of bandwidth
            let harmonic_freq = self.frequency_hz * i as f32;
            let ratio = harmonic_freq / nyquist;
            let taper = if ratio > 0.75 {
                let t = (ratio - 0.75) / 0.25;
                1.0 - t * t
            } else {
                1.0
            };
            output += gain * taper * self.calculate_sine_output_from_freq(harmonic_freq);
            i += harmonic_index_increment;
        }
        output
    }

    fn polyblep_phase(&self) -> (f64, f64) {
        let phase_inc = self.frequency_hz as f64 / self.sample_rate as f64;
        let phase = (self.current_sample_index as f64 * phase_inc) % 1.0;
        (phase, phase_inc)
    }

    fn square_wave_time_based(&self) -> f32 {
        let (phase, phase_inc) = self.polyblep_phase();
        polyblep::polyblep_square(phase, phase_inc)
    }

    fn saw_wave_time_based(&self) -> f32 {
        let (phase, phase_inc) = self.polyblep_phase();
        polyblep::polyblep_saw(phase, phase_inc)
    }

    fn triangle_wave_time_based(&self) -> f32 {
        self.generative_waveform_time_based(2, 2.0)
    }

    fn ring_mod_wave_time_based(&self) -> f32 {
        let carrier = self.calculate_sine_output_from_freq(self.frequency_hz);
        let modulator = self.calculate_sine_output_from_freq(self.modulator_frequency_hz);
        carrier * modulator
    }

    fn noise_wave_time_based(&self) -> f32 {
        // Use current sample index to generate pseudo-random noise
        let mut hasher = DefaultHasher::new();
        (self.current_sample_index as u64).hash(&mut hasher);
        let hash = hasher.finish();

        // Convert hash to float in range [-1, 1.0]
        let normalized = (hash as f32) / (u64::MAX as f32);
        (normalized * 2.0) - 1.0
    }

    pub fn trigger(&mut self, time: f64) {
        self.envelope.trigger(time);
        // Reset phase for consistent sound on each trigger
        self.current_sample_index = 0.0;
    }

    pub fn release(&mut self, time: f64) {
        self.envelope.release(time);
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
    }

    pub fn set_adsr(&mut self, config: ADSRConfig) {
        self.envelope.set_config(config);
    }

    pub fn set_modulator_frequency(&mut self, frequency_hz: f32) {
        self.modulator_frequency_hz = frequency_hz.max(0.0);
    }

    pub fn get_modulator_frequency(&self) -> f32 {
        self.modulator_frequency_hz
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn tick(&mut self, current_time: f64) -> f32 {
        if !self.enabled {
            return 0.0;
        }

        // Update phase based on time elapsed since trigger
        let elapsed_since_trigger = if self.envelope.is_active {
            (current_time - self.envelope.trigger_time) as f32
        } else {
            0.0
        };

        // Calculate phase in samples for consistent waveform generation
        self.current_sample_index = elapsed_since_trigger * self.sample_rate;

        let raw_output = match self.waveform {
            Waveform::Sine => self.sine_wave_time_based(),
            Waveform::Square => self.square_wave_time_based(),
            Waveform::Saw => self.saw_wave_time_based(),
            Waveform::Triangle => self.triangle_wave_time_based(),
            Waveform::RingMod => self.ring_mod_wave_time_based(),
            Waveform::Noise => self.noise_wave_time_based(),
        };

        let envelope_amplitude = self.envelope.get_amplitude(current_time);
        raw_output * envelope_amplitude * self.volume
    }
}
