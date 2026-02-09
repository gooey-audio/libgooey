/// Absolute blend override for a sequencer step (X/Y in 0.0-1.0)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SequencerBlendOverride {
    pub x: f32,
    pub y: f32,
}

impl SequencerBlendOverride {
    /// Create a new absolute blend override (values are clamped to 0.0-1.0)
    pub fn new(x: f32, y: f32) -> Self {
        Self {
            x: x.clamp(0.0, 1.0),
            y: y.clamp(0.0, 1.0),
        }
    }
}

/// Represents a single sequencer step with enabled state, velocity, and optional overrides
#[derive(Clone, Copy, Debug)]
pub struct SequencerStep {
    /// Whether this step triggers the instrument
    pub enabled: bool,
    /// Velocity for this step (0.0-1.0, defaults to 1.0)
    pub velocity: f32,
    /// Optional absolute blend override for this step
    pub blend_override: Option<SequencerBlendOverride>,
}

impl Default for SequencerStep {
    fn default() -> Self {
        Self {
            enabled: true,
            velocity: 1.0,
            blend_override: None,
        }
    }
}

impl SequencerStep {
    /// Create a new step with the given enabled state and full velocity
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            velocity: 1.0,
            blend_override: None,
        }
    }

    /// Create a new step with the given enabled state and velocity
    pub fn with_velocity(enabled: bool, velocity: f32) -> Self {
        Self {
            enabled,
            velocity: velocity.clamp(0.0, 1.0),
            blend_override: None,
        }
    }

    /// Create a new step with the given enabled state, velocity, and blend override
    pub fn with_velocity_and_blend_override(
        enabled: bool,
        velocity: f32,
        blend_override: Option<SequencerBlendOverride>,
    ) -> Self {
        Self {
            enabled,
            velocity: velocity.clamp(0.0, 1.0),
            blend_override,
        }
    }
}

impl From<bool> for SequencerStep {
    fn from(enabled: bool) -> Self {
        Self::new(enabled)
    }
}

/// Trigger info from a sequencer tick
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SequencerTrigger<'a> {
    pub instrument_name: &'a str,
    pub velocity: f32,
    pub blend_override: Option<SequencerBlendOverride>,
}

/// A sample-accurate step sequencer with per-step velocity and optional blend overrides
pub struct Sequencer {
    bpm: f32,
    sample_rate: f32,

    // Sample-accurate timing
    sample_count: u64,
    next_trigger_sample: u64,
    samples_per_step: f32,

    // Pattern and current position (now with velocity per step)
    pattern: Vec<SequencerStep>,
    current_step: usize,

    // The step that is currently being played (for UI display)
    // This is the step that most recently triggered, not the next one
    playhead_step: usize,

    // Instrument to trigger
    instrument_name: String,

    // Whether the sequencer is running
    is_running: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_step_blend_override_set_get_clear() {
        let mut sequencer = Sequencer::new(120.0, 44100.0, 4, "kick");
        assert_eq!(sequencer.get_step_blend_override(0), None);

        sequencer.set_step_blend_override(0, 0.2, 0.8);
        assert_eq!(
            sequencer.get_step_blend_override(0),
            Some(SequencerBlendOverride::new(0.2, 0.8))
        );

        sequencer.clear_step_blend_override(0);
        assert_eq!(sequencer.get_step_blend_override(0), None);
    }

    #[test]
    fn test_step_with_velocity_preserves_override() {
        let mut sequencer = Sequencer::new(120.0, 44100.0, 4, "kick");
        sequencer.set_step_blend_override(1, 0.3, 0.6);
        sequencer.set_step_with_velocity(1, true, 0.4);

        assert_eq!(
            sequencer.get_step_blend_override(1),
            Some(SequencerBlendOverride::new(0.3, 0.6))
        );
    }
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

        // Initialize with all steps enabled at full velocity
        let pattern = vec![SequencerStep::default(); beat_count];

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

