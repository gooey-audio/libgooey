//! End-to-end tests for the transport-synchronized clip-grid C FFI.

use gooey::ffi::*;

const SR: f32 = 1_000.0;

fn mono_clip(value: f32, frames: usize) -> Vec<f32> {
    vec![value; frames]
}

fn noise_clip(frames: usize, mut state: u32) -> Vec<f32> {
    (0..frames)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state as f32 / u32::MAX as f32) * 2.0 - 1.0
        })
        .collect()
}

unsafe fn load(
    engine: *mut GooeyEngine,
    column: u32,
    row: u32,
    value: f32,
    frames: usize,
    source_bpm: f32,
) -> Vec<f32> {
    let samples = mono_clip(value, frames);
    assert!(gooey_engine_clip_load(
        engine,
        column,
        row,
        samples.as_ptr(),
        frames as u32,
        1,
        SR,
        source_bpm,
    ));
    samples
}

unsafe fn render(engine: *mut GooeyEngine, frames: usize) -> Vec<f32> {
    let mut out = vec![0.0; frames * 2];
    gooey_engine_render(engine, out.as_mut_ptr(), frames as u32);
    out
}

#[test]
fn constants_and_load_validation_are_stable() {
    unsafe {
        assert_eq!(CLIP_COLUMN_COUNT, 4);
        assert_eq!(CLIP_ROW_COUNT, 8);
        let engine = gooey_engine_new(SR);
        let samples = mono_clip(0.5, 1000);

        assert!(!gooey_engine_clip_load(
            engine,
            CLIP_COLUMN_COUNT,
            0,
            samples.as_ptr(),
            1000,
            1,
            SR,
            60.0,
        ));
        assert!(!gooey_engine_clip_load(
            engine,
            0,
            CLIP_ROW_COUNT,
            samples.as_ptr(),
            1000,
            1,
            SR,
            60.0,
        ));
        assert!(!gooey_engine_clip_load(
            engine,
            0,
            0,
            samples.as_ptr(),
            1000,
            1,
            SR,
            0.0,
        ));
        assert!(!gooey_engine_clip_load(
            engine,
            0,
            0,
            samples.as_ptr(),
            1000,
            1,
            f32::NAN,
            60.0,
        ));
        assert_eq!(gooey_engine_clip_get_state(engine, 0, 0), 0);

        let _samples = load(engine, 0, 0, 0.5, 1000, 60.0);
        assert_eq!(gooey_engine_clip_get_state(engine, 0, 0), CLIP_STATE_LOADED);
        assert_eq!(
            gooey_engine_clip_get_default_quantization(engine),
            CLIP_QUANTIZE_BAR
        );
        assert!(!gooey_engine_clip_set_default_quantization(engine, 99));
        gooey_engine_clip_clear(engine);
        assert_eq!(gooey_engine_clip_get_state(engine, 0, 0), 0);
        gooey_engine_free(engine);
    }
}

#[test]
fn stopped_launch_starts_empty_column_on_first_transport_sample() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let _samples = load(engine, 0, 0, 0.5, 4000, 60.0);
        assert!(gooey_engine_clip_launch(engine, 0, 0, CLIP_QUANTIZE_BAR));
        assert_eq!(gooey_engine_clip_get_active_row(engine, 0), -1);
        assert_eq!(gooey_engine_clip_get_queued_row(engine, 0), 0);
        assert_eq!(gooey_engine_clip_get_scheduled_beat(engine, 0), 0.0);

        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 1);
        assert_eq!(gooey_engine_clip_get_active_row(engine, 0), 0);
        assert_eq!(
            gooey_engine_clip_get_state(engine, 0, 0),
            CLIP_STATE_LOADED | CLIP_STATE_PLAYING
        );
        assert!((gooey_engine_transport_get_beat_position(engine) - 0.002).abs() < 1e-9);
        gooey_engine_free(engine);
    }
}

