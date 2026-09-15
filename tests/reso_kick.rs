use gooey::instruments::{ResoKick, ResoKickConfig};

const SAMPLE_RATE: f32 = 48_000.0;

fn render(config: ResoKickConfig, velocity: f32, seconds: f32) -> Vec<f32> {
    let mut kick = ResoKick::with_config(SAMPLE_RATE, config);
    kick.trigger_with_velocity(0.0, velocity);
    (0..(seconds * SAMPLE_RATE) as usize)
        .map(|sample| kick.tick(sample as f64 / SAMPLE_RATE as f64))
        .collect()
}

fn rms(samples: &[f32]) -> f64 {
    (samples
        .iter()
        .map(|&sample| (sample as f64).powi(2))
        .sum::<f64>()
        / samples.len().max(1) as f64)
        .sqrt()
}

#[test]
fn stable_at_max_everything_for_ten_seconds() {
    let config = ResoKickConfig {
        frequency: 1.0,
        depth: 1.0,
        pitch_decay: 1.0,
        resonate: 1.0,
        punch: 1.0,
        character: 1.0,
        ripple: 1.0,
        exciter_noise: 1.0,
        volume: 1.0,
    };
    for sample in render(config, 1.0, 10.0) {
        assert!(sample.is_finite());
        assert!(sample.abs() < 2.0, "sample={sample}");
    }
}

#[test]
fn every_preset_is_finite_and_audible() {
    let presets = [
        ResoKickConfig::classic808(),
        ResoKickConfig::punch909(),
        ResoKickConfig::soft_bounce(),
        ResoKickConfig::tom(),
        ResoKickConfig::laser(),
        ResoKickConfig::sub_drone(),
    ];
    for (index, preset) in presets.into_iter().enumerate() {
        let output = render(preset, 0.8, 1.0);
        let peak = output
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        assert!(
            output.iter().all(|sample| sample.is_finite()),
            "preset={index}"
        );
        assert!(peak > 1.0e-3, "preset={index}, peak={peak}");
    }
}

#[test]
fn resonate_sets_duration_monotonically() {
    let audible_duration = |resonate| {
        let mut config = ResoKickConfig::classic808();
        config.resonate = resonate;
        let output = render(config, 0.8, 2.0);
        output
            .iter()
            .rposition(|sample| sample.abs() > 1.0e-5)
            .unwrap_or_default() as f32
            / SAMPLE_RATE
    };

    let short = audible_duration(0.2);
    let medium = audible_duration(0.55);
    let long = audible_duration(0.8);
    assert!(short < medium && medium < long, "{short}, {medium}, {long}");
}

#[test]
fn pitch_envelope_reaches_base() {
    let mut config = ResoKickConfig::classic808();
    config.depth = 1.0;
    config.pitch_decay = 0.75;
    config.ripple = 0.0;
    config.resonate = 0.7;
    let output = render(config, 0.8, 0.8);

    let crossing_rate = |start: usize, end: usize| {
        let crossings = output[start..end]
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        crossings as f32 / ((end - start) as f32 / SAMPLE_RATE)
    };
    let early = crossing_rate(1_000, 4_000);
    let late = crossing_rate(24_000, 36_000);
    assert!(early > late * 1.5, "early={early}, late={late}");
}

#[test]
fn ring_limit_stops_a_drone() {
    let mut kick = ResoKick::with_config(SAMPLE_RATE, ResoKickConfig::sub_drone());
    kick.set_ring_limit_secs(Some(0.1));
    kick.trigger_with_velocity(0.0, 1.0);
    for sample in 0..4_800 {
        let _ = kick.tick(sample as f64 / SAMPLE_RATE as f64);
    }
    assert_eq!(kick.tick(0.101), 0.0);
    assert!(!kick.is_active());
}

#[test]
fn velocity_scales_level_monotonically() {
    let energy = |velocity| rms(&render(ResoKickConfig::classic808(), velocity, 0.5));
    let soft = energy(0.25);
    let medium = energy(0.5);
    let hard = energy(1.0);
    assert!(soft < medium && medium < hard, "{soft}, {medium}, {hard}");
}

#[test]
fn max_resonate_self_oscillates() {
    let output = render(ResoKickConfig::sub_drone(), 1.0, 9.0);
    let tail = rms(&output[8 * 48_000..]);
    assert!(tail > 0.01, "tail rms={tail}");
}
