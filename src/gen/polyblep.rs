/// PolyBLEP (Polynomial Band-Limited Step) anti-aliasing for saw and square waves.
///
/// Uses f64 phase accumulators for precision at low frequencies.

/// Correction term applied near discontinuities in a waveform.
/// `t` is the current phase [0, 1), `dt` is the phase increment per sample.
#[inline]
pub fn poly_blep(t: f64, dt: f64) -> f64 {
    if t < dt {
        // Just passed a discontinuity (phase wrapped)
        let t = t / dt;
        2.0 * t - t * t - 1.0
    } else if t > 1.0 - dt {
        // Approaching a discontinuity
        let t = (t - 1.0) / dt;
        t * t + 2.0 * t + 1.0
    } else {
        0.0
    }
}

/// Band-limited sawtooth wave using PolyBLEP.
/// `phase` in [0, 1), `phase_inc` is frequency / sample_rate.
#[inline]
pub fn polyblep_saw(phase: f64, phase_inc: f64) -> f32 {
    let naive = 2.0 * phase - 1.0;
    let blep = poly_blep(phase, phase_inc);
    (naive - blep) as f32
}

/// Band-limited square wave using PolyBLEP.
/// `phase` in [0, 1), `phase_inc` is frequency / sample_rate.
#[inline]
pub fn polyblep_square(phase: f64, phase_inc: f64) -> f32 {
    let naive: f64 = if phase < 0.5 { 1.0 } else { -1.0 };
    let blep1 = poly_blep(phase, phase_inc);
    let phase2 = (phase + 0.5) % 1.0;
    let blep2 = poly_blep(phase2, phase_inc);
    (naive + blep1 - blep2) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_polyblep_saw_range() {
        let sample_rate = 44100.0;
        let freq = 100.0;
        let phase_inc = freq / sample_rate;
        let mut phase = 0.0_f64;

        for _ in 0..44100 {
            let sample = polyblep_saw(phase, phase_inc);
            assert!(
                sample >= -1.1 && sample <= 1.1,
                "saw out of range: {}",
                sample
            );
            phase += phase_inc;
            phase -= phase.floor();
        }
    }

    #[test]
    fn test_polyblep_square_range() {
        let sample_rate = 44100.0;
        let freq = 100.0;
        let phase_inc = freq / sample_rate;
        let mut phase = 0.0_f64;

        for _ in 0..44100 {
            let sample = polyblep_square(phase, phase_inc);
            assert!(
                sample >= -1.1 && sample <= 1.1,
                "square out of range: {}",
                sample
            );
            phase += phase_inc;
            phase -= phase.floor();
        }
    }

    #[test]
    fn test_polyblep_saw_not_silent() {
        let phase_inc = 100.0 / 44100.0;
        let mut phase = 0.0_f64;
        let mut energy = 0.0_f64;

        for _ in 0..44100 {
            let s = polyblep_saw(phase, phase_inc) as f64;
            energy += s * s;
            phase += phase_inc;
            phase -= phase.floor();
        }

        assert!(energy > 1.0, "saw should produce audible output");
    }

    #[test]
    fn test_polyblep_square_not_silent() {
        let phase_inc = 100.0 / 44100.0;
        let mut phase = 0.0_f64;
        let mut energy = 0.0_f64;

        for _ in 0..44100 {
            let s = polyblep_square(phase, phase_inc) as f64;
            energy += s * s;
            phase += phase_inc;
            phase -= phase.floor();
        }

        assert!(energy > 1.0, "square should produce audible output");
    }
}
