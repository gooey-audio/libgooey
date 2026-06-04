//! Runtime node primitives for the instrument graph engine.
//!
//! Each node produces one `f32` per sample tick. Audio-rate signals and
//! control-rate signals (envelopes, constants) are unified as plain `f32`, so
//! anything can modulate anything. Nodes reuse the existing DSP building blocks
//! (`Oscillator`, `Envelope`, `PinkNoise`, the resonant filters, and
//! `FeedbackWaveshaper`) rather than re-implementing them, which is the whole
//! point: an instrument becomes a visible wiring of the same parts the
//! hand-written instruments use internally.

use std::collections::HashMap;

use crate::effects::feedback_waveshaper::FeedbackWaveshaper;
use crate::envelope::{ADSRConfig, Envelope, EnvelopeCurve};
use crate::filters::{BiquadBandpass, ResonantHighpassFilter, ResonantLowpassFilter};
use crate::gen::oscillator::Oscillator;
use crate::gen::pink_noise::PinkNoise;
use crate::gen::waveform::Waveform;

/// A single processing node in a compiled instrument graph.
///
/// The evaluator calls [`NodeImpl::tick`] once per sample in dependency order,
/// passing the already-computed outputs of this node's connected inputs. Inputs
/// are aligned to [`NodeImpl::input_ports`]: `inputs[i]` is `Some(value)` when
/// the port named `input_ports()[i]` is connected, or `None` when it is left at
/// its scalar default.
pub trait NodeImpl: Send {
    /// Names of the signal-input ports this node accepts, in a fixed order.
    fn input_ports(&self) -> &'static [&'static str];

    /// Produce one sample. `inputs` is aligned to [`input_ports`].
    fn tick(&mut self, time: f64, inputs: &[Option<f32>]) -> f32;

    /// Reset internal state and (re)start envelopes for a new note.
    fn trigger(&mut self, _time: f64, _velocity: f32) {}

    /// Begin the release phase of any envelope this node owns.
    fn release(&mut self, _time: f64) {}

    /// Whether this node is an amplitude/modulation envelope. A graph is
    /// considered "still sounding" while any of its envelope nodes is active.
    fn is_envelope(&self) -> bool {
        false
    }

    /// For envelope nodes: whether the envelope is still producing output.
    /// Non-envelope nodes report `true` and never constrain graph lifetime.
    fn is_active(&self) -> bool {
        true
    }

    /// Set a scalar parameter by name at runtime (used by the GUI / live
    /// tweaking). Unknown parameters return an error.
    fn set_param(&mut self, name: &str, value: f32) -> Result<(), String>;
}

/// Names of the node-type keywords the DSL understands. Used by the parser to
/// decide whether a right-hand side is a primitive constructor or an
/// expression, and to give "unknown node type" errors.
pub const NODE_KEYWORDS: &[&str] = &[
    "osc", "env", "noise", "lp", "hp", "bp", "shape", "gain", "const", "mul", "add",
];

/// Classification of a named or positional argument for a node kind. The parser
/// uses this to decide whether a value becomes a baked-in scalar parameter, an
/// optionally-modulatable parameter, or a signal-input connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// Scalar only. A node reference here is an error (cannot be modulated).
    Param(&'static str),
    /// Scalar default, but a node reference connects a modulation signal.
    Dual(&'static str),
    /// Pure signal input. A constant here is wrapped in a `const` node.
    Port(&'static str),
}

/// Per-kind argument layout: positional slots in order, named slots, and
/// whether a bare waveform word (e.g. `sine`) is accepted.
pub struct KindMeta {
    pub positionals: &'static [Slot],
    pub named: &'static [(&'static str, Slot)],
    pub allow_wave: bool,
}