#[test]
fn running_bar_requests_are_strictly_future_even_on_a_bar_boundary() {
    unsafe {
        let engine = gooey_engine_new(SR);
        gooey_engine_set_bpm(engine, 60.0);
        let _samples = load(engine, 0, 0, 0.5, 8_000, 60.0);
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 4_000); // exactly beat 4
        assert!((gooey_engine_transport_get_beat_position(engine) - 4.0).abs() < 1e-9);
        assert!(gooey_engine_clip_launch(engine, 0, 0, CLIP_QUANTIZE_BAR));
        assert_eq!(gooey_engine_clip_get_scheduled_beat(engine, 0), 8.0);
        gooey_engine_free(engine);
    }
}

#[test]
fn active_playhead_reports_the_real_trimmed_source_cursor() {
    unsafe {
        let engine = gooey_engine_new(SR);
        gooey_engine_set_bpm(engine, 60.0);
        assert_eq!(gooey_engine_clip_get_active_playhead(engine, 0), -1.0);
        let _samples = load(engine, 0, 0, 0.5, 44_100, 60.0);
        // A wrapped window begins at 75% of the full source buffer.
        assert!(gooey_engine_clip_set_trim(
            engine,
            0,
            0,
            0.75,
            0.25,
            CLIP_QUANTIZE_IMMEDIATE
        ));
        assert!(gooey_engine_clip_launch_at_beat(engine, 0, 0, 0.0));
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 1);
        let playhead = gooey_engine_clip_get_active_playhead(engine, 0);
        assert!(
            (0.75..0.752).contains(&playhead),
            "expected trim-start cursor, got {playhead}"
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn quantized_and_exact_launches_cross_render_boundaries() {
    unsafe fn assert_launch(quantization: u32, expected_beat: f64) {
        let engine = gooey_engine_new(SR);
        gooey_engine_set_bpm(engine, 60.0);
        let _samples = load(engine, 0, 0, 0.5, 8000, 60.0);
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 100); // beat 0.1
        assert!(gooey_engine_clip_launch(engine, 0, 0, quantization));
        assert!((gooey_engine_clip_get_scheduled_beat(engine, 0) - expected_beat).abs() < 1e-9);

        let frames_to_boundary = ((expected_beat - 0.1) * SR as f64) as usize;
        let _ = render(engine, frames_to_boundary);
        assert_eq!(gooey_engine_clip_get_active_row(engine, 0), -1);
        let _ = render(engine, 1);
        assert_eq!(gooey_engine_clip_get_active_row(engine, 0), 0);
        gooey_engine_free(engine);
    }

    unsafe {
        assert_launch(CLIP_QUANTIZE_SIXTEENTH, 0.25);
        assert_launch(CLIP_QUANTIZE_QUARTER, 1.0);
        assert_launch(CLIP_QUANTIZE_BAR, 4.0);

        let engine = gooey_engine_new(SR);
        gooey_engine_set_bpm(engine, 60.0);
        let _samples = load(engine, 0, 0, 0.5, 8000, 60.0);
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 100);
        assert!(gooey_engine_clip_launch_at_beat(engine, 0, 0, 0.333));
        assert!(!gooey_engine_clip_launch_at_beat(engine, 0, 0, 0.05));
        assert!((gooey_engine_clip_get_scheduled_beat(engine, 0) - 0.333).abs() < 1e-9);
        let _ = render(engine, 233);
        assert_eq!(gooey_engine_clip_get_active_row(engine, 0), -1);
        let _ = render(engine, 1);
        assert_eq!(gooey_engine_clip_get_active_row(engine, 0), 0);
        gooey_engine_free(engine);
    }
}

#[test]
fn column_is_mutually_exclusive_and_latest_request_wins() {
    unsafe {
        let engine = gooey_engine_new(SR);
        gooey_engine_set_bpm(engine, 60.0);
        let _a = load(engine, 0, 0, 0.25, 4000, 60.0);
        let _b = load(engine, 0, 1, 0.75, 4000, 60.0);
        assert!(gooey_engine_clip_launch_at_beat(engine, 0, 0, 0.0));
        assert!(gooey_engine_clip_launch_at_beat(engine, 0, 1, 0.0));
        assert_eq!(gooey_engine_clip_get_queued_row(engine, 0), 1);
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 1);
        assert_eq!(gooey_engine_clip_get_active_row(engine, 0), 1);
        assert_eq!(gooey_engine_clip_get_state(engine, 0, 0), CLIP_STATE_LOADED);
        assert_eq!(
            gooey_engine_clip_get_state(engine, 0, 1),
            CLIP_STATE_LOADED | CLIP_STATE_PLAYING
        );

        assert!(gooey_engine_clip_launch_at_beat(engine, 0, 0, 0.25));
        gooey_engine_clip_cancel(engine, 0);
        let _ = render(engine, 250);
        assert_eq!(gooey_engine_clip_get_active_row(engine, 0), 1);
        gooey_engine_free(engine);
    }
}

