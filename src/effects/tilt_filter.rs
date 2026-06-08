use crate::effects::Effect;
use crate::filters::state_variable_tpt::StateVariableFilterTpt;
use crate::utils::smoother::SmoothedParam;
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU32, Ordering};

const DENORMAL_THRESHOLD: f32 = 1e-15;

// Frequency range constants for logarithmic sweep
const LP_FREQ_MIN: f32 = 80.0;
const LP_FREQ_MAX: f32 = 20000.0;
const HP_FREQ_MIN: f32 = 20.0;
const HP_FREQ_MAX: f32 = 8000.0;

// Half-width (in knob units) of the bypass dead-zone around center. Inside
// this zone the filter crossfades to pure dry so exact center is a clean
// passthrough detent; outside it the filter runs fully wet. Keeping this
// narrow means the filter's full slope and resonance are available across
// essentially the entire travel, instead of being diluted by a dry blend.
const CENTER_DEAD_ZONE: f32 = 0.02;

struct TiltFilterState {
    cutoff_smoothed: SmoothedParam,
    resonance_smoothed: SmoothedParam,
    svf: StateVariableFilterTpt,
}

/// Tilt filter effect - a single knob controls lowpass/highpass filtering.
///
/// The cutoff parameter (0.0-1.0) works as follows:
/// - 0.5 = no filtering (passthrough)
/// - Below 0.5 = lowpass, getting darker toward 0.0
/// - Above 0.5 = highpass, getting brighter toward 1.0
///
/// The filter runs fully wet across the whole sweep: transparency at center
/// comes from parking the cutoff at the edge of the audible range, not from
/// blending dry signal in. This keeps the full filter slope and resonance
/// available through the entire mid-band. A narrow dead-zone around 0.5
/// crossfades to pure dry so exact center is a clean bypass detent.
pub struct TiltFilterEffect {
    state: UnsafeCell<TiltFilterState>,
    cutoff_target: AtomicU32,
    resonance_target: AtomicU32,
}

// SAFETY: UnsafeCell is only accessed from a single audio thread.
// AtomicU32 fields are inherently thread-safe.
unsafe impl Send for TiltFilterEffect {}
unsafe impl Sync for TiltFilterEffect {}

impl TiltFilterEffect {
    pub fn new(sample_rate: f32) -> Self {
        let default_cutoff: f32 = 0.5;
        let default_resonance: f32 = 0.0;

        Self {
            state: UnsafeCell::new(TiltFilterState {
                cutoff_smoothed: SmoothedParam::new(default_cutoff, 0.0, 1.0, sample_rate, 30.0),
                resonance_smoothed: SmoothedParam::new(
                    default_resonance,
                    0.0,
                    1.0,
                    sample_rate,
                    30.0,
                ),
                svf: StateVariableFilterTpt::new(sample_rate, 1000.0, 0.5),
            }),
            cutoff_target: AtomicU32::new(default_cutoff.to_bits()),
            resonance_target: AtomicU32::new(default_resonance.to_bits()),
        }
    }

    pub fn set_cutoff(&self, value: f32) {
        let clamped = value.clamp(0.0, 1.0);
        self.cutoff_target
            .store(clamped.to_bits(), Ordering::Relaxed);
    }

    pub fn get_cutoff(&self) -> f32 {
        f32::from_bits(self.cutoff_target.load(Ordering::Relaxed))
    }

    pub fn set_resonance(&self, value: f32) {
        let clamped = value.clamp(0.0, 1.0);
        self.resonance_target
            .store(clamped.to_bits(), Ordering::Relaxed);
    }

    pub fn get_resonance(&self) -> f32 {
        f32::from_bits(self.resonance_target.load(Ordering::Relaxed))
    }

    pub fn reset(&self) {
        let state = unsafe { &mut *self.state.get() };
        state.svf.reset();
    }
}

