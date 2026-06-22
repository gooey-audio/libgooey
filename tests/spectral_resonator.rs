//! Integration tests for the STFT-based SpectralResonator effect.
#![cfg(feature = "spectral")]

use gooey::dsl::Program;
use gooey::effects::{Effect, SpectralResonator};
use gooey::frame::StereoFrame;

const SAMPLE_RATE: f32 = 44_100.0;

/// Mono input must stay dual-mono (the two per-channel STFT states share the
/// same parameters and see identical input), and the output must stay finite
/// and bounded even with heavy resonance.
#[test]
fn mono_input_stays_dual_mono_and_bounded() {
    let fx = SpectralResonator::new(SAMPLE_RATE, 220.0, 0.95, 0.8, 1.0);

    let frames = 1usize << 15; // span well past the FFT pre-roll
    let mut max_abs = 0.0_f32;
    for i in 0..frames {
        // Broadband-ish excitation: fundamental + a couple of partials.
        let t = i as f32 / SAMPLE_RATE;
        let x = 0.3
            * ((2.0 * std::f32::consts::PI * 220.0 * t).sin()
                + (2.0 * std::f32::consts::PI * 440.0 * t).sin()
                + (2.0 * std::f32::consts::PI * 660.0 * t).sin());
        let out = fx.process_stereo(StereoFrame::mono(x));
        assert_eq!(out.l, out.r, "left != right at frame {i} for mono input");
        assert!(out.l.is_finite(), "non-finite output at frame {i}");
        max_abs = max_abs.max(out.l.abs());
    }
    assert!(max_abs < 100.0, "resonator output blew up: {max_abs}");
    assert!(
        max_abs > 1e-4,
        "expected audible resonant output, peak {max_abs}"
    );
}

/// The effect wires through the DSL (`fx spectral ...`) and into an engine.
#[test]
fn dsl_builds_and_renders_spectral_effect() {
    let src = r#"
        bpm 120
        master 0.5

        inst kick kick
        seq kick x...x...x...x...

        fx clear
        fx spectral freq=330 res=0.7 sharp=0.6 mix=0.8
    "#;

    let program = Program::parse(src).expect("parse");
    let mut engine = program.build_engine(SAMPLE_RATE).expect("build engine");

    // `fx clear` removes the default limiter; only the spectral effect remains.
    assert_eq!(engine.global_effect_count(), 1);

    // Render past the pre-roll; output must stay finite and bounded.
    let frames = 1usize << 15;
    for i in 0..frames {
        let s = engine.tick_stereo(i as f64 / SAMPLE_RATE as f64);
        assert!(
            s.l.is_finite() && s.r.is_finite() && s.l.abs() < 100.0 && s.r.abs() < 100.0,
            "spectral engine output unstable at frame {i}: {s:?}"
        );
    }
}
