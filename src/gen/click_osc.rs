//! Click oscillator - one-shot wavetable player based on Max/MSP click~
//!
//! Plays a pre-loaded waveform once on trigger, then outputs silence.
//! Used for percussive attack transients in tom drums.

/// The 64-sample impulse waveform from the Max/MSP tom patch setimpulse subpatch
const TOM_IMPULSE: [f32; 64] = [
    0.884058, 0.942029, 0.913043, 0.869565, 0.833333, 0.797101, 0.772947, 0.748792,
    0.724638, 0.695652, 0.666667, 0.637681, 0.619565, 0.601449, 0.583333, 0.565217,
    0.536232, 0.507246, 0.478261, 0.449275, 0.42029, 0.391304, 0.371981, 0.352657,
    0.333333, 0.304348, 0.275362, 0.23913, 0.202899, 0.181159, 0.15942, 0.137681,
    0.115942, 0.101449, 0.086957, 0.072464, 0.057971, 0.043478, 0.028986, 0.014493,
    0.009662, 0.004831, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.014493, 0.0,
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
];

/// Click oscillator that plays a waveform once on trigger
///
/// Based on Max/MSP's `click~` object which plays a buffer once when triggered.
/// This implementation uses a fixed 64-sample impulse waveform designed for
/// tom drum attack transients.
pub struct ClickOsc {
    /// Current playback position (0 to 63)
    position: usize,
    /// Whether currently playing
    is_playing: bool,
}

impl ClickOsc {
    /// Create a new click oscillator
    pub fn new() -> Self {
        Self {
            position: 0,
            is_playing: false,
        }
    }

    /// Reset the oscillator state
    pub fn reset(&mut self) {
        self.position = 0;
        self.is_playing = false;
    }

    /// Trigger playback from the beginning
    pub fn trigger(&mut self) {
        self.position = 0;
        self.is_playing = true;
    }

    /// Check if currently playing
    pub fn is_active(&self) -> bool {
        self.is_playing
    }

    /// Generate one sample
    ///
    /// Returns the next sample from the waveform, or 0.0 if not playing.
    /// Automatically stops when the waveform is complete.
    pub fn tick(&mut self) -> f32 {
        if !self.is_playing {
            return 0.0;
        }

        if self.position >= TOM_IMPULSE.len() {
            self.is_playing = false;
            return 0.0;
        }

        let sample = TOM_IMPULSE[self.position];
        self.position += 1;

        // Auto-stop at end of waveform
        if self.position >= TOM_IMPULSE.len() {
            self.is_playing = false;
        }

        sample
    }
}

impl Default for ClickOsc {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_click_starts_silent() {
        let mut click = ClickOsc::new();
        assert_eq!(click.tick(), 0.0);
        assert!(!click.is_active());
    }

    #[test]
    fn test_click_plays_on_trigger() {
        let mut click = ClickOsc::new();
        click.trigger();
        assert!(click.is_active());

        // First sample should be non-zero
        let first = click.tick();
        assert!(first > 0.0);
        assert!((first - 0.884058).abs() < 0.0001);
    }

    #[test]
    fn test_click_plays_exactly_64_samples() {
        let mut click = ClickOsc::new();
        click.trigger();

        let mut count = 0;
        while click.is_active() {
            click.tick();
            count += 1;
            // Safety limit
            if count > 100 {
                break;
            }
        }

        assert_eq!(count, 64);
    }

    #[test]
    fn test_click_stops_after_waveform() {
        let mut click = ClickOsc::new();
        click.trigger();

        // Play through all 64 samples
        for _ in 0..64 {
            click.tick();
        }

        assert!(!click.is_active());
        assert_eq!(click.tick(), 0.0);
    }

    #[test]
    fn test_click_can_retrigger() {
        let mut click = ClickOsc::new();
        click.trigger();

        // Play a few samples
        for _ in 0..10 {
            click.tick();
        }

        // Retrigger
        click.trigger();
        assert!(click.is_active());

        // Should start from beginning again
        let first = click.tick();
        assert!((first - 0.884058).abs() < 0.0001);
    }

    #[test]
    fn test_reset() {
        let mut click = ClickOsc::new();
        click.trigger();

        // Play a few samples
        for _ in 0..10 {
            click.tick();
        }

        click.reset();
        assert!(!click.is_active());
        assert_eq!(click.tick(), 0.0);
    }
}
