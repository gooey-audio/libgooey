//! Feedback waveshaper distortion effect
//!
//! Extends the basic tanh waveshaper with a feedback loop for richer,
//! self-exciting distortion. A one-pole lowpass filter and DC blocker
//! in the feedback path tame high-frequency accumulation and prevent
//! DC drift. Higher feedback on kicks creates sub-harmonic growl,
//! moderate feedback on snares adds a gritty, self-exciting tail.

/// Threshold for flushing denormal numbers to zero
const DENORMAL_THRESHOLD: f32 = 1e-15;

/// DC blocker coefficient (R in RC circuit, ~20Hz cutoff at 44.1kHz)
const DC_BLOCKER_COEFF: f32 = 0.995;

/// Amplitude-envelope follower attack time (ms): fast, to catch the transient
/// so the attack isn't over-compensated by the makeup gain.
const ENV_ATTACK_MS: f32 = 1.0;

/// Amplitude-envelope follower release time (ms): slow, to ride the kick body
/// without chasing individual sub-oscillator cycles.
const ENV_RELEASE_MS: f32 = 120.0;

/// Floor for the envelope reference used by the makeup gain. Avoids division
/// blow-up as the envelope approaches zero and prevents extreme boost in silence.
const ENV_FLOOR: f32 = 0.05;

/// Waveshaper with feedback loop for richer harmonic distortion
///
/// The feedback path includes a one-pole lowpass filter (controllable cutoff)
/// and a DC blocker to keep the loop stable.
pub struct FeedbackWaveshaper {
    // Parameters
    drive: f32,
    mix: f32,
    feedback: f32,
    sample_rate: f32,
    filter_cutoff: f32,
    filter_coeff: f32,
    env_att_coeff: f32,
    env_rel_coeff: f32,

    // State
    last_out: f32,
    filter_state: f32,
    dc_x1: f32,
    dc_y1: f32,
    env: f32,
}

impl FeedbackWaveshaper {
    /// Create a new feedback waveshaper
    ///
    /// # Arguments
    /// * `sample_rate` - Audio sample rate in Hz
    /// * `drive` - Distortion amount (1.0-100.0, clamped, 1.0 = bypass)
    /// * `feedback` - Feedback gain (0.0-0.98, clamped)
    /// * `filter_cutoff` - Feedback lowpass cutoff in Hz (200-20000, clamped)
    /// * `mix` - Dry/wet mix (0.0-1.0, clamped)
    pub fn new(sample_rate: f32, drive: f32, feedback: f32, filter_cutoff: f32, mix: f32) -> Self {
        let drive = drive.clamp(1.0, 100.0);
        let feedback = feedback.clamp(0.0, 0.98);
        let filter_cutoff = filter_cutoff.clamp(200.0, 20000.0);
        let mix = mix.clamp(0.0, 1.0);
        Self {
            drive,
            mix,
            feedback,
            sample_rate,
            filter_cutoff,
            filter_coeff: Self::compute_filter_coeff(filter_cutoff, sample_rate),
            env_att_coeff: Self::compute_env_coeff(ENV_ATTACK_MS, sample_rate),
            env_rel_coeff: Self::compute_env_coeff(ENV_RELEASE_MS, sample_rate),
            last_out: 0.0,
            filter_state: 0.0,
            dc_x1: 0.0,
            dc_y1: 0.0,
            env: 0.0,
        }
    }

    /// Create a feedback waveshaper with default settings (bypass)
    pub fn default(sample_rate: f32) -> Self {
        Self::new(sample_rate, 1.0, 0.0, 2000.0, 0.0)
    }

    /// Process a single sample through the feedback waveshaper
    ///
    /// Algorithm:
    /// 1. Mix input with filtered feedback from previous sample
    /// 2. Apply drive gain and soft-clip with tanh
    /// 3. Gain-compensate to maintain consistent output level
    /// 4. DC-block the output
    /// 5. Lowpass-filter into the feedback buffer for next sample
    /// 6. Mix dry/wet (wet signal is full-bandwidth, lowpass is feedback-only)
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        // NaN guard: protect stateful feedback loop
        if !input.is_finite() {
            self.reset();
            return 0.0;
        }

        // Bypass if no effect
        if self.mix <= 0.0001 || self.drive <= 1.0 {
            return input;
        }

