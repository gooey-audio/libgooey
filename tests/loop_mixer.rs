//! End-to-end tests for the loop-mixer FFI (`gooey_engine_loop_*`). Mirrors the
//! host-side calling sequence an embedded app would use: create engine → load a
//! loop into a channel → submix (gain/mute/solo) → add per-channel effects →
//! render → inspect the interleaved stereo output.

use gooey::ffi::*;

const SAMPLE_RATE: f32 = 44_100.0;

/// Build an interleaved stereo sine loop (both channels identical).
fn stereo_sine(seconds: f32, hz: f32) -> Vec<f32> {
    let frames = (SAMPLE_RATE * seconds) as usize;
    let mut out = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let s = (i as f32 / SAMPLE_RATE * hz * std::f32::consts::TAU).sin() * 0.5;
        out.push(s);
        out.push(s);
    }
    out
}

/// Render `frames` stereo frames and return the peak absolute sample of the
/// second half (skips the gain-smoother attack transient).
unsafe fn render_peak(engine: *mut GooeyEngine, frames: usize) -> f32 {
    let mut buffer = vec![0.0_f32; frames * 2];
    gooey_engine_render(engine, buffer.as_mut_ptr(), frames as u32);
    buffer[frames..].iter().fold(0.0_f32, |acc, s| {
        assert!(s.is_finite(), "non-finite sample in render output");
        acc.max(s.abs())
    })
}

#[test]
fn load_and_render_produces_audio() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        let loop_samples = stereo_sine(0.5, 220.0);
        let frames = loop_samples.len() / 2;
        assert!(gooey_engine_loop_load(
            engine,
            0,
            loop_samples.as_ptr(),
            frames as u32,
            2,
            SAMPLE_RATE,
        ));
        gooey_engine_loop_set_playing(engine, 0, true);

        let peak = render_peak(engine, 8192);
        assert!(peak > 1e-3, "expected audible loop output, got {peak}");

        // Playhead should have advanced.
        assert!(gooey_engine_loop_get_position(engine, 0) > 0.0);
        gooey_engine_free(engine);
    }
}

#[test]
fn load_rejects_invalid_inputs() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        let samples = stereo_sine(0.1, 440.0);
        let frames = (samples.len() / 2) as u32;

        // Null pointer.
        assert!(!gooey_engine_loop_load(
            engine,
            0,
            std::ptr::null(),
            frames,
            2,
            SAMPLE_RATE
        ));
        // Zero frames / channels.
        assert!(!gooey_engine_loop_load(
            engine,
            0,
            samples.as_ptr(),
            0,
            2,
            SAMPLE_RATE
        ));
        assert!(!gooey_engine_loop_load(
            engine,
            0,
            samples.as_ptr(),
            frames,
            0,
            SAMPLE_RATE
        ));
        // Bad sample rate.
        assert!(!gooey_engine_loop_load(
            engine,
            0,
            samples.as_ptr(),
            frames,
            2,
            0.0
        ));
        // Out-of-range channel index.
        assert!(!gooey_engine_loop_load(
            engine,
            LOOP_CHANNEL_COUNT,
            samples.as_ptr(),
            frames,
            2,
            SAMPLE_RATE,
        ));
        gooey_engine_free(engine);
    }
}

