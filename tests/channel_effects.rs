//! Voice inserts are independent of synth type and precede the kit sum.
use gooey::ffi::*;
const SR: f32 = 44_100.0;
unsafe fn render(engine: *mut GooeyEngine, frames: usize) -> Vec<f32> {
    let mut audio = vec![0.0; frames * 2];
    gooey_engine_render(engine, audio.as_mut_ptr(), frames as u32);
    assert!(audio.iter().all(|x| x.is_finite()));
    audio
}
unsafe fn install(
    control: *mut GooeyLiveControl,
    channel: u32,
    effect: u32,
    params: &[(u32, f32)],
) -> u64 {
    let params: Vec<_> = params
        .iter()
        .map(|&(param, value)| GooeyEffectParamDescriptor { param, value })
        .collect();
    let descriptor = GooeyEffectDescriptor {
        effect,
        params: params.as_ptr(),
        param_count: params.len() as u32,
    };
    gooey_live_control_replace_channel_rack(control, channel, &descriptor, 1)
}
fn energy(audio: &[f32]) -> f32 {
    audio.iter().map(|x| x * x).sum()
}
#[test]
fn empty_racks_preserve_output_and_duplicate_kick_inserts_are_isolated() {
    unsafe {
        let dry = gooey_engine_new(SR);
        let wet = gooey_engine_new(SR);
        let control = gooey_engine_live_control_new(wet);
        assert_ne!(
            gooey_live_control_replace_channel_rack(control, 0, std::ptr::null(), 0),
            0
        );
        render(dry, 512);
        render(wet, 512);
        gooey_engine_trigger_channel(dry, 0);
        gooey_engine_trigger_channel(wet, 0);
        assert_eq!(render(dry, 4096), render(wet, 4096));
        gooey_engine_set_channel_instrument_type(dry, 1, INSTRUMENT_KICK);
        gooey_engine_set_channel_instrument_type(wet, 1, INSTRUMENT_KICK);
        assert_ne!(
            install(
                control,
                1,
                EFFECT_SATURATION,
                &[(SATURATION_PARAM_DRIVE, 1.0), (SATURATION_PARAM_MIX, 1.0)]
            ),
            0
        );
        render(dry, 512);
        render(wet, 512);
        gooey_engine_trigger_channel(dry, 0);
        gooey_engine_trigger_channel(wet, 0);
        assert_eq!(render(dry, 4096), render(wet, 4096));
        gooey_engine_trigger_channel(dry, 1);
        gooey_engine_trigger_channel(wet, 1);
        let a = render(dry, 4096);
        let b = render(wet, 4096);
        assert!(a.iter().zip(&b).any(|(a, b)| (a - b).abs() > 1e-4));
        gooey_engine_set_channel_instrument_type(wet, 1, INSTRUMENT_RESONATOR);
        assert!(gooey_engine_set_channel_preset(wet, 1, PERC_PRESET_KICK));
        gooey_engine_trigger_channel(wet, 1);
        assert!(energy(&render(wet, 4096)) > 0.0);
        gooey_live_control_free(control);
        gooey_engine_free(wet);
        gooey_engine_free(dry);
    }
}
#[test]
fn hihat_delay_is_stereo_and_mute_and_solo_silence_its_tails() {
    unsafe {
        let engine = gooey_engine_new(SR);
        gooey_engine_set_instrument_pan(engine, 2, 1.0);
        let control = gooey_engine_live_control_new(engine);
        assert_ne!(
            install(
                control,
                2,
                EFFECT_DELAY,
                &[
                    (DELAY_PARAM_TIMING, DELAY_TIMING_SIXTEENTH as f32),
                    (DELAY_PARAM_FEEDBACK, 0.8),
                    (DELAY_PARAM_MIX, 0.8),
                    (DELAY_PARAM_FILTER_CUTOFF, 8000.0),
                    (DELAY_PARAM_PINGPONG, 1.0)
                ]
            ),
            0
        );
        render(engine, 1024);
        gooey_engine_trigger_channel(engine, 2);
        let audio = render(engine, 15_000);
        let left: f32 = audio.chunks_exact(2).map(|x| x[0] * x[0]).sum();
        let right: f32 = audio.chunks_exact(2).map(|x| x[1] * x[1]).sum();
        assert!(left > 0.0 && right > 0.0);
        assert!(audio.chunks_exact(2).any(|x| (x[0] - x[1]).abs() > 1e-4));
        gooey_engine_set_instrument_mute(engine, 2, true);
        render(engine, 10_000);
        assert!(energy(&render(engine, 4096)) < 1e-12);
        gooey_engine_set_instrument_mute(engine, 2, false);
        gooey_engine_trigger_channel(engine, 2);
        render(engine, 4096);
        gooey_engine_set_instrument_solo(engine, 0, true);
        render(engine, 10_000);
        assert!(energy(&render(engine, 4096)) < 1e-12);
        gooey_live_control_free(control);
        gooey_engine_free(engine);
    }
}
#[test]
fn validation_and_stale_or_busy_commands_preserve_accepted_rack() {
    unsafe {
        assert_eq!(install(std::ptr::null_mut(), 0, EFFECT_DELAY, &[]), 0);
        let engine = gooey_engine_new(SR);
        let control = gooey_engine_live_control_new(engine);
        assert_eq!(install(control, 99, EFFECT_DELAY, &[]), 0);
        assert_eq!(install(control, 0, EFFECT_LIMITER, &[]), 0);
        assert_eq!(
            install(
                control,
                0,
                EFFECT_PLATE_REVERB,
                &[(PLATE_PARAM_SIZE, f32::NAN)]
            ),
            0
        );
        assert_eq!(
            install(
                control,
                0,
                EFFECT_DELAY,
                &[(DELAY_PARAM_MIX, 0.2), (DELAY_PARAM_MIX, 0.5)]
            ),
            0
        );
        let generation = install(
            control,
            0,
            EFFECT_FEEDBACK_WAVESHAPER,
            &[
                (FEEDBACK_WAVESHAPER_PARAM_DRIVE, 31.0),
                (FEEDBACK_WAVESHAPER_PARAM_FEEDBACK, 0.9),
                (FEEDBACK_WAVESHAPER_PARAM_FILTER_CUTOFF, 8000.0),
                (FEEDBACK_WAVESHAPER_PARAM_MIX, 1.0),
            ],
        );
        assert_ne!(generation, 0);
        assert_eq!(install(control, 0, EFFECT_DELAY, &[]), 0);
        assert_eq!(
            gooey_live_control_set_channel_effect_param(
                control,
                0,
                0,
                generation + 1,
                FEEDBACK_WAVESHAPER_PARAM_DRIVE,
                2.0
            ),
            0
        );
        assert_eq!(
            gooey_live_control_set_channel_effect_param(
                control,
                1,
                0,
                generation,
                FEEDBACK_WAVESHAPER_PARAM_DRIVE,
                2.0
            ),
            0
        );
        let edit = gooey_live_control_set_channel_effect_param(
            control,
            0,
            0,
            generation,
            FEEDBACK_WAVESHAPER_PARAM_DRIVE,
            2.0,
        );
        assert_ne!(edit, 0);
        render(engine, 1024);
        assert_eq!(
            gooey_live_control_get_last_applied_generation(control),
            edit
        );
        assert_ne!(
            install(
                control,
                0,
                EFFECT_PLATE_REVERB,
                &[(PLATE_PARAM_SIZE, 0.7), (PLATE_PARAM_WIDTH, 0.9)]
            ),
            0
        );
        assert_eq!(
            gooey_live_control_set_channel_effect_param(
                control,
                0,
                0,
                generation,
                FEEDBACK_WAVESHAPER_PARAM_DRIVE,
                2.0
            ),
            0
        );
        render(engine, 1024);
        gooey_engine_trigger_channel(engine, 0);
        assert!(energy(&render(engine, 4096)) > 0.0);
        gooey_live_control_free(control);
        gooey_engine_free(engine);
    }
}
#[test]
fn full_queue_rejects_without_displacing_channel_generation() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let control = gooey_engine_live_control_new(engine);
        let generation = install(control, 0, EFFECT_DELAY, &[]);
        assert_ne!(generation, 0);
        for _ in 1..GOOEY_LIVE_CONTROL_QUEUE_CAPACITY {
            assert_ne!(gooey_live_control_set_track_gain(control, 0, 1.0), 0);
        }
        assert_eq!(install(control, 1, EFFECT_DELAY, &[]), 0);
        assert_eq!(
            gooey_live_control_set_channel_effect_param(
                control,
                0,
                0,
                generation,
                DELAY_PARAM_MIX,
                0.5
            ),
            0
        );
        render(engine, 1024);
        assert_ne!(
            gooey_live_control_set_channel_effect_param(
                control,
                0,
                0,
                generation,
                DELAY_PARAM_MIX,
                0.5
            ),
            0
        );
        assert_ne!(install(control, 1, EFFECT_DELAY, &[]), 0);
        render(engine, 1024);
        gooey_live_control_free(control);
        gooey_engine_free(engine);
    }
}

