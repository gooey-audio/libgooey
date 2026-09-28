use gooey::engine::{Instrument, Modulatable};
use gooey::instruments::{TwinCorePercConfig, TwinCorePercNoiseMode, TwinCorePercVoice};

const SAMPLE_RATE: f32 = 48_000.0;

fn render(config: TwinCorePercConfig, seconds: f32) -> Vec<f32> {
    let mut voice = TwinCorePercVoice::with_config(SAMPLE_RATE, config);
    voice.trigger_with_velocity(0.0, 0.8);
    (0..(seconds * SAMPLE_RATE) as usize)
        .map(|index| voice.tick(index as f64 / SAMPLE_RATE as f64))
        .collect()
}

fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|sample| sample * sample).sum()
}

#[test]
fn factory_presets_are_finite_audible_and_distinct() {
    let presets = [
        TwinCorePercConfig::kick(),
        TwinCorePercConfig::tom(),
        TwinCorePercConfig::snare(),
        TwinCorePercConfig::clap(),
        TwinCorePercConfig::metallic(),
    ];
    let renders: Vec<_> = presets
        .into_iter()
        .map(|preset| render(preset, 1.0))
        .collect();
    for samples in &renders {
        assert!(samples.iter().all(|sample| sample.is_finite()));
        assert!(energy(samples) > 1.0e-5);
        assert!(samples.iter().all(|sample| sample.abs() <= 4.0));
    }
    for pair in renders.windows(2) {
        let difference: f32 = pair[0]
            .iter()
            .zip(&pair[1])
            .map(|(left, right)| (left - right).abs())
            .sum();
        assert!(difference > 0.1);
    }
}

#[test]
fn trigger_delay_moves_only_the_body_onset() {
    let mut config = TwinCorePercConfig::kick();
    config.trigger_delay_seconds = 0.04;
    config.noise_bias = 0.4;
    config.noise_decay_seconds = 0.1;
    let samples = render(config, 0.1);
    let split = (0.04 * SAMPLE_RATE) as usize;
    assert!(
        energy(&samples[..split]) > 1.0e-5,
        "noise should start immediately"
    );
    let before = energy(&samples[split - 100..split]);
    let after = energy(&samples[split..split + 100]);
    assert!(after > before * 1.2, "delayed body should add impact");
}

#[test]
fn body_noise_routing_differs_from_direct_noise() {
    let mut direct = TwinCorePercConfig::snare();
    direct.noise_mode = TwinCorePercNoiseMode::Lowpass;
    let mut routed = direct;
    routed.noise_mode = TwinCorePercNoiseMode::Body;
    let direct = render(direct, 0.5);
    let routed = render(routed, 0.5);
    let difference: f32 = direct
        .iter()
        .zip(routed)
        .map(|(left, right)| (left - right).abs())
        .sum();
    assert!(difference > 1.0);
}

#[test]
fn independent_noise_trigger_does_not_require_a_body_trigger() {
    let mut config = TwinCorePercConfig::snare();
    config.noise_mode = TwinCorePercNoiseMode::Highpass;
    let mut voice = TwinCorePercVoice::with_config(SAMPLE_RATE, config);
    voice.trigger_noise_with_velocity(0.0, 0.7);
    let samples: Vec<_> = (0..4_800)
        .map(|index| voice.tick(index as f64 / SAMPLE_RATE as f64))
        .collect();
    assert!(energy(&samples) > 1.0e-5);
    assert!(samples.iter().all(|sample| sample.is_finite()));
}

#[test]
fn seed_and_modulation_are_deterministic_and_sanitized() {
    let config = TwinCorePercConfig::snare();
    assert_eq!(render(config, 0.2), render(config, 0.2));

    let mut voice = TwinCorePercVoice::with_config(SAMPLE_RATE, config);
    assert_eq!(voice.parameter_range("harmonics"), Some((0.0, 1.0)));
    voice.apply_modulation("harmonics", f32::NAN).unwrap();
    voice.trigger_with_velocity(0.0, 1.0);
    for index in 0..48_000 {
        assert!(voice.tick(index as f64 / SAMPLE_RATE as f64).is_finite());
    }
    assert!(voice.apply_modulation("not-a-control", 0.5).is_err());
}

#[test]
fn ring_limit_stops_long_metallic_patch() {
    let mut voice = TwinCorePercVoice::with_config(SAMPLE_RATE, TwinCorePercConfig::metallic());
    voice.set_ring_limit_secs(Some(0.05));
    voice.trigger_with_velocity(0.0, 1.0);
    for index in 0..3_000 {
        voice.tick(index as f64 / SAMPLE_RATE as f64);
    }
    assert!(!voice.is_active());
}
