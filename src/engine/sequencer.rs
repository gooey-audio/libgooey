/// A sample-accurate step sequencer
pub struct Sequencer {
    bpm: f32,
    sample_rate: f32,

    // Sample-accurate timing
    sample_count: u64,
    next_trigger_sample: u64,
    samples_per_step: f32,

    // Pattern and current position
    pattern: Vec<bool>,
    current_step: usize,

    // Instrument to trigger
    instrument_name: String,

    // Whether the sequencer is running
    is_running: bool,
}

impl Sequencer {
    /// Create a new sequencer
    /// - bpm: Beats per minute
    /// - sample_rate: Audio sample rate
    /// - beat_count: Number of steps in the pattern
    /// - instrument_name: Name of the instrument to trigger
    pub fn new(
        bpm: f32,
        sample_rate: f32,
        beat_count: usize,
        instrument_name: impl Into<String>,
    ) -> Self {
        let samples_per_step = Self::calculate_samples_per_step(bpm, sample_rate);

        // Initialize with all steps enabled
        let pattern = vec![true; beat_count];

        Self {
            bpm,
            sample_rate,
            sample_count: 0,
            next_trigger_sample: 0,
            samples_per_step,
            pattern,
            current_step: 0,
            instrument_name: instrument_name.into(),
            is_running: false,
        }
    }

    /// Create a sequencer with a custom pattern
    pub fn with_pattern(
        bpm: f32,
        sample_rate: f32,
        pattern: Vec<bool>,
        instrument_name: impl Into<String>,
    ) -> Self {
        let samples_per_step = Self::calculate_samples_per_step(bpm, sample_rate);

        Self {
            bpm,
            sample_rate,
            sample_count: 0,
            next_trigger_sample: 0,
            samples_per_step,
            pattern,
            current_step: 0,
            instrument_name: instrument_name.into(),
            is_running: false,
        }
    }

    /// Calculate how many samples represent one step at the given BPM and sample rate
    /// This uses 16th notes as the base unit
    fn calculate_samples_per_step(bpm: f32, sample_rate: f32) -> f32 {
        // One quarter note = 60 seconds / BPM
        // One 16th note = (60 / BPM) / 4
        let seconds_per_16th = (60.0 / bpm) / 4.0;
        seconds_per_16th * sample_rate
    }

    /// Start the sequencer
    pub fn start(&mut self) {
        self.is_running = true;
        self.next_trigger_sample = self.sample_count;
    }

    /// Stop the sequencer
    pub fn stop(&mut self) {
        self.is_running = false;
    }

    /// Reset the sequencer to step 0
    pub fn reset(&mut self) {
        self.sample_count = 0;
        self.next_trigger_sample = 0;
        self.current_step = 0;
    }

    /// Set the BPM and recalculate timing
    pub fn set_bpm(&mut self, bpm: f32) {
        self.bpm = bpm;
        self.samples_per_step = Self::calculate_samples_per_step(bpm, self.sample_rate);
    }

    /// Set a step in the pattern
    pub fn set_step(&mut self, step: usize, enabled: bool) {
        if step < self.pattern.len() {
            self.pattern[step] = enabled;
        }
    }

    /// Get the pattern
    pub fn pattern(&self) -> &[bool] {
        &self.pattern
    }

    /// Set the entire pattern
    pub fn set_pattern(&mut self, pattern: Vec<bool>) {
        self.pattern = pattern;
        // Reset to beginning if current step is beyond new pattern length
        if self.current_step >= self.pattern.len() {
            self.current_step = 0;
        }
    }

    /// Get the current step
    pub fn current_step(&self) -> usize {
        self.current_step
    }

    /// Check if the sequencer is running
    pub fn is_running(&self) -> bool {
        self.is_running
    }

    /// Get the instrument name this sequencer triggers
    pub fn instrument_name(&self) -> &str {
        &self.instrument_name
    }

    /// Process one sample and return the instrument name if it should be triggered
    /// Returns Some(instrument_name) if a trigger should happen, None otherwise
    pub fn tick(&mut self) -> Option<&str> {
        if !self.is_running || self.pattern.is_empty() {
            self.sample_count += 1;
            return None;
        }

        let mut should_trigger = None;

        // Check if we've reached the next trigger point
        if self.sample_count >= self.next_trigger_sample {
            // Check if this step should trigger
            if self.pattern[self.current_step] {
                should_trigger = Some(self.instrument_name.as_str());
            }

            // Advance to the next step
            self.current_step = (self.current_step + 1) % self.pattern.len();

            // Calculate the next trigger sample (accumulate fractional samples for accuracy)
            self.next_trigger_sample =
                (self.next_trigger_sample as f32 + self.samples_per_step).round() as u64;
        }

        self.sample_count += 1;
        should_trigger
    }

    /// Get BPM
    pub fn bpm(&self) -> f32 {
        self.bpm
    }
}