#[test]
fn channel_racks_can_change_concurrently_with_rendering() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let control = gooey_engine_live_control_new(engine);
        let address = engine as usize;
        let worker = std::thread::spawn(move || {
            for _ in 0..4000 {
                let mut audio = [0.0; 128];
                gooey_engine_render(address as *mut GooeyEngine, audio.as_mut_ptr(), 64);
                assert!(audio.iter().all(|x| x.is_finite()));
            }
        });
        let mut accepted = 0;
        for i in 0..4000 {
            let effect = if i % 2 == 0 {
                EFFECT_DELAY
            } else {
                EFFECT_SATURATION
            };
            let generation = install(control, i % 4, effect, &[]);
            if generation != 0 {
                accepted += 1;
                let param = if effect == EFFECT_DELAY {
                    DELAY_PARAM_MIX
                } else {
                    SATURATION_PARAM_MIX
                };
                gooey_live_control_set_channel_effect_param(
                    control,
                    i % 4,
                    0,
                    generation,
                    param,
                    0.5,
                );
            }
        }
        worker.join().unwrap();
        assert!(accepted > 0);
        render(engine, 1024);
        gooey_live_control_free(control);
        gooey_engine_free(engine);
    }
}

#[test]
fn offline_bounce_includes_voice_effects_and_resets_preexisting_echoes() {
    unsafe {
        let engine = gooey_engine_new(SR);
        let control = gooey_engine_live_control_new(engine);
        let generation = install(
            control,
            0,
            EFFECT_DELAY,
            &[(DELAY_PARAM_MIX, 1.0), (DELAY_PARAM_FEEDBACK, 0.8)],
        );
        assert_ne!(generation, 0);
        render(engine, 1024);
        gooey_engine_trigger_channel(engine, 0);
        render(engine, 30_000);
        // No sequencer notes: bounce must clear any history from the live hit.
        gooey_engine_set_channel_instrument_type(engine, 0, INSTRUMENT_TOM);
        let mut count = 0;
        let silent = gooey_engine_bounce_to_buffer(engine, 1, &mut count);
        assert!(!silent.is_null());
        assert!(energy(std::slice::from_raw_parts(silent, count as usize)) < 1e-12);
        gooey_engine_free_buffer(silent, count);
        // A programmed hit passes through the rack in offline rendering.
        gooey_engine_sequencer_set_instrument_step(engine, 0, 0, true);
        let audible = gooey_engine_bounce_to_buffer(engine, 1, &mut count);
        assert!(energy(std::slice::from_raw_parts(audible, count as usize)) > 0.0);
        gooey_engine_free_buffer(audible, count);
        assert_ne!(
            gooey_live_control_set_channel_effect_param(
                control,
                0,
                0,
                generation,
                DELAY_PARAM_MIX,
                0.5
            ),
            0
        );
        render(engine, 1024);
        gooey_live_control_free(control);
        gooey_engine_free(engine);
    }
}