    /// Create a sequencer with a custom pattern (bool array for backwards compatibility)
    pub fn with_pattern(
        bpm: f32,
        sample_rate: f32,
        pattern: Vec<bool>,
        instrument_name: impl Into<String>,
    ) -> Self {
        let samples_per_step = Self::calculate_samples_per_step(bpm, sample_rate);

        // Convert bool pattern to SequencerStep with full velocity
        let pattern: Vec<SequencerStep> = pattern.into_iter().map(SequencerStep::from).collect();

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

    /// Create a sequencer with a custom pattern including velocities
    pub fn with_velocity_pattern(
        bpm: f32,
        sample_rate: f32,
        pattern: Vec<SequencerStep>,
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

    /// Set a step's enabled state in the pattern (maintains current velocity)
    pub fn set_step(&mut self, step: usize, enabled: bool) {
        if step < self.pattern.len() {
            self.pattern[step].enabled = enabled;
        }
    }

    /// Set a step's velocity (0.0-1.0)
    pub fn set_step_velocity(&mut self, step: usize, velocity: f32) {
        if step < self.pattern.len() {
            self.pattern[step].velocity = velocity.clamp(0.0, 1.0);
        }
    }

    /// Set both enabled state and velocity for a step (preserves any blend override)
    pub fn set_step_with_velocity(&mut self, step: usize, enabled: bool, velocity: f32) {
        if step < self.pattern.len() {
            let blend_override = self.pattern[step].blend_override;
            self.pattern[step] =
                SequencerStep::with_velocity_and_blend_override(enabled, velocity, blend_override);
        }
    }

    /// Get a step's enabled state
    pub fn get_step_enabled(&self, step: usize) -> bool {
        self.pattern.get(step).map(|s| s.enabled).unwrap_or(false)
    }

    /// Get a step's velocity (0.0-1.0)
    pub fn get_step_velocity(&self, step: usize) -> f32 {
        self.pattern.get(step).map(|s| s.velocity).unwrap_or(0.0)
    }

    /// Set a step's absolute blend override (0.0-1.0)
    pub fn set_step_blend_override(&mut self, step: usize, x: f32, y: f32) {
        if step < self.pattern.len() {
            self.pattern[step].blend_override = Some(SequencerBlendOverride::new(x, y));
        }
    }

    /// Clear a step's blend override
    pub fn clear_step_blend_override(&mut self, step: usize) {
        if step < self.pattern.len() {
            self.pattern[step].blend_override = None;
        }
    }

    /// Get a step's blend override
    pub fn get_step_blend_override(&self, step: usize) -> Option<SequencerBlendOverride> {
        self.pattern.get(step).and_then(|s| s.blend_override)
    }

    /// Get the pattern with velocity information
    pub fn pattern_steps(&self) -> &[SequencerStep] {
        &self.pattern
    }

    /// Get the pattern as enabled booleans (for backwards compatibility)
    pub fn pattern(&self) -> Vec<bool> {
        self.pattern.iter().map(|s| s.enabled).collect()
    }

    /// Set the entire pattern from bool array (sets all velocities to 1.0)
    pub fn set_pattern(&mut self, pattern: Vec<bool>) {
        self.pattern = pattern.into_iter().map(SequencerStep::from).collect();
        // Reset to beginning if current step is beyond new pattern length
        if self.current_step >= self.pattern.len() {
            self.current_step = 0;
        }
    }

    /// Set the entire pattern with velocity information
    pub fn set_pattern_with_velocity(&mut self, pattern: Vec<SequencerStep>) {
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

    /// Process one sample and return trigger info if applicable (with overrides)
    pub fn tick_with_overrides(&mut self) -> Option<SequencerTrigger<'_>> {
        if !self.is_running || self.pattern.is_empty() {
            self.sample_count += 1;
            return None;
        }

        let mut should_trigger: Option<SequencerTrigger<'_>> = None;

        // Check if we've reached the next trigger point
        if self.sample_count >= self.next_trigger_sample {
            // Update playhead to show the step that's about to play
            self.playhead_step = self.current_step;

            // Check if this step should trigger
            let step = &self.pattern[self.current_step];
            if step.enabled {
                should_trigger = Some(SequencerTrigger {
                    instrument_name: self.instrument_name.as_str(),
                    velocity: step.velocity,
                    blend_override: step.blend_override,
                });
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

    /// Process one sample and return trigger info if applicable
    /// Returns Some((instrument_name, velocity)) if a trigger should happen, None otherwise
    pub fn tick(&mut self) -> Option<(&str, f32)> {
        self.tick_with_overrides()
            .map(|trigger| (trigger.instrument_name, trigger.velocity))
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
