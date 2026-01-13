use crate::utils::{SmoothedParam, DEFAULT_SMOOTH_TIME_MS};

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

    // Swing timing (0.0-1.0, where 0.5 = neutral/no swing)
    swing: SmoothedParam,
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
            swing: SmoothedParam::new(0.5, 0.0, 1.0, sample_rate, DEFAULT_SMOOTH_TIME_MS),
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
            swing: SmoothedParam::new(0.5, 0.0, 1.0, sample_rate, DEFAULT_SMOOTH_TIME_MS),
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

    /// Set the swing amount (0.0-1.0, where 0.5 = no swing)
    ///
    /// Swing delays off-beat steps (odd-numbered: 1, 3, 5...) to create a groovy feel.
    /// - 0.5 = neutral/straight timing
    /// - 0.67 = typical "medium" swing (MPC-style 67%)
    /// - 1.0 = maximum swing (full step delay)
    /// - Values below 0.5 create "reverse swing" (off-beats early)
    pub fn set_swing(&mut self, swing: f32) {
        self.swing.set_target(swing.clamp(0.0, 1.0));
    }

    /// Get the current swing amount
    pub fn swing(&self) -> f32 {
        self.swing.get()
    }

    /// Check if a step index is a "swing step" (off-beat)
    /// In 16th note grid: steps 1, 3, 5, 7, 9, 11, 13, 15 are off-beats
    #[inline]
    fn is_swing_step(&self, step: usize) -> bool {
        step % 2 == 1
    }

    /// Process one sample and return the instrument name if it should be triggered
    /// Returns Some(instrument_name) if a trigger should happen, None otherwise
    pub fn tick(&mut self) -> Option<&str> {
        if !self.is_running || self.pattern.is_empty() {
            self.sample_count += 1;
            return None;
        }

        // Tick the swing smoother for smooth parameter changes
        self.swing.tick();

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

            // Calculate swing offset for the next step
            // Swing delays off-beat steps (odd-numbered) by a percentage of the step length
            let swing_offset = if self.is_swing_step(self.current_step) {
                // Map 0.0-1.0 to -1.0 to +1.0, then multiply by step length
                (self.swing.get() - 0.5) * 2.0 * self.samples_per_step
            } else {
                0.0
            };

            // Calculate the next trigger sample (accumulate fractional samples for accuracy)
            self.next_trigger_sample = (self.next_trigger_sample as f32
                + self.samples_per_step
                + swing_offset)
                .round() as u64;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_swing_default_neutral() {
        let seq = Sequencer::new(120.0, 44100.0, 16, "test");
        assert!(
            (seq.swing() - 0.5).abs() < 0.001,
            "Default swing should be 0.5 (neutral)"
        );
    }

    #[test]
    fn test_swing_set_and_get() {
        let mut seq = Sequencer::new(120.0, 44100.0, 16, "test");

        seq.set_swing(0.67);
        // Tick to let smoother settle
        for _ in 0..4410 {
            seq.swing.tick();
        }
        assert!(
            (seq.swing() - 0.67).abs() < 0.01,
            "Swing should be ~0.67 after settling"
        );
    }

    #[test]
    fn test_swing_clamping() {
        let mut seq = Sequencer::new(120.0, 44100.0, 16, "test");

        // Test clamping above max
        seq.set_swing(1.5);
        for _ in 0..4410 {
            seq.swing.tick();
        }
        assert!(
            (seq.swing() - 1.0).abs() < 0.01,
            "Swing should be clamped to 1.0"
        );

        // Test clamping below min
        seq.set_swing(-0.5);
        for _ in 0..4410 {
            seq.swing.tick();
        }
        assert!(
            (seq.swing() - 0.0).abs() < 0.01,
            "Swing should be clamped to 0.0"
        );
    }

    #[test]
    fn test_is_swing_step() {
        let seq = Sequencer::new(120.0, 44100.0, 16, "test");

        // Even steps (on-beat) should not be swing steps
        assert!(!seq.is_swing_step(0), "Step 0 should not be a swing step");
        assert!(!seq.is_swing_step(2), "Step 2 should not be a swing step");
        assert!(!seq.is_swing_step(4), "Step 4 should not be a swing step");
        assert!(!seq.is_swing_step(14), "Step 14 should not be a swing step");

        // Odd steps (off-beat) should be swing steps
        assert!(seq.is_swing_step(1), "Step 1 should be a swing step");
        assert!(seq.is_swing_step(3), "Step 3 should be a swing step");
        assert!(seq.is_swing_step(5), "Step 5 should be a swing step");
        assert!(seq.is_swing_step(15), "Step 15 should be a swing step");
    }

    #[test]
    fn test_swing_timing_affects_triggers() {
        // Create two sequencers with the same pattern
        let mut seq_straight = Sequencer::with_pattern(120.0, 44100.0, vec![true; 4], "test");
        let mut seq_swing = Sequencer::with_pattern(120.0, 44100.0, vec![true; 4], "test");

        // Set swing on one - use immediate to avoid needing to settle
        seq_swing.swing.set_immediate(0.75);

        seq_straight.start();
        seq_swing.start();

        // Run and record first few trigger times (only need first 2 to test)
        let mut triggers_straight: Vec<u64> = Vec::new();
        let mut triggers_swing: Vec<u64> = Vec::new();

        for _ in 0..50000 {
            if seq_straight.tick().is_some() {
                triggers_straight.push(seq_straight.sample_count());
            }
            if seq_swing.tick().is_some() {
                triggers_swing.push(seq_swing.sample_count());
            }

            // Only need first few triggers
            if triggers_straight.len() >= 4 && triggers_swing.len() >= 4 {
                break;
            }
        }

        // Should have at least 2 triggers each
        assert!(
            triggers_straight.len() >= 2 && triggers_swing.len() >= 2,
            "Both sequencers should have at least 2 triggers"
        );

        // With swing, step 1 should be delayed relative to straight
        let straight_gap = triggers_straight[1] - triggers_straight[0];
        let swing_gap = triggers_swing[1] - triggers_swing[0];

        // Swing should make the gap to step 1 longer
        assert!(
            swing_gap > straight_gap,
            "Swung step 1 should be delayed (gap {} vs {})",
            swing_gap,
            straight_gap
        );
    }
}