#[test]
fn mute_silences_a_channel() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        let loop_samples = stereo_sine(0.5, 220.0);
        let frames = loop_samples.len() / 2;
        gooey_engine_loop_load(
            engine,
            0,
            loop_samples.as_ptr(),
            frames as u32,
            2,
            SAMPLE_RATE,
        );
        gooey_engine_loop_set_playing(engine, 0, true);

        let audible = render_peak(engine, 8192);
        assert!(audible > 1e-3);

        gooey_engine_loop_set_mute(engine, 0, true);
        let muted = render_peak(engine, 8192);
        assert!(
            muted < audible * 0.02,
            "muted channel should be ~silent: audible {audible}, muted {muted}"
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn solo_isolates_channels() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        let loop_samples = stereo_sine(0.5, 220.0);
        let frames = (loop_samples.len() / 2) as u32;
        for ch in 0..2 {
            gooey_engine_loop_load(engine, ch, loop_samples.as_ptr(), frames, 2, SAMPLE_RATE);
            gooey_engine_loop_set_playing(engine, ch, true);
        }
        let both = render_peak(engine, 8192);

        gooey_engine_loop_set_solo(engine, 0, true);
        let solo = render_peak(engine, 8192);
        assert!(
            solo < both,
            "soloing one of two channels should reduce the summed peak: both {both}, solo {solo}"
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn per_channel_effect_changes_output() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        // A bright 6 kHz loop so a low-cutoff lowpass has obvious effect.
        let loop_samples = stereo_sine(0.5, 6_000.0);
        let frames = loop_samples.len() / 2;
        gooey_engine_loop_load(
            engine,
            0,
            loop_samples.as_ptr(),
            frames as u32,
            2,
            SAMPLE_RATE,
        );
        gooey_engine_loop_set_playing(engine, 0, true);

        let dry = render_peak(engine, 8192);

        // Add a lowpass filter to channel 0 only and clamp its cutoff low.
        let slot = gooey_engine_loop_effect_add(engine, 0, EFFECT_LOWPASS_FILTER);
        assert_eq!(slot, 0, "first effect should land in slot 0");
        assert_eq!(gooey_engine_loop_effect_count(engine, 0), 1);
        assert_eq!(gooey_engine_loop_effect_count(engine, 1), 0);
        assert_eq!(
            gooey_engine_loop_effect_type_at(engine, 0, 0),
            EFFECT_LOWPASS_FILTER as i32
        );
        gooey_engine_loop_effect_set_param(engine, 0, 0, FILTER_PARAM_CUTOFF, 300.0);

        // Let the filter settle, then measure.
        let _ = render_peak(engine, 8192);
        let filtered = render_peak(engine, 8192);
        assert!(
            filtered < dry * 0.6,
            "lowpass should attenuate a 6 kHz loop: dry {dry}, filtered {filtered}"
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn effect_chain_edit_operations() {
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);

        assert_eq!(
            gooey_engine_loop_effect_add(engine, 0, EFFECT_LOWPASS_FILTER),
            0
        );
        assert_eq!(gooey_engine_loop_effect_add(engine, 0, EFFECT_DELAY), 1);
        assert_eq!(gooey_engine_loop_effect_add(engine, 0, EFFECT_REVERB), 2);
        assert_eq!(gooey_engine_loop_effect_count(engine, 0), 3);

        // Move reverb (slot 2) to the front.
        assert!(gooey_engine_loop_effect_move(engine, 0, 2, 0));
        assert_eq!(
            gooey_engine_loop_effect_type_at(engine, 0, 0),
            EFFECT_REVERB as i32
        );

        // Remove the first effect.
        assert!(gooey_engine_loop_effect_remove(engine, 0, 0));
        assert_eq!(gooey_engine_loop_effect_count(engine, 0), 2);

        // The limiter is not a per-channel effect: add must fail.
        assert_eq!(gooey_engine_loop_effect_add(engine, 0, EFFECT_LIMITER), -1);

        // Clear leaves the chain empty.
        gooey_engine_loop_effect_clear(engine, 0);
        assert_eq!(gooey_engine_loop_effect_count(engine, 0), 0);
        gooey_engine_free(engine);
    }
}

#[test]
fn master_gain_scales_loops() {
    // Regression: the loop mixer must sit *before* master gain so the master
    // fader (and headroom) scales loops, not just the synth/drum bus.
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        let loop_samples = stereo_sine(0.5, 220.0);
        let frames = (loop_samples.len() / 2) as u32;
        gooey_engine_loop_load(engine, 0, loop_samples.as_ptr(), frames, 2, SAMPLE_RATE);
        gooey_engine_loop_set_playing(engine, 0, true);

        gooey_engine_set_master_gain(engine, 1.0);
        let loud = render_peak(engine, 8192);
        assert!(
            loud > 1e-2,
            "expected audible loop at unity master gain, got {loud}"
        );

        gooey_engine_set_master_gain(engine, 0.0);
        let _ = render_peak(engine, 8192); // let the 30 ms master-gain glide settle
        let silent = render_peak(engine, 8192);
        assert!(
            silent < loud * 0.01,
            "master gain 0 must silence loops too: loud {loud}, silent {silent}"
        );
        gooey_engine_free(engine);
    }
}

#[test]
fn bpm_change_retempos_existing_loop_delay() {
    // Regression: changing the host BPM must re-tempo per-channel delays that
    // already exist, not only effects added afterwards.
    unsafe fn render_impulse_through_delay(bpm: f32) -> Vec<f32> {
        let engine = gooey_engine_new(SAMPLE_RATE);
        // 1s loop holding a single impulse at frame 0 (won't wrap within render).
        let n = SAMPLE_RATE as usize;
        let mut samples = vec![0.0_f32; n * 2];
        samples[0] = 0.8;
        samples[1] = 0.8;
        gooey_engine_loop_load(engine, 0, samples.as_ptr(), n as u32, 2, SAMPLE_RATE);
        gooey_engine_loop_set_playing(engine, 0, true);
        gooey_engine_set_master_gain(engine, 1.0);

        // Add a (quarter-note) delay, THEN change BPM — the order that exposes
        // the bug.
        assert_eq!(gooey_engine_loop_effect_add(engine, 0, EFFECT_DELAY), 0);
        gooey_engine_loop_effect_set_param(engine, 0, 0, DELAY_PARAM_MIX, 0.9);
        gooey_engine_loop_effect_set_param(engine, 0, 0, DELAY_PARAM_FEEDBACK, 0.4);
        gooey_engine_set_bpm(engine, bpm);

        let frames = 30_000usize;
        let mut buffer = vec![0.0_f32; frames * 2];
        gooey_engine_render(engine, buffer.as_mut_ptr(), frames as u32);
        gooey_engine_free(engine);
        buffer
    }

    unsafe {
        // quarter ≈ 22050 samples at 120 BPM, ≈ 11025 at 240 BPM.
        let slow = render_impulse_through_delay(120.0);
        let fast = render_impulse_through_delay(240.0);
        // If existing delays weren't re-tempo'd both renders would be identical.
        let diff: f32 = slow.iter().zip(&fast).map(|(a, b)| (a - b).abs()).sum();
        assert!(
            diff > 1.0,
            "BPM change should move an existing delay's echoes (diff {diff})"
        );
    }
}

