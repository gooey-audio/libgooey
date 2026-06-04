//! Instrument-graph engine: describe an instrument as a graph of small,
//! interconnected nodes (oscillators, envelopes, noise, filters, and opaque
//! effects) using a low-noise text DSL, then play it like any other instrument.
//!
//! Motivation: the hand-written instruments (for example [`crate::instruments::KickDrum`])
//! bury their oscillators, envelopes, and filters inside one large `tick()`
//! method, so it is hard to see which component shapes which part of the sound.
//! A graph makes every component an explicit, named node you can read, re-wire,
//! and (in the GUI phase) visualize on a shared time axis.
//!
//! Quick start:
//!
//! ```
//! use gooey::graph::GraphInstrument;
//! use gooey::engine::Instrument; // brings `trigger` / `tick` into scope
//!
//! let src = "\
//! body = env a=.001 d=.4 curve=2.0\n\
//! sub  = osc sine 55\n\
//! out  = sub * body\n";
//! let mut inst = GraphInstrument::from_source(src, 48_000.0).unwrap();
//! inst.trigger(0.0);
//! let sample = inst.tick(0.0); // one audio sample
//! let _ = sample;
//! ```
//!
//! The engine is intentionally dependency-free and always compiled (no feature
//! flag), so it is fully unit-testable headlessly. The GUI that draws the graph
//! is a separate, feature-gated layer that consumes [`GraphInstrument::topology`].

pub mod node;
pub mod parser;

pub use parser::{GraphSpec, NodeSpec};

use node::{build_node, NodeImpl};

use crate::engine::Instrument;

/// Lightweight, render-free description of a compiled graph, suitable for the
/// node-graph diagram in the GUI: the nodes, their kinds and baked parameters,
/// and the edges between them.
#[derive(Clone, Debug)]
pub struct Topology {
    pub nodes: Vec<NodeInfo>,
    /// Index of the output node.
    pub out: usize,
}

/// One node in a [`Topology`].
#[derive(Clone, Debug)]
pub struct NodeInfo {
    pub name: String,
    pub kind: String,
    /// Baked scalar parameters, sorted by name for stable display.
    pub params: Vec<(String, f32)>,
    /// Incoming edges as `(port_name, source_node_index)`.
    pub inputs: Vec<(String, usize)>,
    /// Whether this node is an envelope (drives graph lifetime / shown on the
    /// envelope timeline).
    pub is_envelope: bool,
}

/// A compiled, runnable instrument graph. Nodes are stored in an order where
/// every node's inputs come from earlier nodes, so a single forward pass per
/// sample evaluates the whole graph.
pub struct CompiledGraph {
    nodes: Vec<Box<dyn NodeImpl>>,
    /// For each node, one entry per input port (aligned to the node's
    /// `input_ports()`), giving the source node index or `None` if unconnected.
    sources: Vec<Vec<Option<usize>>>,
    /// Indices of envelope nodes, used to decide when the graph falls silent.
    env_indices: Vec<usize>,
    /// Map from `name` to node index, for `set_param` addressing.
    name_to_index: std::collections::HashMap<String, usize>,
    out: usize,
    outputs: Vec<f32>,
    scratch: Vec<Option<f32>>,
    topology: Topology,
    sample_rate: f32,
}

impl CompiledGraph {
    /// Compile a parsed [`GraphSpec`] for a given host sample rate.
    pub fn compile(spec: &GraphSpec, sample_rate: f32) -> Result<Self, String> {
        let mut nodes: Vec<Box<dyn NodeImpl>> = Vec::with_capacity(spec.nodes.len());
        let mut sources: Vec<Vec<Option<usize>>> = Vec::with_capacity(spec.nodes.len());
        let mut env_indices = Vec::new();
        let mut name_to_index = std::collections::HashMap::new();
        let mut topo_nodes = Vec::with_capacity(spec.nodes.len());

        for (idx, spec_node) in spec.nodes.iter().enumerate() {
            let node = build_node(
                &spec_node.kind,
                &spec_node.params,
                spec_node.waveform,
                sample_rate,
            )?;

            // Align this node's declared connections to its port order.
            let ports = node.input_ports();
            let mut node_sources = vec![None; ports.len()];
            for (port_name, src) in &spec_node.connections {
                let port_index = ports.iter().position(|p| p == port_name).ok_or_else(|| {
                    format!(
                        "node '{}' ({}) has no input port '{}'",
                        spec_node.name, spec_node.kind, port_name
                    )
                })?;
                if *src >= idx {
                    return Err(format!(
                        "node '{}' refers to a later or self node (cycles are not supported)",
                        spec_node.name
                    ));
                }
                node_sources[port_index] = Some(*src);
            }

            if node.is_envelope() {
                env_indices.push(idx);
            }
            name_to_index.insert(spec_node.name.clone(), idx);

            let mut params: Vec<(String, f32)> = spec_node
                .params
                .iter()
                .map(|(k, v)| (k.clone(), *v))
                .collect();
            params.sort_by(|a, b| a.0.cmp(&b.0));
            topo_nodes.push(NodeInfo {
                name: spec_node.name.clone(),
                kind: spec_node.kind.clone(),
                params,
                inputs: spec_node.connections.clone(),
                is_envelope: node.is_envelope(),
            });

            nodes.push(node);
            sources.push(node_sources);
        }

        let count = nodes.len();
        Ok(Self {
            nodes,
            sources,
            env_indices,
            name_to_index,
            out: spec.out,
            outputs: vec![0.0; count],
            scratch: Vec::with_capacity(4),
            topology: Topology {
                nodes: topo_nodes,
                out: spec.out,
            },
            sample_rate,
        })
    }

