//! Symmetry, cross mod, low-pass gate and chaos on the Twin Core voice.

use gooey::engine::Instrument;
use gooey::instruments::{TwinCorePercConfig, TwinCorePercVoice};

const SAMPLE_RATE: f32 = 44_100.0;

fn render_voice(voice: &mut TwinCorePercVoice, velocity: f32, seconds: f32) -> Vec<f32> {
    voice.trigger_with_velocity(0.0, velocity);
    (0..(seconds * SAMPLE_RATE) as usize)
        .map(|index| voice.tick(index as f64 / SAMPLE_RATE as f64))
        .collect()
}

fn render(config: TwinCorePercConfig, velocity: f32, seconds: f32) -> Vec<f32> {
    render_voice(
        &mut TwinCorePercVoice::with_config(SAMPLE_RATE, config),
        velocity,
        seconds,
    )
}

fn window(samples: &[f32], from: f32, to: f32) -> &[f32] {
    &samples[(from * SAMPLE_RATE) as usize..(to * SAMPLE_RATE) as usize]
}

fn mean_square(samples: &[f32]) -> f32 {
    samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32
}

fn goertzel_power(samples: &[f32], hz: f32) -> f32 {
    let coeff = 2.0 * (std::f32::consts::TAU * hz / SAMPLE_RATE).cos();
    let (mut s1, mut s2) = (0.0_f32, 0.0_f32);
    for &x in samples {
        let s0 = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - coeff * s1 * s2) / samples.len() as f32
}

/// First-difference energy over signal energy; rises with brightness.
fn brightness(samples: &[f32]) -> f32 {
    let diff: f32 = samples.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
    diff / samples.iter().map(|s| s * s).sum::<f32>().max(1.0e-12)
}

fn zero_crossings(samples: &[f32]) -> usize {
    samples
        .windows(2)
        .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
        .count()
}

/// A plain 200 Hz ring: unison cores, no pitch sweep, no noise.
fn pure_tone() -> TwinCorePercConfig {
    TwinCorePercConfig {
        master_tune_hz: 200.0,
        detune_octaves: 0.0,
        length_seconds: 1.0,
        body_bias: 0.0,
        fm_depth_octaves: 0.0,
        noise_bias: -1.0,
        ..TwinCorePercConfig::kick()
    }
}

#[test]
fn neutral_extensions_leave_the_presets_unchanged() {
    // Energies of a 1.5 s, velocity 0.9 hit, captured before the four
    // extension parameters existed.
    let reference = [
        (TwinCorePercConfig::kick(), 1.170870408e3),
        (TwinCorePercConfig::tom(), 2.094036316e1),
        (TwinCorePercConfig::snare(), 3.409641714e1),
        (TwinCorePercConfig::clap(), 7.548698322e0),
        (TwinCorePercConfig::metallic(), 2.564043036e2),
    ];
    for (config, expected) in reference {
        assert_eq!(
            (config.symmetry, config.cross_mod, config.lpg, config.chaos),
            (0.0, 0.0, 0.0, 0.0)
        );
        let energy: f64 = render(config, 0.9, 1.5)
            .iter()
            .map(|s| (*s as f64).powi(2))
            .sum();
        assert!(
            ((energy - expected) / expected).abs() < 1.0e-4,
            "energy {energy} vs {expected}"
        );
    }
}

#[test]
fn symmetry_adds_even_harmonics_without_dc() {
    let second_harmonic = |symmetry| {
        let out = render(
            TwinCorePercConfig {
                harmonics: 0.6,
                symmetry,
                ..pure_tone()
            },
            1.0,
            0.5,
        );
        let w = window(&out, 0.05, 0.4);
        let mean = w.iter().sum::<f32>() / w.len() as f32;
        assert!(mean.abs() < 0.01, "symmetry {symmetry} dc {mean}");
        goertzel_power(w, 400.0) / goertzel_power(w, 200.0)
    };
    let (flat, half, full) = (
        second_harmonic(0.0),
        second_harmonic(0.5),
        second_harmonic(1.0),
    );
    assert!(flat < 1.0e-3, "flat {flat}");
    assert!(half > flat * 10.0 && full > half, "{flat} {half} {full}");
}

#[test]
fn symmetry_needs_harmonics_to_be_heard() {
    let clean = TwinCorePercConfig {
        harmonics: 0.0,
        ..pure_tone()
    };
    let dry = render(clean, 1.0, 0.3);
    let shifted = render(
        TwinCorePercConfig {
            symmetry: 1.0,
            ..clean
        },
        1.0,
        0.3,
    );
    let diff = dry
        .iter()
        .zip(&shifted)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f32, f32::max);
    assert!(diff < 1.0e-3, "max diff {diff}");
}

