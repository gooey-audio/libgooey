//! Integration tests for the instrument-graph DSL and engine.
//!
//! These render audio with a plain tick loop (no audio hardware, no `bounce`
//! feature), so they run anywhere `cargo test` runs.

use gooey::engine::{Engine, Instrument};
use gooey::graph::{GraphInstrument, GraphSpec};

const SAMPLE_RATE: f32 = 48_000.0;

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|s| (*s as f64).powi(2)).sum();
    (sum / samples.len() as f64).sqrt() as f32
}

fn render_instrument(mut inst: GraphInstrument, samples: usize) -> Vec<f32> {
    inst.trigger(0.0);
    let dt = 1.0 / SAMPLE_RATE as f64;
    let mut t = 0.0;
    let mut buf = Vec::with_capacity(samples);
    for _ in 0..samples {
        buf.push(inst.tick(t));
        t += dt;
    }
    buf
}

#[test]
fn graph_instrument_plays_through_the_engine() {
    let src = "body = env a=.001 d=.3 curve=2.0\nsub = osc sine 110\nout = sub * body";
    let inst = GraphInstrument::from_source(src, SAMPLE_RATE).expect("compile");

    let mut engine = Engine::new(SAMPLE_RATE);
    engine.set_master_gain(1.0);
    engine.add_instrument("voice", Box::new(inst));
    engine.trigger_instrument_with_velocity("voice", 1.0);

    let dt = 1.0 / SAMPLE_RATE as f64;
    let mut t = 0.0;
    let mut buf = Vec::new();
    for _ in 0..9_600 {
        buf.push(engine.tick(t));
        t += dt;
    }
    assert!(
        rms(&buf) > 0.005,
        "engine produced no audible output: rms={}",
        rms(&buf)
    );
    assert!(buf.iter().all(|s| s.is_finite()));
}

#[test]
fn reference_kick_graph_compiles_and_sounds() {
    let src = include_str!("../examples/instruments/kick.graph");
    let spec = GraphSpec::parse(src).expect("kick.graph should parse");

    // The graph the author wrote should expose the named components.
    let names: Vec<&str> = spec.nodes.iter().map(|n| n.name.as_str()).collect();
    for expected in [
        "pitch", "body", "sub", "punch", "click", "nz", "voice", "out",
    ] {
        assert!(
            names.contains(&expected),
            "missing node '{expected}' in {names:?}"
        );
    }

    let inst = GraphInstrument::from_spec(&spec, SAMPLE_RATE).expect("compile");
    let buf = render_instrument(inst, 24_000);
    assert!(rms(&buf) > 0.01, "kick graph was silent: rms={}", rms(&buf));
    assert!(
        buf.iter().all(|s| s.is_finite()),
        "kick graph produced non-finite samples"
    );
    let peak = buf.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
    assert!(peak < 8.0, "kick graph output ran away: peak={peak}");
}

#[test]
fn tone_graph_compiles() {
    let src = include_str!("../examples/instruments/tone.graph");
    let inst = GraphInstrument::from_source(src, SAMPLE_RATE).expect("tone.graph should compile");
    let buf = render_instrument(inst, 4_800);
    assert!(rms(&buf) > 0.01);
}

#[test]
fn positional_and_named_args_are_equivalent() {
    let positional = GraphSpec::parse("c = lp (osc saw 200) 1500 0.7\nout = c").unwrap();
    let named = GraphSpec::parse("c = lp in=(osc saw 200) cutoff=1500 q=0.7\nout = c").unwrap();
    // Both should produce a low-pass node with the same baked parameters.
    let lp_pos = positional.nodes.iter().find(|n| n.kind == "lp").unwrap();
    let lp_named = named.nodes.iter().find(|n| n.kind == "lp").unwrap();
    assert_eq!(lp_pos.params.get("cutoff"), Some(&1500.0));
    assert_eq!(lp_named.params.get("cutoff"), Some(&1500.0));
    assert_eq!(lp_pos.params.get("q"), Some(&0.7));
    assert_eq!(lp_named.params.get("q"), Some(&0.7));
}

#[test]
fn parentheses_control_precedence() {
    // `a + b * c` should multiply before adding (b*c), so the top node is add.
    let spec = GraphSpec::parse(
        "a = osc sine 100\nb = osc sine 200\nc = env a=.001 d=.2\nout = a + b * c",
    )
    .unwrap();
    assert_eq!(spec.nodes[spec.out].kind, "add");
}

#[test]
fn signal_into_param_only_slot_is_rejected() {
    // `q` on a filter is a constant-only parameter; feeding it a node is an error.
    let err = GraphSpec::parse("e = env a=.001 d=.2\nout = lp (osc saw 100) q=e").unwrap_err();
    assert!(err.contains("q"), "err={err}");
}

#[test]
fn reserved_keyword_cannot_be_a_node_name() {
    let err = GraphSpec::parse("osc = env a=.001 d=.2\nout = osc").unwrap_err();
    assert!(err.contains("reserved"), "err={err}");
}

#[test]
fn live_param_change_alters_output() {
    let src = "body = env a=.001 d=2.0 s=1.0\nsub = osc sine 110\nout = sub * body";
    let mut quiet = GraphInstrument::from_source(src, SAMPLE_RATE).unwrap();
    let mut loud = GraphInstrument::from_source(src, SAMPLE_RATE).unwrap();

    // Retune the oscillator on `loud` up to 440 Hz; the rendered audio should
    // then differ from the untouched `quiet` instrument.
    loud.set_param("sub", "freq", 440.0).unwrap();

    quiet.trigger(0.0);
    loud.trigger(0.0);
    let dt = 1.0 / SAMPLE_RATE as f64;
    let mut t = 0.0;
    let mut diff = 0.0_f64;
    for _ in 0..2_400 {
        let q = quiet.tick(t);
        let l = loud.tick(t);
        diff += (q - l).abs() as f64;
        t += dt;
    }
    assert!(
        diff > 1.0,
        "changing a node param had no audible effect (diff={diff})"
    );
}

#[test]
fn set_param_on_unknown_node_errors() {
    let mut inst = GraphInstrument::from_source("out = osc sine 110", SAMPLE_RATE).unwrap();
    assert!(inst.set_param("nope", "freq", 100.0).is_err());
}