#[test]
fn scene_launch_is_atomic_and_empty_cells_stop_columns() {
    unsafe {
        let engine = gooey_engine_new(SR);
        gooey_engine_set_bpm(engine, 60.0);
        let mut buffers = Vec::new();
        for column in 0..CLIP_COLUMN_COUNT {
            buffers.push(load(
                engine,
                column,
                0,
                0.1 + column as f32 * 0.1,
                4000,
                60.0,
            ));
        }
        assert!(gooey_engine_clip_launch_scene_at_beat(engine, 0, 0.0));
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 1);
        for column in 0..CLIP_COLUMN_COUNT {
            assert_eq!(gooey_engine_clip_get_active_row(engine, column), 0);
        }

        buffers.push(load(engine, 0, 1, 0.9, 4000, 60.0));
        assert!(gooey_engine_clip_launch_scene_at_beat(engine, 1, 0.25));
        for column in 1..CLIP_COLUMN_COUNT {
            assert!(gooey_engine_clip_is_stop_queued(engine, column));
        }
        let _ = render(engine, 249);
        for column in 0..CLIP_COLUMN_COUNT {
            assert_eq!(gooey_engine_clip_get_active_row(engine, column), 0);
        }
        let _ = render(engine, 1);
        assert_eq!(gooey_engine_clip_get_active_row(engine, 0), 1);
        for column in 1..CLIP_COLUMN_COUNT {
            assert_eq!(gooey_engine_clip_get_active_row(engine, column), -1);
        }
        gooey_engine_free(engine);
    }
}

#[test]
fn transport_stop_freezes_active_phase_and_cancels_queue() {
    unsafe {
        let engine = gooey_engine_new(SR);
        gooey_engine_set_bpm(engine, 60.0);
        let _a = load(engine, 0, 0, 0.4, 4000, 60.0);
        let _b = load(engine, 0, 1, 0.8, 4000, 60.0);
        gooey_engine_clip_launch_at_beat(engine, 0, 0, 0.0);
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 300);
        let beat = gooey_engine_transport_get_beat_position(engine);
        let position = gooey_engine_loop_get_position(engine, 0);
        gooey_engine_clip_launch_at_beat(engine, 0, 1, 1.0);

        gooey_engine_sequencer_stop(engine);
        assert_eq!(gooey_engine_clip_get_queued_row(engine, 0), -1);
        let _ = render(engine, 500);
        assert_eq!(gooey_engine_transport_get_beat_position(engine), beat);
        assert_eq!(gooey_engine_loop_get_position(engine, 0), position);
        assert_eq!(gooey_engine_clip_get_active_row(engine, 0), 0);

        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 10);
        assert!(gooey_engine_transport_get_beat_position(engine) > beat);
        gooey_engine_free(engine);
    }
}

