use gooey::dsl::Program;

#[test]
fn parses_and_builds_basic_program() {
    let src = r#"
        # Sequencer + LFO + FX
        bpm 120
        master 0.25

        inst hihat hihat closed
        seq hihat x.x.x.x.|x.x.x.x.

        lfo 1bar hihat.decay amt=1
        fx clear
        fx lowpass 2000 0.3
        fx limiter 0.9
    "#;

    let program = Program::parse(src).expect("parse");
    assert_eq!(program.bpm(), Some(120.0));

    let sample_rate = 44100.0;
    let engine = program.build_engine(sample_rate).expect("build engine");

    assert_eq!(engine.bpm(), 120.0);
    assert_eq!(engine.master_gain(), 0.25);

    // fx clear means no default limiter + 2 explicitly added effects.
    assert_eq!(engine.global_effect_count(), 2);

    assert_eq!(engine.sequencer_count(), 1);
    let seq = engine.sequencer(0).unwrap();
    assert_eq!(seq.instrument_name(), "hihat");
    assert!(seq.is_running());

    assert_eq!(engine.lfo(0).unwrap().target_instrument, "hihat");
    assert_eq!(engine.lfo(0).unwrap().target_parameter, "decay");
    assert_eq!(engine.lfo(0).unwrap().amount, 1.0);
}

#[test]
fn lfo_hz_rate_and_offset_syntax() {
    let src = r#"
        inst kick kick
        lfo hz 0.5 -> kick.pitch_drop *0.7 @0.1
    "#;

    let program = Program::parse(src).expect("parse");
    let engine = program.build_engine(44100.0).expect("build engine");

    assert_eq!(engine.lfo(0).unwrap().target_instrument, "kick");
    // DSL alias: kick.pitch_drop => kick.pitch_envelope_amount
    assert_eq!(
        engine.lfo(0).unwrap().target_parameter,
        "pitch_envelope_amount"
    );
    assert_eq!(engine.lfo(0).unwrap().amount, 0.7);
    assert_eq!(engine.lfo(0).unwrap().offset, 0.1);
}
