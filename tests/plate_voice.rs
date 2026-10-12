//! Unit-level tests for the modal plate voice (no FFI).

use gooey::engine::{Instrument, Modulatable};
use gooey::instruments::{
    plate_mode_shape, PlateConfig, PlateMaterial, PlateVoice, PLATE_MAX_MODES,
};

const SR: f32 = 48_000.0;

fn render(config: PlateConfig, velocity: f32, seconds: f32) -> Vec<f32> {
    let mut voice = PlateVoice::with_config(SR, config);
    voice.trigger_with_velocity(0.0, velocity);
    (0..(seconds * SR) as usize)
        .map(|i| voice.tick(i as f64 / SR as f64))
        .collect()
}

/// Zero-crossing frequency of a slice, interpolating each crossing.
fn zero_crossing_hz(x: &[f32]) -> f32 {
    let crossings: Vec<f32> = x
        .windows(2)
        .enumerate()
        .filter(|(_, w)| w[0] <= 0.0 && w[1] > 0.0)
        .map(|(i, w)| i as f32 + w[0] / (w[0] - w[1]))
        .collect();
    assert!(crossings.len() > 2, "too few crossings to measure pitch");
    let span = crossings[crossings.len() - 1] - crossings[0];
    (crossings.len() - 1) as f32 * SR / span
}

/// Share of energy above `cutoff` Hz, using two cascaded one-pole low-passes
/// as the complementary split.
fn energy_above(x: &[f32], cutoff: f32) -> f32 {
    let a = (-std::f32::consts::TAU * cutoff / SR).exp();
    let (mut l1, mut l2) = (0.0_f32, 0.0_f32);
    let (mut high, mut total) = (0.0_f64, 0.0_f64);
    for &s in x {
        l1 = a * l1 + (1.0 - a) * s;
        l2 = a * l2 + (1.0 - a) * l1;
        let h = s - l2;
        high += (h * h) as f64;
        total += (s * s) as f64;
    }
    (high / total.max(1.0e-12)) as f32
}

/// A plate that rings mostly at its fundamental so zero crossings track f0.
fn pitch_probe(tension: f32) -> PlateConfig {
    PlateConfig {
        pitch_hz: 150.0,
        strike_pos: 0.0,
        pickup_pos: 0.0,
        hardness: 0.0,
        decay_seconds: 2.0,
        hf_damping: 8.0,
        tension,
        cascade: 0.0,
        deplete: 0.0,
        brightness_hz: 500.0,
        material: PlateMaterial::Membrane,
        ..PlateConfig::timbale()
    }
}

fn glide_ratio(tension: f32, velocity: f32) -> f32 {
    let x = render(pitch_probe(tension), velocity, 1.0);
    let window = (0.1 * SR) as usize;
    let start = zero_crossing_hz(&x[(0.02 * SR) as usize..][..window]);
    let end = zero_crossing_hz(&x[x.len() - window..]);
    end / start
}

#[test]
fn material_tables_increase_and_stay_finite() {
    for material in PlateMaterial::ALL {
        let ratios = material.ratios(1234);
        assert_eq!(ratios.len(), PLATE_MAX_MODES);
        assert!(ratios.iter().all(|r| r.is_finite() && *r > 0.0));
        assert!(
            ratios.windows(2).all(|w| w[1] > w[0]),
            "{material:?} ratios are not strictly increasing"
        );
    }
}

