/// Musical time divisions for BPM-synced LFO speeds
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MusicalDivision {
    /// 4 bars (16 beats)
    FourBars,
    /// 2 bars (8 beats)
    TwoBars,
    /// 1 bar (4 beats)
    OneBar,
    /// Half note (2 beats)
    Half,
    /// Quarter note (1 beat)
    Quarter,
    /// Eighth note (1/2 beat)
    Eighth,
    /// Sixteenth note (1/4 beat)
    Sixteenth,
    /// Thirty-second note (1/8 beat)
    ThirtySecond,
}

impl MusicalDivision {
    /// Get the number of beats this division represents
    pub fn beats(&self) -> f32 {
        match self {
            MusicalDivision::FourBars => 16.0,
            MusicalDivision::TwoBars => 8.0,
            MusicalDivision::OneBar => 4.0,
            MusicalDivision::Half => 2.0,
            MusicalDivision::Quarter => 1.0,
            MusicalDivision::Eighth => 0.5,
            MusicalDivision::Sixteenth => 0.25,
            MusicalDivision::ThirtySecond => 0.125,
        }
    }

    /// Convert to frequency in Hz at the given BPM
    pub fn to_frequency(&self, bpm: f32) -> f32 {
        // Beats per second = BPM / 60
        let beats_per_second = bpm / 60.0;
        // Cycles per second = beats per second / beats per cycle
        beats_per_second / self.beats()
    }

    /// Convert from u32 timing constant (used by FFI)
    /// Returns None if the value is out of range
    pub fn from_timing_constant(value: u32) -> Option<Self> {
        match value {
            0 => Some(MusicalDivision::FourBars),
            1 => Some(MusicalDivision::TwoBars),
            2 => Some(MusicalDivision::OneBar),
            3 => Some(MusicalDivision::Half),
            4 => Some(MusicalDivision::Quarter),
            5 => Some(MusicalDivision::Eighth),
            6 => Some(MusicalDivision::Sixteenth),
            7 => Some(MusicalDivision::ThirtySecond),
            _ => None,
        }
    }
}

/// LFO sync mode
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LfoSyncMode {
    /// Free-running at a specific Hz frequency
    Hz(f32),
    /// Synced to BPM with a musical division
    BpmSync(MusicalDivision),
}

/// Low Frequency Oscillator for modulation
pub struct Lfo {
    sync_mode: LfoSyncMode,
    bpm: f32, // Current BPM (used when in BpmSync mode)
    phase: f32,
    sample_rate: f32,

    // Routing
    pub target_instrument: String,
    pub target_parameter: String,
    pub amount: f32,
    pub offset: f32, // Center point (-1.0 to 1.0)
}

impl Lfo {
    /// Create a new LFO in Hz mode
    /// - frequency: LFO frequency in Hz
    /// - sample_rate: Audio sample rate
    pub fn new(frequency: f32, sample_rate: f32) -> Self {
        Self {
            sync_mode: LfoSyncMode::Hz(frequency),
            bpm: 120.0, // Default BPM
            phase: 0.0,
            sample_rate,
            target_instrument: String::new(),
            target_parameter: String::new(),
            amount: 1.0,
            offset: 0.0,
        }
    }

    /// Create a new LFO with default settings (quarter note timing at 120 BPM)
    /// Used for FFI pool initialization
    pub fn with_sample_rate(sample_rate: f32) -> Self {
        Self {
            sync_mode: LfoSyncMode::BpmSync(MusicalDivision::Quarter),
            bpm: 120.0,
            phase: 0.0,
            sample_rate,
            target_instrument: String::new(),
            target_parameter: String::new(),
            amount: 1.0,
            offset: 0.0,
        }
    }

    /// Set the sample rate (used when sample rate changes)
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }

    /// Create a new BPM-synced LFO
    /// - division: Musical time division (e.g., OneBar, Sixteenth)
    /// - bpm: Beats per minute
    /// - sample_rate: Audio sample rate
    pub fn new_synced(division: MusicalDivision, bpm: f32, sample_rate: f32) -> Self {
        Self {
            sync_mode: LfoSyncMode::BpmSync(division),
            bpm,
            phase: 0.0,
            sample_rate,
            target_instrument: String::new(),
            target_parameter: String::new(),
            amount: 1.0,
            offset: 0.0,
        }
    }

    /// Set the frequency in Hz (switches to Hz mode)
    pub fn set_frequency(&mut self, frequency: f32) {
        self.sync_mode = LfoSyncMode::Hz(frequency);
    }

    /// Set BPM sync mode with a musical division
    pub fn set_sync_mode(&mut self, division: MusicalDivision) {
        self.sync_mode = LfoSyncMode::BpmSync(division);
    }

    /// Update the BPM (used when in BpmSync mode)
    pub fn set_bpm(&mut self, bpm: f32) {
        self.bpm = bpm;
    }

    /// Get the current frequency in Hz
    pub fn frequency(&self) -> f32 {
        match self.sync_mode {
            LfoSyncMode::Hz(freq) => freq,
            LfoSyncMode::BpmSync(division) => division.to_frequency(self.bpm),
        }
    }

    /// Get the current sync mode
    pub fn sync_mode(&self) -> LfoSyncMode {
        self.sync_mode
    }

    /// Generate one sample and advance the phase
    /// Returns: offset + (sine_value * amount)
    /// With default settings (amount=1.0, offset=0.0), this returns -1.0 to 1.0
    pub fn tick(&mut self) -> f32 {
        // Calculate sine wave
        let value = (self.phase * 2.0 * std::f32::consts::PI).sin();

        // Advance phase
        let phase_increment = self.frequency() / self.sample_rate;
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
