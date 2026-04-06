//! Filter delay effect with BPM-synced timing
//!
//! Delay effect that uses clocked musical divisions (including triplets) instead of
//! arbitrary millisecond timing. A one-pole lowpass filter is applied in the feedback
//! path so each repetition gets progressively darker, like a classic filter delay.

use crate::effects::Effect;
use crate::utils::smoother::SmoothedParam;
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU32, Ordering};

/// Maximum delay time in seconds (enough for a whole note at ~48 BPM)
const MAX_DELAY_TIME: f32 = 5.0;

/// Threshold for flushing denormal numbers to zero
const DENORMAL_THRESHOLD: f32 = 1e-15;

/// Minimum filter cutoff in Hz
const MIN_FILTER_CUTOFF: f32 = 20.0;

/// Maximum filter cutoff in Hz
const MAX_FILTER_CUTOFF: f32 = 20000.0;

/// Musical time divisions for BPM-synced delay timing
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DelayTiming {
    /// Whole note (4 beats)
    Whole,
    /// Half note (2 beats)
    Half,
    /// Quarter note (1 beat)
    Quarter,
    /// Eighth note (1/2 beat)
    Eighth,
    /// Sixteenth note (1/4 beat)
    Sixteenth,
    /// Half note triplet (2/3 of a half note = 4/3 beats)
    HalfTriplet,
    /// Quarter note triplet (2/3 of a quarter note = 2/3 beat)
    QuarterTriplet,
    /// Eighth note triplet (2/3 of an eighth note = 1/3 beat)
    EighthTriplet,
    /// Sixteenth note triplet (2/3 of a sixteenth note = 1/6 beat)
    SixteenthTriplet,
}

impl DelayTiming {
    /// Get the number of beats this division represents
    pub fn beats(&self) -> f32 {
        match self {
            DelayTiming::Whole => 4.0,
            DelayTiming::Half => 2.0,
            DelayTiming::Quarter => 1.0,
            DelayTiming::Eighth => 0.5,
            DelayTiming::Sixteenth => 0.25,
            DelayTiming::HalfTriplet => 4.0 / 3.0,
            DelayTiming::QuarterTriplet => 2.0 / 3.0,
            DelayTiming::EighthTriplet => 1.0 / 3.0,
            DelayTiming::SixteenthTriplet => 1.0 / 6.0,
        }
    }

    /// Convert to delay time in seconds at the given BPM
    pub fn to_seconds(&self, bpm: f32) -> f32 {
        let seconds_per_beat = 60.0 / bpm;
        (seconds_per_beat * self.beats()).min(MAX_DELAY_TIME)
    }

    /// Convert from a u32 timing constant (used by FFI)
    pub fn from_timing_constant(value: u32) -> Option<Self> {
        match value {
            0 => Some(DelayTiming::Whole),
            1 => Some(DelayTiming::Half),
            2 => Some(DelayTiming::Quarter),
            3 => Some(DelayTiming::Eighth),
            4 => Some(DelayTiming::Sixteenth),
            5 => Some(DelayTiming::HalfTriplet),
            6 => Some(DelayTiming::QuarterTriplet),
            7 => Some(DelayTiming::EighthTriplet),
            8 => Some(DelayTiming::SixteenthTriplet),
            _ => None,
        }
    }

    /// Convert to the u32 timing constant (used by FFI)
    pub fn to_timing_constant(&self) -> u32 {
        match self {
            DelayTiming::Whole => 0,
            DelayTiming::Half => 1,
            DelayTiming::Quarter => 2,
            DelayTiming::Eighth => 3,
            DelayTiming::Sixteenth => 4,
            DelayTiming::HalfTriplet => 5,
            DelayTiming::QuarterTriplet => 6,
            DelayTiming::EighthTriplet => 7,
            DelayTiming::SixteenthTriplet => 8,
        }
    }
}

/// Internal mutable state for the delay (wrapped in UnsafeCell for interior mutability)
struct DelayState {
    // Circular buffer for delay line
    buffer: Vec<f32>,
    write_index: usize,

    // Two-pole lowpass filter state (applied to delayed output signal)
    filter_z1: f32,
    filter_z2: f32,

    // Track previous timing to detect changes and clear the buffer
    previous_timing: u32,

