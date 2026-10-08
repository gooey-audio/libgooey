//! Device-free regressions for the same C ABI Tide consumes.
use gooey::ffi::*;
use std::ptr;
const RATE: f32 = 48000.;
unsafe fn render(e: *mut GooeyEngine, frames: usize) -> Vec<f32> {
    let mut out = vec![0.; frames * 2];
    gooey_engine_render(e, out.as_mut_ptr(), frames as u32);
    out
}
unsafe fn setup() -> (*mut GooeyEngine, *mut GooeyTrackTape, *mut GooeyLiveControl) {
    let e = gooey_engine_new(RATE);
    let t = gooey_engine_track_tape_new(e, 2, 0, 1);
    assert!(!t.is_null());
    let c = gooey_engine_live_control_new(e);
    assert!(!c.is_null());
    (e, t, c)
}
#[test]
fn dry_capture_survives_bus_mute_and_gain_and_excludes_click() {
    unsafe {
        let (e, t, c) = setup();
        gooey_engine_mixer_set_track_gain(e, 0, 0.);
        gooey_engine_mixer_set_track_mute(e, 2, true);
        gooey_engine_set_metronome_enabled(e, true);
        gooey_engine_sequencer_start(e);
        gooey_track_tape_command(t, 1, 0., 0);
        gooey_engine_trigger_channel_with_velocity(e, 0, 1.);
        let mut chord = GooeyChordEvent {
            target: GOOEY_CHORD_TARGET_POLY,
            target_id: 0,
            chord_set: CHORD_SET_SEVENTHS,
            root: 0,
            scale_type: 0,
            degree: 0,
            voicing: 0,
            preset: 2,
            octave: 4,
            velocity: 1.,
            gate: 0,
        };
        gooey_engine_chord_enqueue_trigger(e, &mut chord);
        render(e, 4096);
        gooey_track_tape_command(t, 2, 0., 0);
        render(e, 64);
        let mut pcm = vec![0.; 4096 * 6];
        assert_eq!(gooey_track_tape_drain(t, pcm.as_mut_ptr(), 4096), 4096);
        assert!(pcm.chunks_exact(6).any(|f| f[0].abs() > 0.0001));
        assert!(pcm.chunks_exact(6).any(|f| f[2].abs() > 0.0001));
        assert!(pcm.chunks_exact(6).all(|f| f[4] == 0. && f[5] == 0.));
        gooey_live_control_free(c);
        gooey_engine_free(e);
        gooey_track_tape_free(t);
    }
}
#[test]
fn replay_uses_current_strip_once_and_does_not_capture_itself() {
    unsafe {
        let (e, t, c) = setup();
        gooey_engine_set_master_gain(e, 1.);
        gooey_engine_mixer_set_track_gain(e, 0, 0.5);
        gooey_engine_mixer_set_track_pan(e, 0, 0.5);
        render(e, 24000);
        let frames = 4096;
        let mut pcm = vec![0.; frames * 6];
        for f in pcm.chunks_exact_mut(6) {
            f[2] = 0.2;
            f[3] = 0.4;
        }
        assert_eq!(
            gooey_track_tape_feed(t, pcm.as_ptr(), frames as u32),
            frames as u32
        );
        gooey_track_tape_command(t, 3, 0., frames as u64);
        let out = render(e, frames);
        assert!((out[out.len() - 2] - 0.1).abs() < 0.0001);
        assert!((out[out.len() - 1] - 0.2).abs() < 0.0001);
        let end = render(e, 32);
        assert!(end.iter().all(|x| *x == 0.));
        assert_eq!(gooey_track_tape_get_state(t), 5);
        assert_eq!(
            gooey_track_tape_drain(t, pcm.as_mut_ptr(), frames as u32),
            0
        );
        gooey_track_tape_free(t);
        gooey_live_control_free(c);
        gooey_engine_free(e);
    }
}
#[test]
fn count_in_aligns_all_lanes_at_exact_next_bar() {
    unsafe {
        let (e, t, c) = setup();
        gooey_engine_set_bpm(e, 120.);
        gooey_engine_sequencer_start(e);
        let g = gooey_track_tape_command(t, 1, 4., 0);
        render(e, 96000);
        assert_eq!(gooey_track_tape_get_applied_generation(t), g);
        assert_eq!(gooey_track_tape_get_frames(t), 0);
        render(e, 256);
        assert!(gooey_track_tape_get_frames(t) > 0);
        assert!(gooey_track_tape_get_frames(t) <= 256);
        gooey_live_control_free(c);
        gooey_engine_free(e);
        gooey_track_tape_free(t);
    }
}
#[test]
fn endpoints_validate_arguments_and_survive_engine_shutdown() {
    unsafe {
        let (e, t, c) = setup();
        assert!(gooey_engine_track_tape_new(e, 2, 0, 1).is_null());
        assert_eq!(gooey_track_tape_command(t, 1, f64::NAN, 0), 0);
        assert_eq!(gooey_track_tape_command(t, 3, 0., 0), 0);
        assert_eq!(gooey_track_tape_feed(t, ptr::null(), 1), 0);
        gooey_live_control_free(c);
        gooey_engine_free(e);
        assert_eq!(gooey_track_tape_get_state(t), 0);
        gooey_track_tape_free(t);
    }
}
#[test]
fn prepared_voice_swaps_keep_patterns_and_reclaim_off_render() {
    unsafe {
        let (e, t, c) = setup();
        let g = gooey_live_control_replace_voice(c, 0, INSTRUMENT_RESONATOR, 2);
        assert!(g > 0);
        render(e, 64);
        assert_eq!(
            gooey_engine_get_channel_instrument_type(e, 0),
            INSTRUMENT_RESONATOR
        );
        assert_eq!(gooey_live_control_get_last_applied_generation(c), g);
        let g = gooey_live_control_replace_voice(c, 0, INSTRUMENT_TWIN_CORE, 4);
        assert!(g > 0);
        render(e, 64);
        assert_eq!(
            gooey_engine_get_channel_instrument_type(e, 0),
            INSTRUMENT_TWIN_CORE
        );
        gooey_live_control_free(c);
        gooey_engine_free(e);
        gooey_track_tape_free(t);
    }
}

