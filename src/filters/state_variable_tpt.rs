use std::f32::consts::PI;

/// State Variable Filter (TPT/ZDF) - stable at high cutoff
///
/// This implementation uses the topology-preserving transform form described
/// by Andrew Simper. It provides lowpass, bandpass, and highpass outputs with
/// good behavior near Nyquist (useful for percussive tone shaping).
pub struct StateVariableFilterTpt {
    sample_rate: f32,
    cutoff_freq: f32,
    resonance: f32, // Q factor

    g: f32,
    r: f32,
    h: f32,

    ic1eq: f32,
    ic2eq: f32,
}

impl StateVariableFilterTpt {
    pub fn new(sample_rate: f32, cutoff_freq: f32, resonance: f32) -> Self {
        let mut filter = Self {
            sample_rate,
            cutoff_freq: cutoff_freq.clamp(20.0, 20000.0),
            resonance: resonance.max(0.5),
            g: 0.0,
            r: 0.0,
            h: 0.0,
            ic1eq: 0.0,
            ic2eq: 0.0,
        };
        filter.update_coefficients();
        filter
    }

    pub fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }

    fn update_coefficients(&mut self) {
        let cutoff = self.cutoff_freq.clamp(20.0, self.sample_rate * 0.45);
        let q = self.resonance.max(0.5);

        let g = (PI * cutoff / self.sample_rate).tan();
        let r = 1.0 / q;
        let h = 1.0 / (1.0 + r * g + g * g);

        self.g = g;
        self.r = r;
        self.h = h;
    }

    #[inline]
    pub fn process_all(&mut self, input: f32) -> (f32, f32, f32) {
        // TPT SVF core
        let v1 = (self.g * (input - self.ic2eq) + self.ic1eq) * self.h;
        let v2 = self.ic2eq + self.g * v1;

        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;

        let low = v2;
        let band = v1;
        let high = input - (self.r * v1 + v2);

        (low, band, high)
    }

    #[inline]
    pub fn process_mode(&mut self, input: f32, filter_type: u8) -> f32 {
        let (low, band, high) = self.process_all(input);
        match filter_type {
            0 => low,
            1 => band,
            2 => high,
            3 => low + high,
            _ => band,
        }
    }

    pub fn set_params(&mut self, cutoff_freq: f32, resonance: f32) {
        let new_cutoff = cutoff_freq.clamp(20.0, self.sample_rate * 0.45);
        let new_res = resonance.max(0.5);
        if (new_cutoff - self.cutoff_freq).abs() > 0.001 || (new_res - self.resonance).abs() > 0.001
        {
            self.cutoff_freq = new_cutoff;
            self.resonance = new_res;
            self.update_coefficients();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tpt_svf_creation() {
        let filter = StateVariableFilterTpt::new(44100.0, 1000.0, 1.0);
        assert_eq!(filter.sample_rate, 44100.0);
        assert_eq!(filter.cutoff_freq, 1000.0);
        assert_eq!(filter.resonance, 1.0);
    }

    #[test]
    fn test_tpt_svf_reset() {
        let mut filter = StateVariableFilterTpt::new(44100.0, 1000.0, 1.0);
        for _ in 0..100 {
            filter.process_all(1.0);
        }
        filter.reset();
        assert_eq!(filter.ic1eq, 0.0);
        assert_eq!(filter.ic2eq, 0.0);
    }
}