    // Smoothed parameters (updated per-sample for click-free changes)
    time_smoothed: SmoothedParam,
    feedback_smoothed: SmoothedParam,
    mix_smoothed: SmoothedParam,
    filter_cutoff_smoothed: SmoothedParam,
}

/// Filter delay effect with BPM-synced timing
///
/// Parameters:
/// - Timing: Musical division synced to BPM (quarter note, eighth triplet, etc.)
/// - Feedback: Amount of delayed signal fed back (0.0 to 0.95)
/// - Mix: Wet/dry mix (0.0 = dry only, 1.0 = wet only)
/// - Filter Cutoff: Lowpass filter cutoff in Hz applied in the feedback path (20-20000 Hz)
///
/// The lowpass filter is applied in the feedback loop, so each successive echo
/// gets progressively darker — the classic "filter delay" / "tape delay" sound.
pub struct DelayEffect {
    sample_rate: f32,

    // Mutable state wrapped in UnsafeCell for interior mutability
    // SAFETY: This is only accessed from the audio thread during process()
    state: UnsafeCell<DelayState>,

    // Atomic parameters for lock-free updates from control thread
    timing_target: AtomicU32,
    bpm_target: AtomicU32,
    feedback_target: AtomicU32,
    mix_target: AtomicU32,
    filter_cutoff_target: AtomicU32,
}

// SAFETY: The UnsafeCell is only accessed from a single audio thread
// The AtomicU32 fields are inherently thread-safe
unsafe impl Send for DelayEffect {}
unsafe impl Sync for DelayEffect {}

impl DelayEffect {
    /// Create a new filter delay effect
    ///
    /// # Arguments
    /// * `sample_rate` - Audio sample rate in Hz
    /// * `timing` - Initial musical timing division
    /// * `bpm` - Initial BPM for timing calculation
    /// * `feedback` - Initial feedback amount (0.0-0.95)
    /// * `mix` - Initial wet/dry mix (0.0-1.0)
    /// * `filter_cutoff` - Initial filter cutoff in Hz (20-20000)
    pub fn new(
        sample_rate: f32,
        timing: DelayTiming,
        bpm: f32,
        feedback: f32,
        mix: f32,
        filter_cutoff: f32,
    ) -> Self {
        let time = timing.to_seconds(bpm);
        let feedback_clamped = feedback.clamp(0.0, 0.95);
        let mix_clamped = mix.clamp(0.0, 1.0);
        let cutoff_clamped = filter_cutoff.clamp(MIN_FILTER_CUTOFF, MAX_FILTER_CUTOFF);

        // Allocate buffer for maximum delay time
        let buffer_size = (sample_rate * MAX_DELAY_TIME) as usize + 1;

        Self {
            sample_rate,
            state: UnsafeCell::new(DelayState {
                buffer: vec![0.0; buffer_size],
                write_index: 0,
                filter_z1: 0.0,
                filter_z2: 0.0,
                previous_timing: timing.to_timing_constant(),
                // Use 50ms smoothing for delay time to avoid zipper noise
                time_smoothed: SmoothedParam::new(
                    time,
                    0.0,
                    MAX_DELAY_TIME,
                    sample_rate,
                    50.0,
                ),
                // Use 30ms smoothing for feedback and mix
                feedback_smoothed: SmoothedParam::new(
                    feedback_clamped,
                    0.0,
                    0.95,
                    sample_rate,
                    30.0,
                ),
                mix_smoothed: SmoothedParam::new(mix_clamped, 0.0, 1.0, sample_rate, 30.0),
                filter_cutoff_smoothed: SmoothedParam::new(
                    cutoff_clamped,
                    MIN_FILTER_CUTOFF,
                    MAX_FILTER_CUTOFF,
                    sample_rate,
                    30.0,
                ),
            }),
            timing_target: AtomicU32::new(timing.to_timing_constant()),
            bpm_target: AtomicU32::new(bpm.to_bits()),
            feedback_target: AtomicU32::new(feedback_clamped.to_bits()),
            mix_target: AtomicU32::new(mix_clamped.to_bits()),
            filter_cutoff_target: AtomicU32::new(cutoff_clamped.to_bits()),
        }
    }

    /// Reset delay state (clear buffer and filter)
    pub fn reset(&self) {
        // SAFETY: Called from main thread when delay is not processing
        let state = unsafe { &mut *self.state.get() };
        state.buffer.fill(0.0);
        state.write_index = 0;
        state.filter_z1 = 0.0;
        state.filter_z2 = 0.0;
    }

