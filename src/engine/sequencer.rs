use crate::utils::{SmoothedParam, DEFAULT_SMOOTH_TIME_MS};

/// Absolute blend setting for a sequencer step (X/Y in 0.0-1.0)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SequencerBlendSetting {
    pub x: f32,
    pub y: f32,
}

impl SequencerBlendSetting {
    /// Create a new absolute blend setting (values are clamped to 0.0-1.0)
    pub fn new(x: f32, y: f32) -> Self {
        Self {
            x: x.clamp(0.0, 1.0),
            y: y.clamp(0.0, 1.0),
        }
    }
}

/// Optional per-step settings for fields that may be omitted.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SequencerStepSettings {
    pub velocity: Option<f32>,
    pub blend: Option<SequencerBlendSetting>,
}

/// Represents a single sequencer step with enabled state, velocity, and optional blend setting
#[derive(Clone, Copy, Debug)]
pub struct SequencerStep {
    /// Whether this step triggers the instrument
    pub enabled: bool,
    /// Velocity for this step (0.0-1.0, defaults to 1.0)
    pub velocity: f32,
    /// Optional absolute blend setting for this step
    pub blend: Option<SequencerBlendSetting>,
}

impl Default for SequencerStep {
    fn default() -> Self {
        Self {
            enabled: true,
            velocity: 1.0,
            blend: None,
        }
    }
}