/// Look up the argument layout for a node-type keyword. Returns `None` for
/// unknown kinds.
pub fn kind_meta(kind: &str) -> Option<KindMeta> {
    use Slot::*;
    let meta = match kind {
        "osc" => KindMeta {
            positionals: &[Dual("freq")],
            named: &[("freq", Dual("freq")), ("fm", Port("fm"))],
            allow_wave: true,
        },
        "env" => KindMeta {
            positionals: &[],
            named: &[
                ("a", Param("a")),
                ("d", Param("d")),
                ("s", Param("s")),
                ("r", Param("r")),
                ("curve", Param("curve")),
                ("acurve", Param("acurve")),
            ],
            allow_wave: false,
        },
        "noise" => KindMeta {
            positionals: &[],
            named: &[],
            allow_wave: false,
        },
        "lp" | "hp" | "bp" => KindMeta {
            positionals: &[Port("in"), Dual("cutoff"), Param("q")],
            named: &[
                ("in", Port("in")),
                ("cutoff", Dual("cutoff")),
                ("q", Param("q")),
            ],
            allow_wave: false,
        },
        "shape" => KindMeta {
            positionals: &[Port("in"), Param("drive")],
            named: &[
                ("in", Port("in")),
                ("drive", Param("drive")),
                ("fb", Param("fb")),
                ("fbcutoff", Param("fbcutoff")),
                ("mix", Param("mix")),
            ],
            allow_wave: false,
        },
        "gain" => KindMeta {
            positionals: &[Port("in"), Dual("amt")],
            named: &[("in", Port("in")), ("amt", Dual("amt"))],
            allow_wave: false,
        },
        "const" => KindMeta {
            positionals: &[Param("value")],
            named: &[("value", Param("value"))],
            allow_wave: false,
        },
        "mul" => KindMeta {
            positionals: &[Port("a"), Port("b")],
            named: &[("a", Port("a")), ("b", Port("b"))],
            allow_wave: false,
        },
        "add" => KindMeta {
            positionals: &[Port("a"), Port("b")],
            named: &[("a", Port("a")), ("b", Port("b"))],
            allow_wave: false,
        },
        _ => return None,
    };
    Some(meta)
}

/// Parse a waveform keyword. Returns `None` if the word is not a waveform.
pub fn parse_waveform(word: &str) -> Option<Waveform> {
    Some(match word {
        "sine" | "sin" => Waveform::Sine,
        "square" | "sq" => Waveform::Square,
        "saw" => Waveform::Saw,
        "tri" | "triangle" => Waveform::Triangle,
        "noise" | "white" => Waveform::Noise,
        _ => return None,
    })
}

/// Build a runtime node from a kind keyword, its baked scalar parameters, an
/// optional waveform, and the sample rate. Called once at compile time.
pub fn build_node(
    kind: &str,
    params: &HashMap<String, f32>,
    waveform: Option<Waveform>,
    sample_rate: f32,
) -> Result<Box<dyn NodeImpl>, String> {
    let p = |name: &str, default: f32| params.get(name).copied().unwrap_or(default);
    let node: Box<dyn NodeImpl> = match kind {
        "osc" => Box::new(OscNode::new(
            sample_rate,
            waveform.unwrap_or(Waveform::Sine),
            p("freq", 110.0),
        )),
        "env" => Box::new(EnvNode::new(
            p("a", 0.001),
            p("d", 0.3),
            p("s", 0.0),
            p("r", 0.01),
            p("curve", 1.0),
            p("acurve", 1.0),
        )),
        "noise" => Box::new(NoiseNode::new(sample_rate)),
        "lp" => Box::new(FilterNode::lowpass(
            sample_rate,
            p("cutoff", 2000.0),
            p("q", 0.707),
        )),
        "hp" => Box::new(FilterNode::highpass(
            sample_rate,
            p("cutoff", 2000.0),
            p("q", 0.707),
        )),
        "bp" => Box::new(FilterNode::bandpass(
            sample_rate,
            p("cutoff", 2000.0),
            p("q", 1.0),
        )),
        "shape" => Box::new(ShapeNode::new(
            sample_rate,
            p("drive", 0.2),
            p("fb", 0.0),
            p("fbcutoff", 2000.0),
            p("mix", 1.0),
        )),
        "gain" => Box::new(GainNode::new(p("amt", 1.0))),
        "const" => Box::new(ConstNode::new(p("value", 0.0))),
        "mul" => Box::new(BinOpNode::new(BinOp::Mul)),
        "add" => Box::new(BinOpNode::new(BinOp::Add)),
        other => return Err(format!("unknown node type '{other}'")),
    };
    Ok(node)
}

/// Map a normalized 0-1 drive amount to a `FeedbackWaveshaper` drive value with
/// a gentle cubic curve, matching the kick drum's mapping so `shape` sounds the
/// same as the kick's built-in overdrive.
fn drive_to_waveshaper(drive: f32) -> f32 {
    let d = drive.clamp(0.0, 1.0);
    1.0 + d * d * d * 40.0
}

