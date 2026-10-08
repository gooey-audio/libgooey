//! Loop-aware selection of named chord voicings.
//!
//! The optimizer changes only [`VoicingType`]. Chord identity, timing and
//! playback settings remain host-owned. Both strategies score the progression
//! as a cycle so the final chord leads back into the first without a bad seam.

use super::{apply_voicing, available_voicings, Chord, VoicingType};

const OMITTED_TONE_COST: i64 = 10;
const VOICE_INSERTION_COST: i64 = 12;
const CHANGE_COST: i64 = 1;
const WIDE_SPAN_START: i64 = 24;
const RANDOM_ATTEMPTS: usize = 48;
const RANDOM_NOISE_RADIUS: i64 = 12;

/// One unchanged chord in a progression, plus the named voicing the host
/// currently displays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProgressionChord {
    pub chord: Chord,
    pub octave: i8,
    pub current_voicing: VoicingType,
}

/// How [`transform_progression_voicings`] chooses among musically valid named
/// voicings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceLeadingStrategy {
    /// The deterministic lowest-cost cycle.
    Best,
    /// A seeded, near-optimal cycle that differs from the current one when a
    /// changed cycle fits the quality bound.
    Randomized { seed: u64 },
}

#[derive(Clone, Debug)]
struct Candidate {
    voicing: VoicingType,
    notes: Vec<u8>,
    node_cost: i64,
    changes_current: bool,
}

/// Return one valid named voicing per input chord.
///
/// Empty input is a successful empty transformation. `Best` is deterministic
/// and idempotent. `Randomized` is deterministic for a fixed seed.
pub fn transform_progression_voicings(
    progression: &[ProgressionChord],
    strategy: VoiceLeadingStrategy,
) -> Vec<VoicingType> {
    if progression.is_empty() {
        return Vec::new();
    }

    let candidates = progression
        .iter()
        .map(candidate_voicings)
        .collect::<Vec<_>>();
    let zero_noise = candidates
        .iter()
        .map(|choices| vec![0; choices.len()])
        .collect::<Vec<_>>();
    let best = solve_cycle(&candidates, &zero_noise, false)
        .expect("every chord quality has at least root and first-inversion voicings");

    match strategy {
        VoiceLeadingStrategy::Best => best,
        VoiceLeadingStrategy::Randomized { seed } => randomized_cycle(&candidates, best, seed),
    }
}

fn candidate_voicings(chord: &ProgressionChord) -> Vec<Candidate> {
    let full_note_count = chord.chord.quality.note_count();
    available_voicings(&chord.chord.quality)
        .into_iter()
        .map(|voicing| {
            let notes = apply_voicing(&chord.chord, voicing, chord.octave);
            let omitted = full_note_count.saturating_sub(notes.len()) as i64;
            let span = notes
                .first()
                .zip(notes.last())
                .map_or(0, |(low, high)| i64::from(*high) - i64::from(*low));
            let wide_span_cost = (span - WIDE_SPAN_START).max(0) * 2;
            let changes_current = voicing != chord.current_voicing;
            Candidate {
                voicing,
                notes,
                node_cost: omitted * OMITTED_TONE_COST
                    + wide_span_cost
                    + i64::from(changes_current) * CHANGE_COST,
                changes_current,
            }
        })
        .collect()
}

