use gooey::engine::{Engine, Lfo, MusicalDivision};
use gooey::instruments::{Granulator, SampleBuffer};

fn test_buffer() -> SampleBuffer {
    let sample_rate = 44100.0;
    let samples = (0..44100)
        .map(|i| {
            let t = i as f32 / sample_rate;
            (std::f32::consts::TAU * 220.0 * t).sin() * 0.5
        })
        .collect();
    SampleBuffer::from_mono(samples, sample_rate).unwrap()
}

#[test]
fn engine_can_trigger_granulator() {
    let sample_rate = 44100.0;
    let mut engine = Engine::new(sample_rate);
    engine.add_instrument(
        "granulator",
        Box::new(Granulator::new(sample_rate, test_buffer())),
    );
    engine.trigger_instrument_with_velocity("granulator", 1.0);

    let mut max_abs = 0.0_f32;
    for i in 0..44100 {
        let sample = engine.tick(i as f64 / sample_rate as f64);
        assert!(sample.is_finite());
        max_abs = max_abs.max(sample.abs());
    }

    assert!(max_abs > 0.001);
}

#[test]
fn granulator_parameters_are_modulatable() {
    let sample_rate = 44100.0;
    let bpm = 120.0;
    let mut engine = Engine::new(sample_rate);
    engine.set_bpm(bpm);
    engine.add_instrument(
        "granulator",
        Box::new(Granulator::new(sample_rate, test_buffer())),
    );

    let parameters = [
        "scan_position",
        "grain_length",
        "spray",
        "pitch",
        "density",
        "texture",
        "direction",
        "volume",
    ];

    for parameter in parameters {
        let lfo_index = engine.add_lfo(Lfo::new_synced(MusicalDivision::OneBar, bpm, sample_rate));
        let result = engine.map_lfo_to_parameter(lfo_index, "granulator", parameter, 1.0);
        assert!(
            result.is_ok(),
            "granulator parameter '{}' should be modulatable",
            parameter
        );
    }
}