/// Build an `EnvelopeCurve` from a raw exponent, treating ~1.0 as linear.
fn curve_from_exponent(exponent: f32) -> EnvelopeCurve {
    if (exponent - 1.0).abs() < 0.01 {
        EnvelopeCurve::Linear
    } else {
        EnvelopeCurve::Exponential(exponent.clamp(0.1, 10.0))
    }
}

// ---------------------------------------------------------------------------
// Oscillator node
// ---------------------------------------------------------------------------

/// A raw waveform oscillator. The wrapped [`Oscillator`] is held at unity gain
/// with an instant-attack, infinite-sustain envelope so its output is the bare
/// waveform; amplitude shaping is done by separate `env` nodes. The `freq` port
/// (if connected) overrides the base frequency in Hz; the `fm` port multiplies
/// frequency by `(1 + fm)`, matching how the kick sweeps pitch.
pub struct OscNode {
    osc: Oscillator,
    base_freq: f32,
}

impl OscNode {
    pub fn new(sample_rate: f32, waveform: Waveform, base_freq: f32) -> Self {
        let mut osc = Oscillator::new(sample_rate, base_freq.max(0.0));
        osc.waveform = waveform;
        osc.set_volume(1.0);
        // Instant attack, full sustain, long decay: the internal envelope stays
        // pinned at 1.0 so the node emits the raw waveform.
        osc.set_adsr(ADSRConfig::new(0.001, 1000.0, 1.0, 0.001));
        Self { osc, base_freq }
    }
}

impl NodeImpl for OscNode {
    fn input_ports(&self) -> &'static [&'static str] {
        &["freq", "fm"]
    }

    fn tick(&mut self, time: f64, inputs: &[Option<f32>]) -> f32 {
        let base = inputs[0].unwrap_or(self.base_freq);
        let fm = inputs[1].unwrap_or(0.0);
        self.osc.frequency_hz = (base * (1.0 + fm)).max(0.0);
        self.osc.tick(time)
    }

    fn trigger(&mut self, time: f64, _velocity: f32) {
        self.osc.trigger(time);
    }

    fn set_param(&mut self, name: &str, value: f32) -> Result<(), String> {
        match name {
            "freq" => {
                self.base_freq = value.max(0.0);
                Ok(())
            }
            other => Err(format!("osc has no parameter '{other}'")),
        }
    }
}

// ---------------------------------------------------------------------------
// Envelope node
// ---------------------------------------------------------------------------

/// An ADSR envelope, output 0..1. With the default sustain of 0 it behaves like
/// a one-shot percussion envelope (auto-releases after decay), which is what
/// drums want. `curve` is the decay exponent (`<1` = punchy fast-then-slow,
/// `>1` = soft slow-then-fast, `1` = linear).
pub struct EnvNode {
    env: Envelope,
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    decay_curve: f32,
    attack_curve: f32,
}

impl EnvNode {
    pub fn new(
        attack: f32,
        decay: f32,
        sustain: f32,
        release: f32,
        decay_curve: f32,
        attack_curve: f32,
    ) -> Self {
        Self {
            env: Envelope::new(),
            attack: attack.max(0.0),
            decay: decay.max(0.0),
            sustain: sustain.clamp(0.0, 1.0),
            release: release.max(0.0),
            decay_curve,
            attack_curve,
        }
    }

    fn config(&self) -> ADSRConfig {
        ADSRConfig::new(self.attack, self.decay, self.sustain, self.release)
            .with_attack_curve(curve_from_exponent(self.attack_curve))
            .with_decay_curve(curve_from_exponent(self.decay_curve))
    }
}

impl NodeImpl for EnvNode {
    fn input_ports(&self) -> &'static [&'static str] {
        &[]
    }

    fn tick(&mut self, time: f64, _inputs: &[Option<f32>]) -> f32 {
        self.env.get_amplitude(time)
    }

    fn trigger(&mut self, time: f64, _velocity: f32) {
        self.env.set_config(self.config());
        self.env.trigger(time);
    }

    fn release(&mut self, time: f64) {
        self.env.release(time);
    }

    fn is_envelope(&self) -> bool {
        true
    }

    fn is_active(&self) -> bool {
        self.env.is_active
    }

    fn set_param(&mut self, name: &str, value: f32) -> Result<(), String> {
        match name {
            "a" => self.attack = value.max(0.0),
            "d" => self.decay = value.max(0.0),
            "s" => self.sustain = value.clamp(0.0, 1.0),
            "r" => self.release = value.max(0.0),
            "curve" => self.decay_curve = value,
            "acurve" => self.attack_curve = value,
            other => return Err(format!("env has no parameter '{other}'")),
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Pink noise node
// ---------------------------------------------------------------------------

/// A pink (1/f) noise source. White noise is available via `osc noise`.
pub struct NoiseNode {
    pink: PinkNoise,
}

impl NoiseNode {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            pink: PinkNoise::new(sample_rate),
        }
    }
}

