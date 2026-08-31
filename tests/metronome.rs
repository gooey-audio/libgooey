//! End-to-end tests for the optional monitor click, driving the FFI exactly as
//! an iOS host does. The two structural properties worth guarding are that the
//! click never touches the mix path (test `metronome_never_feeds_the_effects`)
//! and that it never reaches an export (`metronome_is_absent_from_offline_bounce`).

use gooey::ffi::*;

const SAMPLE_RATE: f32 = 44_100.0;
/// At the default 120 BPM: 22 050 frames per quarter note, 88 200 per bar.
const FRAMES_PER_BEAT: usize = 22_050;
/// Anything below this counts as digital silence, matching the rest of the suite.
const SILENCE: f32 = 1.0e-9;

unsafe fn render(engine: *mut GooeyEngine, frames: usize) -> Vec<f32> {
    let mut buffer = vec![0.0_f32; frames * GOOEY_OUTPUT_CHANNELS as usize];
    gooey_engine_render(engine, buffer.as_mut_ptr(), frames as u32);
    buffer
}

fn max_abs(buffer: &[f32]) -> f32 {
    buffer.iter().map(|s| s.abs()).fold(0.0, f32::max)
}

/// Frame indices where a transient begins, paired with the peak of that
/// transient. A click decays in ~36 ms (~1 600 frames here), so a 4 000-frame
/// window captures one click without ever reaching the next.
fn onsets(buffer: &[f32]) -> Vec<(usize, f32)> {
    const THRESHOLD: f32 = 1.0e-4;
    let frames: Vec<f32> = buffer.chunks_exact(2).map(|f| f[0].abs()).collect();
    let mut found = Vec::new();
    let mut index = 0;
    while index < frames.len() {
        if frames[index] > THRESHOLD {
            let end = (index + 4_000).min(frames.len());
            let peak = frames[index..end].iter().fold(0.0_f32, |a, b| a.max(*b));
            found.push((index, peak));
            // Skip past the decay so one click is never counted twice.
            index = end;
        } else {
            index += 1;
        }
    }
    found
}

/// An engine with the transport running and no programmed material, so the
/// only thing that can make sound is the click.
unsafe fn silent_running_engine() -> *mut GooeyEngine {
    let engine = gooey_engine_new(SAMPLE_RATE);
    gooey_engine_sequencer_start(engine);
    engine
}

#[test]
fn defaults_are_off_quarter_notes_and_accented() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        assert!(!gooey_engine_get_metronome_enabled(engine));
        assert_eq!(gooey_engine_get_metronome_level(engine), 0.35);
        assert_eq!(
            gooey_engine_get_metronome_division(engine),
            METRONOME_DIVISION_QUARTER
        );
        assert!(gooey_engine_get_metronome_accent_enabled(engine));
        gooey_engine_free(engine);
    }
}