/// Estimate a mono signal's dominant frequency from its zero-crossing rate.
/// Each full cycle of a (roughly) periodic signal produces two zero crossings.
fn zero_crossing_frequency(samples: &[f32], sample_rate: f32) -> f32 {
    let crossings = samples
        .windows(2)
        .filter(|w| (w[0] <= 0.0 && w[1] > 0.0) || (w[0] >= 0.0 && w[1] < 0.0))
        .count();
    (crossings as f32 / 2.0) / (samples.len() as f32 / sample_rate)
}

#[test]
fn resample_mode_tempo_ratio_matches_bpm_ratio() {
    // Naive resample warp: cursor advance scales directly with engine_bpm /
    // source_bpm, so doubling the engine BPM should double how far the
    // playhead travels over a fixed render window.
    unsafe {
        let position_after = |pitch_mode: u32, engine_bpm: f32| -> f32 {
            let engine = gooey_engine_new(SAMPLE_RATE);
            let loop_samples = stereo_sine(2.0, 220.0); // long enough: no wrap
            let frames = (loop_samples.len() / 2) as u32;
            gooey_engine_loop_load(engine, 0, loop_samples.as_ptr(), frames, 2, SAMPLE_RATE);
            gooey_engine_loop_set_source_bpm(engine, 0, 120.0);
            gooey_engine_loop_set_pitch_mode(engine, 0, pitch_mode);
            gooey_engine_set_bpm(engine, engine_bpm);
            gooey_engine_loop_set_playing(engine, 0, true);
            let mut buf = vec![0.0_f32; 8192 * 2];
            gooey_engine_render(engine, buf.as_mut_ptr(), 8192);
            let pos = gooey_engine_loop_get_position(engine, 0);
            gooey_engine_free(engine);
            pos
        };

        let baseline = position_after(PITCH_MODE_OFF, 120.0);
        let warped = position_after(PITCH_MODE_RESAMPLE, 240.0);

        assert!(baseline > 0.0, "expected the playhead to advance");
        let ratio = warped / baseline;
        assert!(
            (ratio - 2.0).abs() < 0.05,
            "expected ~2x position advance under a 2x BPM warp, got ratio {ratio} \
             (baseline {baseline}, warped {warped})"
        );
    }
}

#[test]
fn preserve_pitch_mode_tempo_ratio_matches_bpm_ratio() {
    // WSOLA mode: pitch is held, but the *rate of source-material consumption*
    // (i.e. how far the analysis cursor travels — reported via
    // gooey_engine_loop_get_position) should still track the BPM ratio, same
    // as Resample mode. That's what "tempo changed" means even when pitch
    // doesn't move with it.
    unsafe {
        let position_after = |engine_bpm: f32| -> f32 {
            let engine = gooey_engine_new(SAMPLE_RATE);
            let loop_samples = stereo_sine(2.0, 220.0);
            let frames = (loop_samples.len() / 2) as u32;
            gooey_engine_loop_load(engine, 0, loop_samples.as_ptr(), frames, 2, SAMPLE_RATE);
            gooey_engine_loop_set_source_bpm(engine, 0, 120.0);
            gooey_engine_loop_set_pitch_mode(engine, 0, PITCH_MODE_PRESERVE_PITCH);
            gooey_engine_set_bpm(engine, engine_bpm);
            gooey_engine_loop_set_playing(engine, 0, true);
            let mut buf = vec![0.0_f32; 16_384 * 2];
            gooey_engine_render(engine, buf.as_mut_ptr(), 16_384);
            let pos = gooey_engine_loop_get_position(engine, 0);
            gooey_engine_free(engine);
            pos
        };

        let baseline = position_after(120.0);
        let warped = position_after(240.0);

        assert!(baseline > 0.0, "expected the analysis cursor to advance");
        let ratio = warped / baseline;
        // Wider tolerance than the resample test: WSOLA's similarity search
        // deliberately deviates from the naive per-hop jump to keep grains
        // phase-aligned, which trades a little tempo precision for
        // continuity — a periodic test tone (many near-equally-good
        // alignment candidates per hop) is close to a worst case for that
        // drift, so the achieved ratio is only approximately 2x, not exact.
        assert!(
            (ratio - 2.0).abs() < 0.25,
            "expected ~2x source consumption under a 2x BPM warp in PreservePitch \
             mode, got ratio {ratio} (baseline {baseline}, warped {warped})"
        );
    }
}

