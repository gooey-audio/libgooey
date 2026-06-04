# Instrument Graph DSL and Node-Graph GUI


## Purpose and intent


Today every drum instrument in this repository (see `src/instruments/kick.rs`)
is a single large Rust struct whose oscillators, envelopes, filters, and
saturation are all wired together by hand inside one ~250-line `tick()` method.
That makes it slow and error-prone to answer a simple question: "which component
of this kick actually shapes which part of the sound?" You cannot mute one
oscillator, retune one layer, or see where each envelope sits in time without
editing and recompiling Rust.

This work introduces a small **domain-specific language (DSL)** — a tiny text
format — for describing an instrument as a **node graph**: a set of named,
interconnected building blocks (oscillators, envelopes, a noise source, filters,
and an opaque saturation "effect" block). Each line of the DSL is one node. The
DSL compiles to a runnable instrument that plays through the existing engine, and
exposes its structure (nodes and the edges between them) so a graphical user
interface (GUI) can draw the graph and, later, show where each envelope is active
on a shared time axis.

After this change a user can write a file like `examples/instruments/kick.graph`,
run `cargo run --example graph --features bounce -- examples/instruments/kick.graph`,
see the instrument's structure printed as a node list, and get a rendered
`kick.wav`. They can then edit one line — drop a layer, retune an oscillator,
change an envelope's decay — and immediately hear and see the difference, without
touching Rust. The second phase adds a window (built with the `glfw` windowing
library already used here, plus the `glow` OpenGL wrapper and hand-written
shaders) that draws the node graph as boxes and connections.


## What exists now (Phase 1, COMPLETE)


The node-graph engine lives in `src/graph/` and is pure Rust with no feature
flag, so it is always compiled and fully unit-testable in a headless container.