#[test]
fn active_replace_relaunch_and_unload_are_quantized() {
    unsafe {
        let engine = gooey_engine_new(SR);
        gooey_engine_set_bpm(engine, 60.0);
        assert!(gooey_engine_clip_set_default_quantization(
            engine,
            CLIP_QUANTIZE_SIXTEENTH
        ));
        let _a = load(engine, 0, 0, 0.25, 4000, 60.0);
        gooey_engine_clip_launch_at_beat(engine, 0, 0, 0.0);
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 100);

        let _replacement = load(engine, 0, 0, 0.75, 4000, 60.0);
        assert_eq!(
            gooey_engine_clip_get_state(engine, 0, 0),
            CLIP_STATE_LOADED | CLIP_STATE_PLAYING | CLIP_STATE_QUEUED
        );
        assert!((gooey_engine_clip_get_scheduled_beat(engine, 0) - 0.25).abs() < 1e-9);
        let _ = render(engine, 150);
        assert_ne!(
            gooey_engine_clip_get_state(engine, 0, 0) & CLIP_STATE_QUEUED,
            0
        );
        let _ = render(engine, 1);
        assert_eq!(
            gooey_engine_clip_get_state(engine, 0, 0),
            CLIP_STATE_LOADED | CLIP_STATE_PLAYING
        );

        assert!(gooey_engine_clip_launch(
            engine,
            0,
            0,
            CLIP_QUANTIZE_SIXTEENTH
        ));
        assert_ne!(
            gooey_engine_clip_get_state(engine, 0, 0) & CLIP_STATE_QUEUED,
            0
        );
        gooey_engine_clip_cancel(engine, 0);

        assert!(gooey_engine_clip_unload(engine, 0, 0));
        assert!(gooey_engine_clip_is_stop_queued(engine, 0));
        let target = gooey_engine_clip_get_scheduled_beat(engine, 0);
        let current = gooey_engine_transport_get_beat_position(engine);
        let frames = ((target - current) * SR as f64).ceil() as usize + 1;
        let _ = render(engine, frames);
        assert_eq!(gooey_engine_clip_get_active_row(engine, 0), -1);
        assert_eq!(gooey_engine_clip_get_state(engine, 0, 0), 0);
        gooey_engine_free(engine);
    }
}

#[test]
fn legacy_playhead_mutation_detaches_grid_state_but_keeps_slots() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let _samples = load(engine, 0, 0, 0.5, 4000, 60.0);
        gooey_engine_clip_launch_at_beat(engine, 0, 0, 0.0);
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 1);
        assert_eq!(gooey_engine_clip_get_active_row(engine, 0), 0);

        gooey_engine_loop_set_position(engine, 0, 0.5);
        assert_eq!(gooey_engine_clip_get_active_row(engine, 0), -1);
        assert_eq!(gooey_engine_clip_get_state(engine, 0, 0), CLIP_STATE_LOADED);
        gooey_engine_free(engine);
    }
}

#[test]
fn different_source_tempos_hold_the_same_musical_phase() {
    unsafe {
        const AUDIO_SR: f32 = 44_100.0;
        let engine = gooey_engine_new(AUDIO_SR);
        gooey_engine_set_bpm(engine, 120.0);

        // Both clips contain four beats: 2 s at 120 BPM and 4 s at 60 BPM.
        // Non-periodic material gives WSOLA one unambiguous correlation peak;
        // constant/sine buffers intentionally permit many equally good offsets.
        let fast = noise_clip((AUDIO_SR * 2.0) as usize, 1);
        let slow = noise_clip((AUDIO_SR * 4.0) as usize, 2);
        assert!(gooey_engine_clip_load(
            engine,
            0,
            0,
            fast.as_ptr(),
            fast.len() as u32,
            1,
            AUDIO_SR,
            120.0,
        ));
        assert!(gooey_engine_clip_load(
            engine,
            1,
            0,
            slow.as_ptr(),
            slow.len() as u32,
            1,
            AUDIO_SR,
            60.0,
        ));
        gooey_engine_clip_launch_scene_at_beat(engine, 0, 0.0);
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, (AUDIO_SR * 0.5) as usize); // one beat at 120 BPM

        let fast_phase = gooey_engine_loop_get_position(engine, 0);
        let slow_phase = gooey_engine_loop_get_position(engine, 1);
        assert!(
            (fast_phase - slow_phase).abs() < 0.08,
            "musical phases diverged: fast={fast_phase}, slow={slow_phase}"
        );
        assert!(
            (fast_phase - 0.25).abs() < 0.10,
            "unexpected phase {fast_phase}"
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn trim_validation_and_round_trip() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let _samples = load(engine, 0, 0, 0.5, 1000, 60.0);

        // Fresh slot defaults to the full [0, 1) buffer.
        assert_eq!(gooey_engine_clip_get_trim_start(engine, 0, 0), 0.0);
        assert_eq!(gooey_engine_clip_get_trim_end(engine, 0, 0), 1.0);

        // Store and read back.
        assert!(gooey_engine_clip_set_trim(
            engine,
            0,
            0,
            0.2,
            0.8,
            CLIP_QUANTIZE_IMMEDIATE
        ));
        assert_eq!(gooey_engine_clip_get_trim_start(engine, 0, 0), 0.2);
        assert_eq!(gooey_engine_clip_get_trim_end(engine, 0, 0), 0.8);

        // Rejections: out-of-range slot, start == end, out of [0,1], non-finite,
        // unknown quantization.
        assert!(!gooey_engine_clip_set_trim(
            engine,
            CLIP_COLUMN_COUNT,
            0,
            0.2,
            0.8,
            0
        ));
        assert!(!gooey_engine_clip_set_trim(engine, 0, 0, 0.5, 0.5, 0));
        assert!(!gooey_engine_clip_set_trim(engine, 0, 0, -0.1, 0.8, 0));
        assert!(!gooey_engine_clip_set_trim(engine, 0, 0, 0.2, f64::NAN, 0));
        assert!(!gooey_engine_clip_set_trim(engine, 0, 0, 0.2, 0.8, 99));

        // Empty slot -> -1.0 sentinel.
        assert_eq!(gooey_engine_clip_get_trim_start(engine, 0, 1), -1.0);
        assert_eq!(gooey_engine_clip_get_trim_end(engine, 0, 1), -1.0);

        // Reloading a slot resets its trim to the full buffer.
        let _reload = load(engine, 0, 0, 0.5, 1000, 60.0);
        assert_eq!(gooey_engine_clip_get_trim_start(engine, 0, 0), 0.0);
        assert_eq!(gooey_engine_clip_get_trim_end(engine, 0, 0), 1.0);

        gooey_engine_free(engine);
    }
}