#[test]
fn preserve_pitch_holds_frequency_while_resample_shifts_it() {
    // The headline behavioral difference between the two warp modes: at the
    // same 1.5x BPM ratio, Resample should shift a 440 Hz tone to ~660 Hz,
    // while PreservePitch should hold ~440 Hz.
    unsafe {
        let render_left_channel = |pitch_mode: u32, frames: usize| -> Vec<f32> {
            let engine = gooey_engine_new(SAMPLE_RATE);
            let loop_samples = stereo_sine(2.0, 440.0);
            let n = (loop_samples.len() / 2) as u32;
            gooey_engine_loop_load(engine, 0, loop_samples.as_ptr(), n, 2, SAMPLE_RATE);
            gooey_engine_loop_set_source_bpm(engine, 0, 120.0);
            gooey_engine_loop_set_pitch_mode(engine, 0, pitch_mode);
            gooey_engine_set_bpm(engine, 180.0); // 1.5x source_bpm
            gooey_engine_loop_set_playing(engine, 0, true);
            let mut buf = vec![0.0_f32; frames * 2];
            gooey_engine_render(engine, buf.as_mut_ptr(), frames as u32);
            gooey_engine_free(engine);
            buf.chunks_exact(2).map(|f| f[0]).collect()
        };

        // Skip past the stretcher's first (fade-in) hop before measuring.
        let warmup = 4096;
        let measure = 8192;
        let total = warmup + measure;

        let preserved = render_left_channel(PITCH_MODE_PRESERVE_PITCH, total);
        let resampled = render_left_channel(PITCH_MODE_RESAMPLE, total);

        let preserved_hz = zero_crossing_frequency(&preserved[warmup..], SAMPLE_RATE);
        let resampled_hz = zero_crossing_frequency(&resampled[warmup..], SAMPLE_RATE);

        assert!(
            (preserved_hz - 440.0).abs() < 440.0 * 0.1,
            "PreservePitch should hold ~440 Hz, measured {preserved_hz}"
        );
        assert!(
            (resampled_hz - 660.0).abs() < 660.0 * 0.1,
            "Resample should shift to ~660 Hz (1.5x), measured {resampled_hz}"
        );
    }
}

#[test]
fn preserve_pitch_finite_across_loop_seam() {
    // A short loop wraps many times within the render, and at a warped tempo
    // most hops land near the loop boundary — exercises the search-window /
    // grain-extraction clamp and the wrap/reseed path in WsolaStretcher.
    unsafe {
        let engine = gooey_engine_new(SAMPLE_RATE);
        let loop_samples = stereo_sine(0.05, 330.0); // 50ms loop
        let n = (loop_samples.len() / 2) as u32;
        gooey_engine_loop_load(engine, 0, loop_samples.as_ptr(), n, 2, SAMPLE_RATE);
        gooey_engine_loop_set_source_bpm(engine, 0, 120.0);
        gooey_engine_loop_set_pitch_mode(engine, 0, PITCH_MODE_PRESERVE_PITCH);
        gooey_engine_set_bpm(engine, 150.0);
        gooey_engine_loop_set_playing(engine, 0, true);

        let frames = SAMPLE_RATE as usize; // 1s: many loop wraps, many hops
        let mut buf = vec![0.0_f32; frames * 2];
        gooey_engine_render(engine, buf.as_mut_ptr(), frames as u32);
        for s in &buf {
            assert!(
                s.is_finite(),
                "non-finite sample in PreservePitch output across a loop seam"
            );
        }
        gooey_engine_free(engine);
    }
}

#[test]
fn null_engine_is_safe() {
    unsafe {
        let null = std::ptr::null_mut();
        // Setters are no-ops; getters return defaults.
        gooey_engine_loop_set_playing(null, 0, true);
        gooey_engine_loop_set_gain(null, 0, 1.0);
        assert_eq!(gooey_engine_loop_get_position(std::ptr::null(), 0), 0.0);
        assert_eq!(gooey_engine_loop_effect_add(null, 0, EFFECT_DELAY), -1);
        assert_eq!(gooey_engine_loop_effect_count(std::ptr::null(), 0), 0);
        assert_eq!(gooey_engine_loop_effect_type_at(std::ptr::null(), 0, 0), -1);
    }
}
