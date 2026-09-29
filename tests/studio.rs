#![cfg(feature = "studio")]
use gooey::{ffi::*, studio::*};
use std::sync::atomic::{AtomicU64, Ordering};
fn path(name: &str) -> std::path::PathBuf {
    static ID: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join("gooey-studio-tests");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!(
        "studio-test-{}-{}-{name}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ))
}
fn render(s: &mut Studio, frames: usize) -> Vec<f32> {
    let mut out = vec![0.0; frames * 2];
    s.render(&mut out).unwrap();
    out
}
fn energy(v: &[f32]) -> f64 {
    v.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / v.len() as f64
}
fn bass_song() -> Session {
    let mut s = Session::default();
    s.steps[4][0] = 0.8;
    s.steps[4][8] = 0.8;
    s
}

#[test]
fn studio_demo_exports_finite_stereo_audio_and_exact_duration() {
    let file = path("demo.wav");
    let song = Session::demo();
    let report = song.export_wav(&file, 1, 0.25).unwrap();
    assert_eq!(
        report.frames,
        (240.0 / song.bpm as f64 * 48000.0).round() as u64 + 12000
    );
    assert!(report.rms > 0.01 && report.peak <= 1.0 && report.peak > 0.05);
    let r = hound::WavReader::open(&file).unwrap();
    assert_eq!(r.spec().channels, 2);
    assert_eq!(r.spec().sample_rate, 48000);
    assert_eq!(r.duration() as u64, report.frames);
    assert!(r.into_samples::<f32>().all(|s| s.unwrap().is_finite()));
    std::fs::remove_file(file).unwrap();
}
#[test]
fn studio_session_roundtrip_retains_every_payload() {
    let mut song = Session::demo();
    song.hits.push(Hit {
        tick: 42,
        instrument: INSTRUMENT_KICK,
        note: 36,
        velocity: 0.7,
    });
    song.automation.push(Lane {
        control: Control::Gain(1),
        points: vec![
            Point {
                tick: 0,
                value: 0.5,
            },
            Point {
                tick: 192,
                value: 0.1,
            },
        ],
    });
    song.audio_loop = Some(AudioLoop {
        samples: vec![0.1, -0.1, 0.2, -0.2],
        sample_rate: 48000,
        source_bpm: 116.0,
        name: "embedded.wav".into(),
    });
    let file = path("session.json");
    song.save(&file).unwrap();
    assert_eq!(Session::load(&file).unwrap(), song);
    std::fs::remove_file(file).unwrap();
}
#[test]
fn studio_rejects_malformed_sessions_and_preserves_live_song() {
    let good = Session::demo();
    let mut s = Studio::new(good.clone(), 48000).unwrap();
    let mut bad = good.clone();
    bad.bpm = f32::NAN;
    assert!(s.replace(bad).is_err());
    assert_eq!(s.session(), &good);
    let mut bad = good.clone();
    bad.automation.push(Lane {
        control: Control::Gain(99),
        points: vec![Point {
            tick: 0,
            value: 1.0,
        }],
    });
    assert!(bad.validate().is_err());
    let mut bad = good.clone();
    bad.chords[0].degree = 99;
    assert!(bad.validate().is_err());
    let mut bad = good.clone();
    bad.audio_loop = Some(AudioLoop {
        samples: vec![f32::INFINITY, 0.0],
        sample_rate: 48000,
        source_bpm: 116.0,
        name: "bad".into(),
    });
    assert!(bad.validate().is_err());
    let mut bad = good.clone();
    bad.automation.push(Lane {
        control: Control::MasterGain,
        points: vec![
            Point {
                tick: 100,
                value: 0.2,
            },
            Point {
                tick: 0,
                value: 0.3,
            },
        ],
    });
    assert!(bad.validate().is_err());
    assert!(s.set_control(Control::Gain(99), 0.5).is_err());
    assert!(s.render(&mut [0.0; 3]).is_err());
    assert!(s.set_step(5, 0, 0.5, 36).is_err());
    assert!(Studio::new(good, 0).is_err());
}
#[test]
fn studio_mute_solo_and_pan_really_change_audio() {
    let mut s = Studio::new(bass_song(), 48000).unwrap();
    s.play(true);
    let normal = render(&mut s, 48000);
    assert!(energy(&normal) > 1e-5);
    s.set_mute(1, true).unwrap();
    render(&mut s, 48000);
    assert!(energy(&render(&mut s, 48000)) < 1e-12);
    s.set_mute(1, false).unwrap();
    s.set_solo(0, true).unwrap();
    render(&mut s, 48000);
    assert!(energy(&render(&mut s, 48000)) < 1e-12);
    s.set_solo(0, false).unwrap();
    s.set_control(Control::Pan(1), 0.0).unwrap();
    render(&mut s, 48000);
    let panned = render(&mut s, 48000);
    assert!(energy(&panned) > 1e-6);
    assert!(panned.as_chunks::<2>().0.iter().all(|f| f[1].abs() < 1e-8));
}
#[test]
fn studio_automation_is_callback_size_independent_and_audible() {
    let mut song = bass_song();
    song.automation.push(Lane {
        control: Control::Gain(1),
        points: vec![
            Point {
                tick: 0,
                value: 0.7,
            },
            Point {
                tick: 192,
                value: 0.0,
            },
        ],
    });
    let mut a = Studio::new(song.clone(), 48000).unwrap();
    let mut b = Studio::new(song, 48000).unwrap();
    a.play(true);
    b.play(true);
    let whole = render(&mut a, 96000);
    let mut chunks = Vec::new();
    while chunks.len() < whole.len() {
        let n = 137.min((whole.len() - chunks.len()) / 2);
        chunks.extend(render(&mut b, n));
    }
    let error = whole
        .iter()
        .zip(&chunks)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f32, f32::max);
    assert!(error < 1e-5, "callback-dependent output: {error}");
    assert!(energy(&whole[..40000]) > 1e-5);
    assert!(energy(&whole[120000..180000]) < 1e-10);
}
#[test]
fn studio_records_slider_and_hit_and_replays_after_reload() {
    let mut s = Studio::new(Session::default(), 48000).unwrap();
    s.play(true);
    s.record(true);
    render(&mut s, 6000);
    s.set_control(Control::Gain(0), 0.2).unwrap();
    s.hit(INSTRUMENT_KICK, 36, 0.9).unwrap();
    s.record(false);
    let song = s.snapshot_session();
    assert_eq!(song.hits.len(), 1);
    assert!(song.hits[0].tick > 0);
    assert_eq!(song.automation.len(), 1);
    assert_eq!(song.automation[0].points.len(), 2);
    let mut replay = Studio::new(song, 48000).unwrap();
    replay.play(true);
    assert!(energy(&render(&mut replay, 48000)) > 1e-7);
}
#[test]
fn studio_existing_chord_recorder_captures_held_gate() {
    let mut s = Studio::new(Session::default(), 48000).unwrap();
    s.record(true);
    s.play(true);
    render(&mut s, 1024);
    assert!(s.chord_recording());
    s.chord_on(3, 0, true, 4, POLY_PRESET_PAD).unwrap();
    render(&mut s, 12000);
    s.chord_off();
    s.record(false);
    let song = s.snapshot_session();
    assert_eq!(song.chords.len(), 1);
    assert_eq!(song.chords[0].degree, 3);
    assert!(song.chords[0].duration > 20);
    let file = path("chord.json");
    song.save(&file).unwrap();
    let mut replay = Studio::new(Session::load(&file).unwrap(), 48000).unwrap();
    replay.play(true);
    assert!(energy(&render(&mut replay, 48000)) > 1e-6);
    std::fs::remove_file(file).unwrap();
}
#[test]
fn studio_embedded_audio_loop_imports_and_mixes() {
    let file = path("loop.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 48000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&file, spec).unwrap();
    for i in 0..4800 {
        w.write_sample(((i as f32 * 0.05).sin() * 10000.0) as i16)
            .unwrap();
    }
    w.finalize().unwrap();
    let mut song = Session::default();
    song.import_wav(&file).unwrap();
    assert_eq!(song.audio_loop.as_ref().unwrap().samples.len(), 9600);
    let mut s = Studio::new(song, 48000).unwrap();
    s.play(true);
    assert!(energy(&render(&mut s, 24000)) > 1e-4);
    s.set_mute(3, true).unwrap();
    render(&mut s, 24000);
    assert!(energy(&render(&mut s, 24000)) < 1e-12);
    std::fs::remove_file(file).unwrap();
}
#[test]
fn studio_snapshot_before_first_render_preserves_chord_clip() {
    let song = Session::demo();
    let mut s = Studio::new(song.clone(), 48000).unwrap();
    assert_eq!(s.snapshot_session().chords, song.chords);
}
#[test]
fn studio_offline_export_does_not_move_live_transport() {
    let mut s = Studio::new(Session::demo(), 48000).unwrap();
    s.play(true);
    render(&mut s, 2048);
    let tick = s.tick();
    let file = path("independent.wav");
    s.snapshot_session().export_wav(&file, 1, 0.0).unwrap();
    assert_eq!(s.tick(), tick);
    assert!(s.playing());
    std::fs::remove_file(file).unwrap();
}

#[test]
fn studio_filter_and_track_master_effect_controls_are_audible() {
    let mut song = Session::default();
    song.audio_loop = Some(AudioLoop {
        samples: (0..4800)
            .flat_map(|i| {
                let value = (i as f32 * std::f32::consts::TAU * 4000.0 / 48000.0).sin() * 0.2;
                [value, value]
            })
            .collect(),
        sample_rate: 48000,
        source_bpm: song.bpm,
        name: "test tone".into(),
    });
    let mut bright = Studio::new(song.clone(), 48000).unwrap();
    bright.play(true);
    let normal = render(&mut bright, 48000);
    song.strips[3].cutoff = 100.0;
    let mut dark = Studio::new(song, 48000).unwrap();
    dark.play(true);
    let filtered = render(&mut dark, 48000);
    assert!(energy(&filtered[10000..]) < energy(&normal[10000..]) * 0.05);
    let dry = bright.snapshot_session();
    for control in [
        Control::Delay(3),
        Control::Reverb(3),
        Control::MasterDelay,
        Control::MasterReverb,
    ] {
        let mut wet = Studio::new(dry.clone(), 48000).unwrap();
        wet.set_control(control, 0.7).unwrap();
        wet.play(true);
        let output = render(&mut wet, 48000);
        let difference: f64 = normal
            .iter()
            .zip(output)
            .map(|(a, b)| f64::from(*a - b).powi(2))
            .sum();
        assert!(difference > 0.01, "inaudible effect {control:?}");
    }
}

#[test]
fn studio_chord_tone_survives_preset_recall() {
    let mut bright = Session::default();
    bright.chords.push(ChordEvent {
        tick: 0,
        duration: 300,
        degree: 0,
        root: 0,
        minor: false,
        octave: 4,
        preset: POLY_PRESET_PAD,
        velocity: 0.8,
    });
    bright.chord_tone = 0.95;
    let mut dark = bright.clone();
    dark.chord_tone = 0.05;
    let mut a = Studio::new(bright, 48000).unwrap();
    let mut b = Studio::new(dark, 48000).unwrap();
    a.play(true);
    b.play(true);
    let high = render(&mut a, 48000);
    let low = render(&mut b, 48000);
    let error: f64 = high
        .iter()
        .zip(low)
        .map(|(a, b)| f64::from(*a - b).powi(2))
        .sum();
    assert!(error > 0.1, "preset recall lost synth tone");
}

#[test]
fn studio_arm_mid_loop_waits_for_boundary_and_transport_freezes_when_stopped() {
    let mut s = Studio::new(Session::default(), 48000).unwrap();
    s.play(true);
    render(&mut s, 12000);
    s.record(true);
    render(&mut s, 12000);
    assert!(!s.chord_recording());
    render(&mut s, 90000);
    assert!(s.chord_recording());
    s.play(false);
    let tick = s.tick();
    render(&mut s, 48000);
    assert_eq!(s.tick(), tick);
    s.record(false);
    s.rewind().unwrap();
    assert_eq!(s.tick(), 0);
    assert!(!s.playing());
}

#[test]
fn studio_mute_automation_roundtrips_and_replays() {
    let mut s = Studio::new(bass_song(), 48000).unwrap();
    s.play(true);
    s.record(true);
    render(&mut s, 48000);
    s.set_mute(1, true).unwrap();
    s.record(false);
    let song = s.snapshot_session();
    assert_eq!(song.automation[0].control, Control::Mute(1));
    let mut replay = Studio::new(song, 48000).unwrap();
    replay.play(true);
    let audio = render(&mut replay, 96000);
    assert!(energy(&audio[..48000]) > 1e-5);
    assert!(energy(&audio[120000..180000]) < 1e-12);
}

#[test]
fn studio_overlapping_chords_and_duplicate_lanes_fail_before_engine_allocation() {
    let mut song = Session::demo();
    let first = song.chords[0].clone();
    song.chords.push(first);
    assert!(song.validate().is_err());
    let mut song = Session::default();
    let lane = Lane {
        control: Control::MasterGain,
        points: vec![Point {
            tick: 0,
            value: 0.5,
        }],
    };
    song.automation.extend([lane.clone(), lane]);
    assert!(song.validate().is_err());
}

#[test]
fn studio_play_is_idempotent_and_stop_play_recues_every_lane() {
    let song = Session::demo();
    let mut reference = Studio::new(song.clone(), 44100).unwrap();
    let mut actual = Studio::new(song, 44100).unwrap();
    reference.play(true);
    actual.play(true);
    let first = render(&mut reference, 1777);
    assert_eq!(first, render(&mut actual, 1777));
    actual.play(true);
    assert_eq!(
        render(&mut reference, 1777),
        render(&mut actual, 1777),
        "repeated Play retriggered sources"
    );
    let cursor = actual.position_beats();
    actual.record(true);
    actual.play(false);
    actual.play(false);
    assert!(!actual.recording());
    render(&mut actual, 1000);
    assert_eq!(actual.position_beats(), cursor);
    actual.play(true);
    assert_eq!(actual.tick(), 0);
    assert_eq!(actual.position_beats(), 0.0);
    render(&mut actual, 1777);
    assert!((actual.position_beats() - 1777.0 * 116.0 / (60.0 * 44100.0)).abs() < 1e-12);
}

#[test]
fn studio_tempo_change_recues_and_same_tempo_is_idempotent_at_non_48k_rate() {
    for rate in [8000, 44100, 96000] {
        let mut studio = Studio::new(bass_song(), rate).unwrap();
        studio.play(true);
        render(&mut studio, 1777);
        let position = studio.position_beats();
        studio.set_tempo(116.0).unwrap();
        assert_eq!(studio.position_beats(), position);
        studio.set_tempo(137.0).unwrap();
        assert!(studio.playing());
        assert_eq!(studio.position_beats(), 0.0);
        let audio = render(&mut studio, 1777);
        assert!(audio.iter().all(|s| s.is_finite()));
        assert!((studio.position_beats() - 1777.0 * 137.0 / (60.0 * rate as f64)).abs() < 1e-12);
    }
}

#[test]
fn studio_multilane_master_and_track_automation_is_callback_invariant() {
    let mut song = bass_song();
    song.bpm = 137.0;
    song.automation = vec![
        Lane {
            control: Control::Gain(1),
            points: vec![
                Point {
                    tick: 0,
                    value: 0.3,
                },
                Point {
                    tick: 96,
                    value: 0.7,
                },
            ],
        },
        Lane {
            control: Control::MasterGain,
            points: vec![
                Point {
                    tick: 0,
                    value: 0.6,
                },
                Point {
                    tick: 192,
                    value: 0.0,
                },
            ],
        },
        Lane {
            control: Control::Pan(1),
            points: vec![
                Point {
                    tick: 0,
                    value: 0.0,
                },
                Point {
                    tick: 288,
                    value: 1.0,
                },
            ],
        },
    ];
    let mut a = Studio::new(song.clone(), 44100).unwrap();
    let mut b = Studio::new(song, 44100).unwrap();
    a.play(true);
    b.play(true);
    let full = render(&mut a, 100000);
    let mut split = Vec::new();
    for size in [1, 137, 2048, 79].into_iter().cycle() {
        let n = size.min((full.len() - split.len()) / 2);
        if n == 0 {
            break;
        }
        split.extend(render(&mut b, n));
    }
    assert_eq!(full, split);
    assert!(energy(&full[100000..140000]) < 1e-10);
    assert_eq!(
        a.session().value(Control::MasterGain),
        b.session().value(Control::MasterGain)
    );
    assert_eq!(a.tick(), b.tick());
}

#[test]
fn studio_more_than_one_thousand_bars_keep_integer_frame_clock_and_downbeat() {
    // Fractional samples per step expose the old f32 trigger counter's
    // cumulative rounding drift; 240 BPM would make every boundary integral.
    let song = Session {
        bpm: 237.0,
        ..Session::default()
    };
    let mut studio = Studio::new(song, 8000).unwrap();
    studio.play(true);
    let mut buffer = [0.0; 2048];
    let total_frames = (1001.0_f64 * 240.0 / 237.0 * 8000.0).ceil() as usize;
    let mut rendered = 0;
    for size in [137, 511, 1024].into_iter().cycle() {
        let n = size.min(total_frames - rendered);
        if n == 0 {
            break;
        }
        studio.render(&mut buffer[..n * 2]).unwrap();
        assert!(buffer[..n * 2].iter().all(|s| s.is_finite()));
        rendered += n;
    }
    assert!(studio.position_beats() >= 4004.0);
    assert!(studio.position_beats() - 4004.0 < 237.0 / (60.0 * 8000.0));
    assert_eq!(studio.tick(), 0);
    studio.set_step(0, 0, 0.9, 36).unwrap();
    assert!(
        energy(&render(&mut studio, 512)) > 1e-6,
        "downbeat drifted after 1001 bars"
    );
}

#[test]
fn studio_repeated_fresh_mixdowns_are_sample_identical_in_this_build() {
    let song = Session::demo();
    let first = path("repeat-a.wav");
    let second = path("repeat-b.wav");
    song.export_wav(&first, 2, 0.25).unwrap();
    song.export_wav(&second, 2, 0.25).unwrap();
    assert_eq!(
        std::fs::read(&first).unwrap(),
        std::fs::read(&second).unwrap()
    );
    std::fs::remove_file(first).unwrap();
    std::fs::remove_file(second).unwrap();
}

#[test]
fn studio_rejects_hostile_wav_rate_and_lengths_before_reading_payload() {
    // Tiny hand-built WAV with a hostile rate or oversized declared data. A
    // complete header suffices: validation must reject before sample iteration.
    fn header(rate: u32, bytes: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(b"RIFF");
        out.extend(bytes.saturating_add(36).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16_u32.to_le_bytes());
        out.extend(1_u16.to_le_bytes());
        out.extend(1_u16.to_le_bytes());
        out.extend(rate.to_le_bytes());
        out.extend(rate.saturating_mul(2).to_le_bytes());
        out.extend(2_u16.to_le_bytes());
        out.extend(16_u16.to_le_bytes());
        out.extend(b"data");
        out.extend(bytes.to_le_bytes());
        out
    }
    let original = Session::default();
    for (rate, bytes) in [
        (u32::MAX, 2),
        (0, 2),
        (8000, 1_920_002),
        (192000, 11_520_002),
    ] {
        let file = path("hostile.wav");
        std::fs::write(&file, header(rate, bytes)).unwrap();
        let mut song = original.clone();
        assert!(song.import_wav(&file).is_err());
        assert_eq!(song, original);
        std::fs::remove_file(file).unwrap();
    }
    let mut too_long = original;
    too_long.audio_loop = Some(AudioLoop {
        samples: vec![0.0; 8000 * 120 * 2 + 2],
        sample_rate: 8000,
        source_bpm: 116.0,
        name: "overlength".into(),
    });
    assert!(too_long.validate().is_err());
}

#[test]
fn studio_demo_held_overdub_crossing_original_starts_saves_exports_and_reloads() {
    let mut studio = Studio::new(Session::demo(), 48000).unwrap();
    studio.record(true);
    studio.play(true);
    render(&mut studio, 2000);
    let start = studio.tick();
    studio.chord_on(2, 0, true, 4, POLY_PRESET_PAD).unwrap();
    render(&mut studio, 50000);
    studio.chord_off();
    studio.record(false);
    let song = studio.snapshot_session();
    song.validate().unwrap();
    let take = song
        .chords
        .iter()
        .find(|e| e.degree == 2 && e.tick == start)
        .expect("latest held take was lost");
    assert!(take.duration > 190);
    assert!(
        !song.chords.iter().any(|e| e.tick == 96),
        "covered original note-on survived latest take"
    );
    let file = path("overdub.json");
    let wav = path("overdub.wav");
    song.save(&file).unwrap();
    let loaded = Session::load(&file).unwrap();
    assert_eq!(song, loaded);
    assert!(loaded.export_wav(&wav, 1, 0.0).unwrap().rms > 0.01);
    render(&mut studio, 1000);
    assert_eq!(
        studio.snapshot_session().chords,
        song.chords,
        "live replay did not install persisted canonical gates"
    );
    std::fs::remove_file(file).unwrap();
    std::fs::remove_file(wav).unwrap();
}

#[test]
fn studio_demo_same_tick_and_wrapping_overdubs_keep_the_newest_take() {
    let mut studio = Studio::new(Session::demo(), 48000).unwrap();
    studio.record(true);
    studio.play(true);
    render(&mut studio, 1);
    studio.chord_on(1, 0, true, 4, POLY_PRESET_PAD).unwrap();
    studio.chord_off();
    studio.chord_on(6, 0, true, 4, POLY_PRESET_PAD).unwrap();
    render(&mut studio, 3000);
    studio.chord_off();
    studio.record(false);
    let song = studio.snapshot_session();
    song.validate().unwrap();
    assert!(song.chords.iter().any(|e| e.tick == 0 && e.degree == 6));
    assert!(!song.chords.iter().any(|e| e.degree == 1));
    render(&mut studio, 512);
    studio.rewind().unwrap();
    studio.record(true);
    render(&mut studio, 85000);
    let start = studio.tick();
    assert!(start > 300);
    studio.chord_on(2, 0, true, 4, POLY_PRESET_PAD).unwrap();
    render(&mut studio, 22000);
    studio.chord_off();
    studio.record(false);
    let song = studio.snapshot_session();
    song.validate().unwrap();
    let wrapping = song
        .chords
        .iter()
        .find(|e| e.degree == 2 && e.tick == start)
        .expect("wrapping take lost");
    assert!(wrapping.tick + wrapping.duration > LOOP_TICKS);
    let file = path("wrap-overdub.json");
    let wav = path("wrap-overdub.wav");
    song.save(&file).unwrap();
    let loaded = Session::load(&file).unwrap();
    assert_eq!(loaded, song);
    assert!(loaded.export_wav(&wav, 1, 0.0).unwrap().rms > 0.01);
    render(&mut studio, 512);
    assert_eq!(studio.snapshot_session().chords, song.chords);
    std::fs::remove_file(file).unwrap();
    std::fs::remove_file(wav).unwrap();
}

#[test]
fn studio_bar_reanchor_preserves_multibar_pcm_phrase_position() {
    let song = Session {
        bpm: 240.0,
        audio_loop: Some(AudioLoop {
            samples: (0..16000)
                .flat_map(|frame| {
                    let value = if frame < 8000 { 0.1 } else { 0.2 };
                    [value, value]
                })
                .collect(),
            sample_rate: 8000,
            source_bpm: 240.0,
            name: "two-bar phrase".into(),
        }),
        ..Session::default()
    };
    let mut studio = Studio::new(song, 8000).unwrap();
    studio.play(true);
    let first = render(&mut studio, 8000);
    let second = render(&mut studio, 8000);
    let mean =
        |audio: &[f32]| audio.iter().map(|v| v.abs() as f64).sum::<f64>() / audio.len() as f64;
    assert!(
        mean(&second[4000..12000]) > mean(&first[4000..12000]) * 1.7,
        "bar reanchor rewound the two-bar PCM phrase: first {} second {}",
        mean(&first[4000..12000]),
        mean(&second[4000..12000])
    );
    studio.play(false);
    render(&mut studio, 2000);
    studio.play(true);
    let restarted = render(&mut studio, 8000);
    assert!(
        (mean(&restarted[4000..12000]) - mean(&first[4000..12000])).abs() < 1e-5,
        "Stop/Play left PCM at its old phase"
    );
}
