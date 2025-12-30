/// Low Frequency Oscillator for modulation
pub struct Lfo {
    frequency: f32,
    phase: f32,
    sample_rate: f32,
    
    // Routing
    pub target_instrument: String,
    pub target_parameter: String,
    pub amount: f32,
    pub offset: f32, // Center point (-1.0 to 1.0)
}

impl Lfo {
    /// Create a new LFO
    /// - frequency: LFO frequency in Hz
    /// - sample_rate: Audio sample rate
    pub fn new(frequency: f32, sample_rate: f32) -> Self {
        Self {
            frequency,
            phase: 0.0,
            sample_rate,
            target_instrument: String::new(),
            target_parameter: String::new(),
            amount: 1.0,
            offset: 0.0,
        }
    }
    
    /// Set the frequency in Hz
    pub fn set_frequency(&mut self, frequency: f32) {
        self.frequency = frequency;
    }
    
    /// Get the current frequency
    pub fn frequency(&self) -> f32 {
        self.frequency
    }
    
    /// Generate one sample and advance the phase
    /// Returns a value from -1.0 to 1.0 (sine wave)
    pub fn tick(&mut self) -> f32 {
        // Calculate sine wave
        let value = (self.phase * 2.0 * std::f32::consts::PI).sin();
        
        // Advance phase
        let phase_increment = self.frequency / self.sample_rate;
        self.phase += phase_increment;
        
        // Wrap phase to 0.0-1.0
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        
        // Apply offset and amount
        self.offset + (value * self.amount)
    }
    
    /// Reset the phase to 0
    pub fn reset(&mut self) {
        self.phase = 0.0;
    }
    
    /// Get the current phase (0.0 to 1.0)
    pub fn phase(&self) -> f32 {
        self.phase
    }
}

