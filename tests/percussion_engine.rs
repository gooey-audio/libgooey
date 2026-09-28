use gooey::engine::Instrument;
use gooey::instruments::{PercussionEngine, PercussionEngineKind, PercussionPreset};

const SAMPLE_RATE: f32 = 48_000.0;

fn render(engine: &mut PercussionEngine, seconds: f32) -> Vec<f32> {
    engine.trigger_with_velocity(0.0, 0.8);
    (0..(seconds * SAMPLE_RATE) as usize)
        .map(|index| engine.tick(index as f64 / SAMPLE_RATE as f64))
        .collect()
}

#[test]
fn every_engine_and_preset_combination_is_audible() {
    for kind in PercussionEngineKind::ALL {
        for preset in PercussionPreset::ALL {
            let mut engine = PercussionEngine::with_selection(SAMPLE_RATE, kind, preset);
            let samples = render(&mut engine, 0.5);
            let energy: f32 = samples.iter().map(|sample| sample * sample).sum();
            assert!(samples.iter().all(|sample| sample.is_finite()));
            assert!(energy > 1.0e-5, "{kind:?} / {preset:?} was silent");
        }
    }
}

#[test]
fn switching_engine_preserves_the_musical_preset() {
    let mut engine = PercussionEngine::with_selection(
        SAMPLE_RATE,
        PercussionEngineKind::RoutingMatrix,
        PercussionPreset::Snare,
    );
    assert!(engine.routing_matrix().is_some());
    engine.select_engine(PercussionEngineKind::TwinCore);
    assert_eq!(engine.kind(), PercussionEngineKind::TwinCore);
    assert_eq!(engine.preset(), PercussionPreset::Snare);
    assert!(engine.twin_core().is_some());
}

#[test]
fn engine_and_preset_have_dropdown_labels() {
    assert_eq!(
        PercussionEngineKind::RoutingMatrix.display_name(),
        "Routing Matrix"
    );
    assert_eq!(PercussionEngineKind::TwinCore.display_name(), "Twin Core");
    assert_eq!(PercussionPreset::ClapHybrid.display_name(), "Clap / Hybrid");
}