#[test]
fn modes_above_nyquist_are_muted() {
    // A 2 kHz bar puts most of its stretched tail far above Nyquist.
    let config = PlateConfig {
        pitch_hz: 2_000.0,
        geometry: 0.5,
        hardness: 1.0,
        material: PlateMaterial::Bar,
        ..PlateConfig::bell()
    };
    let x = render(config, 1.0, 0.5);
    assert!(x.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    assert!(x.iter().any(|s| s.abs() > 1.0e-3));
}

#[test]
fn tension_glides_hard_hits_down() {
    let glide = glide_ratio(1.0, 1.0);
    assert!(glide < 0.98, "tension 1 glide ratio {glide}");
    let flat = glide_ratio(0.0, 1.0);
    assert!((flat - 1.0).abs() < 0.002, "tension 0 glide ratio {flat}");
}

#[test]
fn soft_hits_glide_less_than_hard_hits() {
    let hard = 1.0 - glide_ratio(1.0, 1.0);
    let soft = 1.0 - glide_ratio(1.0, 0.3);
    assert!(soft < hard * 0.6, "soft glide {soft} vs hard {hard}");
}

#[test]
fn cascade_moves_crash_energy_above_4khz() {
    let with = energy_above(&render(PlateConfig::crash(), 1.0, 2.0), 4_000.0);
    let without = energy_above(
        &render(
            PlateConfig {
                cascade: 0.0,
                ..PlateConfig::crash()
            },
            1.0,
            2.0,
        ),
        4_000.0,
    );
    assert!(
        with > without * 1.5,
        "cascade share {with} vs dry share {without}"
    );
}

#[test]
fn strike_position_changes_the_spectrum() {
    let centre = PlateConfig {
        strike_pos: 0.0,
        cascade: 0.0,
        ..PlateConfig::ride()
    };
    let edge = PlateConfig {
        strike_pos: 1.0,
        ..centre
    };
    let a = energy_above(&render(centre, 1.0, 0.5), 2_000.0);
    let b = energy_above(&render(edge, 1.0, 0.5), 2_000.0);
    assert!((a - b).abs() > 0.05, "centre {a} vs edge {b}");
}

#[test]
fn every_parameter_extreme_stays_finite_and_bounded() {
    for material in PlateMaterial::ALL {
        for &(low, high) in &[(0.0, 0.0), (1.0, 1.0), (0.0, 1.0), (1.0, 0.0)] {
            let mut voice = PlateVoice::new(SR);
            for index in 0..11 {
                let value = if index % 2 == 0 { low } else { high };
                voice.set_parameter_normalized(index, value);
            }
            // The worst case named in the plan: max tension, cascade, decay.
            voice.set_parameter_normalized(5, 1.0);
            voice.set_parameter_normalized(7, 1.0);
            voice.set_parameter_normalized(8, 1.0);
            voice.set_material(material);
            for hit in 0..4 {
                voice.trigger_with_velocity(hit as f64 * 0.05, 1.0);
            }
            for i in 0..(SR as usize) {
                let s = voice.tick(i as f64 / SR as f64);
                assert!(
                    s.is_finite() && s.abs() <= 1.0,
                    "{material:?} ({low}, {high}) sample {i} = {s}"
                );
            }
        }
    }
}

#[test]
fn voice_goes_inactive_after_decay() {
    let mut voice = PlateVoice::with_config(
        SR,
        PlateConfig {
            decay_seconds: 0.1,
            cascade: 0.0,
            ..PlateConfig::timbale()
        },
    );
    voice.trigger_with_velocity(0.0, 1.0);
    assert!(voice.is_active());
    for i in 0..(2.0 * SR) as usize {
        voice.tick(i as f64 / SR as f64);
    }
    assert!(!voice.is_active());
}

#[test]
fn ring_limit_stops_a_long_gong() {
    let mut voice = PlateVoice::with_config(SR, PlateConfig::gong());
    voice.set_ring_limit_secs(Some(0.25));
    voice.trigger_with_velocity(0.0, 1.0);
    for i in 0..(0.3 * SR) as usize {
        voice.tick(i as f64 / SR as f64);
    }
    assert!(!voice.is_active());
}

#[test]
fn normalized_parameters_round_trip() {
    let mut voice = PlateVoice::new(SR);
    for index in 0..12 {
        voice.set_parameter_normalized(index, 0.75);
        let value = voice.parameter_normalized(index).unwrap();
        assert!((value - 0.75).abs() < 1.0e-4, "param {index} = {value}");
    }
    assert_eq!(voice.material(), PlateMaterial::Bell);
    assert!(voice.parameter_normalized(12).is_none());
    assert!(voice.apply_modulation("cascade", 0.2).is_ok());
    assert!(voice.apply_modulation("volume", 0.2).is_err());
}

#[test]
fn channel_tuning_shifts_pitch_an_octave() {
    let measure = |tuning: f32| {
        let mut voice = PlateVoice::with_config(SR, pitch_probe(0.0));
        voice.set_channel_tuning(tuning);
        voice.trigger_with_velocity(0.0, 1.0);
        let x: Vec<f32> = (0..(0.5 * SR) as usize)
            .map(|i| voice.tick(i as f64 / SR as f64))
            .collect();
        zero_crossing_hz(&x[(0.1 * SR) as usize..])
    };
    let up = measure(1.0) / measure(0.5);
    assert!((up - 2.0).abs() < 0.03, "tuning ratio {up}");
}

#[test]
fn mode_states_report_ringing_and_decay() {
    let mut voice = PlateVoice::with_config(SR, PlateConfig::ride());
    assert!(voice.mode_states().iter().all(|mode| mode.amplitude == 0.0));
    voice.trigger_with_velocity(0.0, 1.0);
    for i in 0..(0.05 * SR) as usize {
        voice.tick(i as f64 / SR as f64);
    }
    let early = voice.mode_states();
    assert!(early[0].amplitude > 1.0e-3, "fundamental should ring");
    assert!(early
        .windows(2)
        .all(|pair| pair[0].frequency_hz < pair[1].frequency_hz || pair[1].gain == 0.0));
    assert!(early.iter().all(|mode| mode.band <= 3));
    for i in (0.05 * SR) as usize..(1.0 * SR) as usize {
        voice.tick(i as f64 / SR as f64);
    }
    let late = voice.mode_states();
    assert!(late[0].amplitude < early[0].amplitude);
}

#[test]
fn mode_states_expose_the_latched_strike_shape() {
    let mut config = PlateConfig::bell();
    config.strike_pos = 0.0;
    let mut voice = PlateVoice::with_config(SR, config);
    voice.trigger_with_velocity(0.0, 1.0);
    let states = voice.mode_states();
    // A centre hit sits on a node of every even-numbered shape.
    assert!(states[1].strike_weight.abs() < 1.0e-4);
    assert!(states[0].strike_weight.abs() > 0.5);
    // The weights are the public mode shapes up to one normalisation.
    let scale = states[0].strike_weight / plate_mode_shape(0, 0.0);
    for (k, state) in states.iter().enumerate() {
        assert!((state.strike_weight - scale * plate_mode_shape(k, 0.0)).abs() < 1.0e-4);
    }
}
