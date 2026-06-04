//! Parser for the instrument-graph DSL.
//!
//! The DSL is line-oriented and deliberately low-noise. Each line is either a
//! directive or an assignment `name = <rhs>`, where the right-hand side is
//! either a primitive constructor (`osc sine 55`, `env a=.001 d=.5`, `lp ...`)
//! or an arithmetic expression over previously declared nodes (`sub * amp`,
//! `(a + b) * c`). Operators `*`, `+`, `-` build `mul`/`add` nodes; bare numbers
//! and constant sub-expressions are folded at parse time and only become `const`
//! nodes when they actually feed a signal input. Comments start with `#`.
//!
//! A graph must define `out`, the final output node. An optional `sr <hz>`
//! directive sets the sample rate the graph was authored for (informational; the
//! host sample rate wins when the graph is compiled).
//!
//! Example:
//!
//! ```text
//! sr 48000
//! pitch = env a=.001 d=.16 curve=.4
//! body  = env a=.001 d=.5  curve=2.0
//! sub   = osc sine 55 fm=pitch*5
//! clk   = hp (osc noise 2000) cutoff=7000 q=.6
//! voice = (sub + clk*.12) * body
//! out   = shape voice drive=.25
//! ```

use std::collections::HashMap;

use crate::gen::waveform::Waveform;

use super::node::{kind_meta, parse_waveform, Slot, NODE_KEYWORDS};

/// A parsed node, before compilation into runnable form. `connections` maps a
/// signal-input port name to the index of the node feeding it.
#[derive(Clone, Debug)]
pub struct NodeSpec {
    pub name: String,
    pub kind: String,
    pub waveform: Option<Waveform>,
    pub params: HashMap<String, f32>,
    pub connections: Vec<(String, usize)>,
}

/// A fully parsed graph: a flat list of nodes (declaration order is a valid
/// evaluation order, because references may only point at earlier nodes), plus
/// the index of the `out` node.
#[derive(Clone, Debug)]
pub struct GraphSpec {
    pub nodes: Vec<NodeSpec>,
    pub out: usize,
    pub sample_rate: Option<f32>,
}

impl GraphSpec {
    /// Parse DSL source into a [`GraphSpec`]. Returns a human-readable error
    /// (with a line number) on the first problem encountered.
    pub fn parse(source: &str) -> Result<Self, String> {
        let mut builder = Builder::new();

        for (i, raw_line) in source.lines().enumerate() {
            let line_number = i + 1;
            let line = strip_comment(raw_line).trim();
            if line.is_empty() {
                continue;
            }

            match line.split_once('=') {
                Some((lhs, rhs)) => builder.assignment(line_number, lhs.trim(), rhs.trim())?,
                None => builder.directive(line_number, line)?,
            }
        }

        let out = builder
            .out
            .ok_or_else(|| "graph has no `out` node; add a line like `out = ...`".to_string())?;

        Ok(GraphSpec {
            nodes: builder.nodes,
            out,
            sample_rate: builder.sample_rate,
        })
    }
}

