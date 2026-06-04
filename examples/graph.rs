//! Load an instrument-graph DSL file, print its node topology, and render it to
//! a WAV file by triggering it on a four-on-the-floor sequencer.
//!
//! Usage:
//!
//!     cargo run --example graph --features bounce -- examples/instruments/kick.graph
//!
//! With no argument it loads `examples/instruments/kick.graph`. The output is
//! written next to the input as `<name>.wav`.

use std::path::{Path, PathBuf};

use gooey::bounce::{bounce_to_wav, BounceLength, WavConfig};
use gooey::engine::{Engine, Sequencer, SequencerStep};
use gooey::graph::{GraphInstrument, Topology};

fn main() -> anyhow::Result<()> {
    let input = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "examples/instruments/kick.graph".to_string());
    let input = PathBuf::from(input);

    let source = std::fs::read_to_string(&input)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", input.display()))?;

    let sample_rate = 48_000.0;
    let bpm = 120.0;

    let instrument = GraphInstrument::from_source(&source, sample_rate)
        .map_err(|e| anyhow::anyhow!("failed to compile {}: {e}", input.display()))?;

    println!("Loaded {}", input.display());
    print_topology(instrument.topology());

    // Play the graph four-on-the-floor for one bar and bounce it.
    let mut engine = Engine::new(sample_rate);
    engine.set_bpm(bpm);
    engine.set_master_gain(0.9);
    engine.add_instrument("voice", Box::new(instrument));

    let pattern: Vec<SequencerStep> = (0..16)
        .map(|i| SequencerStep {
            enabled: i % 4 == 0,
            velocity: if i == 0 { 1.0 } else { 0.85 },
            blend: None,
            note: None,
        })
        .collect();
    engine.add_sequencer(Sequencer::with_velocity_pattern(
        bpm,
        sample_rate,
        pattern,
        "voice",
    ));

    let out_path = output_path(&input);
    bounce_to_wav(
        &mut engine,
        BounceLength::Bars(1),
        &out_path,
        WavConfig::default(),
    )
    .map_err(|e| anyhow::anyhow!("bounce failed: {e}"))?;

    println!("\nWrote {}", out_path.display());
    Ok(())
}

/// Print the graph as an indented node list with each node's inputs, so the
/// signal flow is readable in the terminal (a text stand-in for the GUI
/// diagram that this topology will later drive).
fn print_topology(topo: &Topology) {
    println!(
        "\nGraph ({} nodes, output = '{}'):",
        topo.nodes.len(),
        topo.nodes[topo.out].name
    );
    for (idx, node) in topo.nodes.iter().enumerate() {
        let marker = if idx == topo.out { "*" } else { " " };
        let env = if node.is_envelope { " [env]" } else { "" };

        let mut line = format!("  {marker} {:<10} {}{}", node.name, node.kind, env);

        if !node.params.is_empty() {
            let params: Vec<String> = node
                .params
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            line.push_str(&format!("  {{{}}}", params.join(", ")));
        }

        if !node.inputs.is_empty() {
            let edges: Vec<String> = node
                .inputs
                .iter()
                .map(|(port, src)| format!("{port}<-{}", topo.nodes[*src].name))
                .collect();
            line.push_str(&format!("  ({})", edges.join(", ")));
        }

        println!("{line}");
    }
}

fn output_path(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("graph");
    input.with_file_name(format!("{stem}.wav"))
}