The term "node" means one processing unit that produces a single `f32` audio
sample per "tick" (one tick = one audio sample). Audio signals and control
signals (such as an envelope's 0..1 output) are both just `f32`, so anything can
modulate anything. The term "port" means a named signal input on a node (for
example an oscillator's `fm` port, or a filter's `in` and `cutoff` ports). The
term "edge" means a connection feeding one node's output into another node's
port.

The three files:

- `src/graph/node.rs` defines the `NodeImpl` trait (`tick`, `trigger`, `release`,
  `is_active`, `set_param`, `input_ports`) and the concrete node kinds, each of
  which reuses an existing DSP building block rather than re-implementing it:
  `osc` wraps `Oscillator`, `env` wraps `Envelope`, `noise` wraps `PinkNoise`,
  `lp`/`hp`/`bp` wrap the resonant/biquad filters, `shape` wraps
  `FeedbackWaveshaper` (the "opaque effect" that cannot be expressed in terms of
  oscillators and envelopes), plus `gain`, `const`, and `mul`/`add` arithmetic
  nodes. It also holds `kind_meta`, a per-kind table describing how positional
  and named arguments map to either baked scalar parameters or signal-input
  ports.

- `src/graph/parser.rs` turns DSL text into a `GraphSpec` (a flat list of
  `NodeSpec` plus the index of the `out` node). It tokenizes each right-hand
  side and runs a recursive-descent expression parser with **constant folding**:
  `110*2` becomes the constant `220` at parse time, and a constant only becomes a
  real `const` node when it actually feeds a signal input (for example the `0.12`
  in `clk * 0.12`). Declaration order is a valid evaluation order because a
  reference may only point at an earlier node; this both guarantees the graph is
  acyclic and means no separate topological sort is needed.

- `src/graph/mod.rs` compiles a `GraphSpec` into a `CompiledGraph` (the runnable
  form: `tick`, `trigger`, `release`, `is_active`, `last_outputs`, `set_param`)
  and wraps it in `GraphInstrument`, which implements the existing
  `crate::engine::Instrument` trait so a graph plays through `Engine`, the
  sequencer, and the offline `bounce` path with no special handling. It also
  exposes `Topology { nodes, out }` with `NodeInfo { name, kind, params, inputs,
  is_envelope }` — the render-free description the GUI will consume.

The DSL syntax, by example (`examples/instruments/kick.graph`):

    sr 48000

    pitch = env a=.001 d=.09 curve=.4
    body  = env a=.001 d=.45 curve=2.0
    sub   = osc sine 50 fm=pitch*5
    punch = osc tri 125 fm=pitch*5
    ctap  = env a=.001 d=.02 curve=.3
    click = hp (osc noise 2000) cutoff=7000 q=.6
    nz    = lp (noise) cutoff=2000 q=2.0
    voice = (sub + punch*.5 + click*ctap*.6 + nz*.15) * body
    out   = shape voice drive=.25

Rules a reader needs to know: a line is `name = <right-hand-side>` or a directive
(`sr <hz>`, informational only — the host sample rate wins at compile time).
The right-hand side is either a primitive constructor (it starts with a node-type
keyword such as `osc`, `env`, `lp`, `shape`) or an arithmetic expression over
previously declared node names. `*` and `+` (and `-`, and `/` by a constant)
build `mul`/`add` nodes; parentheses control precedence and also let a primitive
be used inside an expression, e.g. `lp (osc noise 2000) ...`. An oscillator may
take a bare waveform word (`sine`, `tri`, `square`, `saw`, `noise`) and a base
frequency in hertz; `fm=<signal>` multiplies its frequency by `(1 + signal)`,
which is how the kick sweeps pitch. A graph must define `out`. Node-type keywords
are reserved and cannot be used as node names. Comments start with `#`.

How to see it working:

    cargo test --no-default-features --lib graph
    cargo test --no-default-features --test graph_dsl
    cargo run --no-default-features --features bounce --example graph -- examples/instruments/kick.graph

The first two commands run the unit and integration tests (parser edge cases,
deterministic render, envelope-driven lifetime, engine integration, live
`set_param` changes, and that the reference kick graph compiles and makes
finite, audible, non-runaway audio). The third prints the kick's node topology to
the terminal and writes `examples/instruments/kick.wav`.

Note on building: the crate's default `native` feature pulls in CPAL, which needs
the system ALSA library. In a headless container without ALSA, build and test
with `--no-default-features` (optionally `--features bounce` for the example).
On a normal desktop, `cargo test` and `cargo run --example graph --features bounce`
work as written.


## Phase 2: the node-graph GUI (NEXT, not yet implemented)


Goal: a window that draws the compiled graph as labelled boxes (one per node)
with lines for the edges, so the signal and modulation routing is visible at a
glance. This is the user's chosen first visualization. It must be built with the
`glfw` windowing library (already a dependency, see `src/visualization/waveform_display.rs`
for the existing pattern) plus the `glow` OpenGL wrapper and hand-written GLSL
shaders. The container here is headless, so this phase is authored and
compiled here but visually verified by the user on their own machine.

Orientation for a newcomer: `glfw` opens a window and an OpenGL context and
delivers keyboard/mouse events; `glow` is a thin, safe-ish Rust wrapper over
OpenGL calls; a "shader" is a small program that runs on the GPU to color the
pixels of the triangles we submit. The existing `WaveformDisplay` in
`src/visualization/waveform_display.rs` already shows how to initialize glfw,
load OpenGL function pointers, compile a shader program, and upload vertex data —
read it first and mirror its structure. The only new dependency is `glow`.

Concrete steps:

1. Add `glow = "0.13"` to `Cargo.toml` under `[dependencies]` as `optional = true`,
   and extend the existing `visualization` feature so it also enables `glow`
   (the feature already enables `glfw`, `gl`, and `rustfft`). Create the GUI
   module behind `#[cfg(feature = "visualization")]` so default builds are
   unaffected.

2. Create `src/graph/view.rs` (gated on `visualization`) exporting a
   `GraphView` struct, mirroring `WaveformDisplay`: it owns the glfw window and
   event receiver, a `glow::Context` obtained via
   `glow::Context::from_loader_function(|s| window.get_proc_address(s) as *const _)`,
   and a compiled shader program plus vertex buffers. Give it
   `new(topology: &Topology, width, height) -> Result<Self, String>`,
   `should_close()`, and `render(&mut self, outputs: &[f32])`. Re-export it from
   `src/graph/mod.rs` under the same cfg.

3. Lay the graph out for drawing. A simple, dependency-free layout that reads
   well: assign each node a "column" equal to its longest path length from any
   source (a node with no inputs is column 0; otherwise one past the maximum
   column of its inputs). Within a column, stack nodes vertically in index order.
   Map columns to x and stack position to y in normalized device coordinates
   (-1..1). Compute this once in `GraphView::new` from the `Topology`. Store a
   `Vec` of node rectangles (center, half-size) keyed by node index so edges can
   be drawn between port positions.

4. Draw with two shader passes sharing one window. First pass: filled rectangles
   for nodes (two triangles each), colored by kind (for example envelopes one
   hue, oscillators another, the `shape` effect another) so categories are
   visible; tint or grow a node by its current value from `last_outputs()` passed
   into `render` so the graph "lights up" as it plays. Second pass: lines for
   edges from each source node's right edge to the destination node's left edge.
   Node labels (the node name and kind) can be deferred — either draw a tiny
   built-in bitmap font as textured quads, or, acceptable for a first cut, print
   the legend to stdout and rely on color plus position. Keep all GLSL inline as
   `&str` constants like the existing waveform shaders.

5. Add an example `examples/graph_view.rs` gated on
   `required-features = ["native", "visualization"]` that loads a `.graph` file,
   builds a `GraphInstrument`, opens a `GraphView` from its `topology()`, runs
   the audio engine (reuse `EngineOutput` from `src/engine/engine_output.rs`, as
   the other native examples do), and each frame calls
   `view.render(instrument_or_engine.last_outputs())` until the window closes.
   Re-trigger the instrument on a timer or on the space bar (mirror
   `DisplayEvent::SpacePressed` in the existing display) so the user can watch the
   graph light up on each hit.

6. Acceptance for Phase 2: on a desktop with a display,
   `cargo run --example graph_view --features native,visualization -- examples/instruments/kick.graph`
   opens a window showing one labelled/colored box per node with connecting
   lines, and pressing space triggers the kick so the nodes pulse in time with
   the sound. Add a headless compile check to CI-style validation:
   `cargo build --example graph_view --features native,visualization` (or at
   minimum `cargo build --features visualization` for the library module).


## Phase 3 and beyond (future, sketch only)


- **Envelope timeline**: a second panel plotting each `env` node's amplitude over
  the note's lifetime on a shared time axis (the original "time location of each
  envelope" request). The data is already available — sample each envelope node
  by ticking a throwaway `CompiledGraph` clone, or expose per-node output history
  from a render pass — then draw one polyline per envelope.
- **Per-node solo/mute**: add `mute(index)`/`solo(index)` to `CompiledGraph`
  (force a node's output to 0, or force all but one to 0) and bind them to clicks
  in the GUI, so a user can isolate one component's contribution by ear.
- **Live parameter editing**: `GraphInstrument::set_param(node, param, value)`
  already exists; wire GUI sliders or drag gestures to it.
- **Modulation as a first-class graph output**: optionally implement the
  `Modulatable` trait on `GraphInstrument` by exposing `node.param` paths so the
  engine's LFOs can target graph internals.


## Design decisions log


- The graph engine is a *new* module, separate from the existing `src/dsl.rs`.
  `src/dsl.rs` wires whole engine programs (which instruments, their sequencer
  patterns, LFOs, global effects) and treats instruments as black boxes; the new
  `src/graph/` describes a single instrument's *internals*. Conflating them would
  have muddied both. This was confirmed with the user ("Generic synth graph
  engine").
- Nodes reuse existing DSP building blocks instead of re-implementing them, so a
  graph-built kick shares the exact oscillator/filter/saturation behavior of the
  hand-written kick. The `osc` node holds its wrapped `Oscillator` at unity gain
  with an instant-attack, infinite-sustain internal envelope so it emits the raw
  waveform; amplitude shaping is always done by explicit `env` nodes. The kick
  sets `frequency_hz` per sample and relies on the same "instantaneous phase"
  behavior, so the graph's pitch sweeps match the kick's.
- Constant folding keeps the visible graph small and readable: arithmetic over
  literals never creates nodes, and a literal only becomes a `const` node when it
  feeds a real signal input. This keeps the eventual diagram uncluttered.
- No feature flag on the engine: it is pure Rust and always compiled so it is
  unit-testable headlessly. The GUI is the only feature-gated part, gated on the
  existing `visualization` feature (extended to also enable `glow`).
- GUI backend is `glfw` + `glow` + hand-written shaders, reusing the existing
  glfw setup in `src/visualization/`, per the user's choice. Sequencing is
  "engine first, GUI after" so Phase 1 is fully verifiable in this container.
- The first visualization is the node-graph *diagram* (user's choice). The
  envelope timeline and solo/mute are deferred to Phase 3.


## Progress


- [x] Node primitives and `NodeImpl` trait (`src/graph/node.rs`)
- [x] Constant-folding DSL parser → `GraphSpec` (`src/graph/parser.rs`)
- [x] `CompiledGraph` evaluator + `GraphInstrument` + `Topology` (`src/graph/mod.rs`)
- [x] `pub mod graph;` wired into `src/lib.rs`
- [x] Reference DSL files `examples/instruments/kick.graph` and `tone.graph`
- [x] `examples/graph.rs` (prints topology, bounces to WAV; `bounce` feature)
- [x] Unit tests (8, in `src/graph/mod.rs`) and integration tests (9, `tests/graph_dsl.rs`)
- [x] `cargo fmt` clean; `cargo clippy` clean for all new files; full suite green (`--no-default-features --features bounce`)
- [x] Docs: AGENTS.md module map + "Two DSLs" note; this plan
- [ ] Phase 2: `glow` dependency + `visualization`-gated `src/graph/view.rs` (`GraphView`)
- [ ] Phase 2: column-based layout from `Topology`; node-box + edge shader passes
- [ ] Phase 2: `examples/graph_view.rs` with live audio + space-to-trigger
- [ ] Phase 2: node labels (bitmap font or deferred legend)
- [ ] Phase 3: envelope timeline panel
- [ ] Phase 3: per-node solo/mute in `CompiledGraph` + GUI binding