impl NodeImpl for NoiseNode {
    fn input_ports(&self) -> &'static [&'static str] {
        &[]
    }

    fn tick(&mut self, _time: f64, _inputs: &[Option<f32>]) -> f32 {
        self.pink.tick()
    }

    fn trigger(&mut self, _time: f64, _velocity: f32) {
        self.pink.reset();
    }

    fn set_param(&mut self, name: &str, _value: f32) -> Result<(), String> {
        Err(format!("noise has no parameter '{name}'"))
    }
}

// ---------------------------------------------------------------------------
// Filter node
// ---------------------------------------------------------------------------

enum FilterKind {
    Lowpass(ResonantLowpassFilter),
    Highpass(ResonantHighpassFilter),
    Bandpass(BiquadBandpass),
}

/// A resonant filter over its `in` port. The `cutoff` port (if connected)
/// modulates cutoff frequency in Hz per sample; otherwise the baked `cutoff`
/// parameter is used.
pub struct FilterNode {
    filter: FilterKind,
    cutoff: f32,
    q: f32,
}

impl FilterNode {
    pub fn lowpass(sample_rate: f32, cutoff: f32, q: f32) -> Self {
        Self {
            filter: FilterKind::Lowpass(ResonantLowpassFilter::new(sample_rate, cutoff, q)),
            cutoff,
            q,
        }
    }

    pub fn highpass(sample_rate: f32, cutoff: f32, q: f32) -> Self {
        Self {
            filter: FilterKind::Highpass(ResonantHighpassFilter::new(sample_rate, cutoff, q)),
            cutoff,
            q,
        }
    }

    pub fn bandpass(sample_rate: f32, cutoff: f32, q: f32) -> Self {
        let mut bp = BiquadBandpass::new(sample_rate);
        bp.set_params(cutoff, q, 1.0);
        Self {
            filter: FilterKind::Bandpass(bp),
            cutoff,
            q,
        }
    }
}

