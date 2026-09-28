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

fn mean(samples: &[f32]) -> f64 {
    samples.iter().map(|&sample| sample as f64).sum::<f64>() / samples.len().max(1) as f64
}

fn first_difference_rms(samples: &[f32]) -> f64 {
    let differences = samples.windows(2).map(|pair| (pair[1] - pair[0]) as f64);
    let count = samples.len().saturating_sub(1).max(1);
    (differences.map(|sample| sample * sample).sum::<f64>() / count as f64).sqrt()
}

#[test]
fn legacy_presets_keep_their_deterministic_render() {
    let presets = [
        ResoKickConfig::classic808(),
        ResoKickConfig::punch909(),
        ResoKickConfig::soft_bounce(),
        ResoKickConfig::tom(),
        ResoKickConfig::laser(),
        ResoKickConfig::sub_drone(),
    ];
    // Raw f32-bit hashes are not portable across architectures because libm
    // implementations may round intermediate transcendental results differently.
    // These samples cover the attack, body, and tail while allowing only the
    // explicitly documented maximum cross-platform error of 1e-7.
    const SAMPLE_INDICES: [usize; 8] = [1, 8, 64, 257, 1_024, 4_096, 12_000, 23_999];
    const EXPECTED: [[f32; 8]; 6] = [
        [
            3.655425e-5,
            0.030273503,
            0.6922764,
            -0.8284299,
            -0.66958857,
            0.25257415,
            -0.12698698,
            -0.11504954,
        ],
        [
            0.00021404584,
            0.22682175,
            0.6059649,
            0.5998171,
            -0.6060858,
            -0.4617524,
            -0.022315413,
            0.0005306672,
        ],
        [
            2.8332466e-5,
            0.021878693,
            0.51704097,
            0.1352375,
            -0.09335334,
            -0.50820714,
            0.124641374,
            -0.015998457,
        ],
        [
            9.1014146e-5,
            0.08351501,
            0.64778435,
            0.4205897,
            0.63300973,
            -0.6283577,
            0.33210063,
            -0.26136887,
        ],
        [
            5.0452018e-5,
            0.06528428,
            -0.10985839,
            -0.5286398,
            -0.0903473,
            0.57981676,
            -0.09935882,
            0.3061434,
        ],
        [
            3.6695998e-5,
            0.025908226,
            0.3933043,
            0.29188013,
            -0.20135984,
            -0.110243924,
            -0.5727104,
            -0.4914294,
        ],
    ];

    for (preset_index, (preset, expected)) in presets.into_iter().zip(EXPECTED).enumerate() {
        let samples = render(preset, 0.75, 0.5);
        for (sample_index, expected_sample) in SAMPLE_INDICES.into_iter().zip(expected) {
            let actual = samples[sample_index];
            let error = (actual - expected_sample).abs();
            assert!(
                error <= 1.0e-7,
                "preset={preset_index}, sample={sample_index}, expected={expected_sample}, actual={actual}, error={error}"
            );
        }
    }
}

fn positive_crossing_rate(samples: &[f32]) -> f32 {
    let crossings = samples
        .windows(2)
        .enumerate()
        .filter_map(|(index, pair)| (pair[0] <= 0.0 && pair[1] > 0.0).then_some(index))
        .collect::<Vec<_>>();
    if crossings.len() < 2 {
        return 0.0;
    }
    let body_periods = crossings
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .filter(|&period| period <= (SAMPLE_RATE / 20.0) as usize)
        .collect::<Vec<_>>();
    if body_periods.is_empty() {
        return 0.0;
    }
    body_periods.len() as f32 * SAMPLE_RATE / body_periods.iter().sum::<usize>() as f32
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
        assert!(sample.abs() < 3.0, "sample={sample}");
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
fn max_resonate_rings_for_seconds_without_dc() {
    let output = render(ResoKickConfig::sub_drone(), 1.0, 9.0);
    let tail = &output[8 * 48_000..];
    let tail_rms = rms(tail);
    let tail_mean = mean(tail);
    let last_audible = output
        .iter()
        .rposition(|sample| sample.abs() > 1.0e-5)
        .unwrap_or_default() as f32
        / SAMPLE_RATE;
    assert!(
        tail_rms > 0.01,
        "tail rms={tail_rms}, mean={tail_mean}, last_audible={last_audible}s"
    );
    assert!(tail_mean.abs() < 0.005, "tail mean={tail_mean}");
}

#[test]
fn body_sustains_after_the_initial_peak() {
    let output = render(ResoKickConfig::classic808(), 0.75, 0.3);
    let peak = output
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs())) as f64;
    let body_rms = rms(&output[4_800..14_400]);
    let relative_db = 20.0 * (body_rms / peak).log10();

    assert!(
        relative_db >= -15.0,
        "body rms={body_rms}, peak={peak}, relative={relative_db:.2} dB"
    );
}

