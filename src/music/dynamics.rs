//! Chord dynamics: how hard each voice of a chord is struck.
//!
//! [`super::voicing`] decides *which* notes a chord becomes. This module
//! decides *how hard* each one is played, which is what separates a sampled
//! chord from a mechanical one.
//!
//! Two things are going on, and they are independent:
//!
//! 1. **Weighting** — a pianist does not strike every note of a chord equally.
//!    Bringing out the top voice makes a melody sing over its accompaniment;
//!    leaning on the root anchors a comping figure. [`VelocityProfile`] picks
//!    which voice is emphasized and by how much.
//! 2. **Humanizing** — even the "same" note struck twice is never identical.
//!    A small random spread per note keeps repeated chords from sounding
//!    photocopied.
//!
//! Randomness is seeded and deterministic, so an offline bounce of the same
//! material renders identically every time, while successive chords in one
//! performance still differ as the generator advances.

use std::fmt;

use crate::utils::XorShift32;

/// How velocity is distributed across the voices of a chord.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum VelocityProfile {
    /// Every voice struck equally — what a sequencer does, and the flattest
    /// sounding option. Humanizing still applies on top.
    #[default]
    Even,
    /// Top voice loudest, inner voices pulled back, root kept present. This is
    /// how a pianist voices a melody over an accompaniment, and it is the most
    /// useful default for chord progressions with a tune on top.
    MelodyLead,
    /// Root loudest, thinning toward the top. Anchors a left-hand comping
    /// figure without the upper extensions dominating.
    BassLead,
}

impl VelocityProfile {
    pub const ALL: [VelocityProfile; 3] = [
        VelocityProfile::Even,
        VelocityProfile::MelodyLead,
        VelocityProfile::BassLead,
    ];

    pub fn next(self) -> Self {
        match self {
            VelocityProfile::Even => VelocityProfile::MelodyLead,
            VelocityProfile::MelodyLead => VelocityProfile::BassLead,
            VelocityProfile::BassLead => VelocityProfile::Even,
        }
    }

    /// Relative strength of voice `index` (0 = lowest) in a chord of `count`
    /// voices, as a multiplier in roughly `[0.6, 1.0]`.
    ///
    /// A single-voice chord is always 1.0: there is nothing to balance against,
    /// and attenuating a lone note would just make the instrument quieter.
    pub fn weight(self, index: usize, count: usize) -> f32 {
        if count <= 1 || index >= count {
            return 1.0;
        }
        // Position within the chord: 0.0 at the lowest voice, 1.0 at the top.
        let position = index as f32 / (count - 1) as f32;

        match self {
            VelocityProfile::Even => 1.0,
            // The three roles are stated outright rather than left to fall out
            // of a curve: a curve steep enough to keep every inner voice under
            // the root also flattens the melody, and the two constraints fight
            // each other as the chord grows.
            VelocityProfile::MelodyLead if index == count - 1 => 1.0,
            VelocityProfile::MelodyLead if index == 0 => 0.86,
            VelocityProfile::MelodyLead => {
                // Inner voices sit below both, leaning gently toward the melody.
                let inner = (index - 1) as f32 / (count - 2).max(1) as f32;
                0.68 + 0.10 * inner
            }
            VelocityProfile::BassLead => 1.0 - 0.34 * position,
        }
    }
}

impl fmt::Display for VelocityProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VelocityProfile::Even => write!(f, "Even"),
            VelocityProfile::MelodyLead => write!(f, "Melody"),
            VelocityProfile::BassLead => write!(f, "Bass"),
        }
    }
}

/// Largest random deviation applied at `humanize == 1.0`, as a fraction of the
/// base velocity. Kept modest: past roughly a fifth, jitter stops reading as a
/// human touch and starts sounding like a fault.
const MAX_HUMANIZE: f32 = 0.22;

