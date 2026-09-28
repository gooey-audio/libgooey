use gooey::engine::{Instrument, Modulatable};
use gooey::instruments::{
    ResonatorExciterShape, ResonatorNoiseConfig, ResonatorOutputTap, ResonatorRoutingConfig,
    ResonatorVoice, ResonatorVoiceConfig,
};

const SAMPLE_RATE: f32 = 48_000.0;

fn render(config: ResonatorVoiceConfig, seconds: f32) -> Vec<f32> {
    let mut voice = ResonatorVoice::with_config(SAMPLE_RATE, config);
    voice.trigger_with_velocity(0.0, 0.8);
    (0..(seconds * SAMPLE_RATE) as usize)
        .map(|sample| voice.tick(sample as f64 / SAMPLE_RATE as f64))
        .collect()
}

fn rms(samples: &[f32]) -> f64 {
    (samples
        .iter()
        .map(|&sample| f64::from(sample).powi(2))
        .sum::<f64>()
        / samples.len().max(1) as f64)
        .sqrt()
}

fn difference_rms(samples: &[f32]) -> f64 {
    rms(&samples
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect::<Vec<_>>())
}

#[test]
fn factory_patches_are_finite_audible_and_distinct() {
    let patches = [
        ResonatorVoiceConfig::kick(),
        ResonatorVoiceConfig::tom(),
        ResonatorVoiceConfig::snare(),
        ResonatorVoiceConfig::hybrid(),
        ResonatorVoiceConfig::metallic_drone(),
    ];
    let signatures = patches.map(|patch| {
        let output = render(patch, 1.0);
        assert!(output.iter().all(|sample| sample.is_finite()));
        let level = rms(&output);
        assert!(level > 1.0e-5, "level={level}");
        (level, difference_rms(&output))
    });

    assert!(
        signatures[2].1 > signatures[1].1,
        "snare must be brighter than tom"
    );
    assert_ne!(signatures[0], signatures[3]);
    assert_ne!(signatures[3], signatures[4]);
}

#[test]
fn extreme_patch_remains_bounded_for_ten_seconds() {
    let mut patch = ResonatorVoiceConfig::metallic_drone();
    for mode in [&mut patch.mode1, &mut patch.mode2] {
        mode.frequency_ratio = 32.0;
        mode.pitch_sweep_octaves = 8.0;
        mode.decay_seconds = 20.0;
        mode.feedback = 0.8;
        mode.drive = 24.0;
        mode.level = 2.0;
    }
    patch.routing = ResonatorRoutingConfig {
        transient_to_mode1: 1.0,
        transient_to_mode2: 1.0,
        noise_to_mode1: 1.0,
        noise_to_mode2: 1.0,
        mode1_to_mode2: 1.0,
        transient_to_output: 1.0,
        noise_to_output: 1.0,
        mode1_to_output: 1.0,
        mode2_to_output: 1.0,
    };
    for sample in render(patch, 10.0) {
        assert!(sample.is_finite());
        assert!(sample.abs() < 8.0, "sample={sample}");
    }
}

#[test]
fn noise_tail_outlives_the_transient() {
    let output = render(ResonatorVoiceConfig::snare(), 0.5);
    let early = rms(&output[..2_400]);
    let tail = rms(&output[7_200..14_400]);
    assert!(early > tail, "early={early}, tail={tail}");
    assert!(tail > 1.0e-4, "tail={tail}");
}

#[test]
fn routing_can_isolate_the_direct_noise_layer() {
    let mut patch = ResonatorVoiceConfig::snare();
    patch.exciter.level = 0.0;
    patch.routing = ResonatorRoutingConfig {
        noise_to_output: 1.0,
        ..ResonatorRoutingConfig::default()
    };
    patch.routing.transient_to_mode1 = 0.0;
    patch.routing.mode1_to_output = 0.0;
    patch.routing.mode2_to_output = 0.0;
    let level = rms(&render(patch, 0.3));
    assert!(level > 1.0e-6, "level={level}");
}

#[test]
fn discrete_tap_changes_latch_at_the_next_trigger() {
    let mut voice = ResonatorVoice::new(SAMPLE_RATE);
    voice.trigger_with_velocity(0.0, 1.0);
    assert_eq!(
        voice.active_output_tap(0),
        Some(ResonatorOutputTap::Lowpass)
    );
    voice.set_output_tap(0, ResonatorOutputTap::Bandpass);
    assert_eq!(
        voice.active_output_tap(0),
        Some(ResonatorOutputTap::Lowpass)
    );
    voice.trigger_with_velocity(0.1, 1.0);
    assert_eq!(
        voice.active_output_tap(0),
        Some(ResonatorOutputTap::Bandpass)
    );
}

#[test]
fn equal_seeds_render_identically_and_different_seeds_diverge() {
    let mut a = ResonatorVoiceConfig::snare();
    let mut b = a;
    let first = render(a, 0.1);
    let second = render(b, 0.1);
    assert_eq!(first, second);
    a.exciter.seed = 1;
    b.exciter.seed = 2;
    assert_ne!(render(a, 0.1), render(b, 0.1));
}

#[test]
fn continuous_edits_are_smoothed_and_sanitized() {
    let mut voice = ResonatorVoice::new(SAMPLE_RATE);
    voice.trigger_with_velocity(0.0, 1.0);
    let before = voice.tick(0.0);
    voice.set_mode_frequency_ratio(0, f32::NAN);
    voice.set_mode_decay_seconds(0, f32::INFINITY);
    voice.set_noise_config(ResonatorNoiseConfig {
        level: f32::NAN,
        filter_hz: f32::INFINITY,
        ..ResonatorNoiseConfig::default()
    });
    voice.set_macro("drive", 1.0).unwrap();
    let after = voice.tick(1.0 / SAMPLE_RATE as f64);
    assert!(before.is_finite() && after.is_finite());
    assert!((after - before).abs() < 4.0);
}

#[test]
fn instrument_controls_and_ring_limit_work() {
    let mut voice = ResonatorVoice::new(SAMPLE_RATE);
    assert_eq!(voice.modulatable_parameters().len(), 9);
    Instrument::set_midi_note(&mut voice, 48);
    voice.set_exciter_shape(ResonatorExciterShape::Click);
    voice.set_ring_limit_secs(Some(0.01));
    voice.trigger_with_velocity(0.0, 0.5);
    for sample in 0..480 {
        let _ = voice.tick(sample as f64 / SAMPLE_RATE as f64);
    }
    assert_eq!(voice.tick(0.011), 0.0);
    assert!(!voice.is_active());
}