#[test]
fn every_track_effect_is_prepared_and_parameters_are_queued() {
    unsafe {
        let (e, t, c) = setup();
        let cases: &[(u32, Vec<(u32, f32)>)] = &[
            (0, vec![(0, 8000.), (1, 0.2)]),
            (1, vec![(0, 2.), (1, 0.3), (2, 0.3), (3, 4000.), (4, 1.)]),
            (2, vec![(0, 0.3), (1, 0.3), (2, 0.5)]),
            (3, vec![(0, -12.), (1, 4.), (2, 10.), (3, 100.), (4, 1.)]),
            (4, vec![(0, 0.5), (1, 0.2)]),
            (6, vec![(0, 0.5), (1, 0.25), (2, 0.4)]),
            (7, vec![(0, 2.), (1, 0.5)]),
            (8, vec![(0, 5.), (1, 0.3), (2, 2000.), (3, 0.5)]),
            (
                9,
                vec![(0, 0.5), (1, 0.25), (2, 0.4), (3, 0.1), (4, 1.), (5, 0.5)],
            ),
        ];
        for (effect, parameters) in cases {
            let params: Vec<_> = parameters
                .iter()
                .map(|(param, value)| GooeyEffectParamDescriptor {
                    param: *param,
                    value: *value,
                })
                .collect();
            let descriptor = GooeyEffectDescriptor {
                effect: *effect,
                params: params.as_ptr(),
                param_count: params.len() as u32,
            };
            let g = gooey_live_control_replace_track_rack(c, 0, &descriptor, 1);
            assert!(g > 0, "effect {effect}");
            render(e, 1024);
            assert_eq!(gooey_engine_track_effect_type_at(e, 0, 0), *effect as i32);
            assert!(
                gooey_live_control_set_track_effect_param(
                    c,
                    0,
                    0,
                    g,
                    params[0].param,
                    params[0].value
                ) > 0
            );
            render(e, 64);
        }
        gooey_live_control_free(c);
        gooey_engine_free(e);
        gooey_track_tape_free(t);
    }
}
#[test]
fn interruption_can_finalize_without_another_audio_callback() {
    unsafe {
        let (e, t, c) = setup();
        gooey_engine_sequencer_start(e);
        gooey_track_tape_command(t, 1, 0., 0);
        render(e, 128);
        let g = gooey_track_tape_command(t, 2, 0., 0);
        gooey_engine_track_tape_flush_stopped(e);
        assert_eq!(gooey_track_tape_get_applied_generation(t), g);
        assert_eq!(gooey_track_tape_get_state(t), 3);
        let mut pcm = vec![0.; 128 * 6];
        assert_eq!(gooey_track_tape_drain(t, pcm.as_mut_ptr(), 128), 128);
        gooey_live_control_free(c);
        gooey_engine_free(e);
        gooey_track_tape_free(t);
    }
}
#[test]
fn ten_minute_capture_keeps_rings_bounded_and_lanes_aligned() {
    unsafe {
        let (e, t, c) = setup();
        gooey_engine_set_bpm(e, 120.);
        gooey_engine_sequencer_start(e);
        gooey_track_tape_command(t, 1, 0., 0);
        let mut audio = vec![0.; 512 * 2];
        let mut pcm = vec![0.; 512 * 6];
        let mut frames = 0;
        for _ in 0..(48000 * 600 / 512) {
            gooey_engine_render(e, audio.as_mut_ptr(), 512);
            assert!(audio.iter().all(|x| x.is_finite()));
            let n = gooey_track_tape_drain(t, pcm.as_mut_ptr(), 512);
            assert_eq!(n, 512);
            assert!(pcm.iter().all(|x| x.is_finite()));
            frames += n as u64;
        }
        assert_eq!(gooey_track_tape_get_frames(t), frames);
        assert_eq!(gooey_track_tape_get_state(t), 2);
        gooey_live_control_free(c);
        gooey_engine_free(e);
        gooey_track_tape_free(t);
    }
}