        // Sum input with filtered feedback
        let fb_input = self.drive * input + self.feedback * self.last_out;

        // Waveshape with tanh
        let shaped = fb_input.tanh();

        // Track the amplitude envelope of the dry input with a slow follower.
        // Referencing the makeup gain to this level (instead of a fixed 0.5)
        // keeps loud transients punchy and quiet tails from being boosted.
        // The envelope must be slow relative to the waveform: an instantaneous
        // |input| reference would cancel the waveshaping entirely.
        let rect = input.abs();
        let coeff = if rect > self.env {
            self.env_att_coeff
        } else {
            self.env_rel_coeff
        };
        self.env += (1.0 - coeff) * (rect - self.env);
        if self.env.abs() < DENORMAL_THRESHOLD {
            self.env = 0.0;
        }

        // Gain compensation: maintain consistent output level across all drive values.
        // Since last_out is already compensated, the loop gain is feedback * compensation.
        // Solving for equal loudness with and without feedback gives:
        //   compensation = comp_no_fb / (1 + comp_no_fb * feedback)
        let reference = self.env.max(ENV_FLOOR);
        let comp_no_fb = reference.tanh() / (reference * self.drive).tanh();
        let compensation = comp_no_fb / (1.0 + comp_no_fb * self.feedback);
        let compensated = shaped * compensation;

        // DC block the output
        let dc_blocked = Self::dc_block(compensated, &mut self.dc_x1, &mut self.dc_y1);

        // One-pole lowpass in feedback path
        self.filter_state += self.filter_coeff * (dc_blocked - self.filter_state);

        // Flush denormals
        if self.filter_state.abs() < DENORMAL_THRESHOLD {
            self.filter_state = 0.0;
        }

        // Store filtered output for next sample's feedback
        self.last_out = self.filter_state;

        // Safety: clamp feedback state to prevent runaway
        if !self.last_out.is_finite() || self.last_out.abs() > 50.0 {
            self.reset();
            return input;
        }