impl SequencerStep {
    /// Create a new step with the given enabled state and full velocity
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            velocity: 1.0,
            blend: None,
        }
    }

    /// Create a new step with the given enabled state and velocity
    pub fn with_velocity(enabled: bool, velocity: f32) -> Self {
        Self {
            enabled,
            velocity: velocity.clamp(0.0, 1.0),
            blend: None,
        }
    }

    /// Create a new step with the given enabled state, velocity, and blend setting
    pub fn with_velocity_and_blend(
        enabled: bool,
        velocity: f32,
        blend: Option<SequencerBlendSetting>,
    ) -> Self {
        Self {
            enabled,
            velocity: velocity.clamp(0.0, 1.0),
            blend,
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
    pub blend: Option<SequencerBlendSetting>,
}

/// A sample-accurate step sequencer with per-step velocity and optional blend settings
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

    // Swing timing (0.0-1.0, where 0.5 = neutral/no swing)
    swing: SmoothedParam,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_step_blend_setting_set_get_clear() {
        let mut sequencer = Sequencer::new(120.0, 44100.0, 4, "kick");
        assert_eq!(sequencer.get_step_blend(0), None);

        sequencer.set_step_blend(0, 0.2, 0.8);
        assert_eq!(
            sequencer.get_step_blend(0),
            Some(SequencerBlendSetting::new(0.2, 0.8))
        );

        sequencer.clear_step_blend(0);
        assert_eq!(sequencer.get_step_blend(0), None);
    }

    #[test]
    fn test_step_with_velocity_preserves_blend_setting() {
        let mut sequencer = Sequencer::new(120.0, 44100.0, 4, "kick");
        sequencer.set_step_blend(1, 0.3, 0.6);
        sequencer.set_step_with_velocity(1, true, 0.4);

        assert_eq!(
            sequencer.get_step_blend(1),
            Some(SequencerBlendSetting::new(0.3, 0.6))
        );
    }

    #[test]
    fn test_step_settings_omitted_fields_preserve_existing_values() {
        let mut sequencer = Sequencer::new(120.0, 44100.0, 4, "kick");
        sequencer.set_step_velocity(2, 0.75);
        sequencer.set_step_blend(2, 0.4, 0.9);

        sequencer.set_step_with_settings(2, true, SequencerStepSettings::default());

        assert_eq!(sequencer.get_step_velocity(2), 0.75);
        assert_eq!(
            sequencer.get_step_blend(2),
            Some(SequencerBlendSetting::new(0.4, 0.9))
        );
    }

    #[test]
    fn test_swing_default_neutral() {
        let seq = Sequencer::new(120.0, 44100.0, 16, "test");
        assert!(
            (seq.swing() - 0.5).abs() < 0.001,
            "Default swing should be 0.5 (neutral)"
        );
    }

    #[test]
    fn test_is_swing_step() {
        let seq = Sequencer::new(120.0, 44100.0, 16, "test");
        assert!(!seq.is_swing_step(0));
        assert!(seq.is_swing_step(1));
        assert!(!seq.is_swing_step(2));
        assert!(seq.is_swing_step(3));
        assert!(seq.is_swing_step(15));
    }

    #[test]
    fn test_swing_timing_affects_triggers() {
        let mut seq_straight = Sequencer::with_pattern(120.0, 44100.0, vec![true; 4], "test");
        let mut seq_swing = Sequencer::with_pattern(120.0, 44100.0, vec![true; 4], "test");

        seq_swing.swing.set_immediate(0.75);

        seq_straight.start();
        seq_swing.start();

        let mut triggers_straight: Vec<u64> = Vec::new();
        let mut triggers_swing: Vec<u64> = Vec::new();

        for _ in 0..50000 {
            if seq_straight.tick().is_some() {
                triggers_straight.push(seq_straight.sample_count());
            }
            if seq_swing.tick().is_some() {
                triggers_swing.push(seq_swing.sample_count());
            }
            if triggers_straight.len() >= 4 && triggers_swing.len() >= 4 {
                break;
            }
        }

        assert!(triggers_straight.len() >= 2 && triggers_swing.len() >= 2);

        let straight_gap = triggers_straight[1] - triggers_straight[0];
        let swing_gap = triggers_swing[1] - triggers_swing[0];

        assert!(
            swing_gap > straight_gap,
            "Swung step 1 should be delayed (gap {} vs {})",
            swing_gap,
            straight_gap
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
            swing: SmoothedParam::new(0.5, 0.0, 1.0, sample_rate, DEFAULT_SMOOTH_TIME_MS),
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
            swing: SmoothedParam::new(0.5, 0.0, 1.0, sample_rate, DEFAULT_SMOOTH_TIME_MS),
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

    /// Set both enabled state and velocity for a step (preserves any blend setting)
    pub fn set_step_with_velocity(&mut self, step: usize, enabled: bool, velocity: f32) {
        if step < self.pattern.len() {
            let blend = self.pattern[step].blend;
            self.pattern[step] = SequencerStep::with_velocity_and_blend(enabled, velocity, blend);
        }
    }

    /// Set a step with optional settings.
    /// Omitted fields in `settings` are left unchanged.
    pub fn set_step_with_settings(
        &mut self,
        step: usize,
        enabled: bool,
        settings: SequencerStepSettings,
    ) {
        if step < self.pattern.len() {
            self.pattern[step].enabled = enabled;
            if let Some(velocity) = settings.velocity {
                self.pattern[step].velocity = velocity.clamp(0.0, 1.0);
            }
            if let Some(blend) = settings.blend {
                self.pattern[step].blend = Some(blend);
            }
        }
    }

    /// Legacy alias for `set_step_blend`.
    pub fn set_step_blend_override(&mut self, step: usize, x: f32, y: f32) {
        self.set_step_blend(step, x, y);
    }

    /// Legacy alias for `clear_step_blend`.
    pub fn clear_step_blend_override(&mut self, step: usize) {
        self.clear_step_blend(step);
    }

    /// Legacy alias for `get_step_blend`.
    pub fn get_step_blend_override(&self, step: usize) -> Option<SequencerBlendSetting> {
        self.get_step_blend(step)
    }

    /// Get a step's enabled state
    pub fn get_step_enabled(&self, step: usize) -> bool {
        self.pattern.get(step).map(|s| s.enabled).unwrap_or(false)
    }

    /// Get a step's velocity (0.0-1.0)
    pub fn get_step_velocity(&self, step: usize) -> f32 {
        self.pattern.get(step).map(|s| s.velocity).unwrap_or(0.0)
    }

    /// Set a step's absolute blend setting (0.0-1.0)
    pub fn set_step_blend(&mut self, step: usize, x: f32, y: f32) {
        if step < self.pattern.len() {
            self.pattern[step].blend = Some(SequencerBlendSetting::new(x, y));
        }
    }

    /// Clear a step's blend setting
    pub fn clear_step_blend(&mut self, step: usize) {
        if step < self.pattern.len() {
            self.pattern[step].blend = None;
        }
    }

    /// Get a step's blend setting
    pub fn get_step_blend(&self, step: usize) -> Option<SequencerBlendSetting> {
        self.pattern.get(step).and_then(|s| s.blend)
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
    #[inline]
    fn is_swing_step(&self, step: usize) -> bool {
        step % 2 == 1
    }

    /// Process one sample and return trigger info if applicable (with settings)
    pub fn tick_with_settings(&mut self) -> Option<SequencerTrigger<'_>> {
        if !self.is_running || self.pattern.is_empty() {
            self.sample_count += 1;
            return None;
        }

        // Tick the swing smoother for smooth parameter changes
        self.swing.tick();

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
                    blend: step.blend,
                });
            }

            // Advance to the next step (internal tracking)
            self.current_step = (self.current_step + 1) % self.pattern.len();

            // Calculate swing offset for the next step
            // Swing delays off-beat steps (odd-numbered) by a percentage of the step length
            let swing_offset = if self.is_swing_step(self.current_step) {
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

    /// Process one sample and return trigger info if applicable
    /// Returns Some((instrument_name, velocity)) if a trigger should happen, None otherwise
    pub fn tick(&mut self) -> Option<(&str, f32)> {
        self.tick_with_settings()
            .map(|trigger| (trigger.instrument_name, trigger.velocity))
    }

    /// Legacy alias for `tick_with_settings`.
    pub fn tick_with_overrides(&mut self) -> Option<SequencerTrigger<'_>> {
        self.tick_with_settings()
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
