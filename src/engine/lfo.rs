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

    /// Inverse of [`MusicalDivision::from_timing_constant`].
    pub fn timing_constant(&self) -> u32 {
        match self {
            MusicalDivision::FourBars => 0,
            MusicalDivision::TwoBars => 1,
            MusicalDivision::OneBar => 2,
            MusicalDivision::Half => 3,
            MusicalDivision::Quarter => 4,
            MusicalDivision::Eighth => 5,
            MusicalDivision::Sixteenth => 6,
            MusicalDivision::ThirtySecond => 7,
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

/// Nonzero xorshift seed for sample & hold.
const RNG_SEED: u32 = 0x9E37_79B9;

/// LFO waveform. Every shape is bipolar (-1 to 1) and, like sine, starts at 0
/// and rises, so switching shape keeps the cycle's alignment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LfoWaveform {
    #[default]
    Sine,
    /// Linear 0 → 1 → 0 → -1 → 0.
    Triangle,
    /// Linear ramp from 0 up to 1, jump to -1, ramp back to 0.
    Saw,
    /// 1 for the first half of the cycle, -1 for the second.
    Square,
    /// A random level held for each cycle.
    SampleHold,
}

impl LfoWaveform {
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Sine),
            1 => Some(Self::Triangle),
            2 => Some(Self::Saw),
            3 => Some(Self::Square),
            4 => Some(Self::SampleHold),
            _ => None,
        }
    }

    pub fn as_u32(self) -> u32 {
        self as u32
    }

    /// Value at `phase` (0-1). Sample & hold has no fixed shape and returns
    /// `held`, the level chosen for the current cycle.
    pub fn eval(self, phase: f32, held: f32) -> f32 {
        let phase = phase.rem_euclid(1.0);
        match self {
            Self::Sine => (phase * 2.0 * std::f32::consts::PI).sin(),
            Self::Triangle => {
                if phase < 0.25 {
                    4.0 * phase
                } else if phase < 0.75 {
                    2.0 - 4.0 * phase
                } else {
                    4.0 * phase - 4.0
                }
            }
            Self::Saw => 2.0 * (phase + 0.5).fract() - 1.0,
            Self::Square => {
                if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Self::SampleHold => held,
        }
    }
}

/// Low Frequency Oscillator for modulation
pub struct Lfo {
    sync_mode: LfoSyncMode,
    bpm: f32, // Current BPM (used when in BpmSync mode)
    phase: f32,
    sample_rate: f32,
    waveform: LfoWaveform,
    /// Latest `tick()` output.
    last_value: f32,
    /// Sample & hold level for the current cycle, its xorshift state, and the
    /// seed `reset` returns to so repeated renders match.
    held: f32,
    rng_state: u32,
    seed: u32,

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
            waveform: LfoWaveform::Sine,
            last_value: 0.0,
            held: 0.0,
            rng_state: RNG_SEED,
            seed: RNG_SEED,
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
            waveform: LfoWaveform::Sine,
            last_value: 0.0,
            held: 0.0,
            rng_state: RNG_SEED,
            seed: RNG_SEED,
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
            waveform: LfoWaveform::Sine,
            last_value: 0.0,
            held: 0.0,
            rng_state: RNG_SEED,
            seed: RNG_SEED,
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

    /// Set the waveform. Phase is kept, so the cycle stays aligned.
    pub fn set_waveform(&mut self, waveform: LfoWaveform) {
        self.waveform = waveform;
    }

    pub fn waveform(&self) -> LfoWaveform {
        self.waveform
    }

    /// Generate one sample and advance the phase
    /// Returns: offset + (waveform_value * amount)
    /// With default settings (amount=1.0, offset=0.0), this returns -1.0 to 1.0
    pub fn tick(&mut self) -> f32 {
        let value = self.waveform.eval(self.phase, self.held);

        // Advance phase
        let phase_increment = self.frequency() / self.sample_rate;
        self.phase += phase_increment;

        // Wrap phase to 0.0-1.0, picking the next sample & hold level
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            self.held = self.next_random();
        }

