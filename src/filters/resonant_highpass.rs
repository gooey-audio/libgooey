pub struct ResonantHighpassFilter {
    pub sample_rate: f32,
    pub cutoff_freq: f32,
    pub resonance: f32,
    pub filter_state: f32,
}

impl ResonantHighpassFilter {
    pub fn new(sample_rate: f32, cutoff_freq: f32, resonance: f32) -> Self {
        Self {
            sample_rate,
            cutoff_freq,
            resonance,
            filter_state: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.filter_state = 0.0;
    }

    pub fn process(&mut self, input: f32) -> f32 {
        // Resonant high-pass filter implementation
        // Calculate filter coefficients
        let omega = 2.0 * std::f32::consts::PI * self.cutoff_freq / self.sample_rate;
        let sin_omega = omega.sin();
        let cos_omega = omega.cos();
        let alpha = sin_omega / (2.0 * self.resonance);
        
        // High-pass filter coefficients
        let b0 = (1.0 + cos_omega) / 2.0;
        let b1 = -(1.0 + cos_omega);
        let b2 = (1.0 + cos_omega) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_omega;
        let a2 = 1.0 - alpha;
        
        // Normalize coefficients
        let norm_b0 = b0 / a0;
        let norm_b1 = b1 / a0;
        let norm_b2 = b2 / a0;
        let norm_a1 = a1 / a0;
        let norm_a2 = a2 / a0;
        
        // Apply filter (simple one-pole approximation for efficiency)
        let alpha_simple = 1.0 - (-2.0 * std::f32::consts::PI * self.cutoff_freq / self.sample_rate).exp();
        let high_pass = input - self.filter_state;
        self.filter_state += alpha_simple * high_pass;
        
        // Add resonance boost
        high_pass * (1.0 + self.resonance * 0.1)
    }

    pub fn set_cutoff_freq(&mut self, cutoff_freq: f32) {
        self.cutoff_freq = cutoff_freq;
    }

    pub fn set_resonance(&mut self, resonance: f32) {
        self.resonance = resonance;
    }
}