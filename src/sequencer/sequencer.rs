/// A sample-accurate step sequencer that triggers callbacks on subdivisions of the beat.
/// Currently supports 8th note subdivisions.
pub struct Sequencer {
    pub bpm: f32,
    pub sample_rate: f32,

    // Sample-accurate timing
    sample_count: u64,
    next_trigger_sample: u64,
    samples_per_8th_note: f32,

    // Current step (0-based)
    current_step: usize,

    // Whether the sequencer is running
    is_running: bool,
}

impl Sequencer {
    /// Create a new sequencer with the given BPM and sample rate
    pub fn new(bpm: f32, sample_rate: f32) -> Self {
        let samples_per_8th_note = Self::calculate_samples_per_8th_note(bpm, sample_rate);

        Self {
            bpm,
            sample_rate,
            sample_count: 0,
            next_trigger_sample: 0,
            samples_per_8th_note,
            current_step: 0,
            is_running: false,
        }
    }

    /// Calculate how many samples represent one 8th note at the given BPM and sample rate
    fn calculate_samples_per_8th_note(bpm: f32, sample_rate: f32) -> f32 {
        // One quarter note = 60 seconds / BPM
        // One 8th note = (60 / BPM) / 2
        let seconds_per_8th_note = (60.0 / bpm) / 2.0;
        seconds_per_8th_note * sample_rate
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

    /// Reset the sequencer to step 0 and sample count 0
    pub fn reset(&mut self) {
        self.sample_count = 0;
        self.next_trigger_sample = 0;
        self.current_step = 0;
    }

    /// Set the BPM and recalculate timing
    pub fn set_bpm(&mut self, bpm: f32) {
        self.bpm = bpm;
        self.samples_per_8th_note = Self::calculate_samples_per_8th_note(bpm, self.sample_rate);
    }

    /// Get the current step
    pub fn get_current_step(&self) -> usize {
        self.current_step
    }

    /// Check if the sequencer is running
    pub fn is_running(&self) -> bool {
        self.is_running
    }

    /// Process one sample and call the callback when a step should trigger.
    /// Returns true if the callback was triggered on this sample.
    pub fn tick<F>(&mut self, mut callback: F) -> bool
    where
        F: FnMut(usize),
    {
        if !self.is_running {
            self.sample_count += 1;
            return false;
        }

        let mut triggered = false;

        // Check if we've reached the next trigger point
        if self.sample_count >= self.next_trigger_sample {
            // Trigger the callback with the current step
            callback(self.current_step);
            triggered = true;

            // Advance to the next step
            self.current_step += 1;

            // Calculate the next trigger sample (accumulate fractional samples for accuracy)
            self.next_trigger_sample =
                (self.next_trigger_sample as f32 + self.samples_per_8th_note).round() as u64;
        }

        self.sample_count += 1;
        triggered
    }
}