impl Effect for TiltFilterEffect {
    fn process(&self, input: f32) -> f32 {
        let state = unsafe { &mut *self.state.get() };

        // Read atomic targets and update smoothers
        let cutoff_target = f32::from_bits(self.cutoff_target.load(Ordering::Relaxed));
        let resonance_target = f32::from_bits(self.resonance_target.load(Ordering::Relaxed));

        state.cutoff_smoothed.set_target(cutoff_target);
        state.resonance_smoothed.set_target(resonance_target);

        let knob = state.cutoff_smoothed.tick();
        let resonance = state.resonance_smoothed.tick();

        // Frequency + filter type from knob position. The filter runs fully
        // wet across the whole sweep; the center is made transparent by parking
        // the cutoff at the edge of the audible range (LP near 20 kHz, HP near
        // 20 Hz) rather than by blending in dry signal. This preserves the full
        // filter slope and resonance through the entire mid-band, which is where
        // the old depth-tied-to-position blend washed the filter out.
        let (filter_freq, use_lowpass) = if knob < 0.5 {
            // LP region: knob 0.5 -> 0.0 sweeps cutoff 20 kHz -> 80 Hz
            let t = knob * 2.0;
            let freq = LP_FREQ_MIN * (LP_FREQ_MAX / LP_FREQ_MIN).powf(t);
            (freq, true)
        } else {
            // HP region: knob 0.5 -> 1.0 sweeps cutoff 20 Hz -> 8 kHz
            let t = (knob - 0.5) * 2.0;
            let freq = HP_FREQ_MIN * (HP_FREQ_MAX / HP_FREQ_MIN).powf(t);
            (freq, false)
        };

        // Engagement crossfade: 0 at exact center (clean bypass detent),
        // ramping to fully wet at the edge of the narrow dead-zone. The SVF is
        // still processed even when bypassed so its state stays warm and
        // re-engaging from center doesn't produce a transient.
        let engage = ((knob - 0.5).abs() / CENTER_DEAD_ZONE).min(1.0);

        // Map resonance (0-1) to Q factor (0.5 to 8.5)
        let q = 0.5 + resonance * 8.0;

        state.svf.set_params(filter_freq, q);
        let (low, _band, high) = state.svf.process_all(input);

        let wet = if use_lowpass { low } else { high };
        let output = input * (1.0 - engage) + wet * engage;

        // NaN/infinity protection
        if !output.is_finite() {
            state.svf.reset();
            return 0.0;
        }

        // Flush denormals
        if output.abs() < DENORMAL_THRESHOLD {
            return 0.0;
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_passthrough_at_center() {
        let filter = TiltFilterEffect::new(44100.0);
        // Cutoff defaults to 0.5 (center)

        // Let smoother settle
        for _ in 0..4410 {
            filter.process(0.0);
        }

        // At center, output should equal input
        let input = 0.7;
        let output = filter.process(input);
        assert!(
            (output - input).abs() < 0.001,
            "Center position should be passthrough, got {} for input {}",
            output,
            input
        );
    }

    #[test]
    fn test_lowpass_attenuates_high_freq() {
        let filter = TiltFilterEffect::new(44100.0);
        filter.set_cutoff(0.0); // Full lowpass at 80 Hz

        // Let smoother settle
        for _ in 0..4410 {
            filter.process(0.0);
        }

        // Feed a high-frequency sine (10 kHz) and measure amplitude
        let freq = 10000.0;
        let mut max_output: f32 = 0.0;
        for i in 0..4410 {
            let t = i as f32 / 44100.0;
            let input = (2.0 * std::f32::consts::PI * freq * t).sin();
            let output = filter.process(input);
            max_output = max_output.max(output.abs());
        }

        assert!(
            max_output < 0.1,
            "Lowpass should attenuate 10kHz, got peak {}",
            max_output
        );
    }

    #[test]
    fn test_highpass_attenuates_low_freq() {
        let filter = TiltFilterEffect::new(44100.0);
        filter.set_cutoff(1.0); // Full highpass at 8 kHz

        // Let smoother settle
        for _ in 0..4410 {
            filter.process(0.0);
        }

        // Feed a low-frequency sine (100 Hz) and measure amplitude
        let freq = 100.0;
        let mut max_output: f32 = 0.0;
        for i in 0..4410 {
            let t = i as f32 / 44100.0;
            let input = (2.0 * std::f32::consts::PI * freq * t).sin();
            let output = filter.process(input);
            max_output = max_output.max(output.abs());
        }

        assert!(
            max_output < 0.1,
            "Highpass should attenuate 100Hz, got peak {}",
            max_output
        );
    }

    #[test]
    fn test_midband_filters_fully() {
        // Regression for the depth-tied-to-position blend: a mid-travel
        // lowpass setting must actually attenuate content well above its
        // cutoff, not just shelve it down by a fraction. With the cutoff
        // parked near ~1.3 kHz, a 12 kHz tone should be strongly reduced.
        let filter = TiltFilterEffect::new(44100.0);
        filter.set_cutoff(0.25); // mid lowpass region, well inside full-wet zone

        for _ in 0..4410 {
            filter.process(0.0);
        }

        let freq = 12000.0;
        let mut max_output: f32 = 0.0;
        for i in 0..4410 {
            let t = i as f32 / 44100.0;
            let input = (2.0 * std::f32::consts::PI * freq * t).sin();
            max_output = max_output.max(filter.process(input).abs());
        }

        // A diluted shelf would leave this near unity; a real lowpass crushes it.
        assert!(
            max_output < 0.25,
            "Mid-travel lowpass should strongly attenuate 12kHz, got peak {}",
            max_output
        );
    }

    #[test]
    fn test_parameter_clamping() {
        let filter = TiltFilterEffect::new(44100.0);

        filter.set_cutoff(2.0);
        assert_eq!(filter.get_cutoff(), 1.0);

        filter.set_cutoff(-1.0);
        assert_eq!(filter.get_cutoff(), 0.0);

        filter.set_resonance(5.0);
        assert_eq!(filter.get_resonance(), 1.0);

        filter.set_resonance(-1.0);
        assert_eq!(filter.get_resonance(), 0.0);
    }

    #[test]
    fn test_nan_protection() {
        let filter = TiltFilterEffect::new(44100.0);
        filter.set_cutoff(0.0);

        for _ in 0..1000 {
            filter.process(0.5);
        }

        let output = filter.process(f32::NAN);
        assert!(output.is_finite(), "Should handle NaN input");
    }

    #[test]
    fn test_stability_full_sweep() {
        let filter = TiltFilterEffect::new(44100.0);

        // Sweep cutoff from 0 to 1 while processing
        for i in 0..44100 {
            let t = i as f32 / 44100.0;
            filter.set_cutoff(t);
            filter.set_resonance(0.8);

            let input = (i as f32 * 0.05).sin();
            let output = filter.process(input);

            assert!(
                output.is_finite() && output.abs() < 10.0,
                "Should remain stable during sweep at t={}, got {}",
                t,
                output
            );
        }
    }

    #[test]
    fn test_stability_high_resonance() {
        let filter = TiltFilterEffect::new(44100.0);
        filter.set_cutoff(0.1);
        filter.set_resonance(1.0);

        for i in 0..44100 {
            let input = if i < 100 { 1.0 } else { 0.0 };
            let output = filter.process(input);
            assert!(
                output.is_finite() && output.abs() < 100.0,
                "Should remain stable at high resonance"
            );
        }
    }
}