        // Apply offset and amount
        self.last_value = self.offset + (value * self.amount);
        self.last_value
    }

    /// Latest output of `tick()`, including amount and offset.
    pub fn value(&self) -> f32 {
        self.last_value
    }

    /// Seed sample & hold so LFOs sharing a timing don't produce identical
    /// levels. Zero (invalid for xorshift) falls back to the default seed.
    /// Restarts the random sequence.
    pub fn set_random_seed(&mut self, seed: u32) {
        self.seed = if seed == 0 { RNG_SEED } else { seed };
        self.reset();
    }

    /// Reset the phase to 0 and restart the sample & hold sequence, so a
    /// render after a reset (e.g. each bounce) modulates identically.
    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.held = 0.0;
        self.rng_state = self.seed;
    }

    /// Uniform value in -1 to 1 (xorshift32; allocation- and lock-free).
    fn next_random(&mut self) -> f32 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng_state = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Get the current phase (0.0 to 1.0)
    pub fn phase(&self) -> f32 {
        self.phase
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHAPES: [LfoWaveform; 4] = [
        LfoWaveform::Sine,
        LfoWaveform::Triangle,
        LfoWaveform::Saw,
        LfoWaveform::Square,
    ];

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn waveforms_hit_expected_points() {
        let points = [0.0, 0.25, 0.5, 0.75];
        let expected = [
            [0.0, 1.0, 0.0, -1.0],
            [0.0, 1.0, 0.0, -1.0],
            [0.0, 0.5, -1.0, -0.5],
            [1.0, 1.0, -1.0, -1.0],
        ];
        for (shape, values) in SHAPES.iter().zip(expected) {
            for (phase, value) in points.iter().zip(values) {
                assert!(
                    approx(shape.eval(*phase, 0.0), value),
                    "{shape:?} at {phase}: {}",
                    shape.eval(*phase, 0.0)
                );
            }
        }
    }

    #[test]
    fn waveform_constants_round_trip() {
        for value in 0..5 {
            assert_eq!(LfoWaveform::from_u32(value).unwrap().as_u32(), value);
        }
        assert_eq!(LfoWaveform::from_u32(5), None);
    }

    #[test]
    fn sample_hold_holds_within_a_cycle_and_changes_across_cycles() {
        // One cycle per 4 samples (an exact phase increment).
        let mut lfo = Lfo::new(1.0, 4.0);
        lfo.set_waveform(LfoWaveform::SampleHold);
        let mut cycles = Vec::new();
        for _ in 0..4 {
            let first = lfo.tick();
            for _ in 1..4 {
                assert_eq!(lfo.tick(), first);
            }
            assert!((-1.0..=1.0).contains(&first));
            cycles.push(first);
        }
        // The first cycle holds the initial level; later cycles are random.
        assert!(cycles[1] != cycles[2] || cycles[2] != cycles[3]);
    }

    fn sample_hold_cycles(lfo: &mut Lfo, cycles: usize) -> Vec<f32> {
        (0..cycles * 4).map(|_| lfo.tick()).step_by(4).collect()
    }

    #[test]
    fn reset_restarts_the_sample_hold_sequence() {
        let mut lfo = Lfo::new(1.0, 4.0);
        lfo.set_waveform(LfoWaveform::SampleHold);
        let first = sample_hold_cycles(&mut lfo, 6);
        lfo.reset();
        assert_eq!(sample_hold_cycles(&mut lfo, 6), first);
    }

    #[test]
    fn distinct_seeds_give_distinct_sample_hold_levels() {
        let mut a = Lfo::new(1.0, 4.0);
        let mut b = Lfo::new(1.0, 4.0);
        a.set_waveform(LfoWaveform::SampleHold);
        b.set_waveform(LfoWaveform::SampleHold);
        b.set_random_seed(12345);
        assert_ne!(sample_hold_cycles(&mut a, 6), sample_hold_cycles(&mut b, 6));
        // Zero is not a valid xorshift state; it falls back to the default.
        let mut c = Lfo::new(1.0, 4.0);
        c.set_waveform(LfoWaveform::SampleHold);
        c.set_random_seed(0);
        a.reset();
        assert_eq!(sample_hold_cycles(&mut c, 6), sample_hold_cycles(&mut a, 6));
    }

    #[test]
    fn value_reports_latest_tick_with_amount_and_offset() {
        let mut lfo = Lfo::new(1.0, 4.0);
        lfo.set_waveform(LfoWaveform::Square);
        lfo.amount = 0.5;
        lfo.offset = 0.25;
        assert_eq!(lfo.value(), 0.0);
        let output = lfo.tick();
        assert!(approx(output, 0.75));
        assert_eq!(lfo.value(), output);
    }
}
