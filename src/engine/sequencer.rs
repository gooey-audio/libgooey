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

    // The step that is currently being played (for UI display)
    // This is the step that most recently triggered, not the next one
    playhead_step: usize,

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
            playhead_step: 0,
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
            playhead_step: 0,
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
        self.playhead_step = 0;
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

    /// Get the current playhead step (the step currently being played)
    /// This is suitable for UI display
    pub fn current_step(&self) -> usize {
        self.playhead_step
    }

    /// Get the next step that will be triggered (internal use)
    pub fn next_step(&self) -> usize {
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
            // Update playhead to show the step that's about to play
            self.playhead_step = self.current_step;

            // Check if this step should trigger
            if self.pattern[self.current_step] {
                should_trigger = Some(self.instrument_name.as_str());
            }

            // Advance to the next step (internal tracking)
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

    /// Get the current sample count
    pub fn sample_count(&self) -> u64 {
        self.sample_count
    }

    /// Get the sample number when the current step started
    /// This can be used to calculate how far into the current step we are
    pub fn current_step_start_sample(&self) -> u64 {
        // The current step started at the previous trigger point
        // which is next_trigger_sample - samples_per_step
        if self.next_trigger_sample as f32 > self.samples_per_step {
            (self.next_trigger_sample as f32 - self.samples_per_step) as u64
        } else {
            0
        }
    }

    /// Get samples per step (useful for UI timing calculations)
    pub fn samples_per_step(&self) -> f32 {
        self.samples_per_step
    }

    /// Get the step that will be playing after a given number of samples
    /// This is useful for UI display to compensate for audio latency
    ///
    /// lookahead_samples: How many samples ahead to look (e.g., audio buffer size)
    pub fn step_at_lookahead(&self, lookahead_samples: u64) -> usize {
        if !self.is_running || self.pattern.is_empty() {
            return self.playhead_step;
        }

        let future_sample = self.sample_count + lookahead_samples;

        // Calculate how many steps ahead this puts us
        if future_sample >= self.next_trigger_sample {
            // We've crossed into future steps
            let samples_past_next = future_sample - self.next_trigger_sample;
            let additional_steps = (samples_past_next as f32 / self.samples_per_step) as usize;
            (self.current_step + additional_steps) % self.pattern.len()
        } else {
            // Still on the current playhead step
            self.playhead_step
        }
    }
}