fn strip_comment(line: &str) -> &str {
    line.split_once('#').map(|(head, _)| head).unwrap_or(line)
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Accumulates parsed nodes and the name table as statements are processed.
struct Builder {
    nodes: Vec<NodeSpec>,
    names: HashMap<String, usize>,
    sample_rate: Option<f32>,
    out: Option<usize>,
    anon_counter: usize,
}

impl Builder {
    fn new() -> Self {
        Self {
            nodes: Vec::new(),
            names: HashMap::new(),
            sample_rate: None,
            out: None,
            anon_counter: 0,
        }
    }

    fn directive(&mut self, line: usize, text: &str) -> Result<(), String> {
        let mut tokens = text.split_whitespace();
        let name = tokens.next().unwrap_or("");
        match name {
            "sr" => {
                let value = tokens
                    .next()
                    .ok_or_else(|| format!("line {line}: `sr` expects a sample rate"))?;
                let hz = value
                    .parse::<f32>()
                    .map_err(|_| format!("line {line}: invalid sample rate '{value}'"))?;
                self.sample_rate = Some(hz);
                Ok(())
            }
            other => Err(format!(
                "line {line}: expected `name = ...` or a directive, got '{other}'"
            )),
        }
    }

    fn assignment(&mut self, line: usize, name: &str, rhs: &str) -> Result<(), String> {
        if name.is_empty() || !name.chars().next().is_some_and(is_ident_start) {
            return Err(format!("line {line}: invalid node name '{name}'"));
        }
        if !name.chars().all(is_ident_char) {
            return Err(format!("line {line}: invalid node name '{name}'"));
        }
        if NODE_KEYWORDS.contains(&name) {
            return Err(format!(
                "line {line}: '{name}' is a reserved node-type keyword and cannot be a node name"
            ));
        }

        let tokens = tokenize(line, rhs)?;
        if tokens.is_empty() {
            return Err(format!("line {line}: '{name}' has an empty definition"));
        }

        let start = self.nodes.len();
        let mut parser = Parser {
            tokens: &tokens,
            pos: 0,
            builder: self,
            line,
        };
        let ev = parser.parse_rhs()?;
        if parser.pos != parser.tokens.len() {
            return Err(format!(
                "line {line}: unexpected trailing tokens after definition of '{name}'"
            ));
        }

        let idx = self.materialize(ev);
        // If the root node was freshly created in this statement, give it the
        // user's name; otherwise this is an alias to an existing node.
        if idx >= start {
            self.nodes[idx].name = name.to_string();
        }
        self.names.insert(name.to_string(), idx);
        if name == "out" {
            self.out = Some(idx);
        }
        Ok(())
    }

    /// Turn an evaluation result into a concrete node index, creating a `const`
    /// node for bare constants.
    fn materialize(&mut self, ev: Ev) -> usize {
        match ev {
            Ev::Node(idx) => idx,
            Ev::Const(value) => self.push_const(value),
        }
    }

    fn push_const(&mut self, value: f32) -> usize {
        let mut params = HashMap::new();
        params.insert("value".to_string(), value);
        let name = self.anon_name("const");
        self.push(NodeSpec {
            name,
            kind: "const".to_string(),
            waveform: None,
            params,
            connections: Vec::new(),
        })
    }

    fn push(&mut self, node: NodeSpec) -> usize {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    fn anon_name(&mut self, kind: &str) -> String {
        let n = self.anon_counter;
        self.anon_counter += 1;
        format!("{kind}#{n}")
    }
}

/// Result of evaluating a sub-expression: either a compile-time constant (folded)
/// or a reference to a node in the graph.
#[derive(Clone, Copy, Debug)]
enum Ev {
    Const(f32),
    Node(usize),
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Num(f32),
    Eq,
    Star,
    Plus,
    Minus,
    Slash,
    LParen,
    RParen,
}

fn tokenize(line: usize, src: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        match c {
            '=' => {
                tokens.push(Tok::Eq);
                i += 1;
            }
            '*' => {
                tokens.push(Tok::Star);
                i += 1;
            }
            '+' => {
                tokens.push(Tok::Plus);
                i += 1;
            }
            '-' => {
                tokens.push(Tok::Minus);
                i += 1;
            }
            '/' => {
                tokens.push(Tok::Slash);
                i += 1;
            }
            '(' => {
                tokens.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Tok::RParen);
                i += 1;
            }
            _ if is_ident_start(c) => {
                let start = i;
                while i < chars.len() && is_ident_char(chars[i]) {
                    i += 1;
                }
                tokens.push(Tok::Ident(chars[start..i].iter().collect()));
            }
            _ if c.is_ascii_digit() || c == '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                let value = text
                    .parse::<f32>()
                    .map_err(|_| format!("line {line}: invalid number '{text}'"))?;
                tokens.push(Tok::Num(value));
            }
            other => {
                return Err(format!("line {line}: unexpected character '{other}'"));
            }
        }
    }
    Ok(tokens)
}

