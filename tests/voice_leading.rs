//! End-to-end coverage for the progression voice-leading C ABI.

use gooey::ffi::*;
use std::ptr;

fn chord(degree: u32) -> GooeyChordEvent {
    GooeyChordEvent {
        target: GOOEY_CHORD_TARGET_POLY,
        target_id: 17,
        chord_set: CHORD_SET_SEVENTHS,
        root: 0,
        scale_type: SCALE_MAJOR,
        degree,
        voicing: VOICING_ROOT_POSITION,
        preset: POLY_PRESET_PAD,
        octave: 4,
        velocity: 0.73,
        gate: GOOEY_CHORD_GATE_STRUCK,
    }
}

fn assert_non_voicing_fields_equal(left: &GooeyChordEvent, right: &GooeyChordEvent) {
    assert_eq!(left.target, right.target);
    assert_eq!(left.target_id, right.target_id);
    assert_eq!(left.chord_set, right.chord_set);
    assert_eq!(left.root, right.root);
    assert_eq!(left.scale_type, right.scale_type);
    assert_eq!(left.degree, right.degree);
    assert_eq!(left.preset, right.preset);
    assert_eq!(left.octave, right.octave);
    assert_eq!(left.velocity.to_bits(), right.velocity.to_bits());
    assert_eq!(left.gate, right.gate);
}

#[test]
fn best_and_random_transform_only_valid_voicing_ids() {
    unsafe {
        // I - V - vi - IV
        let source = [chord(0), chord(4), chord(5), chord(3)];
        let mut best = source;
        assert!(gooey_chord_progression_transform_voicings(
            best.as_mut_ptr(),
            best.len() as u32,
            GOOEY_VOICE_LEADING_BEST,
            0,
        ));
        assert!(best
            .iter()
            .any(|event| event.voicing != VOICING_ROOT_POSITION));

        let mut random = source;
        assert!(gooey_chord_progression_transform_voicings(
            random.as_mut_ptr(),
            random.len() as u32,
            GOOEY_VOICE_LEADING_RANDOM,
            99,
        ));
        assert_ne!(
            random.iter().map(|event| event.voicing).collect::<Vec<_>>(),
            source.iter().map(|event| event.voicing).collect::<Vec<_>>()
        );

        for (original, transformed) in source.iter().zip(best.iter()) {
            assert_non_voicing_fields_equal(original, transformed);
            assert!(transformed.voicing <= VOICING_ROOTLESS);
        }
        for (original, transformed) in source.iter().zip(random.iter()) {
            assert_non_voicing_fields_equal(original, transformed);
            assert!(transformed.voicing <= VOICING_ROOTLESS);
        }
    }
}

#[test]
fn random_seed_is_repeatable() {
    unsafe {
        let mut first = [chord(0), chord(4), chord(5), chord(3)];
        let mut second = first;
        assert!(gooey_chord_progression_transform_voicings(
            first.as_mut_ptr(),
            first.len() as u32,
            GOOEY_VOICE_LEADING_RANDOM,
            1_234,
        ));
        assert!(gooey_chord_progression_transform_voicings(
            second.as_mut_ptr(),
            second.len() as u32,
            GOOEY_VOICE_LEADING_RANDOM,
            1_234,
        ));
        assert_eq!(
            first.iter().map(|event| event.voicing).collect::<Vec<_>>(),
            second.iter().map(|event| event.voicing).collect::<Vec<_>>()
        );
    }
}

#[test]
fn invalid_input_never_partially_writes() {
    unsafe {
        let mut events = [chord(0), chord(4), chord(5)];
        events[1].voicing = u32::MAX;
        let before = events;
        assert!(!gooey_chord_progression_transform_voicings(
            events.as_mut_ptr(),
            events.len() as u32,
            GOOEY_VOICE_LEADING_BEST,
            0,
        ));
        for (original, after) in before.iter().zip(events.iter()) {
            assert_non_voicing_fields_equal(original, after);
            assert_eq!(original.voicing, after.voicing);
        }

        let mut invalid_set = [chord(0)];
        invalid_set[0].chord_set = CHORD_SET_COUNT;
        assert!(!gooey_chord_progression_transform_voicings(
            invalid_set.as_mut_ptr(),
            1,
            GOOEY_VOICE_LEADING_BEST,
            0,
        ));
    }
}

#[test]
fn pointer_count_and_strategy_contract_is_total() {
    unsafe {
        assert!(gooey_chord_progression_transform_voicings(
            ptr::null_mut(),
            0,
            GOOEY_VOICE_LEADING_BEST,
            0,
        ));
        assert!(!gooey_chord_progression_transform_voicings(
            ptr::null_mut(),
            1,
            GOOEY_VOICE_LEADING_BEST,
            0,
        ));
        assert!(!gooey_chord_progression_transform_voicings(
            ptr::null_mut(),
            GOOEY_CHORD_LOOP_MAX_EVENTS + 1,
            GOOEY_VOICE_LEADING_BEST,
            0,
        ));

        let mut event = chord(0);
        assert!(!gooey_chord_progression_transform_voicings(
            &mut event,
            1,
            u32::MAX,
            0,
        ));
    }
}