fn randomized_cycle(
    candidates: &[Vec<Candidate>],
    best: Vec<VoicingType>,
    seed: u64,
) -> Vec<VoicingType> {
    let can_change = candidates
        .iter()
        .any(|choices| choices.iter().any(|candidate| candidate.changes_current));
    if !can_change {
        return best;
    }

    let best_score = progression_score(candidates, &best);
    let allowance = 18.max(best_score * 3 / 10);
    let maximum_score = best_score + allowance;
    let mut rng = SplitMix64::new(seed);
    let mut options: Vec<Vec<VoicingType>> = Vec::new();

    for _ in 0..RANDOM_ATTEMPTS {
        let noise = candidates
            .iter()
            .map(|choices| {
                choices
                    .iter()
                    .map(|_| rng.next_range(-RANDOM_NOISE_RADIUS, RANDOM_NOISE_RADIUS))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let Some(candidate) = solve_cycle(candidates, &noise, true) else {
            continue;
        };
        if progression_score(candidates, &candidate) <= maximum_score
            && !options.contains(&candidate)
        {
            options.push(candidate);
        }
    }

    if options.is_empty() {
        let zero_noise = candidates
            .iter()
            .map(|choices| vec![0; choices.len()])
            .collect::<Vec<_>>();
        if let Some(candidate) = solve_cycle(candidates, &zero_noise, true) {
            if progression_score(candidates, &candidate) <= maximum_score {
                return candidate;
            }
        }
        return best;
    }

    let index = (rng.next_u64() as usize) % options.len();
    options.swap_remove(index)
}

/// Find the cheapest cycle. `node_noise` is zero for Best and seeded for the
/// randomized search. Tracking whether any node changed lets the dice action
/// exclude the unchanged path without retrying indefinitely.
fn solve_cycle(
    candidates: &[Vec<Candidate>],
    node_noise: &[Vec<i64>],
    require_change: bool,
) -> Option<Vec<VoicingType>> {
    debug_assert_eq!(candidates.len(), node_noise.len());
    let event_count = candidates.len();
    let mut winning_cost = i64::MAX;
    let mut winning_path: Option<Vec<usize>> = None;

    for first_index in 0..candidates[0].len() {
        let mut costs = vec![[i64::MAX; 2]; candidates[0].len()];
        let first_changed = usize::from(candidates[0][first_index].changes_current);
        costs[first_index][first_changed] =
            candidates[0][first_index].node_cost + node_noise[0][first_index];

        let mut backtrack = (0..event_count)
            .map(|event| vec![[None; 2]; candidates[event].len()])
            .collect::<Vec<Vec<[Option<(usize, usize)>; 2]>>>();

        for event in 1..event_count {
            let mut next_costs = vec![[i64::MAX; 2]; candidates[event].len()];
            for (next_index, next) in candidates[event].iter().enumerate() {
                for (previous_index, previous) in candidates[event - 1].iter().enumerate() {
                    for previous_changed in 0..=1 {
                        let prior_cost = costs[previous_index][previous_changed];
                        if prior_cost == i64::MAX {
                            continue;
                        }
                        let changed = previous_changed | usize::from(next.changes_current);
                        let cost = prior_cost
                            + transition_cost(previous, next)
                            + next.node_cost
                            + node_noise[event][next_index];
                        if cost < next_costs[next_index][changed] {
                            next_costs[next_index][changed] = cost;
                            backtrack[event][next_index][changed] =
                                Some((previous_index, previous_changed));
                        }
                    }
                }
            }
            costs = next_costs;
        }

        for last_index in 0..candidates[event_count - 1].len() {
            for changed in 0..=1 {
                if require_change && changed == 0 {
                    continue;
                }
                let mut cost = costs[last_index][changed];
                if cost == i64::MAX {
                    continue;
                }
                if event_count > 1 {
                    cost += transition_cost(
                        &candidates[event_count - 1][last_index],
                        &candidates[0][first_index],
                    );
                }
                if cost >= winning_cost {
                    continue;
                }

                let mut path = vec![0; event_count];
                path[event_count - 1] = last_index;
                let mut cursor = (last_index, changed);
                for event in (1..event_count).rev() {
                    cursor = backtrack[event][cursor.0][cursor.1]
                        .expect("a finite dynamic-programming state has a predecessor");
                    path[event - 1] = cursor.0;
                }
                debug_assert_eq!(path[0], first_index);
                winning_cost = cost;
                winning_path = Some(path);
            }
        }
    }

    winning_path.map(|path| {
        path.into_iter()
            .enumerate()
            .map(|(event, candidate)| candidates[event][candidate].voicing)
            .collect()
    })
}

fn progression_score(candidates: &[Vec<Candidate>], voicings: &[VoicingType]) -> i64 {
    let chosen = voicings
        .iter()
        .enumerate()
        .map(|(event, voicing)| {
            candidates[event]
                .iter()
                .find(|candidate| candidate.voicing == *voicing)
                .expect("a solved path contains only enumerated candidates")
        })
        .collect::<Vec<_>>();
    let node_cost = chosen
        .iter()
        .map(|candidate| candidate.node_cost)
        .sum::<i64>();
    if chosen.len() < 2 {
        return node_cost;
    }
    node_cost
        + chosen
            .iter()
            .zip(chosen.iter().cycle().skip(1))
            .take(chosen.len())
            .map(|(from, to)| transition_cost(from, to))
            .sum::<i64>()
}

fn transition_cost(from: &Candidate, to: &Candidate) -> i64 {
    let alignment = non_crossing_voice_cost(&from.notes, &to.notes);
    let bass = outer_voice_distance(&from.notes, &to.notes, true);
    let soprano = outer_voice_distance(&from.notes, &to.notes, false);
    alignment + bass * 2 + soprano
}

fn outer_voice_distance(from: &[u8], to: &[u8], bass: bool) -> i64 {
    let pair = if bass {
        from.first().zip(to.first())
    } else {
        from.last().zip(to.last())
    };
    pair.map_or(VOICE_INSERTION_COST, |(left, right)| {
        (i64::from(*left) - i64::from(*right)).abs()
    })
}

/// Edit distance over sorted pitches. Pairing in order prevents voices from
/// crossing; gap costs stop Shell and Rootless from gaming movement by simply
/// deleting voices.
fn non_crossing_voice_cost(from: &[u8], to: &[u8]) -> i64 {
    let mut costs = vec![vec![0; to.len() + 1]; from.len() + 1];
    for (index, row) in costs.iter_mut().enumerate().skip(1) {
        row[0] = index as i64 * VOICE_INSERTION_COST;
    }
    for index in 1..=to.len() {
        costs[0][index] = index as i64 * VOICE_INSERTION_COST;
    }

    for left in 1..=from.len() {
        for right in 1..=to.len() {
            let distance = (i64::from(from[left - 1]) - i64::from(to[right - 1])).abs();
            let move_cost = distance + (distance - 7).max(0) * 2;
            costs[left][right] = (costs[left - 1][right] + VOICE_INSERTION_COST)
                .min(costs[left][right - 1] + VOICE_INSERTION_COST)
                .min(costs[left - 1][right - 1] + move_cost);
        }
    }
    costs[from.len()][to.len()]
}

struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^ (value >> 31)
    }

    fn next_range(&mut self, minimum: i64, maximum: i64) -> i64 {
        debug_assert!(minimum <= maximum);
        let width = (maximum - minimum + 1) as u64;
        minimum + (self.next_u64() % width) as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::{ChordQuality, NoteName};

    fn chord(root: NoteName, quality: ChordQuality, current: VoicingType) -> ProgressionChord {
        ProgressionChord {
            chord: Chord::new(root, quality),
            octave: 4,
            current_voicing: current,
        }
    }

    fn pop_progression() -> Vec<ProgressionChord> {
        // Cmaj7 - G7 - Am7 - Fmaj7 (I - V - vi - IV)
        vec![
            chord(NoteName::C, ChordQuality::Major7, VoicingType::RootPosition),
            chord(
                NoteName::G,
                ChordQuality::Dominant7,
                VoicingType::RootPosition,
            ),
            chord(NoteName::A, ChordQuality::Minor7, VoicingType::RootPosition),
            chord(NoteName::F, ChordQuality::Major7, VoicingType::RootPosition),
        ]
    }

    fn score(progression: &[ProgressionChord], voicings: &[VoicingType]) -> i64 {
        let candidates = progression
            .iter()
            .map(candidate_voicings)
            .collect::<Vec<_>>();
        progression_score(&candidates, voicings)
    }

    #[test]
    fn best_improves_the_whole_loop_and_prefers_complete_chords() {
        let progression = pop_progression();
        let roots = vec![VoicingType::RootPosition; progression.len()];
        let best = transform_progression_voicings(&progression, VoiceLeadingStrategy::Best);

        assert!(score(&progression, &best) < score(&progression, &roots));
        assert!(!best.contains(&VoicingType::Shell));
        assert!(!best.contains(&VoicingType::Rootless));
    }

    #[test]
    fn best_is_deterministic_and_every_result_is_available() {
        let progression = pop_progression();
        let first = transform_progression_voicings(&progression, VoiceLeadingStrategy::Best);
        let second = transform_progression_voicings(&progression, VoiceLeadingStrategy::Best);
        assert_eq!(first, second);

        for (event, voicing) in progression.iter().zip(first) {
            assert!(available_voicings(&event.chord.quality).contains(&voicing));
        }
    }

    #[test]
    fn one_chord_best_stays_put_but_randomize_changes_when_possible() {
        let progression = vec![chord(
            NoteName::C,
            ChordQuality::Major7,
            VoicingType::RootPosition,
        )];
        assert_eq!(
            transform_progression_voicings(&progression, VoiceLeadingStrategy::Best),
            vec![VoicingType::RootPosition]
        );
        assert_ne!(
            transform_progression_voicings(
                &progression,
                VoiceLeadingStrategy::Randomized { seed: 7 }
            ),
            vec![VoicingType::RootPosition]
        );
    }

    #[test]
    fn randomization_is_seeded_changed_and_varied() {
        let progression = pop_progression();
        let current = vec![VoicingType::RootPosition; progression.len()];
        let first = transform_progression_voicings(
            &progression,
            VoiceLeadingStrategy::Randomized { seed: 42 },
        );
        let repeated = transform_progression_voicings(
            &progression,
            VoiceLeadingStrategy::Randomized { seed: 42 },
        );
        assert_eq!(first, repeated);
        assert_ne!(first, current);

        let mut variants: Vec<Vec<VoicingType>> = Vec::new();
        for seed in 0..16 {
            let variant = transform_progression_voicings(
                &progression,
                VoiceLeadingStrategy::Randomized { seed },
            );
            if !variants.contains(&variant) {
                variants.push(variant);
            }
        }
        assert!(
            variants.len() > 1,
            "different seeds should explore more than one cycle"
        );
    }

    #[test]
    fn randomization_fallback_never_bypasses_the_near_optimal_bound() {
        // With enough identical root-position chords, changing all of them
        // costs more than the fixed allowance, while changing only part of the
        // loop also pays two expensive transition seams. No changed cycle is
        // therefore eligible, regardless of the random search perturbations.
        let repeated = chord(NoteName::C, ChordQuality::Major7, VoicingType::RootPosition);
        let progression = vec![repeated; 32];
        let candidates = progression
            .iter()
            .map(candidate_voicings)
            .collect::<Vec<_>>();
        let zero_noise = candidates
            .iter()
            .map(|choices| vec![0; choices.len()])
            .collect::<Vec<_>>();
        let best = transform_progression_voicings(&progression, VoiceLeadingStrategy::Best);
        let maximum_score = progression_score(&candidates, &best) + 18;
        let lowest_changed = solve_cycle(&candidates, &zero_noise, true)
            .expect("each seventh chord has alternate named voicings");

        assert!(progression_score(&candidates, &lowest_changed) > maximum_score);
        let randomized = transform_progression_voicings(
            &progression,
            VoiceLeadingStrategy::Randomized { seed: 7 },
        );
        assert_eq!(randomized, best);
        assert!(progression_score(&candidates, &randomized) <= maximum_score);
    }

    #[test]
    fn empty_progression_is_a_successful_no_op() {
        assert!(transform_progression_voicings(&[], VoiceLeadingStrategy::Best).is_empty());
    }
}