        // Mix dry/wet (wet signal is full-bandwidth dc_blocked, not the filtered feedback)
        input * (1.0 - self.mix) + dc_blocked * self.mix
    }

    /// Reset all internal state
    pub fn reset(&mut self) {
        self.last_out = 0.0;
        self.filter_state = 0.0;
        self.dc_x1 = 0.0;
        self.dc_y1 = 0.0;
        self.env = 0.0;
    }

    /// Set the drive amount (1.0-100.0)
    pub fn set_drive(&mut self, drive: f32) {
        self.drive = drive.clamp(1.0, 100.0);
    }

    /// Get the current drive amount
    pub fn drive(&self) -> f32 {
        self.drive
    }

    /// Set the feedback gain (0.0-0.98)
    pub fn set_feedback(&mut self, feedback: f32) {
        self.feedback = feedback.clamp(0.0, 0.98);
    }

    /// Get the current feedback gain
    pub fn feedback(&self) -> f32 {
        self.feedback
    }

    /// Set the feedback lowpass filter cutoff in Hz (200-20000)
    pub fn set_filter_cutoff(&mut self, cutoff: f32) {
        self.filter_cutoff = cutoff.clamp(200.0, 20000.0);
        self.filter_coeff = Self::compute_filter_coeff(self.filter_cutoff, self.sample_rate);
    }

    /// Get the current filter cutoff in Hz
    pub fn filter_cutoff(&self) -> f32 {
        self.filter_cutoff
    }

    /// Set the dry/wet mix (0.0-1.0)
    pub fn set_mix(&mut self, mix: f32) {
        self.mix = mix.clamp(0.0, 1.0);
    }

    /// Get the current mix amount
    pub fn mix(&self) -> f32 {
        self.mix
    }

    #[inline]
    fn compute_filter_coeff(cutoff: f32, sample_rate: f32) -> f32 {
        let g = 1.0 - (-2.0 * std::f32::consts::PI * cutoff / sample_rate).exp();
        g.clamp(0.0, 0.9)
    }

    /// One-pole smoothing coefficient for the envelope follower from a time
    /// constant in milliseconds. Returns the retention factor (closer to 1.0 =
    /// slower); the per-sample update applies `(1.0 - coeff)` as the step size.
    #[inline]
    fn compute_env_coeff(time_ms: f32, sample_rate: f32) -> f32 {
        (-1.0 / (time_ms / 1000.0 * sample_rate)).exp()
    }

    #[inline]
    fn dc_block(input: f32, x1: &mut f32, y1: &mut f32) -> f32 {
        let output = input - *x1 + DC_BLOCKER_COEFF * *y1;
        *x1 = input;
        *y1 = if output.abs() < DENORMAL_THRESHOLD {
            0.0
        } else {
            output
        };
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bypass_when_mix_zero() {
        let mut ws = FeedbackWaveshaper::new(44100.0, 5.0, 0.5, 2000.0, 0.0);
        assert_eq!(ws.process(0.5), 0.5);
        assert_eq!(ws.process(-0.3), -0.3);
    }

    #[test]
    fn test_bypass_when_drive_one() {
        let mut ws = FeedbackWaveshaper::new(44100.0, 1.0, 0.5, 2000.0, 1.0);
        assert_eq!(ws.process(0.5), 0.5);
        assert_eq!(ws.process(-0.3), -0.3);
    }

    #[test]
    fn test_soft_clipping() {
        // Drive a full-scale sine so the envelope follower settles near 1.0
        // (a steady DC input would be removed by the DC blocker).
        let mut ws = FeedbackWaveshaper::new(44100.0, 10.0, 0.0, 2000.0, 1.0);
        let mut peak_full: f32 = 0.0;
        for i in 0..2000 {
            let s = (i as f32 * 0.3).sin();
            peak_full = peak_full.max(ws.process(s).abs());
        }
        // Soft-clipped: peak stays below unity but is substantial.
        assert!(
            peak_full > 0.5 && peak_full < 1.0,
            "peak_full was {peak_full}"
        );

        // A much quieter signal yields a smaller peak (level-tracked makeup
        // preserves dynamics instead of boosting the quiet input to unity).
        let mut ws_quiet = FeedbackWaveshaper::new(44100.0, 10.0, 0.0, 2000.0, 1.0);
        let mut peak_quiet: f32 = 0.0;
        for i in 0..2000 {
            let s = (i as f32 * 0.3).sin() * 0.1;
            peak_quiet = peak_quiet.max(ws_quiet.process(s).abs());
        }
        assert!(
            peak_quiet < peak_full,
            "quiet peak ({peak_quiet}) should be below full peak ({peak_full})"
        );
    }

    #[test]
    fn test_parameter_clamping() {
        let ws = FeedbackWaveshaper::new(44100.0, 200.0, 5.0, 50.0, 10.0);
        assert_eq!(ws.drive(), 100.0);
        assert_eq!(ws.feedback(), 0.98);
        assert_eq!(ws.filter_cutoff(), 200.0);
        assert_eq!(ws.mix(), 1.0);
    }

    #[test]
    fn test_zero_input() {
        let mut ws = FeedbackWaveshaper::new(44100.0, 5.0, 0.5, 2000.0, 1.0);
        assert_eq!(ws.process(0.0), 0.0);
    }

    #[test]
    fn test_feedback_changes_output() {
        let mut ws_no_fb = FeedbackWaveshaper::new(44100.0, 5.0, 0.0, 2000.0, 1.0);
        let mut ws_fb = FeedbackWaveshaper::new(44100.0, 5.0, 0.7, 2000.0, 1.0);

        // Use an oscillating signal so feedback has a dynamic effect
        let mut max_diff: f32 = 0.0;
        for i in 0..1000 {
            let signal = (i as f32 * 0.1).sin() * 0.5;
            let out_no_fb = ws_no_fb.process(signal);
            let out_fb = ws_fb.process(signal);
            max_diff = max_diff.max((out_fb - out_no_fb).abs());
        }

        assert!(
            max_diff > 0.01,
            "feedback should change the output, max_diff={max_diff}"
        );
    }

    #[test]
    fn test_feedback_does_not_blow_up() {
        let mut ws = FeedbackWaveshaper::new(44100.0, 10.0, 0.9, 2000.0, 1.0);
        for i in 0..44100 {
            let input = if i % 100 < 50 { 0.8 } else { -0.8 };
            let output = ws.process(input);
            assert!(
                output.is_finite() && output.abs() < 5.0,
                "output blew up at sample {i}: {output}"
            );
        }
    }

    #[test]
    fn test_extreme_settings_stability() {
        let mut ws = FeedbackWaveshaper::new(44100.0, 100.0, 0.98, 2000.0, 1.0);
        for i in 0..44100 {
            let input = if i % 100 < 50 { 0.8 } else { -0.8 };
            let output = ws.process(input);
            assert!(
                output.is_finite() && output.abs() < 5.0,
                "output blew up at extreme settings at sample {i}: {output}"
            );
        }
    }

    #[test]
    fn test_dc_stability() {
        let mut ws = FeedbackWaveshaper::new(44100.0, 5.0, 0.8, 1000.0, 1.0);
        // Feed a DC signal and verify output doesn't drift
        for _ in 0..44100 {
            ws.process(0.3);
        }
        // After settling, feed silence and check it decays
        let mut last = 1.0_f32;
        for _ in 0..4410 {
            last = ws.process(0.0);
        }
        assert!(
            last.abs() < 0.01,
            "DC should decay to near zero, got {last}"
        );
    }

    #[test]
    fn test_nan_protection() {
        let mut ws = FeedbackWaveshaper::new(44100.0, 5.0, 0.5, 2000.0, 1.0);
        // Process some normal samples to build state
        ws.process(0.5);
        ws.process(0.3);
        // Feed NaN - should return 0 and reset
        let output = ws.process(f32::NAN);
        assert_eq!(output, 0.0);
        // After reset, normal input should work
        let output = ws.process(0.5);
        assert!(output.is_finite());
    }

    #[test]
    fn test_feedback_gain_compensation() {
        // Verify that feedback doesn't significantly increase perceived loudness
        let input_signal: Vec<f32> = (0..4000).map(|i| (i as f32 * 0.1).sin() * 0.5).collect();

        let mut ws_no_fb = FeedbackWaveshaper::new(44100.0, 5.0, 0.0, 2000.0, 1.0);
        let mut ws_fb = FeedbackWaveshaper::new(44100.0, 5.0, 0.7, 2000.0, 1.0);

        let rms_no_fb: f32 = input_signal
            .iter()
            .map(|&s| ws_no_fb.process(s).powi(2))
            .sum::<f32>()
            / input_signal.len() as f32;
        let rms_fb: f32 = input_signal
            .iter()
            .map(|&s| ws_fb.process(s).powi(2))
            .sum::<f32>()
            / input_signal.len() as f32;

        let rms_ratio = (rms_fb / rms_no_fb).sqrt();
        assert!(
            rms_ratio < 1.5,
            "feedback increased RMS by {:.0}%, expected <50%",
            (rms_ratio - 1.0) * 100.0
        );
    }

    #[test]
    fn test_reset_clears_state() {
        let mut ws = FeedbackWaveshaper::new(44100.0, 5.0, 0.8, 2000.0, 1.0);
        // Build up state
        for _ in 0..1000 {
            ws.process(0.5);
        }
        ws.reset();

        // Should behave like a fresh instance
        let mut fresh = FeedbackWaveshaper::new(44100.0, 5.0, 0.8, 2000.0, 1.0);
        let out_reset = ws.process(0.5);
        let out_fresh = fresh.process(0.5);
        assert_eq!(out_reset, out_fresh);
    }

    #[test]
    fn test_transient_not_compressed_below_tail() {
        // Level-tracked makeup must preserve dynamics: the loud attack should
        // stay louder than the quiet decay tail. The old fixed-0.5 reference
        // compressed the attack and boosted the tail, flattening the punch.
        let mut ws = FeedbackWaveshaper::new(44100.0, 10.0, 0.0, 2000.0, 1.0);

        let sr = 44100.0;
        let n = (0.2 * sr) as usize; // 200 ms decaying tone
        let attack_window = (0.005 * sr) as usize; // first 5 ms

        let mut attack_peak: f32 = 0.0;
        let mut tail_peak: f32 = 0.0;
        for i in 0..n {
            // Amplitude envelope decays 1.0 -> 0.1 over the tone.
            let amp = (1.0 - i as f32 / n as f32) * 0.9 + 0.1;
            let signal = (i as f32 * 0.2).sin() * amp;
            let out = ws.process(signal).abs();
            if i < attack_window {
                attack_peak = attack_peak.max(out);
            } else if i > n - attack_window {
                tail_peak = tail_peak.max(out);
            }
        }

        assert!(
            attack_peak > tail_peak,
            "attack ({attack_peak}) should stay louder than tail ({tail_peak})"
        );
    }
}
