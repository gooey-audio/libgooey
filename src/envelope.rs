/// Curve shape for envelope phases
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EnvelopeCurve {
    /// Linear (default, maintains backward compatibility)
    Linear,
    /// Power curve: progress.powf(curvature)
    /// < 1.0: Fast initial change, slow approach to target (punchy pitch drop)
    /// > 1.0: Slow initial change, fast approach to target (softer)
    Exponential(f32),
}

impl Default for EnvelopeCurve {
    fn default() -> Self {
        EnvelopeCurve::Linear
    }
}

impl EnvelopeCurve {
    /// Apply the curve to a linear progress value (0.0 to 1.0)
    #[inline]
    pub fn apply(&self, progress: f32) -> f32 {
        match self {
            EnvelopeCurve::Linear => progress,
            EnvelopeCurve::Exponential(c) => progress.powf(c.clamp(0.1, 10.0)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ADSRConfig {
    pub attack_time: f32,            // seconds
    pub decay_time: f32,             // seconds
    pub sustain_level: f32,          // 0.0 to 1.0
    pub release_time: f32,           // seconds
    pub attack_curve: EnvelopeCurve, // Curve shape for attack phase
    pub decay_curve: EnvelopeCurve,  // Curve shape for decay phase
}

impl ADSRConfig {
    pub fn new(attack: f32, decay: f32, sustain: f32, release: f32) -> Self {
        Self {
            attack_time: attack.max(0.001), // Minimum attack to avoid artifacts
            decay_time: decay.max(0.001),   // Minimum decay
            sustain_level: sustain.clamp(0.0, 1.0),
            release_time: release.max(0.001),    // Minimum release
            attack_curve: EnvelopeCurve::Linear, // Default to linear for backward compatibility
            decay_curve: EnvelopeCurve::Linear,  // Default to linear for backward compatibility
        }
    }

    /// Builder method to set attack curve
    pub fn with_attack_curve(mut self, curve: EnvelopeCurve) -> Self {
        self.attack_curve = curve;
        self
    }

    /// Builder method to set decay curve
    pub fn with_decay_curve(mut self, curve: EnvelopeCurve) -> Self {
        self.decay_curve = curve;
        self
    }

    pub fn default() -> Self {
        Self::new(0.01, 0.3, 0.7, 0.5)
    }
}

pub struct Envelope {
    pub attack_time: f32,            // seconds
    pub decay_time: f32,             // seconds
    pub sustain_level: f32,          // 0.0 to 1.0
    pub release_time: f32,           // seconds
    pub attack_curve: EnvelopeCurve, // Curve shape for attack phase
    pub decay_curve: EnvelopeCurve,  // Curve shape for decay phase
    pub current_time: f32,           // current time in the envelope
    pub is_active: bool,
    pub trigger_time: f32,               // when the envelope was triggered
    pub release_time_start: Option<f32>, // when release was triggered
    pub reverse: bool,                   // when true, envelope plays backwards (silence → peak)
}

impl Envelope {
    pub fn new() -> Self {
        let config = ADSRConfig::default();
        Self::with_config(config)
    }

    pub fn with_config(config: ADSRConfig) -> Self {
        Self {
            attack_time: config.attack_time,
            decay_time: config.decay_time,
            sustain_level: config.sustain_level,
            release_time: config.release_time,
            attack_curve: config.attack_curve,
            decay_curve: config.decay_curve,
            current_time: 0.0,
            is_active: false,
            trigger_time: 0.0,
            release_time_start: None,
            reverse: false,
        }
    }

    pub fn set_config(&mut self, config: ADSRConfig) {
        self.attack_time = config.attack_time;
        self.decay_time = config.decay_time;
        self.sustain_level = config.sustain_level;
        self.release_time = config.release_time;
        self.attack_curve = config.attack_curve;
        self.decay_curve = config.decay_curve;
    }

    /// Set attack curve directly
    pub fn set_attack_curve(&mut self, curve: EnvelopeCurve) {
        self.attack_curve = curve;
    }

    /// Set decay curve directly
    pub fn set_decay_curve(&mut self, curve: EnvelopeCurve) {
        self.decay_curve = curve;
    }

    /// Set attack time directly (more efficient than set_config for modulation)
    pub fn set_attack_time(&mut self, attack_time: f32) {
        self.attack_time = attack_time;
    }

    /// Set decay time directly (more efficient than set_config for modulation)
    pub fn set_decay_time(&mut self, decay_time: f32) {
        self.decay_time = decay_time;
    }

    /// Set sustain level directly (more efficient than set_config for modulation)
    pub fn set_sustain_level(&mut self, sustain_level: f32) {
        self.sustain_level = sustain_level;
    }

    /// Set release time directly (more efficient than set_config for modulation)
    pub fn set_release_time(&mut self, release_time: f32) {
        self.release_time = release_time;
    }

    /// Set whether the envelope plays in reverse (silence → peak)
    pub fn set_reverse(&mut self, reverse: bool) {
        self.reverse = reverse;
    }

    pub fn trigger(&mut self, time: f32) {
        self.is_active = true;
        self.trigger_time = time;
        self.current_time = 0.0;
        self.release_time_start = None;
        self.reverse = false;
    }

    pub fn release(&mut self, time: f32) {
        if self.is_active && self.release_time_start.is_none() {
            self.release_time_start = Some(time);
        }
    }

    pub fn get_amplitude(&mut self, current_time: f32) -> f32 {
        if !self.is_active {
            return 0.0;
        }

        let real_elapsed = current_time - self.trigger_time;
        self.current_time = real_elapsed;

        // Reverse mode: play envelope backwards (silence → peak)
        if self.reverse {
            let total_duration = self.attack_time + self.decay_time;
            if real_elapsed >= total_duration {
                self.is_active = false;
                return 1.0; // End at peak
            }
            // Reverse the time: feed (total_duration - elapsed) into the forward envelope
            let elapsed = total_duration - real_elapsed;
            return self.forward_amplitude(elapsed);
        }

        let elapsed = real_elapsed;

        // Check if we're in release phase
        if let Some(release_start) = self.release_time_start {
            let release_elapsed = current_time - release_start;
            if release_elapsed < self.release_time {
                // Calculate amplitude at release start
                let release_amplitude = if elapsed < self.attack_time {
                    // Released during attack - apply attack curve
                    let attack_progress = elapsed / self.attack_time;
                    self.attack_curve.apply(attack_progress)
                } else if elapsed < self.attack_time + self.decay_time {
                    // Released during decay
                    let decay_elapsed = elapsed - self.attack_time;
                    let decay_progress = decay_elapsed / self.decay_time;
                    let curved_progress = self.decay_curve.apply(decay_progress);
                    1.0 - (1.0 - self.sustain_level) * curved_progress
                } else {
                    // Released during sustain
                    self.sustain_level
                };

                // Apply release envelope
                let release_progress = release_elapsed / self.release_time;
                release_amplitude * (1.0 - release_progress)
            } else {
                // Release phase complete
                self.is_active = false;
                0.0
            }
        } else {
            self.forward_amplitude(elapsed)
        }
    }

    /// Calculate forward envelope amplitude for a given elapsed time.
    /// Used by both normal and reverse modes.
    fn forward_amplitude(&mut self, elapsed: f32) -> f32 {
        if elapsed < self.attack_time {
            // Attack phase - apply attack curve
            let attack_progress = elapsed / self.attack_time;
            self.attack_curve.apply(attack_progress)
        } else if elapsed < self.attack_time + self.decay_time {
            // Decay phase
            let decay_elapsed = elapsed - self.attack_time;
            let decay_progress = decay_elapsed / self.decay_time;
            let curved_progress = self.decay_curve.apply(decay_progress);
            1.0 - (1.0 - self.sustain_level) * curved_progress
        } else {
            // Sustain phase (holds until release is triggered)
            // For drums with 0.0 sustain, automatically trigger release
            if self.sustain_level == 0.0 && self.release_time_start.is_none() {
                self.release_time_start = Some(self.trigger_time + elapsed);
            }
            self.sustain_level
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reverse_envelope_starts_near_zero() {
        let config = ADSRConfig::new(0.01, 0.3, 0.0, 0.01);
        let mut env = Envelope::with_config(config);
        env.trigger(0.0);
        env.set_reverse(true);

        // At the very start of reverse, the forward envelope is near its end (silence)
        let amp = env.get_amplitude(0.0);
        assert!(amp < 0.01, "Reverse envelope should start near 0, got {}", amp);
    }

    #[test]
    fn test_reverse_envelope_peaks_near_end() {
        let config = ADSRConfig::new(0.01, 0.3, 0.0, 0.01);
        let mut env = Envelope::with_config(config);
        env.trigger(0.0);
        env.set_reverse(true);

        // In reverse, the forward peak (at attack_time) maps to
        // real_elapsed = total_duration - attack_time
        let total_duration = 0.01 + 0.3;
        let peak_time = total_duration - 0.01; // decay duration from start
        let amp = env.get_amplitude(peak_time);
        assert!(amp > 0.9, "Reverse envelope should peak near the end, got {}", amp);
    }

    #[test]
    fn test_reverse_envelope_deactivates_after_duration() {
        let config = ADSRConfig::new(0.01, 0.3, 0.0, 0.01);
        let mut env = Envelope::with_config(config);
        env.trigger(0.0);
        env.set_reverse(true);

        let total_duration = 0.01 + 0.3;
        let _ = env.get_amplitude(total_duration + 0.01);
        assert!(!env.is_active, "Reverse envelope should deactivate after total duration");
    }

    #[test]
    fn test_forward_envelope_unchanged() {
        // Ensure forward mode still works correctly
        let config = ADSRConfig::new(0.01, 0.3, 0.0, 0.01);
        let mut env = Envelope::with_config(config);
        env.trigger(0.0);

        // At start of attack, should be near 0
        let amp = env.get_amplitude(0.0);
        assert!(amp < 0.01, "Forward should start near 0, got {}", amp);

        // At end of attack, should be near peak
        let amp = env.get_amplitude(0.01);
        assert!(amp > 0.9, "Forward should peak after attack, got {}", amp);
    }
}
