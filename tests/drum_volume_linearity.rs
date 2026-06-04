//! Regression tests for linear kick and snare master volume.

use gooey::instruments::{KickDrum, SnareDrum};

const SAMPLE_RATE: f32 = 44_100.0;
const RENDER_FRAMES: usize = 4_096;
const HALF_VOLUME: f32 = 0.5;
const MAX_ERROR: f32 = 1e-5;

fn render_kick(volume: f32) -> Vec<f32> {
    let mut kick = KickDrum::new(SAMPLE_RATE);
    kick.set_volume(volume);
    kick.snap_params();
    kick.trigger_with_velocity(0.0, 1.0);

    (0..RENDER_FRAMES)
        .map(|sample| kick.tick(sample as f64 / SAMPLE_RATE as f64))
        .collect()
}

fn render_snare(volume: f32) -> Vec<f32> {
    let mut snare = SnareDrum::new(SAMPLE_RATE);
    snare.set_volume(volume);
    snare.snap_params();
    snare.trigger_with_velocity(0.0, 1.0);

    (0..RENDER_FRAMES)
        .map(|sample| snare.tick(sample as f64 / SAMPLE_RATE as f64))
        .collect()
}

fn assert_half_volume_is_linear(name: &str, full: &[f32], half: &[f32]) {
    let peak = full
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0_f32, f32::max);
    assert!(peak > 0.01, "{name} full-volume output should be audible");

    let max_error = full
        .iter()
        .zip(half)
        .map(|(full, half)| (half - full * HALF_VOLUME).abs())
        .fold(0.0_f32, f32::max);
    assert!(
        max_error < MAX_ERROR,
        "{name} 0.5 volume should equal full output scaled by 0.5, max error was {max_error}"
    );
}

#[test]
fn kick_master_volume_is_linear() {
    assert_half_volume_is_linear("kick", &render_kick(1.0), &render_kick(HALF_VOLUME));
}

#[test]
fn snare_master_volume_is_linear() {
    assert_half_volume_is_linear("snare", &render_snare(1.0), &render_snare(HALF_VOLUME));
}