#[test]
fn resonate_does_not_detune_the_tail() {
    let tail_pitch = |resonate| {
        let mut config = ResoKickConfig::classic808();
        config.depth = 0.0;
        config.resonate = resonate;
        config.punch = 0.0;
        config.character = 0.28;
        config.ripple = 0.0;
        config.exciter_noise = 0.0;
        if resonate < 0.5 {
            let output = render(config, 0.75, 0.2);
            positive_crossing_rate(&output[2_400..9_600])
        } else {
            let output = render(config, 0.75, 3.0);
            positive_crossing_rate(&output[96_000..144_000])
        }
    };

    let low = tail_pitch(0.3);
    let high = tail_pitch(0.8);
    let relative_difference = (high - low).abs() / low.max(high);
    assert!(
        relative_difference <= 0.05,
        "low={low} Hz, high={high} Hz, difference={relative_difference:.3}"
    );
}

#[test]
fn character_brightens_the_voice() {
    let brightness = |character| {
        let mut config = ResoKickConfig::classic808();
        config.character = character;
        config.exciter_noise = 1.0;
        first_difference_rms(&render(config, 0.75, 0.05))
    };

    let dark = brightness(0.2);
    let bright = brightness(0.8);
    assert!(bright > dark, "dark={dark}, bright={bright}");
}

#[test]
fn macro_controls_reach_the_extended_palette_endpoints() {
    let mut kick = ResoKick::new(SAMPLE_RATE);
    kick.set_frequency(0.0);
    kick.set_depth(0.0);
    kick.set_pitch_decay(0.0);
    kick.set_resonate(0.0);
    kick.set_punch(0.0);
    kick.set_character(0.0);
    kick.set_ripple(0.0);
    kick.set_exciter_noise(0.0);
    assert!((kick.frequency_hz() - 6.0).abs() < 1.0e-4);
    assert!((kick.pitch_start_multiplier() - 1.0).abs() < 1.0e-4);
    assert!((kick.pitch_decay_ms() - 2.0).abs() < 1.0e-4);
    assert!((kick.resonate_t60_seconds() - 0.02).abs() < 1.0e-4);
    assert!((kick.punch_gain() - 1.0).abs() < 1.0e-4);
    assert!((kick.character_hz() - 3.0).abs() < 1.0e-4);
    assert!((kick.ripple_octaves() - 0.0).abs() < 1.0e-4);
    assert!((kick.exciter_noise_gain() - 0.0).abs() < 1.0e-4);

    kick.set_frequency(1.0);
    kick.set_depth(1.0);
    kick.set_pitch_decay(1.0);
    kick.set_resonate(1.0);
    kick.set_punch(1.0);
    kick.set_character(1.0);
    kick.set_ripple(1.0);
    kick.set_exciter_noise(1.0);
    assert!((kick.frequency_hz() - 180.0).abs() < 1.0e-3);
    assert!((kick.pitch_start_multiplier() - 18.0).abs() < 1.0e-4);
    assert!((kick.pitch_decay_ms() - 1_800.0).abs() < 1.0e-2);
    assert!((kick.resonate_t60_seconds() - 15.0).abs() < 1.0e-3);
    assert!((kick.punch_gain() - 16.0).abs() < 1.0e-4);
    assert!((kick.character_hz() - 16_000.0).abs() < 1.0e-2);
    assert!((kick.ripple_octaves() - 7.0).abs() < 1.0e-4);
    assert!((kick.exciter_noise_gain() - 4.0).abs() < 1.0e-4);
}

#[test]
fn tuning_reaches_lower_without_expanding_the_upper_limit() {
    let mut kick = ResoKick::new(SAMPLE_RATE);
    kick.set_frequency(0.0);
    kick.set_tuning(0.0);
    assert!((kick.tuning_semitones() + 18.0).abs() < 1.0e-4);
    assert!((kick.frequency_hz() - 2.121_320_2).abs() < 1.0e-4);

    kick.set_tuning(0.5);
    assert!(kick.tuning_semitones().abs() < 1.0e-4);
    assert!((kick.frequency_hz() - 6.0).abs() < 1.0e-4);

    kick.set_tuning(1.0);
    assert!((kick.tuning_semitones() - 12.0).abs() < 1.0e-4);
    assert!((kick.frequency_hz() - 12.0).abs() < 1.0e-4);
}

#[test]
fn mid_punch_peak_is_in_the_dynamics_threshold_region() {
    let mut mid_punch = ResoKickConfig::classic808();
    mid_punch.punch = 0.5;
    let output = render(mid_punch, 0.75, 2.0);
    let peak = output
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
    let peak_db = 20.0 * peak.log10();
    assert!((-4.0..=0.0).contains(&peak_db), "peak={peak_db:.2} dBFS");
}