/// Turns a chord's base velocity into one velocity per voice, applying a
/// [`VelocityProfile`] and a seeded random spread.
///
/// Hold one of these per performer (not per chord) so the random sequence keeps
/// advancing — that is what stops the same chord pressed twice from sounding
/// identical.
#[derive(Clone, Debug)]
pub struct ChordDynamics {
    profile: VelocityProfile,
    humanize: f32,
    rng: XorShift32,
}

impl Default for ChordDynamics {
    fn default() -> Self {
        Self::new(VelocityProfile::default(), 0.35)
    }
}

impl ChordDynamics {
    /// `humanize` is normalized 0–1 and clamped; 0.0 is perfectly mechanical.
    pub fn new(profile: VelocityProfile, humanize: f32) -> Self {
        Self::with_seed(profile, humanize, 0x5eed_1e55)
    }

    /// Same, with an explicit seed so a render can be reproduced exactly.
    pub fn with_seed(profile: VelocityProfile, humanize: f32, seed: u32) -> Self {
        Self {
            profile,
            humanize: humanize.clamp(0.0, 1.0),
            rng: XorShift32::new(seed),
        }
    }

    pub fn profile(&self) -> VelocityProfile {
        self.profile
    }

    pub fn set_profile(&mut self, profile: VelocityProfile) {
        self.profile = profile;
    }

    pub fn humanize(&self) -> f32 {
        self.humanize
    }

    pub fn set_humanize(&mut self, humanize: f32) {
        self.humanize = humanize.clamp(0.0, 1.0);
    }

    /// Restart the random sequence, so a bounce can be made repeatable.
    pub fn reseed(&mut self, seed: u32) {
        self.rng = XorShift32::new(seed);
    }