// Thread-local accounting excludes worker/test setup allocations. The guarded
// render includes command application, a newly prepared voice, source triggers,
// LFO route changes, tape capture and replay.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
thread_local! {
    static WATCH: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
    static FREES: Cell<usize> = const { Cell::new(0) };
    static SIZES: Cell<[usize; 16]> = const { Cell::new([0; 16]) };
}
struct CountedAllocator;
unsafe impl GlobalAlloc for CountedAllocator {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if WATCH.try_with(Cell::get).unwrap_or(false) {
            ALLOCS.with(|n| {
                let index = n.get();
                if index < 16 {
                    SIZES.with(|v| {
                        let mut sizes = v.get();
                        sizes[index] = l.size();
                        v.set(sizes);
                    });
                }
                n.set(index + 1);
            });
        }
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        if WATCH.try_with(Cell::get).unwrap_or(false) {
            FREES.with(|n| n.set(n.get() + 1));
        }
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if WATCH.try_with(Cell::get).unwrap_or(false) {
            ALLOCS.with(|n| n.set(n.get() + 1));
        }
        System.realloc(p, l, n)
    }
}
#[global_allocator]
static ALLOCATOR: CountedAllocator = CountedAllocator;
#[test]
fn render_capture_replay_and_live_edits_allocate_and_free_nothing() {
    unsafe {
        let (e, t, c) = setup();
        let mut audio = [0.; 1024];
        let mut pcm = [0.; 512 * 6];
        render(e, 512);
        assert!(gooey_live_control_replace_voice(c, 0, INSTRUMENT_TWIN_CORE, 4) > 0);
        assert!(gooey_live_control_edit(c, 21, 0, 0, 0, 0.1, 0.) > 0);
        assert!(gooey_live_control_edit(c, 24, 0, 0, 0, 0., 0.) > 0);
        gooey_track_tape_command(t, 1, 0., 0);
        gooey_engine_trigger_channel_with_velocity(e, 0, 1.);
        ALLOCS.with(|n| n.set(0));
        FREES.with(|n| n.set(0));
        WATCH.with(|w| w.set(true));
        gooey_engine_render(e, audio.as_mut_ptr(), 512);
        WATCH.with(|w| w.set(false));
        assert_eq!(ALLOCS.with(Cell::get), 0);
        assert_eq!(FREES.with(Cell::get), 0);
        assert_eq!(gooey_track_tape_drain(t, pcm.as_mut_ptr(), 512), 512);
        gooey_track_tape_feed(t, pcm.as_ptr(), 512);
        gooey_track_tape_command(t, 3, 0., 512);
        WATCH.with(|w| w.set(true));
        gooey_engine_render(e, audio.as_mut_ptr(), 512);
        WATCH.with(|w| w.set(false));
        assert_eq!(ALLOCS.with(Cell::get), 0);
        assert_eq!(FREES.with(Cell::get), 0);
        gooey_live_control_free(c);
        gooey_engine_free(e);
        gooey_track_tape_free(t);
    }
}
#[test]
fn late_arm_and_next_bar_are_resolved_by_render() {
    unsafe {
        let (e, t, c) = setup();
        gooey_engine_set_bpm(e, 120.);
        gooey_engine_sequencer_start(e);
        render(e, 100000);
        gooey_track_tape_command(t, 1, 4., 0);
        render(e, 128);
        assert_eq!(gooey_track_tape_get_state(t), 1);
        assert_eq!(gooey_track_tape_get_frames(t), 0);
        render(e, 93000);
        assert!(gooey_track_tape_get_frames(t) > 0);
        gooey_track_tape_command(t, 2, 0., 0);
        render(e, 64);
        gooey_track_tape_discard_capture(t);
        gooey_track_tape_command(t, 4, 0., 0);
        render(e, 64);
        assert_eq!(gooey_track_tape_get_state(t), 1);
        gooey_live_control_free(c);
        gooey_engine_free(e);
        gooey_track_tape_free(t);
    }
}
#[test]
fn capture_overflow_retains_exact_synchronized_prefix() {
    unsafe {
        let (e, t, c) = setup();
        gooey_engine_sequencer_start(e);
        gooey_track_tape_command(t, 1, 0., 0);
        render(e, 132000);
        assert_eq!(gooey_track_tape_get_state(t), 6);
        assert_eq!(gooey_track_tape_get_frames(t), 131072);
        let mut pcm = vec![0.; 131072 * 6];
        assert_eq!(gooey_track_tape_drain(t, pcm.as_mut_ptr(), 131072), 131072);
        gooey_live_control_free(c);
        gooey_engine_free(e);
        gooey_track_tape_free(t);
    }
}