#[test]
fn cross_mod_spreads_energy_away_from_the_core_pitches() {
    for pitch in [52.0, 200.0, 800.0] {
        let tonal_share = |cross_mod| {
            let out = render(
                TwinCorePercConfig {
                    master_tune_hz: pitch,
                    detune_octaves: 0.7,
                    cross_mod,
                    ..pure_tone()
                },
                1.0,
                0.5,
            );
            let w = window(&out, 0.05, 0.4);
            let f2 = pitch * 2.0_f32.powf(-0.7);
            (goertzel_power(w, pitch) + goertzel_power(w, f2)) / mean_square(w)
        };
        let clean = tonal_share(0.0);
        let clang = tonal_share(1.0);
        assert!(clang < clean * 0.6, "{pitch} Hz: {clean} -> {clang}");
    }
}

#[test]
fn lpg_darkens_the_tail_but_not_the_strike() {
    let measure = |lpg| {
        let out = render(
            TwinCorePercConfig {
                harmonics: 0.5,
                lpg,
                length_seconds: 1.5,
                ..pure_tone()
            },
            1.0,
            1.0,
        );
        (
            brightness(window(&out, 0.0, 0.1)),
            brightness(window(&out, 0.4, 0.6)),
            mean_square(window(&out, 0.0, 0.1)),
        )
    };
    let (vca_early, vca_mid, vca_level) = measure(0.0);
    let (lpg_early, lpg_mid, lpg_level) = measure(1.0);
    assert!(
        (lpg_early / vca_early - 1.0).abs() < 0.05,
        "{vca_early} {lpg_early}"
    );
    assert!(
        (lpg_level / vca_level - 1.0).abs() < 0.05,
        "{vca_level} {lpg_level}"
    );
    assert!(lpg_mid < vca_mid * 0.5, "{vca_mid} {lpg_mid}");
}

#[test]
fn chaos_stiffness_makes_hard_hits_glide_down() {
    let glide = |velocity| {
        let out = render(
            TwinCorePercConfig {
                detune_octaves: 0.3,
                chaos: 0.5,
                length_seconds: 2.0,
                ..pure_tone()
            },
            velocity,
            1.0,
        );
        zero_crossings(window(&out, 0.005, 0.105)) as f32
            / zero_crossings(window(&out, 0.6, 0.7)).max(1) as f32
    };
    let (soft, hard) = (glide(0.3), glide(1.0));
    assert!(hard > soft + 0.15, "soft {soft} hard {hard}");
}

#[test]
fn chaos_feedback_changes_the_body_and_still_ends() {
    let config = TwinCorePercConfig {
        detune_octaves: 0.3,
        chaos: 1.0,
        length_seconds: 2.0,
        ..pure_tone()
    };
    let calm = render(
        TwinCorePercConfig {
            chaos: 0.5,
            ..config
        },
        1.0,
        0.7,
    );
    let wild = render(config, 1.0, 0.7);
    let calm_pitch = zero_crossings(window(&calm, 0.5, 0.7));
    let wild_pitch = zero_crossings(window(&wild, 0.5, 0.7));
    assert!(wild_pitch > calm_pitch + 5, "{calm_pitch} {wild_pitch}");

    let mut voice = TwinCorePercVoice::with_config(SAMPLE_RATE, config);
    let out = render_voice(&mut voice, 1.0, 8.0);
    assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 4.0));
    assert!(!voice.is_active());
}

#[test]
fn every_extension_at_full_stays_bounded_and_finishes() {
    for preset in [
        TwinCorePercConfig::kick(),
        TwinCorePercConfig::snare(),
        TwinCorePercConfig::metallic(),
    ] {
        let config = TwinCorePercConfig {
            harmonics: 1.0,
            symmetry: -1.0,
            cross_mod: 1.0,
            lpg: 1.0,
            chaos: 1.0,
            length_seconds: 4.0,
            body_bias: 0.0,
            ..preset
        };
        let mut voice = TwinCorePercVoice::with_config(SAMPLE_RATE, config);
        let out = render_voice(&mut voice, 1.0, 20.0);
        assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 4.0));
        assert!(mean_square(&out) > 1.0e-6);
        assert!(!voice.is_active(), "voice still ringing after 20 s");
    }
}

#[test]
fn extension_parameters_round_trip_by_index_and_name() {
    let mut voice = TwinCorePercVoice::new(SAMPLE_RATE);
    for (index, name) in [
        (12, "symmetry"),
        (13, "cross_mod"),
        (14, "lpg"),
        (15, "chaos"),
    ] {
        voice.set_parameter_normalized(index, 0.25);
        assert!((voice.parameter_normalized(index).unwrap() - 0.25).abs() < 1.0e-6);
        gooey::engine::Modulatable::apply_modulation(&mut voice, name, 0.75).unwrap();
        assert!((voice.parameter_normalized(index).unwrap() - 0.75).abs() < 1.0e-6);
    }
    assert!(voice.parameter_normalized(16).is_none());
    let config = voice.config_targets();
    assert!((config.symmetry - 0.5).abs() < 1.0e-6);
    assert!((config.chaos - 0.75).abs() < 1.0e-6);
}