struct Parser<'a> {
    tokens: &'a [Tok],
    pos: usize,
    builder: &'a mut Builder,
    line: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }

    fn next_is_eq(&self) -> bool {
        matches!(self.tokens.get(self.pos + 1), Some(Tok::Eq))
    }

    fn bump(&mut self) -> Option<Tok> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn expect(&mut self, tok: Tok, what: &str) -> Result<(), String> {
        if self.peek() == Some(&tok) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!("line {}: expected {what}", self.line))
        }
    }

    /// A right-hand side: a primitive constructor (if it starts with a node-type
    /// keyword) or an arithmetic expression.
    fn parse_rhs(&mut self) -> Result<Ev, String> {
        if let Some(Tok::Ident(word)) = self.peek() {
            if kind_meta(word).is_some() && !self.next_is_eq() {
                return self.parse_primitive();
            }
        }
        self.parse_expr()
    }

    fn parse_primitive(&mut self) -> Result<Ev, String> {
        let kind = match self.bump() {
            Some(Tok::Ident(w)) => w,
            _ => unreachable!("parse_primitive entered without an identifier"),
        };
        let meta = kind_meta(&kind)
            .ok_or_else(|| format!("line {}: unknown node type '{kind}'", self.line))?;

        let mut params: HashMap<String, f32> = HashMap::new();
        let mut connections: Vec<(String, usize)> = Vec::new();
        let mut waveform: Option<Waveform> = None;
        let mut positional_index = 0usize;

        loop {
            match self.peek() {
                None | Some(Tok::RParen) => break,
                Some(Tok::Ident(word)) if self.next_is_eq() => {
                    let key = word.clone();
                    self.pos += 1; // identifier
                    self.pos += 1; // '='
                    let slot = meta
                        .named
                        .iter()
                        .find(|(name, _)| *name == key)
                        .map(|(_, slot)| *slot)
                        .ok_or_else(|| {
                            format!("line {}: '{kind}' has no argument '{key}'", self.line)
                        })?;
                    let ev = self.parse_expr()?;
                    self.assign_slot(slot, ev, &mut params, &mut connections)?;
                }
                Some(Tok::Ident(word)) if meta.allow_wave && parse_waveform(word).is_some() => {
                    waveform = parse_waveform(word);
                    self.pos += 1;
                }
                _ => {
                    let slot =
                        meta.positionals
                            .get(positional_index)
                            .copied()
                            .ok_or_else(|| {
                                format!(
                                    "line {}: too many positional arguments for '{kind}'",
                                    self.line
                                )
                            })?;
                    positional_index += 1;
                    let ev = self.parse_expr()?;
                    self.assign_slot(slot, ev, &mut params, &mut connections)?;
                }
            }
        }

        let name = self.builder.anon_name(&kind);
        let idx = self.builder.push(NodeSpec {
            name,
            kind,
            waveform,
            params,
            connections,
        });
        Ok(Ev::Node(idx))
    }

    fn assign_slot(
        &mut self,
        slot: Slot,
        ev: Ev,
        params: &mut HashMap<String, f32>,
        connections: &mut Vec<(String, usize)>,
    ) -> Result<(), String> {
        match slot {
            Slot::Param(name) => match ev {
                Ev::Const(c) => {
                    params.insert(name.to_string(), c);
                    Ok(())
                }
                Ev::Node(_) => Err(format!(
                    "line {}: parameter '{name}' must be a constant; it cannot be driven by a signal",
                    self.line
                )),
            },
            Slot::Dual(name) => {
                match ev {
                    Ev::Const(c) => {
                        params.insert(name.to_string(), c);
                    }
                    Ev::Node(idx) => connections.push((name.to_string(), idx)),
                }
                Ok(())
            }
            Slot::Port(name) => {
                let idx = self.builder.materialize(ev);
                connections.push((name.to_string(), idx));
                Ok(())
            }
        }
    }

    fn parse_expr(&mut self) -> Result<Ev, String> {
        let mut left = self.parse_mul()?;
        loop {
            match self.peek() {
                Some(Tok::Plus) => {
                    self.pos += 1;
                    let right = self.parse_mul()?;
                    left = self.combine(BinKind::Add, left, right);
                }
                Some(Tok::Minus) => {
                    self.pos += 1;
                    let right = self.parse_mul()?;
                    let negated = self.negate(right);
                    left = self.combine(BinKind::Add, left, negated);
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<Ev, String> {
        let mut left = self.parse_unary()?;
        loop {
            match self.peek() {
                Some(Tok::Star) => {
                    self.pos += 1;
                    let right = self.parse_unary()?;
                    left = self.combine(BinKind::Mul, left, right);
                }
                Some(Tok::Slash) => {
                    self.pos += 1;
                    let right = self.parse_unary()?;
                    left = self.divide(left, right)?;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Ev, String> {
        if self.peek() == Some(&Tok::Minus) {
            self.pos += 1;
            let inner = self.parse_unary()?;
            return Ok(self.negate(inner));
        }
        self.parse_atom()
    }

    fn parse_atom(&mut self) -> Result<Ev, String> {
        match self.bump() {
            Some(Tok::Num(n)) => Ok(Ev::Const(n)),
            Some(Tok::LParen) => {
                let ev = self.parse_rhs()?;
                self.expect(Tok::RParen, "')'")?;
                Ok(ev)
            }
            Some(Tok::Ident(word)) => {
                if kind_meta(&word).is_some() {
                    return Err(format!(
                        "line {}: wrap the '{word}' primitive in parentheses to use it inside an expression",
                        self.line
                    ));
                }
                let idx = *self
                    .builder
                    .names
                    .get(&word)
                    .ok_or_else(|| format!("line {}: unknown node '{word}'", self.line))?;
                Ok(Ev::Node(idx))
            }
            other => Err(format!(
                "line {}: expected a value but found {}",
                self.line,
                describe(other.as_ref())
            )),
        }
    }

    fn combine(&mut self, kind: BinKind, left: Ev, right: Ev) -> Ev {
        if let (Ev::Const(a), Ev::Const(b)) = (left, right) {
            return Ev::Const(match kind {
                BinKind::Add => a + b,
                BinKind::Mul => a * b,
            });
        }
        let a = self.builder.materialize(left);
        let b = self.builder.materialize(right);
        let kind_name = match kind {
            BinKind::Add => "add",
            BinKind::Mul => "mul",
        };
        let name = self.builder.anon_name(kind_name);
        let idx = self.builder.push(NodeSpec {
            name,
            kind: kind_name.to_string(),
            waveform: None,
            params: HashMap::new(),
            connections: vec![("a".to_string(), a), ("b".to_string(), b)],
        });
        Ev::Node(idx)
    }

    fn divide(&mut self, left: Ev, right: Ev) -> Result<Ev, String> {
        match (left, right) {
            (Ev::Const(a), Ev::Const(b)) => Ok(Ev::Const(a / b)),
            (_, Ev::Const(b)) => Ok(self.combine(BinKind::Mul, left, Ev::Const(1.0 / b))),
            _ => Err(format!(
                "line {}: division by a signal is not supported (divisor must be a constant)",
                self.line
            )),
        }
    }

    fn negate(&mut self, ev: Ev) -> Ev {
        match ev {
            Ev::Const(c) => Ev::Const(-c),
            node => self.combine(BinKind::Mul, node, Ev::Const(-1.0)),
        }
    }
}

#[derive(Clone, Copy)]
enum BinKind {
    Add,
    Mul,
}

fn describe(tok: Option<&Tok>) -> String {
    match tok {
        None => "end of line".to_string(),
        Some(Tok::RParen) => "')'".to_string(),
        Some(Tok::Eq) => "'='".to_string(),
        Some(t) => format!("{t:?}"),
    }
}