#[test]
fn configurable_layouts_validate_and_keep_legacy_channel_stride() {
    unsafe {
        assert_eq!(gooey_track_tape_get_channel_count(ptr::null()), 0);
        for count in 1..=4 {
            let e = gooey_engine_new(RATE);
            let tracks = [2, 0, 1, 3];
            assert!(gooey_engine_track_tape_new_with_tracks(e, ptr::null(), count).is_null());
            assert!(gooey_engine_track_tape_new_with_tracks(e, tracks.as_ptr(), 0).is_null());
            assert!(gooey_engine_track_tape_new_with_tracks(e, tracks.as_ptr(), 5).is_null());
            assert!(gooey_engine_track_tape_new_with_tracks(e, [0, 0].as_ptr(), 2).is_null());
            assert!(gooey_engine_track_tape_new_with_tracks(e, [99].as_ptr(), 1).is_null());
            let tape = gooey_engine_track_tape_new_with_tracks(e, tracks.as_ptr(), count);
            assert!(!tape.is_null());
            assert_eq!(gooey_track_tape_get_channel_count(tape), count * 2);
            assert!(gooey_engine_track_tape_new_with_tracks(e, tracks.as_ptr(), count).is_null());
            let channels = count as usize * 2;
            gooey_engine_sequencer_start(e);
            gooey_track_tape_command(tape, 1, 0., 0);
            render(e, 128);
            gooey_track_tape_command(tape, 2, 0., 0);
            gooey_engine_track_tape_flush_stopped(e);
            let mut pcm = vec![99.; 128 * channels + 4];
            assert_eq!(gooey_track_tape_drain(tape, pcm.as_mut_ptr(), 128), 128);
            assert_eq!(&pcm[128 * channels..], &[99.; 4]);
            assert_eq!(gooey_track_tape_feed(tape, pcm.as_ptr(), 128), 128);
            gooey_engine_free(e);
            gooey_track_tape_free(tape);
        }
        let (e, t, c) = setup();
        assert_eq!(gooey_track_tape_get_channel_count(t), 6);
        gooey_live_control_free(c);
        gooey_engine_free(e);
        gooey_track_tape_free(t);
    }
}