#[test]
fn quantized_retrim_lands_on_boundary_without_detaching() {
    unsafe {
        let engine = gooey_engine_new(SR);
        gooey_engine_set_bpm(engine, 60.0);
        let _samples = load(engine, 0, 0, 0.5, 8000, 60.0); // 8 beats
        gooey_engine_clip_launch_at_beat(engine, 0, 0, 0.0);
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 100); // beat 0.1; playhead early in the full loop

        // Retrim to a window that does not contain the current playhead, on the
        // next sixteenth (0.25 beat).
        assert!(gooey_engine_clip_set_trim(
            engine,
            0,
            0,
            0.8,
            0.95,
            CLIP_QUANTIZE_SIXTEENTH
        ));
        let target = gooey_engine_clip_get_scheduled_beat(engine, 0);
        assert_eq!(
            target, -1.0,
            "a retrim must not occupy the launch queue slot"
        );

        // Up to one frame before the boundary: still in the old (full) window.
        let frames_to_boundary = ((0.25 - 0.1) * SR as f64) as usize;
        let _ = render(engine, frames_to_boundary - 1);
        assert!(gooey_engine_loop_get_position(engine, 0) < 0.8);
        assert_eq!(gooey_engine_clip_get_active_row(engine, 0), 0);

        // Cross the boundary: the playhead snaps into the new window and the
        // column is still owned by the grid (active row unchanged -> no detach).
        let _ = render(engine, 2);
        assert!(gooey_engine_loop_get_position(engine, 0) >= 0.8);
        assert_eq!(gooey_engine_clip_get_active_row(engine, 0), 0);
        assert_eq!(
            gooey_engine_clip_get_state(engine, 0, 0),
            CLIP_STATE_LOADED | CLIP_STATE_PLAYING
        );

        gooey_engine_free(engine);
    }
}

