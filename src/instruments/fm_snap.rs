use std::f32::consts::PI;

pub struct FMSnapSynthesizer {
    pub sample_rate: f32,
    pub attack_time: f32,
    pub decay_time: f32,
    pub carrier_freq: f32,
    pub modulator_freq: f32,
    pub modulation_index: f32,
    pub phase: f32,
    pub trigger_time: f32,
    pub is_active: bool,
}

impl FMSnapSynthesizer {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            attack_time: 0.001, // 1ms attack
            decay_time: 0.008,  // 8ms decay
            carrier_freq: 50.0,
            modulator_freq: 500.0,
            modulation_index: 2.0,
            phase: 0.0,
            trigger_time: 0.0,
            is_active: false,
        }
    }

    pub fn trigger(&mut self, time: f32) {
        self.trigger_time = time;
        self.phase = 0.0;
        self.is_active = true;
    }

    pub fn tick(&mut self, current_time: f32) -> f32 {
        if !self.is_active {
            return 0.0;
        }

        let t = current_time - self.trigger_time;

        // Check if we're past the envelope duration
        if t > self.attack_time + self.decay_time {
            self.is_active = false;
            return 0.0;
        }

        // Generate envelope (exponential decay)
        let env = if t < self.attack_time {
            t / self.attack_time
        } else {
            let decay = (-(t - self.attack_time) / self.decay_time).exp();
            decay.clamp(0.0, 1.0)
        };

        // FM synthesis
        let dt = 1.0 / self.sample_rate;
        let mod_signal = (2.0 * PI * self.modulator_freq * t).sin();
        let instantaneous_freq = self.carrier_freq + self.modulation_index * mod_signal * env;

        // Update phase
        self.phase += 2.0 * PI * instantaneous_freq * dt;

        // Wrap phase to prevent overflow
        if self.phase > 2.0 * PI {
            self.phase -= 2.0 * PI;
        }

        // Generate output
        let output = self.phase.sin() * env;

        output
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    pub fn set_params(
        &mut self,
        attack_time: f32,
        decay_time: f32,
        carrier_freq: f32,
        modulator_freq: f32,
        modulation_index: f32,
    ) {
        self.attack_time = attack_time;
        self.decay_time = decay_time;
        self.carrier_freq = carrier_freq;
        self.modulator_freq = modulator_freq;
        self.modulation_index = modulation_index;
    }
}

/// Phase modulator for DS Kick-style transients
/// Implements a two-stage envelope: 0→1 attack, 1→0 decay
/// Used to briefly boost oscillator frequency at note onset
///
/// DS Kick uses: "0, 1 1 -0.8 0 5. -0.6"
/// Meaning: 0→1 over 1ms with -0.8 curve, 1→0 over 5ms with -0.6 curve
pub struct PhaseModulator {
    pub sample_rate: f32,
    pub attack_time: f32,  // Time to peak (default 0.001s = 1ms)
    pub decay_time: f32,   // Time from peak to zero (default 0.005s = 5ms)
    pub attack_curve: f32, // Curve for attack (0.3 approximates Max's -0.8)
    pub decay_curve: f32,  // Curve for decay (0.4 approximates Max's -0.6)
    pub trigger_time: f32,
    pub is_active: bool,
}

impl PhaseModulator {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            attack_time: 0.001, // 1ms - matches DS Kick
            decay_time: 0.005,  // 5ms - matches DS Kick
            attack_curve: 0.3,  // Fast rise (approximates -0.8)
            decay_curve: 0.4,   // Medium fall (approximates -0.6)
            trigger_time: 0.0,
            is_active: false,
        }
    }

    pub fn trigger(&mut self, time: f32) {
        self.trigger_time = time;
        self.is_active = true;
    }

    /// Returns phase modulation value (0.0-1.0)
    /// This value can be used to modulate oscillator frequency
    pub fn tick(&mut self, current_time: f32) -> f32 {
        if !self.is_active {
            return 0.0;
        }

        let elapsed = current_time - self.trigger_time;
        let total_time = self.attack_time + self.decay_time;

        if elapsed > total_time {
            self.is_active = false;
            return 0.0;
        }

        if elapsed < self.attack_time {
            // Attack phase: 0→1 with curve
            let progress = elapsed / self.attack_time;
            progress.powf(self.attack_curve)
        } else {
            // Decay phase: 1→0 with curve
            let decay_elapsed = elapsed - self.attack_time;
            let progress = decay_elapsed / self.decay_time;
            1.0 - progress.powf(self.decay_curve)
        }
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    pub fn set_times(&mut self, attack_time: f32, decay_time: f32) {
        self.attack_time = attack_time.max(0.0001); // Minimum 0.1ms
        self.decay_time = decay_time.max(0.0001);
    }

    pub fn set_curves(&mut self, attack_curve: f32, decay_curve: f32) {
        self.attack_curve = attack_curve.clamp(0.1, 10.0);
        self.decay_curve = decay_curve.clamp(0.1, 10.0);
    }
}
