//! Decibel conversions shared by the instruments and effects.
//!
//! The same three lines of `powf`/`log10` had accumulated in several modules;
//! these are the canonical versions. [`db_to_gain`] keeps an exact early return
//! at 0 dB, which callers rely on to prove a control is bit-identical to
//! bypass when it sits at its neutral position.

/// Decibels to a linear amplitude multiplier. `0.0 dB` returns exactly `1.0`.
#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    if db == 0.0 {
        1.0
    } else {
        10.0_f32.powf(db / 20.0)
    }
}

/// Linear amplitude to decibels, floored at [`SILENCE_DB`] so a silent signal
/// returns a usable number instead of `-inf`.
#[inline]
pub fn gain_to_db(gain: f32) -> f32 {
    let gain = gain.abs();
    if gain <= 0.0 {
        SILENCE_DB
    } else {
        (20.0 * gain.log10()).max(SILENCE_DB)
    }
}

/// Power (a mean square, not an amplitude) to decibels, floored at
/// [`SILENCE_DB`]. Use this on energy measurements so the `10 * log10` is not
/// mistakenly written as the `20 * log10` an amplitude needs.
#[inline]
pub fn power_to_db(power: f32) -> f32 {
    if power <= 0.0 {
        SILENCE_DB
    } else {
        (10.0 * power.log10()).max(SILENCE_DB)
    }
}

/// The floor both conversions return for silence. Far below anything audible,
/// and finite, so it can be averaged and compared without special cases.
pub const SILENCE_DB: f32 = -160.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_db_is_exactly_unity() {
        assert_eq!(db_to_gain(0.0), 1.0);
    }

    #[test]
    fn six_db_is_about_double() {
        assert!((db_to_gain(6.0206) - 2.0).abs() < 1e-4);
        assert!((db_to_gain(-6.0206) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn gain_and_db_round_trip() {
        for gain in [0.001_f32, 0.05, 0.5, 1.0, 2.0] {
            assert!((db_to_gain(gain_to_db(gain)) - gain).abs() < 1e-5);
        }
    }

    #[test]
    fn power_is_ten_log_ten_not_twenty() {
        // An amplitude of 0.1 has a mean square of 0.01; both are -20 dB.
        assert!((gain_to_db(0.1) + 20.0).abs() < 1e-4);
        assert!((power_to_db(0.01) + 20.0).abs() < 1e-4);
    }

    #[test]
    fn silence_is_floored_and_finite() {
        assert_eq!(power_to_db(0.0), SILENCE_DB);
        assert_eq!(gain_to_db(0.0), SILENCE_DB);
        assert!(power_to_db(1e-30).is_finite());
    }
}