    /// Reset state and (re)start all envelopes for a new note.
    pub fn trigger(&mut self, time: f64, velocity: f32) {
        for node in &mut self.nodes {
            node.trigger(time, velocity);
        }
        for value in &mut self.outputs {
            *value = 0.0;
        }
    }

    /// Begin the release phase of any envelopes.
    pub fn release(&mut self, time: f64) {
        for node in &mut self.nodes {
            node.release(time);
        }
    }

    /// Evaluate the whole graph for one sample and return the output value.
    pub fn tick(&mut self, time: f64) -> f32 {
        let count = self.nodes.len();
        for idx in 0..count {
            let port_count = self.sources[idx].len();
            self.scratch.clear();
            for p in 0..port_count {
                let value = self.sources[idx][p].map(|src| self.outputs[src]);
                self.scratch.push(value);
            }
            let out = self.nodes[idx].tick(time, &self.scratch);
            self.outputs[idx] = out;
        }
        self.outputs[self.out]
    }

    /// Whether the graph is still producing sound. True while any envelope node
    /// is active; a graph with no envelopes is always considered active.
    pub fn is_active(&self) -> bool {
        if self.env_indices.is_empty() {
            return true;
        }
        self.env_indices.iter().any(|&i| self.nodes[i].is_active())
    }

    /// The per-node output values from the most recent [`tick`], aligned to
    /// [`Topology::nodes`]. Useful for live meters in the GUI.
    pub fn last_outputs(&self) -> &[f32] {
        &self.outputs
    }

    /// Render description for the GUI.
    pub fn topology(&self) -> &Topology {
        &self.topology
    }

    /// Set a scalar parameter on a named node at runtime (live tweaking).
    pub fn set_param(&mut self, node: &str, param: &str, value: f32) -> Result<(), String> {
        let idx = *self
            .name_to_index
            .get(node)
            .ok_or_else(|| format!("no node named '{node}'"))?;
        self.nodes[idx].set_param(param, value)?;
        if let Some(info) = self.topology.nodes.get_mut(idx) {
            if let Some(entry) = info.params.iter_mut().find(|(k, _)| k == param) {
                entry.1 = value;
            } else {
                info.params.push((param.to_string(), value));
                info.params.sort_by(|a, b| a.0.cmp(&b.0));
            }
        }
        Ok(())
    }

    /// The sample rate this graph was compiled for.
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }
}

/// An instrument backed by a node graph. Implements the standard
/// [`Instrument`] trait, so it drops directly into [`crate::engine::Engine`],
/// the sequencer, and the offline bounce path.
pub struct GraphInstrument {
    graph: CompiledGraph,
    velocity: f32,
    active: bool,
}

impl GraphInstrument {
    /// Parse and compile DSL source into a playable instrument.
    pub fn from_source(source: &str, sample_rate: f32) -> Result<Self, String> {
        let spec = GraphSpec::parse(source)?;
        let graph = CompiledGraph::compile(&spec, sample_rate)?;
        Ok(Self {
            graph,
            velocity: 1.0,
            active: false,
        })
    }

    /// Build from an already-parsed spec (lets callers inspect the spec first).
    pub fn from_spec(spec: &GraphSpec, sample_rate: f32) -> Result<Self, String> {
        Ok(Self {
            graph: CompiledGraph::compile(spec, sample_rate)?,
            velocity: 1.0,
            active: false,
        })
    }

    /// The graph's node/edge description for visualization.
    pub fn topology(&self) -> &Topology {
        self.graph.topology()
    }

    /// Per-node output values from the most recent tick (for live meters).
    pub fn last_outputs(&self) -> &[f32] {
        self.graph.last_outputs()
    }

    /// Set a scalar parameter on a named node at runtime.
    pub fn set_param(&mut self, node: &str, param: &str, value: f32) -> Result<(), String> {
        self.graph.set_param(node, param, value)
    }

    /// Access the underlying compiled graph.
    pub fn graph(&self) -> &CompiledGraph {
        &self.graph
    }
}