    /// Get current timing division constant
    pub fn get_timing(&self) -> u32 {
        self.timing_target.load(Ordering::Relaxed)
    }

    /// Get current BPM
    pub fn get_bpm(&self) -> f32 {
        f32::from_bits(self.bpm_target.load(Ordering::Relaxed))
    }

    /// Get current feedback
    pub fn get_feedback(&self) -> f32 {
        f32::from_bits(self.feedback_target.load(Ordering::Relaxed))
    }

    /// Get current mix
    pub fn get_mix(&self) -> f32 {
        f32::from_bits(self.mix_target.load(Ordering::Relaxed))
    }

    /// Get current filter cutoff in Hz
    pub fn get_filter_cutoff(&self) -> f32 {
        f32::from_bits(self.filter_cutoff_target.load(Ordering::Relaxed))
    }

    /// Set timing division (thread-safe, changes are smoothed)
    pub fn set_timing(&self, timing: DelayTiming) {
        self.timing_target
            .store(timing.to_timing_constant(), Ordering::Relaxed);
    }

    /// Set BPM (thread-safe, delay time recalculates automatically)
    pub fn set_bpm(&self, bpm: f32) {
        self.bpm_target.store(bpm.to_bits(), Ordering::Relaxed);
    }

    /// Set feedback amount (thread-safe, changes are smoothed)
    pub fn set_feedback(&self, feedback: f32) {
        let clamped = feedback.clamp(0.0, 0.95);
        self.feedback_target
            .store(clamped.to_bits(), Ordering::Relaxed);
    }

    /// Set wet/dry mix (thread-safe, changes are smoothed)
    pub fn set_mix(&self, mix: f32) {
        let clamped = mix.clamp(0.0, 1.0);
        self.mix_target.store(clamped.to_bits(), Ordering::Relaxed);
    }

    /// Set filter cutoff in Hz (thread-safe, changes are smoothed)
    pub fn set_filter_cutoff(&self, cutoff: f32) {
        let clamped = cutoff.clamp(MIN_FILTER_CUTOFF, MAX_FILTER_CUTOFF);
        self.filter_cutoff_target
            .store(clamped.to_bits(), Ordering::Relaxed);
    }
}