#[test]
fn immediate_retrim_applies_playing_and_stopped() {
    unsafe {
        // Playing: the retrim lands within the same call.
        let engine = gooey_engine_new(SR);
        gooey_engine_set_bpm(engine, 60.0);
        let _samples = load(engine, 0, 0, 0.5, 8000, 60.0);
        gooey_engine_clip_launch_at_beat(engine, 0, 0, 0.0);
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 200); // playhead early in the full loop (< 0.8)
        assert!(gooey_engine_loop_get_position(engine, 0) < 0.8);
        assert!(gooey_engine_clip_set_trim(
            engine,
            0,
            0,
            0.8,
            0.95,
            CLIP_QUANTIZE_IMMEDIATE
        ));
        assert!(gooey_engine_loop_get_position(engine, 0) >= 0.8);
        assert_eq!(gooey_engine_clip_get_active_row(engine, 0), 0);
        gooey_engine_free(engine);

        // Transport stopped but a clip is frozen active: immediate still applies.
        let engine = gooey_engine_new(SR);
        gooey_engine_set_bpm(engine, 60.0);
        let _samples = load(engine, 0, 0, 0.5, 8000, 60.0);
        gooey_engine_clip_launch_at_beat(engine, 0, 0, 0.0);
        gooey_engine_sequencer_start(engine);
        let _ = render(engine, 200);
        gooey_engine_sequencer_stop(engine); // freeze; column stays active
        assert_eq!(gooey_engine_clip_get_active_row(engine, 0), 0);
        let before = gooey_engine_loop_get_position(engine, 0);
        assert!(before < 0.8);
        assert!(gooey_engine_clip_set_trim(
            engine,
            0,
            0,
            0.8,
            0.95,
            CLIP_QUANTIZE_IMMEDIATE
        ));
        assert!(gooey_engine_loop_get_position(engine, 0) >= 0.8);
        gooey_engine_free(engine);
    }
}

#[test]
fn wrapped_trim_plays_both_segments() {
    unsafe {
        let engine = gooey_engine_new(SR);
        gooey_engine_set_bpm(engine, 60.0);
        // Two-valued buffer: first 200 frames = 0.3, last 200 = 0.9.
        let mut samples = vec![0.3f32; 400];
        for s in samples.iter_mut().skip(200) {
            *s = 0.9;
        }
        assert!(gooey_engine_clip_load(
            engine,
            0,
            0,
            samples.as_ptr(),
            400,
            1,
            SR,
            60.0,
        ));
        // Wrap-around trim: [0.6, 1.0) ∪ [0.0, 0.4) spans the 0.9 tail and the
        // 0.3 head, so both amplitudes must appear in the output.
        assert!(gooey_engine_clip_set_trim(
            engine,
            0,
            0,
            0.6,
            0.4,
            CLIP_QUANTIZE_IMMEDIATE
        ));
        gooey_engine_clip_launch_at_beat(engine, 0, 0, 0.0);
        gooey_engine_sequencer_start(engine);

        let out = render(engine, 2000);
        // The mixer applies master gain, so compare amplitudes relative to the
        // loudest sample. The 0.9 tail forms a cluster near the max; the 0.3
        // head forms a distinct cluster at ~1/3 of it.
        let max_abs = out.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(max_abs > 0.0);
        let saw_low = out
            .iter()
            .any(|&s| (0.15 * max_abs..0.6 * max_abs).contains(&s.abs()));
        let saw_high = out.iter().any(|&s| s.abs() > 0.75 * max_abs);
        assert!(saw_low, "0.3 segment never played");
        assert!(saw_high, "0.9 segment never played");

        gooey_engine_free(engine);
    }
}

#[test]
fn host_time_arm_starts_transport_and_due_clip_on_same_sample() {
    unsafe {
        let engine = gooey_engine_new(SR);
        gooey_engine_set_bpm(engine, 60.0);
        let _samples = load(engine, 0, 0, 0.5, 4000, 60.0);
        gooey_engine_set_render_host_time(engine, 1_000, 1.0);
        gooey_engine_sequencer_start_at_host_time(engine, 1_050, 1.0);
        assert!(gooey_engine_clip_launch_at_beat(engine, 0, 0, 1.0));

        let _ = render(engine, 50);
        assert_eq!(gooey_engine_clip_get_active_row(engine, 0), -1);
        assert_eq!(gooey_engine_transport_get_beat_position(engine), 0.0);
        gooey_engine_set_render_host_time(engine, 1_050, 1.0);
        let _ = render(engine, 1);
        assert_eq!(gooey_engine_clip_get_active_row(engine, 0), 0);
        assert!(gooey_engine_transport_get_beat_position(engine) > 1.0);
        gooey_engine_free(engine);
    }
}