#[test]
fn fourth_lane_captures_ai_before_strip_and_replay_replaces_live_ai() {
    unsafe {
        let e = gooey_engine_new(RATE);
        let tape = gooey_engine_track_tape_new_with_tracks(e, [2, 0, 1, 3].as_ptr(), 4);
        assert!(!tape.is_null());
        assert!(gooey_engine_clip_set_phase_aligned_launches(e, true));
        let clip = vec![0.7; 96000 * 2];
        // One submission loads and stages first playback while stopped.
        assert!(gooey_engine_clip_load_and_launch(
            e,
            3,
            0,
            clip.as_ptr(),
            96000,
            2,
            RATE,
            120.
        ));
        gooey_engine_set_master_gain(e, 1.);
        gooey_engine_mixer_set_track_gain(e, 3, 0.5);
        gooey_engine_sequencer_start(e);
        gooey_track_tape_command(tape, 1, 0., 0);
        render(e, 8192);
        gooey_track_tape_command(tape, 2, 0., 0);
        render(e, 64);
        let mut captured = vec![0.; 8192 * 8];
        assert_eq!(
            gooey_track_tape_drain(tape, captured.as_mut_ptr(), 8192),
            8192
        );
        assert!(captured.chunks_exact(8).any(|f| f[6] > 0.6));
        assert!(captured
            .chunks_exact(8)
            .all(|f| f[..6].iter().all(|s| *s == 0.)));
        let mut playback = vec![0.; 4096 * 8];
        for f in playback.chunks_exact_mut(8) {
            f[6] = 0.2;
            f[7] = 0.4;
        }
        assert_eq!(gooey_track_tape_feed(tape, playback.as_ptr(), 4096), 4096);
        gooey_track_tape_command(tape, 3, 0., 4096);
        let out = render(e, 4096);
        assert!((out[out.len() - 2] - 0.1).abs() < 0.001);
        assert!((out[out.len() - 1] - 0.2).abs() < 0.001);
        assert!(render(e, 128).iter().all(|s| s.abs() < 0.001));
        assert_eq!(gooey_track_tape_get_state(tape), 5);
        gooey_engine_free(e);
        gooey_track_tape_free(tape);
    }
}

#[test]
fn atomic_clip_load_rejection_preserves_the_active_loop() {
    unsafe {
        let e = gooey_engine_new(RATE);
        let original = vec![0.2; 96000 * 2];
        assert!(gooey_engine_clip_load_and_launch(
            e,
            3,
            0,
            original.as_ptr(),
            96000,
            2,
            RATE,
            120.
        ));
        gooey_engine_set_master_gain(e, 1.);
        gooey_engine_sequencer_start(e);
        render(e, 8192);
        let mut accepted = 0;
        while gooey_engine_clip_set_phase_aligned_launches(e, true) {
            accepted += 1;
            assert!(accepted <= 4096);
        }
        let replacement = vec![0.8; 96000 * 2];
        assert!(!gooey_engine_clip_load_and_launch(
            e,
            3,
            0,
            replacement.as_ptr(),
            96000,
            2,
            RATE,
            120.
        ));
        let out = render(e, 100000);
        assert!((out[out.len() - 1] - 0.2).abs() < 0.001);
        gooey_engine_free(e);
    }
}

#[test]
fn ai_load_replace_tempo_and_clear_never_allocate_or_free_on_render() {
    unsafe {
        let e = gooey_engine_new(RATE);
        let tape = gooey_engine_track_tape_new_with_tracks(e, [2, 0, 1, 3].as_ptr(), 4);
        let control = gooey_engine_live_control_new(e);
        assert!(gooey_engine_clip_set_phase_aligned_launches(e, true));
        let pcm = vec![0.2; 96000 * 2];
        let mut out = [0.; 1024];
        // Deliberately change tempo before first clip playback so WSOLA engages.
        assert!(gooey_live_control_edit(control, 22, 0, 0, 0, 160., 0.) > 0);
        assert!(gooey_live_control_edit(control, 24, 0, 0, 0, 0., 0.) > 0);
        for index in 0..5 {
            assert!(gooey_engine_clip_load_and_launch(
                e,
                3,
                0,
                pcm.as_ptr(),
                96000,
                2,
                RATE,
                120.
            ));
            ALLOCS.with(|n| n.set(0));
            FREES.with(|n| n.set(0));
            WATCH.with(|w| w.set(true));
            for _ in 0..150 {
                gooey_engine_render(e, out.as_mut_ptr(), 512);
            }
            WATCH.with(|w| w.set(false));
            assert_eq!(
                ALLOCS.with(Cell::get),
                0,
                "load/replace {index}, allocation sizes {:?}, frees {}",
                SIZES.with(Cell::get),
                FREES.with(Cell::get)
            );
            assert_eq!(FREES.with(Cell::get), 0, "load/replace {index}");
            gooey_engine_reclaim_loop_buffers(e);
        }
        assert!(gooey_engine_clip_unload(e, 3, 0));
        WATCH.with(|w| w.set(true));
        for _ in 0..150 {
            gooey_engine_render(e, out.as_mut_ptr(), 512);
        }
        WATCH.with(|w| w.set(false));
        assert_eq!(ALLOCS.with(Cell::get), 0);
        assert_eq!(FREES.with(Cell::get), 0);
        gooey_engine_reclaim_loop_buffers(e);
        gooey_live_control_free(control);
        gooey_engine_free(e);
        gooey_track_tape_free(tape);
    }
}