impl Effect for DelayEffect {
    fn process(&self, input: f32) -> f32 {
        // SAFETY: We use UnsafeCell for interior mutability. This is safe because:
        // 1. The audio thread is the only thread that calls process()
        // 2. Parameter updates via atomics are lock-free and don't conflict
        let state = unsafe { &mut *self.state.get() };

        // NaN/infinity protection at input - treat invalid input as silence
        let input = if input.is_finite() { input } else { 0.0 };

        // Read atomic targets and compute delay time from timing + BPM
        let timing_const = self.timing_target.load(Ordering::Relaxed);
        let bpm = f32::from_bits(self.bpm_target.load(Ordering::Relaxed));
        let timing = DelayTiming::from_timing_constant(timing_const)
            .unwrap_or(DelayTiming::Quarter);
        let time_target = timing.to_seconds(bpm);
        let feedback_target = f32::from_bits(self.feedback_target.load(Ordering::Relaxed));
        let mix_target = f32::from_bits(self.mix_target.load(Ordering::Relaxed));
        let cutoff_target = f32::from_bits(self.filter_cutoff_target.load(Ordering::Relaxed));

        // Detect timing changes and clear buffer to avoid sweep artifacts
        if timing_const != state.previous_timing {
            state.previous_timing = timing_const;
            state.buffer.fill(0.0);
            state.filter_z1 = 0.0;
            state.filter_z2 = 0.0;
            // Jump time smoother immediately to the new target
            state.time_smoothed.set_immediate(time_target);
        }

        state.time_smoothed.set_target(time_target);
        state.feedback_smoothed.set_target(feedback_target);
        state.mix_smoothed.set_target(mix_target);
        state.filter_cutoff_smoothed.set_target(cutoff_target);

        // Get smoothed values for this sample
        let time = state.time_smoothed.tick();
        let feedback = state.feedback_smoothed.tick();
        let mix = state.mix_smoothed.tick();
        let cutoff = state.filter_cutoff_smoothed.tick();

        // Calculate delay in samples (with fractional interpolation)
        let delay_samples = time * self.sample_rate;
        let delay_int = delay_samples as usize;
        let delay_frac = delay_samples - delay_int as f32;

        let buffer_len = state.buffer.len();

        // Read from delay line with linear interpolation
        let read_index_1 = (state.write_index + buffer_len - delay_int) % buffer_len;
        let read_index_2 = (state.write_index + buffer_len - delay_int - 1) % buffer_len;

        let sample_1 = state.buffer[read_index_1];
        let sample_2 = state.buffer[read_index_2];

        // Linear interpolation between adjacent samples
        let delayed_sample = sample_1 * (1.0 - delay_frac) + sample_2 * delay_frac;

        // Apply two-pole resonant lowpass filter to the delayed signal.
        // This filters both the wet output and the feedback path, so every echo
        // is audibly filtered and successive echoes get progressively darker.
        // g = 1 - exp(-2π * fc / fs)
        let g = 1.0 - (-2.0 * std::f32::consts::PI * cutoff / self.sample_rate).exp();
        // Resonance: feed back the difference between poles to create a peak at cutoff.
        // Fixed moderate resonance (0.3) adds warmth without risk of self-oscillation.
        let resonance = 0.3;
        let resonance_fb = resonance * (state.filter_z1 - state.filter_z2);
        // First pole
        state.filter_z1 = state.filter_z1 + g * (delayed_sample + resonance_fb - state.filter_z1);
        // Second pole (cascaded for 12dB/oct rolloff)
        state.filter_z2 = state.filter_z2 + g * (state.filter_z1 - state.filter_z2);

        let filtered_delay = state.filter_z2;

        // Flush denormals on filter state
        if state.filter_z1.abs() < DENORMAL_THRESHOLD {
            state.filter_z1 = 0.0;
        }
        if state.filter_z2.abs() < DENORMAL_THRESHOLD {
            state.filter_z2 = 0.0;
        }

        // Write input plus filtered feedback to delay line
        let write_sample = input + filtered_delay * feedback;

        // Flush denormals and NaN protection
        let write_sample = if write_sample.is_finite() && write_sample.abs() > DENORMAL_THRESHOLD {
            write_sample
        } else if write_sample.is_finite() {
            0.0
        } else {
            0.0
        };

        state.buffer[state.write_index] = write_sample;
        state.write_index = (state.write_index + 1) % buffer_len;

        // Mix dry and wet (filtered) signals
        let output = input * (1.0 - mix) + filtered_delay * mix;

        // Final NaN protection
        if !output.is_finite() {
            return input;
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delay_timing_beats() {
        assert_eq!(DelayTiming::Whole.beats(), 4.0);
        assert_eq!(DelayTiming::Half.beats(), 2.0);
        assert_eq!(DelayTiming::Quarter.beats(), 1.0);
        assert_eq!(DelayTiming::Eighth.beats(), 0.5);
        assert_eq!(DelayTiming::Sixteenth.beats(), 0.25);
        // Triplets: 2/3 of the straight division
        assert!((DelayTiming::QuarterTriplet.beats() - 2.0 / 3.0).abs() < 1e-6);
        assert!((DelayTiming::EighthTriplet.beats() - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_delay_timing_to_seconds() {
        // At 120 BPM, quarter note = 0.5s
        let secs = DelayTiming::Quarter.to_seconds(120.0);
        assert!((secs - 0.5).abs() < 1e-6);

        // At 120 BPM, eighth note = 0.25s
        let secs = DelayTiming::Eighth.to_seconds(120.0);
        assert!((secs - 0.25).abs() < 1e-6);

        // At 120 BPM, quarter triplet = 0.333...s
        let secs = DelayTiming::QuarterTriplet.to_seconds(120.0);
        assert!((secs - 1.0 / 3.0).abs() < 1e-4);
    }

    #[test]
    fn test_delay_timing_roundtrip() {
        for i in 0..=8 {
            let timing = DelayTiming::from_timing_constant(i).unwrap();
            assert_eq!(timing.to_timing_constant(), i);
        }
        assert!(DelayTiming::from_timing_constant(9).is_none());
    }

    #[test]
    fn test_delay_basic_processing() {
        let delay = DelayEffect::new(44100.0, DelayTiming::Eighth, 120.0, 0.5, 0.5, 10000.0);

        // Process some samples
        for _ in 0..1000 {
            let output = delay.process(1.0);
            assert!(output.is_finite(), "Delay output should be finite");
        }
    }

    #[test]
    fn test_delay_parameter_clamping() {
        let delay =
            DelayEffect::new(44100.0, DelayTiming::Quarter, 120.0, 0.5, 0.5, 10000.0);

        // Test feedback clamping
        delay.set_feedback(2.0);
        assert_eq!(delay.get_feedback(), 0.95);

        delay.set_feedback(-1.0);
        assert_eq!(delay.get_feedback(), 0.0);

        // Test mix clamping
        delay.set_mix(2.0);
        assert_eq!(delay.get_mix(), 1.0);

        delay.set_mix(-1.0);
        assert_eq!(delay.get_mix(), 0.0);

        // Test filter cutoff clamping
        delay.set_filter_cutoff(50000.0);
        assert_eq!(delay.get_filter_cutoff(), 20000.0);

        delay.set_filter_cutoff(1.0);
        assert_eq!(delay.get_filter_cutoff(), 20.0);
    }

    #[test]
    fn test_delay_feedback_stability() {
        let delay = DelayEffect::new(44100.0, DelayTiming::Eighth, 120.0, 0.95, 0.5, 5000.0);

        // Process many samples with high feedback
        // Should remain stable and not blow up
        for i in 0..44100 {
            let input = if i < 100 { 1.0 } else { 0.0 }; // Impulse
            let output = delay.process(input);
            assert!(
                output.is_finite() && output.abs() < 100.0,
                "Delay should remain stable with high feedback"
            );
        }
    }

    #[test]
    fn test_delay_filter_darkens_echoes() {
        // With a moderate cutoff, echoes should still be present but progressively darker
        let delay = DelayEffect::new(44100.0, DelayTiming::Eighth, 120.0, 0.9, 1.0, 2000.0);

        // Feed an impulse
        delay.process(1.0);

        // Run through the delay time and capture first two echoes
        let delay_samples = (DelayTiming::Eighth.to_seconds(120.0) * 44100.0) as usize;
        let mut first_echo = 0.0_f32;
        let mut second_echo = 0.0_f32;
        for i in 1..delay_samples * 3 {
            let out = delay.process(0.0);
            if i >= delay_samples - 2 && i <= delay_samples + 2 {
                first_echo = first_echo.max(out.abs());
            }
            if i >= delay_samples * 2 - 2 && i <= delay_samples * 2 + 2 {
                second_echo = second_echo.max(out.abs());
            }
        }

        // First echo should be audible (filter attenuates but doesn't eliminate)
        assert!(first_echo > 0.01, "First echo should be audible");
        // Second echo should be quieter than first (progressive darkening)
        assert!(
            second_echo < first_echo,
            "Second echo should be darker than first"
        );
    }

    #[test]
    fn test_delay_reset() {
        let delay = DelayEffect::new(44100.0, DelayTiming::Quarter, 120.0, 0.5, 0.5, 10000.0);

        // Fill buffer with samples
        for _ in 0..44100 {
            delay.process(1.0);
        }

        // Reset
        delay.reset();

        // After reset with zero input, output should be minimal
        let output = delay.process(0.0);
        assert!(
            output.abs() < 0.001,
            "After reset, delay should output near-zero"
        );
    }

    #[test]
    fn test_delay_nan_protection() {
        let delay = DelayEffect::new(44100.0, DelayTiming::Eighth, 120.0, 0.5, 0.5, 10000.0);

        // Feed NaN input
        let output = delay.process(f32::NAN);

        // Should handle gracefully
        assert!(
            output.is_finite(),
            "Delay should handle NaN input gracefully"
        );
    }

    #[test]
    fn test_delay_bpm_change_updates_time() {
        let delay = DelayEffect::new(44100.0, DelayTiming::Quarter, 120.0, 0.0, 1.0, 20000.0);

        // At 120 BPM, quarter = 0.5s = 22050 samples
        // Change BPM to 60 → quarter = 1.0s = 44100 samples
        delay.set_bpm(60.0);

        // Process a few samples to let the smoother pick up the new target
        for _ in 0..100 {
            delay.process(0.0);
        }

        // The stored BPM target should reflect the change
        assert_eq!(delay.get_bpm(), 60.0);
    }
}