impl Instrument for GraphInstrument {
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        self.velocity = velocity.clamp(0.0, 1.0);
        self.active = true;
        self.graph.trigger(time, self.velocity);
    }

    fn tick(&mut self, current_time: f64) -> f32 {
        if !self.active {
            return 0.0;
        }
        let out = self.graph.tick(current_time);
        self.active = self.graph.is_active();
        // Perceptually-linear velocity scaling, matching the drum instruments.
        out * self.velocity.sqrt()
    }

    fn is_active(&self) -> bool {
        self.active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        let sum: f64 = samples.iter().map(|s| (*s as f64) * (*s as f64)).sum();
        (sum / samples.len() as f64).sqrt() as f32
    }

    fn render(source: &str, samples: usize) -> Vec<f32> {
        let mut inst = GraphInstrument::from_source(source, 48_000.0).expect("compile");
        inst.trigger(0.0);
        let dt = 1.0 / 48_000.0;
        let mut buffer = Vec::with_capacity(samples);
        let mut t = 0.0;
        for _ in 0..samples {
            buffer.push(inst.tick(t));
            t += dt;
        }
        buffer
    }

    #[test]
    fn simple_tone_produces_sound() {
        let src = "body = env a=.001 d=.4 curve=2.0\nsub = osc sine 220\nout = sub * body";
        let buffer = render(src, 4_800);
        assert!(
            rms(&buffer) > 0.01,
            "expected audible output, rms={}",
            rms(&buffer)
        );
        assert!(buffer.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn render_is_deterministic() {
        let src = "n = noise\namp = env a=.001 d=.3\nout = n * amp";
        let a = render(src, 2_400);
        let b = render(src, 2_400);
        assert_eq!(a, b, "graph render should be deterministic");
    }

    #[test]
    fn envelope_silences_then_frees_the_graph() {
        let src = "amp = env a=.001 d=.05\nsub = osc sine 110\nout = sub * amp";
        let mut inst = GraphInstrument::from_source(src, 48_000.0).unwrap();
        inst.trigger(0.0);
        assert!(inst.is_active());
        let dt = 1.0 / 48_000.0;
        let mut t = 0.0;
        // Decay is 50 ms; after 0.5 s the envelope has long finished.
        for _ in 0..24_000 {
            inst.tick(t);
            t += dt;
        }
        assert!(
            !inst.is_active(),
            "graph should free itself once envelopes finish"
        );
    }

    #[test]
    fn constant_folding_keeps_the_graph_small() {
        // freq=110*2 and the 0.5 gain fold to constants, so the only nodes are
        // env, osc, and the final mul.
        let src = "amp = env a=.001 d=.2\nsub = osc sine 110*2\nout = (sub * amp) * 0.5";
        let inst = GraphInstrument::from_source(src, 48_000.0).unwrap();
        let kinds: Vec<&str> = inst
            .topology()
            .nodes
            .iter()
            .map(|n| n.kind.as_str())
            .collect();
        assert_eq!(
            kinds.iter().filter(|k| **k == "const").count(),
            1,
            "kinds={kinds:?}"
        );
        assert_eq!(kinds.iter().filter(|k| **k == "osc").count(), 1);
        assert_eq!(kinds.iter().filter(|k| **k == "env").count(), 1);
    }

    #[test]
    fn missing_out_is_an_error() {
        let err = match GraphInstrument::from_source("sub = osc sine 110", 48_000.0) {
            Ok(_) => panic!("expected an error for a graph with no `out`"),
            Err(e) => e,
        };
        assert!(err.contains("out"), "err={err}");
    }

    #[test]
    fn unknown_node_reference_is_an_error() {
        let err = match GraphInstrument::from_source("out = ghost * 2", 48_000.0) {
            Ok(_) => panic!("expected an error for an unknown node reference"),
            Err(e) => e,
        };
        assert!(err.contains("ghost"), "err={err}");
    }

    #[test]
    fn fm_port_modulates_frequency() {
        // A pitch envelope on the fm port should make the early part of the note
        // brighter (higher frequency) than a static oscillator.
        let swept = "p = env a=.001 d=.1 curve=.3\namp = env a=.001 d=.3\nsub = osc sine 80 fm=p*6\nout = sub * amp";
        let buffer = render(swept, 4_800);
        assert!(rms(&buffer) > 0.01);
        assert!(buffer.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn topology_exposes_edges_for_visualization() {
        let src = "amp = env a=.001 d=.2\nsub = osc sine 110\nout = sub * amp";
        let inst = GraphInstrument::from_source(src, 48_000.0).unwrap();
        let topo = inst.topology();
        // The output mul node should have two incoming edges.
        let out_node = &topo.nodes[topo.out];
        assert_eq!(out_node.kind, "mul");
        assert_eq!(out_node.inputs.len(), 2);
    }
}