#[test]
fn disabled_metronome_renders_exact_silence() {
    unsafe {
        let engine = silent_running_engine();
        let buffer = render(engine, FRAMES_PER_BEAT * 8);
        assert!(
            max_abs(&buffer) < SILENCE,
            "a disabled metronome must be bit-silent, peak was {}",
            max_abs(&buffer)
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn enabled_metronome_clicks_on_every_beat() {
    unsafe {
        let engine = silent_running_engine();
        gooey_engine_set_metronome_enabled(engine, true);

        let buffer = render(engine, FRAMES_PER_BEAT * 8);
        let found = onsets(&buffer);
        assert_eq!(found.len(), 8, "expected 8 clicks, got {found:?}");
        for (beat, (frame, _)) in found.iter().enumerate() {
            let expected = beat * FRAMES_PER_BEAT;
            assert!(
                frame.abs_diff(expected) <= 2,
                "click {beat} landed at frame {frame}, expected {expected}"
            );
        }
        gooey_engine_free(engine);
    }
}

#[test]
fn downbeat_is_accented_and_the_accent_can_be_disabled() {
    unsafe {
        let engine = silent_running_engine();
        gooey_engine_set_metronome_enabled(engine, true);

        let buffer = render(engine, FRAMES_PER_BEAT * 8);
        let peaks: Vec<f32> = onsets(&buffer).into_iter().map(|(_, p)| p).collect();
        assert_eq!(peaks.len(), 8);
        let downbeats = [peaks[0], peaks[4]];
        let offbeats = [peaks[1], peaks[2], peaks[3], peaks[5], peaks[6], peaks[7]];
        for down in downbeats {
            for off in offbeats {
                assert!(
                    down > off * 1.3,
                    "downbeat {down} should clearly exceed offbeat {off}"
                );
            }
        }

        gooey_engine_set_metronome_accent_enabled(engine, false);
        let buffer = render(engine, FRAMES_PER_BEAT * 8);
        let peaks: Vec<f32> = onsets(&buffer).into_iter().map(|(_, p)| p).collect();
        assert_eq!(peaks.len(), 8);
        let first = peaks[0];
        for peak in &peaks {
            assert!(
                (peak - first).abs() < first * 0.01,
                "unaccented clicks should be uniform, got {peaks:?}"
            );
        }
        gooey_engine_free(engine);
    }
}

#[test]
fn division_controls_the_click_rate() {
    unsafe {
        let engine = silent_running_engine();
        gooey_engine_set_metronome_enabled(engine, true);

        // Two bars' worth of frames at each division.
        let two_bars = FRAMES_PER_BEAT * 8;

        gooey_engine_set_metronome_division(engine, METRONOME_DIVISION_EIGHTH);
        assert_eq!(onsets(&render(engine, two_bars)).len(), 16);

        gooey_engine_set_metronome_division(engine, METRONOME_DIVISION_SIXTEENTH);
        assert_eq!(onsets(&render(engine, two_bars)).len(), 32);

        gooey_engine_set_metronome_division(engine, METRONOME_DIVISION_BAR);
        assert_eq!(onsets(&render(engine, two_bars)).len(), 2);

        // An unrecognized id is ignored and the current division is kept.
        gooey_engine_set_metronome_division(engine, 99);
        assert_eq!(
            gooey_engine_get_metronome_division(engine),
            METRONOME_DIVISION_BAR
        );
        gooey_engine_free(engine);
    }
}

/// Changing division mid-run is picked up by the render thread without the
/// setter reaching into render-owned state, and without a double-click at the
/// switch: the grid index is measured in units of the division, so the stale
/// one must be discarded rather than compared against the new units.
#[test]
fn division_change_mid_run_takes_effect_cleanly() {
    unsafe {
        let engine = silent_running_engine();
        gooey_engine_set_metronome_enabled(engine, true);

        // Land exactly on a beat boundary, having just clicked it.
        let first = onsets(&render(engine, FRAMES_PER_BEAT));
        assert_eq!(first.len(), 1);

        gooey_engine_set_metronome_division(engine, METRONOME_DIVISION_EIGHTH);
        // One beat at eighths is 2 clicks, at the beat and the half-beat, with
        // no extra click squeezed in at the switch itself.
        let after = onsets(&render(engine, FRAMES_PER_BEAT));
        assert_eq!(after.len(), 2, "expected 2 eighth clicks, got {after:?}");
        assert!(after[0].0 <= 2);
        assert!(
            after[1].0.abs_diff(FRAMES_PER_BEAT / 2) <= 2,
            "second eighth landed at {}, expected {}",
            after[1].0,
            FRAMES_PER_BEAT / 2
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn stopped_transport_is_silent() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        gooey_engine_set_metronome_enabled(engine, true);

        let buffer = render(engine, FRAMES_PER_BEAT * 8);
        assert!(
            max_abs(&buffer) < SILENCE,
            "a stopped transport must not click, peak was {}",
            max_abs(&buffer)
        );

        gooey_engine_sequencer_start(engine);
        assert_eq!(onsets(&render(engine, FRAMES_PER_BEAT * 4)).len(), 4);
        gooey_engine_free(engine);
    }
}

#[test]
fn stop_and_restart_resyncs_the_click() {
    unsafe {
        let engine = silent_running_engine();
        gooey_engine_set_metronome_enabled(engine, true);

        render(engine, FRAMES_PER_BEAT + FRAMES_PER_BEAT / 2);
        gooey_engine_sequencer_stop(engine);

        // The in-flight click finishes its ~36 ms decay rather than cutting
        // off, then everything goes quiet.
        let tail = render(engine, FRAMES_PER_BEAT);
        let decay_frames = (0.040 * SAMPLE_RATE) as usize;
        assert!(
            max_abs(&tail[decay_frames * 2..]) < SILENCE,
            "the click must not sound while the transport is stopped"
        );

        gooey_engine_sequencer_set_beat_position(engine, 0.0);
        gooey_engine_sequencer_start(engine);
        let found = onsets(&render(engine, FRAMES_PER_BEAT * 2));
        assert_eq!(found.len(), 2);
        assert!(found[0].0 <= 2, "restart should click immediately");
        gooey_engine_free(engine);
    }
}

/// Regression: transport commands issued between two audio callbacks all land
/// in a single `Mixer::apply_pending` batch, so the render thread never observes
/// `running == false` and cannot infer the discontinuity from that. The click
/// must still resync — this is the ordering `stop_and_restart_resyncs_the_click`
/// hides by rendering between the stop and the start.
#[test]
fn queued_stop_reset_start_burst_still_clicks_the_downbeat() {
    unsafe {
        let engine = silent_running_engine();
        gooey_engine_set_metronome_enabled(engine, true);
        // Stay inside beat 0 so the stale grid index is the downbeat's own, and
        // resetting to 0 lands right back on it.
        render(engine, FRAMES_PER_BEAT / 4);

        gooey_engine_sequencer_stop(engine);
        gooey_engine_sequencer_reset(engine);
        gooey_engine_sequencer_start(engine);

        let found = onsets(&render(engine, FRAMES_PER_BEAT * 2));
        assert_eq!(found.len(), 2, "expected 2 clicks, got {found:?}");
        assert!(
            found[0].0 <= 2,
            "the downbeat was dropped after a queued stop/reset/start; first \
             click landed at frame {}",
            found[0].0
        );
        gooey_engine_free(engine);
    }
}

/// Seeking twice onto the same boundary must click both times: the second seek
/// is a real discontinuity even though the beat position is unchanged.
#[test]
fn repeated_seek_to_the_same_boundary_clicks_each_time() {
    unsafe {
        let engine = silent_running_engine();
        gooey_engine_set_metronome_enabled(engine, true);

        gooey_engine_sequencer_set_beat_position(engine, 4.0);
        let first = onsets(&render(engine, FRAMES_PER_BEAT / 2));
        assert_eq!(first.len(), 1, "first seek should click");

        gooey_engine_sequencer_set_beat_position(engine, 4.0);
        let second = onsets(&render(engine, FRAMES_PER_BEAT / 2));
        assert_eq!(second.len(), 1, "second seek to the same beat should click");
        assert!(second[0].0 <= 2);
        gooey_engine_free(engine);
    }
}

#[test]
fn seek_resyncs_the_click_phase() {
    unsafe {
        let engine = silent_running_engine();
        gooey_engine_set_metronome_enabled(engine, true);
        render(engine, FRAMES_PER_BEAT / 3);

        // Landing exactly on a bar line clicks on the very next sample.
        gooey_engine_sequencer_set_beat_position(engine, 8.0);
        let found = onsets(&render(engine, FRAMES_PER_BEAT * 2));
        assert_eq!(found.len(), 2);
        assert!(
            found[0].0 <= 2,
            "a seek onto a beat should click immediately"
        );
        assert!(found[1].0.abs_diff(FRAMES_PER_BEAT) <= 2);
        gooey_engine_free(engine);
    }
}

#[test]
fn level_scales_the_click_and_zero_is_silent() {
    unsafe {
        let engine = silent_running_engine();
        gooey_engine_set_metronome_enabled(engine, true);
        // Compare like with like: the accent changes both the pitch and the
        // level of a click, so leave every click identical and vary only the
        // level control under test.
        gooey_engine_set_metronome_accent_enabled(engine, false);

        // Each level change is followed by a discarded beat: the level is
        // smoothed over 10 ms, so a click landing immediately after the change
        // still sounds at close to the old level.
        gooey_engine_set_metronome_level(engine, 1.0);
        render(engine, FRAMES_PER_BEAT);
        let loud = max_abs(&render(engine, FRAMES_PER_BEAT));

        gooey_engine_set_metronome_level(engine, 0.25);
        render(engine, FRAMES_PER_BEAT);
        let quiet = max_abs(&render(engine, FRAMES_PER_BEAT));
        let ratio = loud / quiet;
        assert!(
            (ratio - 4.0).abs() < 0.2,
            "level should scale linearly, ratio was {ratio}"
        );

        gooey_engine_set_metronome_level(engine, 0.0);
        render(engine, FRAMES_PER_BEAT);
        assert!(max_abs(&render(engine, FRAMES_PER_BEAT)) < SILENCE);

        // Out-of-range clamps, non-finite is rejected outright.
        gooey_engine_set_metronome_level(engine, 5.0);
        assert_eq!(gooey_engine_get_metronome_level(engine), 1.0);
        gooey_engine_set_metronome_level(engine, f32::NAN);
        assert_eq!(gooey_engine_get_metronome_level(engine), 1.0);
        gooey_engine_free(engine);
    }
}

/// The central claim: enabling the click cannot change the mix. Rendering a
/// real drum pattern with the click on must equal the pattern alone plus the
/// click alone, sample for sample. Any routing that let the click reach the
/// limiter, the compressor sidechain, or an effect would break additivity.
#[test]
fn metronome_is_strictly_additive_over_the_mix() {
    unsafe {
        // Four-on-the-floor kick plus offbeat hats, loud enough that the
        // limiter and compressor have something to work on.
        unsafe fn with_pattern(metronome: bool) -> *mut GooeyEngine {
            let engine = gooey_engine_new(SAMPLE_RATE);
            for step in [0, 4, 8, 12] {
                gooey_engine_sequencer_set_instrument_step(engine, INSTRUMENT_KICK, step, true);
            }
            for step in [2, 6, 10, 14] {
                gooey_engine_sequencer_set_instrument_step(engine, INSTRUMENT_HIHAT, step, true);
            }
            gooey_engine_set_master_gain(engine, 1.0);
            gooey_engine_set_global_effect_enabled(engine, EFFECT_LIMITER, true);
            gooey_engine_set_global_effect_enabled(engine, EFFECT_COMPRESSOR, true);
            gooey_engine_set_metronome_enabled(engine, metronome);
            gooey_engine_set_metronome_level(engine, 1.0);
            gooey_engine_sequencer_start(engine);
            engine
        }

        let frames = FRAMES_PER_BEAT * 4;
        let music_only = render(with_pattern(false), frames);
        let music_and_click = render(with_pattern(true), frames);

        let click_engine = silent_running_engine();
        gooey_engine_set_metronome_enabled(click_engine, true);
        gooey_engine_set_metronome_level(click_engine, 1.0);
        let click_only = render(click_engine, frames);

        assert!(max_abs(&music_only) > 0.1, "the pattern must be audible");
        assert!(max_abs(&click_only) > 0.1, "the click must be audible");

        for (index, ((both, music), click)) in music_and_click
            .iter()
            .zip(&music_only)
            .zip(&click_only)
            .enumerate()
        {
            assert!(
                (both - (music + click)).abs() < 1.0e-6,
                "sample {index}: {both} != {music} + {click}; the click is \
                 interacting with the mix instead of being summed after it"
            );
        }
        gooey_engine_free(click_engine);
    }
}

/// Disabling while a click is sounding must fade it out, not truncate it. A
/// hard cut steps a nonzero sample straight to zero, which is an audible pop.
/// Self-calibrating: the step across the disable seam is compared against the
/// largest step the sine itself takes just before it, so the threshold does not
/// depend on frequency or level.
#[test]
fn disabling_mid_click_does_not_pop() {
    unsafe {
        let engine = silent_running_engine();
        gooey_engine_set_metronome_enabled(engine, true);
        gooey_engine_set_metronome_level(engine, 1.0);

        // 300 frames ≈ 6.8 ms in: past the 1 ms attack, still well inside the
        // 30 ms decay, so the envelope is at a substantial nonzero level.
        let before = render(engine, 300);
        let left: Vec<f32> = before.chunks_exact(2).map(|f| f[0]).collect();
        let amplitude = left[left.len() - 60..]
            .iter()
            .fold(0.0_f32, |a, b| a.max(b.abs()));
        assert!(
            amplitude > 0.05,
            "expected a click in flight at the seam, amplitude was {amplitude}"
        );
        let natural_step = left
            .windows(2)
            .rev()
            .take(60)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0_f32, f32::max);

        gooey_engine_set_metronome_enabled(engine, false);
        let after = render(engine, FRAMES_PER_BEAT);
        let seam_step = (after[0] - left[left.len() - 1]).abs();
        assert!(
            seam_step <= natural_step * 2.0,
            "disabling stepped {seam_step} across the seam vs a natural sine \
             step of {natural_step} — that is a pop, not a fade"
        );

        // And the fade still reaches exact silence promptly (release is 5 ms).
        let settled = (0.010 * SAMPLE_RATE) as usize;
        assert!(
            max_abs(&after[settled * 2..]) < SILENCE,
            "the released tail must reach digital silence"
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn metronome_never_feeds_the_effects() {
    unsafe {
        let engine = silent_running_engine();
        // Maximum-tail reverb and delay. If the click were summed anywhere
        // before the effect chain, disabling it would leave seconds of tail.
        gooey_engine_set_global_effect_enabled(engine, EFFECT_REVERB, true);
        gooey_engine_set_global_effect_param(engine, EFFECT_REVERB, 0, 1.0);
        gooey_engine_set_global_effect_param(engine, EFFECT_REVERB, 1, 1.0);
        gooey_engine_set_global_effect_enabled(engine, EFFECT_DELAY, true);
        gooey_engine_set_global_effect_param(engine, EFFECT_DELAY, 1, 0.9);
        gooey_engine_set_global_effect_param(engine, EFFECT_DELAY, 2, 1.0);

        gooey_engine_set_metronome_enabled(engine, true);
        gooey_engine_set_metronome_level(engine, 1.0);
        let audible = render(engine, FRAMES_PER_BEAT * 8);
        assert!(
            max_abs(&audible) > 0.1,
            "the click should be clearly audible before we test its absence"
        );

        gooey_engine_set_metronome_enabled(engine, false);
        let buffer = render(engine, FRAMES_PER_BEAT * 8);
        assert!(
            max_abs(&buffer) < SILENCE,
            "no effect tail may remain, peak was {}",
            max_abs(&buffer)
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn metronome_is_absent_from_offline_bounce() {
    unsafe {
        let engine = silent_running_engine();
        gooey_engine_set_metronome_enabled(engine, true);
        gooey_engine_set_metronome_level(engine, 1.0);

        let mut length: u32 = 0;
        let buffer = gooey_engine_bounce_to_buffer(engine, 2, &mut length);
        assert!(!buffer.is_null());
        assert_eq!(length as usize, FRAMES_PER_BEAT * 8);
        let bounced = std::slice::from_raw_parts(buffer, length as usize);
        assert!(
            max_abs(bounced) < SILENCE,
            "the bounce must not contain the click, peak was {}",
            max_abs(bounced)
        );
        gooey_engine_free_buffer(buffer, length);

        // Prove the silence came from the bypass and not from a dead metronome,
        // and that the bypass flag was cleared afterwards.
        gooey_engine_sequencer_set_beat_position(engine, 0.0);
        gooey_engine_sequencer_start(engine);
        let live = render(engine, FRAMES_PER_BEAT * 4);
        assert!(
            max_abs(&live) > 0.1,
            "the click must still work after a bounce, peak was {}",
            max_abs(&live)
        );
        gooey_engine_free(engine);
    }
}

/// `gooey_engine_loop_render_to_wav` drives the channel directly instead of
/// going through `GooeyEngine::render`, so it needs no metronome bypass. This
/// pins that property: if the stem export is ever rerouted through the engine
/// render path, a click would appear here and this test would catch it.
#[cfg(feature = "bounce")]
#[test]
fn metronome_is_absent_from_loop_render_to_wav() {
    unsafe {
        use std::ffi::CString;

        const DC: f32 = 0.5;
        let frames = 4_096_u32;
        let engine = gooey_engine_new(SAMPLE_RATE);
        assert!(gooey_engine_loop_load(
            engine,
            0,
            vec![DC; frames as usize * 2].as_ptr(),
            frames,
            2,
            SAMPLE_RATE,
        ));

        gooey_engine_sequencer_start(engine);
        gooey_engine_set_metronome_enabled(engine, true);
        gooey_engine_set_metronome_level(engine, 1.0);

        let path =
            std::env::temp_dir().join(format!("gooey_metronome_stem_{}.wav", std::process::id()));
        let c_path = CString::new(path.to_str().unwrap()).unwrap();
        assert!(gooey_engine_loop_render_to_wav(
            engine,
            0,
            frames,
            0,
            c_path.as_ptr()
        ));

        let mut reader = hound::WavReader::open(&path).unwrap();
        for sample in reader.samples::<f32>() {
            let sample = sample.unwrap();
            assert!(
                (sample - DC).abs() < 1.0e-6,
                "stem export must be pure DC with no click transient, got {sample}"
            );
        }
        let _ = std::fs::remove_file(&path);
        gooey_engine_free(engine);
    }
}

#[test]
fn metronome_tracks_bpm_changes() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        gooey_engine_set_bpm(engine, 180.0);
        gooey_engine_sequencer_start(engine);
        gooey_engine_set_metronome_enabled(engine, true);

        // 60 / 180 * 44_100 = 14_700 frames per quarter note.
        let expected = 14_700;
        let found = onsets(&render(engine, expected * 8));
        assert_eq!(found.len(), 8, "expected 8 clicks, got {found:?}");
        for window in found.windows(2) {
            let spacing = window[1].0 - window[0].0;
            assert!(
                spacing.abs_diff(expected) <= 2,
                "click spacing was {spacing}, expected {expected}"
            );
        }
        gooey_engine_free(engine);
    }
}

#[test]
fn null_engine_is_safe() {
    unsafe {
        let null_mut: *mut GooeyEngine = std::ptr::null_mut();
        let null_const: *const GooeyEngine = std::ptr::null();

        gooey_engine_set_metronome_enabled(null_mut, true);
        gooey_engine_set_metronome_level(null_mut, 0.5);
        gooey_engine_set_metronome_division(null_mut, METRONOME_DIVISION_EIGHTH);
        gooey_engine_set_metronome_accent_enabled(null_mut, false);

        assert!(!gooey_engine_get_metronome_enabled(null_const));
        assert_eq!(gooey_engine_get_metronome_level(null_const), 0.35);
        assert_eq!(
            gooey_engine_get_metronome_division(null_const),
            METRONOME_DIVISION_QUARTER
        );
        assert!(gooey_engine_get_metronome_accent_enabled(null_const));
    }
}