    /// Per-voice velocities for a chord of `count` notes struck at `base`.
    ///
    /// Voice 0 is the lowest note, matching the ordering
    /// [`super::voicing::apply_voicing`] returns. Every result is clamped to
    /// `[0.05, 1.0]` — never to zero, so humanizing can never silently drop a
    /// note out of a chord.
    pub fn velocities(&mut self, base: f32, count: usize) -> Vec<f32> {
        let base = base.clamp(0.0, 1.0);
        (0..count)
            .map(|index| {
                let weighted = base * self.profile.weight(index, count);
                let jitter = self.rng.next_bipolar() * self.humanize * MAX_HUMANIZE;
                (weighted * (1.0 + jitter)).clamp(0.05, 1.0)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mechanical dynamics, so weighting can be tested without jitter.
    fn exact(profile: VelocityProfile) -> ChordDynamics {
        ChordDynamics::new(profile, 0.0)
    }

    #[test]
    fn even_strikes_every_voice_equally() {
        let v = exact(VelocityProfile::Even).velocities(0.8, 4);
        assert!(v.iter().all(|x| (x - 0.8).abs() < 1e-6), "{v:?}");
    }

    #[test]
    fn melody_lead_puts_the_top_voice_on_top() {
        let v = exact(VelocityProfile::MelodyLead).velocities(1.0, 4);
        let top = v[3];
        assert!(
            v[..3].iter().all(|&x| x < top),
            "top voice should be loudest: {v:?}"
        );
        // ...and keeps a foundation: the root beats every inner voice.
        assert!(v[0] > v[1] && v[0] > v[2], "root should anchor: {v:?}");
    }

    #[test]
    fn melody_lead_keeps_its_ordering_at_every_chord_size() {
        // The melody-over-root-over-inner relationship has to survive a triad
        // and a thirteenth chord alike, not just the four-note case.
        for count in 3..=8 {
            let v = exact(VelocityProfile::MelodyLead).velocities(1.0, count);
            let top = v[count - 1];
            let root = v[0];
            let loudest_inner = v[1..count - 1].iter().cloned().fold(0.0_f32, f32::max);
            assert!(top > root, "count {count}: melody under root: {v:?}");
            assert!(
                root > loudest_inner,
                "count {count}: an inner voice beat the root: {v:?}"
            );
        }
    }

    #[test]
    fn bass_lead_descends_from_the_root() {
        let v = exact(VelocityProfile::BassLead).velocities(1.0, 4);
        assert!(
            v.windows(2).all(|w| w[0] > w[1]),
            "should thin upward: {v:?}"
        );
        assert_eq!(v[0], 1.0);
    }

    #[test]
    fn a_single_voice_is_never_attenuated() {
        for profile in VelocityProfile::ALL {
            let v = exact(profile).velocities(0.9, 1);
            assert_eq!(v.len(), 1);
            assert!((v[0] - 0.9).abs() < 1e-6, "{profile:?} -> {v:?}");
        }
    }

    #[test]
    fn humanizing_varies_repeats_of_the_same_chord() {
        let mut dynamics = ChordDynamics::new(VelocityProfile::Even, 1.0);
        let first = dynamics.velocities(0.7, 4);
        let second = dynamics.velocities(0.7, 4);
        assert_ne!(first, second, "successive chords should not be identical");
        // ...and voices within one chord differ from each other.
        assert!(first.windows(2).any(|w| w[0] != w[1]), "{first:?}");
    }

    #[test]
    fn zero_humanize_is_perfectly_repeatable() {
        let mut dynamics = ChordDynamics::new(VelocityProfile::MelodyLead, 0.0);
        let first = dynamics.velocities(0.7, 4);
        let second = dynamics.velocities(0.7, 4);
        assert_eq!(first, second);
    }

    #[test]
    fn the_same_seed_reproduces_a_render() {
        let mut a = ChordDynamics::with_seed(VelocityProfile::MelodyLead, 0.8, 7);
        let mut b = ChordDynamics::with_seed(VelocityProfile::MelodyLead, 0.8, 7);
        for count in 2..6 {
            assert_eq!(a.velocities(0.6, count), b.velocities(0.6, count));
        }
        // Reseeding replays it.
        a.reseed(7);
        b.reseed(7);
        assert_eq!(a.velocities(0.6, 4), b.velocities(0.6, 4));
    }

    #[test]
    fn humanizing_never_drops_a_note_or_clips() {
        // Extremes in both directions: a whisper and a full-force strike.
        for base in [0.0, 0.05, 0.5, 1.0] {
            let mut dynamics = ChordDynamics::new(VelocityProfile::BassLead, 1.0);
            for _ in 0..200 {
                for v in dynamics.velocities(base, 6) {
                    assert!(
                        (0.05..=1.0).contains(&v),
                        "base {base} produced {v}, which would drop out or clip"
                    );
                }
            }
        }
    }

    #[test]
    fn humanize_is_clamped_and_readable() {
        let mut dynamics = ChordDynamics::new(VelocityProfile::Even, 5.0);
        assert_eq!(dynamics.humanize(), 1.0);
        dynamics.set_humanize(-1.0);
        assert_eq!(dynamics.humanize(), 0.0);
        dynamics.set_profile(VelocityProfile::BassLead);
        assert_eq!(dynamics.profile(), VelocityProfile::BassLead);
    }

    #[test]
    fn profiles_cycle_and_render_distinct_labels() {
        let mut seen = Vec::new();
        let mut profile = VelocityProfile::Even;
        for _ in 0..VelocityProfile::ALL.len() {
            seen.push(profile.to_string());
            profile = profile.next();
        }
        assert_eq!(profile, VelocityProfile::Even, "next() should cycle");
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), VelocityProfile::ALL.len());
    }

    #[test]
    fn weight_is_bounded_for_realistic_chord_sizes() {
        for profile in VelocityProfile::ALL {
            for count in 1..=8 {
                for index in 0..count {
                    let w = profile.weight(index, count);
                    assert!(
                        (0.6..=1.0).contains(&w),
                        "{profile:?} voice {index}/{count} -> {w}"
                    );
                }
            }
            // Out-of-range indices are benign rather than a panic.
            assert_eq!(profile.weight(99, 4), 1.0);
        }
    }
}