impl NodeImpl for FilterNode {
    fn input_ports(&self) -> &'static [&'static str] {
        &["in", "cutoff"]
    }

    fn tick(&mut self, _time: f64, inputs: &[Option<f32>]) -> f32 {
        let input = inputs[0].unwrap_or(0.0);
        let cutoff = inputs[1].unwrap_or(self.cutoff);
        match &mut self.filter {
            FilterKind::Lowpass(f) => {
                f.set_params(cutoff, self.q);
                f.process(input)
            }
            FilterKind::Highpass(f) => {
                f.set_cutoff_freq(cutoff);
                f.set_resonance(self.q);
                f.process(input)
            }
            FilterKind::Bandpass(f) => {
                f.set_params(cutoff, self.q, 1.0);
                f.process(input)
            }
        }
    }

    fn trigger(&mut self, _time: f64, _velocity: f32) {
        match &mut self.filter {
            FilterKind::Lowpass(f) => f.reset(),
            FilterKind::Highpass(f) => f.reset(),
            FilterKind::Bandpass(f) => f.reset(),
        }
    }

    fn set_param(&mut self, name: &str, value: f32) -> Result<(), String> {
        match name {
            "cutoff" => self.cutoff = value,
            "q" => self.q = value,
            other => return Err(format!("filter has no parameter '{other}'")),
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Shape (waveshaper / saturation) node — the "opaque effect" building block
// ---------------------------------------------------------------------------

/// A feedback waveshaper / saturator over its `in` port. This is the canonical
/// "effect that can't be expressed in terms of the other primitives" — it wraps
/// `FeedbackWaveshaper` as an opaque block. `drive` is 0..1 (cubic-mapped like
/// the kick); `fb` is the feedback amount; `fbcutoff` is the feedback filter in
/// Hz; `mix` is the dry/wet mix.
pub struct ShapeNode {
    shaper: FeedbackWaveshaper,
}

impl ShapeNode {
    pub fn new(sample_rate: f32, drive: f32, fb: f32, fbcutoff: f32, mix: f32) -> Self {
        Self {
            shaper: FeedbackWaveshaper::new(
                sample_rate,
                drive_to_waveshaper(drive),
                (fb.clamp(0.0, 1.0) * 0.98).clamp(0.0, 0.98),
                fbcutoff,
                mix.clamp(0.0, 1.0),
            ),
        }
    }
}

impl NodeImpl for ShapeNode {
    fn input_ports(&self) -> &'static [&'static str] {
        &["in"]
    }

    fn tick(&mut self, _time: f64, inputs: &[Option<f32>]) -> f32 {
        self.shaper.process(inputs[0].unwrap_or(0.0))
    }

    fn trigger(&mut self, _time: f64, _velocity: f32) {
        self.shaper.reset();
    }

    fn set_param(&mut self, name: &str, value: f32) -> Result<(), String> {
        match name {
            "drive" => self.shaper.set_drive(drive_to_waveshaper(value)),
            "fb" => self
                .shaper
                .set_feedback((value.clamp(0.0, 1.0) * 0.98).clamp(0.0, 0.98)),
            "fbcutoff" => self.shaper.set_filter_cutoff(value),
            "mix" => self.shaper.set_mix(value.clamp(0.0, 1.0)),
            other => return Err(format!("shape has no parameter '{other}'")),
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Gain node
// ---------------------------------------------------------------------------

/// Scales its `in` port by `amt`. The `amt` port, if connected, modulates the
/// gain per sample.
pub struct GainNode {
    amt: f32,
}

impl GainNode {
    pub fn new(amt: f32) -> Self {
        Self { amt }
    }
}

impl NodeImpl for GainNode {
    fn input_ports(&self) -> &'static [&'static str] {
        &["in", "amt"]
    }

    fn tick(&mut self, _time: f64, inputs: &[Option<f32>]) -> f32 {
        let input = inputs[0].unwrap_or(0.0);
        let amt = inputs[1].unwrap_or(self.amt);
        input * amt
    }

    fn set_param(&mut self, name: &str, value: f32) -> Result<(), String> {
        match name {
            "amt" => {
                self.amt = value;
                Ok(())
            }
            other => Err(format!("gain has no parameter '{other}'")),
        }
    }
}

// ---------------------------------------------------------------------------
// Constant node
// ---------------------------------------------------------------------------

/// Emits a constant value every sample. Produced automatically when a constant
/// is used where a signal is required (for example `clk * 0.12`).
pub struct ConstNode {
    value: f32,
}

impl ConstNode {
    pub fn new(value: f32) -> Self {
        Self { value }
    }
}

impl NodeImpl for ConstNode {
    fn input_ports(&self) -> &'static [&'static str] {
        &[]
    }

    fn tick(&mut self, _time: f64, _inputs: &[Option<f32>]) -> f32 {
        self.value
    }

    fn set_param(&mut self, name: &str, value: f32) -> Result<(), String> {
        match name {
            "value" => {
                self.value = value;
                Ok(())
            }
            other => Err(format!("const has no parameter '{other}'")),
        }
    }
}

// ---------------------------------------------------------------------------
// Binary arithmetic node (produced by `*` and `+`)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum BinOp {
    Mul,
    Add,
}

/// Combines its `a` and `b` ports with multiply or add. These are created
/// implicitly by the `*` and `+` operators in the DSL.
pub struct BinOpNode {
    op: BinOp,
}

impl BinOpNode {
    fn new(op: BinOp) -> Self {
        Self { op }
    }
}

impl NodeImpl for BinOpNode {
    fn input_ports(&self) -> &'static [&'static str] {
        &["a", "b"]
    }

    fn tick(&mut self, _time: f64, inputs: &[Option<f32>]) -> f32 {
        match self.op {
            BinOp::Mul => inputs[0].unwrap_or(1.0) * inputs[1].unwrap_or(1.0),
            BinOp::Add => inputs[0].unwrap_or(0.0) + inputs[1].unwrap_or(0.0),
        }
    }

    fn set_param(&mut self, name: &str, _value: f32) -> Result<(), String> {
        Err(format!("arithmetic node has no parameter '{name}'"))
    }
}
