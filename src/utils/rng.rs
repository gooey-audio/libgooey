//! Small deterministic pseudo-random generator.
//!
//! Audio code needs randomness (grain spray, humanized velocities) but must
//! stay reproducible: an offline bounce of the same material has to render
//! identically every time. A seeded xorshift gives that — same seed, same
//! sequence — while still varying from event to event as the state advances.
//!
//! Deliberately not a crypto or statistics-grade generator. It is fast, has no
//! dependencies, and is good enough for perceptual jitter.

/// Xorshift32 PRNG. Cheap enough to call per sample.
#[derive(Clone, Copy, Debug)]
pub struct XorShift32 {
    state: u32,
}

impl XorShift32 {
    /// Seed the generator. A zero seed would make xorshift degenerate to all
    /// zeroes forever, so it is replaced with a fixed non-zero constant.
    pub fn new(seed: u32) -> Self {
        Self {
            state: if seed == 0 { 0x6d2b_79f5 } else { seed },
        }
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }

    /// Next value in `[0, 1]`.
    #[inline]
    pub fn next_f32(&mut self) -> f32 {
        self.next_u32() as f32 / u32::MAX as f32
    }

    /// Next value in `[-1, 1]`.
    #[inline]
    pub fn next_bipolar(&mut self) -> f32 {
        self.next_f32() * 2.0 - 1.0
    }
}

impl Default for XorShift32 {
    fn default() -> Self {
        Self::new(0x1234_abcd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_replays_the_same_sequence() {
        let mut a = XorShift32::new(42);
        let mut b = XorShift32::new(42);
        for _ in 0..64 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = XorShift32::new(1);
        let mut b = XorShift32::new(2);
        assert!((0..16).any(|_| a.next_u32() != b.next_u32()));
    }

    #[test]
    fn a_zero_seed_does_not_stick_at_zero() {
        let mut rng = XorShift32::new(0);
        assert!((0..16).any(|_| rng.next_u32() != 0));
    }

    #[test]
    fn output_stays_in_range() {
        let mut rng = XorShift32::default();
        for _ in 0..1024 {
            let unipolar = rng.next_f32();
            assert!((0.0..=1.0).contains(&unipolar), "{unipolar}");
            let bipolar = rng.next_bipolar();
            assert!((-1.0..=1.0).contains(&bipolar), "{bipolar}");
        }
    }
}
