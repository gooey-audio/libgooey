//! A tiny, line-based DSL for describing simple `Engine` programs.
//!
//! This is a prototype aimed at making the common example-style setup shorter:
//! - Create instruments
//! - Add step sequencers
//! - Add LFO routes
//! - Add a few global effects
//!
//! The syntax is intentionally forgiving and whitespace-friendly.
//! Lines are statements; `#` starts a comment.
//!
//! Example:
//! ```text
//! bpm 120
//! master 0.25
//!
//! inst hihat hihat short
//! seq hihat x.x.x.x.|x.x.x.x.
//!
//! lfo 1bar hihat.decay amt=1
//! fx lowpass 2000 0.3
//! ```

use std::collections::HashSet;

use crate::effects::{BrickWallLimiter, DelayEffect, Effect, LowpassFilterEffect, TubeSaturation};
use crate::engine::{Engine, Instrument, Lfo, MusicalDivision, Sequencer, SequencerStep};
use crate::instruments::{
    HiHat, HiHatConfig, KickConfig, KickDrum, SnareConfig, SnareDrum, Tom2, Tom2Config, TomConfig,
    TomDrum,
};

#[derive(Clone, Debug)]
pub struct Program {
    bpm: Option<f32>,
    master_gain: Option<f32>,
    clear_effects: bool,
    instruments: Vec<InstrumentDef>,
    sequencers: Vec<SequencerDef>,
    lfos: Vec<LfoDef>,
    effects: Vec<EffectDef>,
}

impl Program {
    pub fn parse(source: &str) -> Result<Self, String> {
        let mut program = Self {
            bpm: None,
            master_gain: None,
            clear_effects: false,
            instruments: Vec::new(),
            sequencers: Vec::new(),
            lfos: Vec::new(),
            effects: Vec::new(),
        };

        let mut instrument_names: HashSet<String> = HashSet::new();

        for (line_index, raw_line) in source.lines().enumerate() {
            let line_number = line_index + 1;
            let line = strip_comment(raw_line).trim();
            if line.is_empty() {
                continue;
            }

            let tokens: Vec<&str> = line.split_whitespace().collect();
            let cmd = tokens[0].to_ascii_lowercase();

            match cmd.as_str() {
                "bpm" => {
                    let bpm = parse_single_f32_arg("bpm", line_number, &tokens)?;
                    program.bpm = Some(bpm);
                }
                "master" | "gain" => {
                    let gain = parse_single_f32_arg("master", line_number, &tokens)?;
                    program.master_gain = Some(gain);
                }
                "inst" | "i" => {
                    if tokens.len() < 3 {
                        return Err(format!(
                            "line {}: inst expects: inst <name> <type> [preset]",
                            line_number
                        ));
                    }

                    let name = tokens[1].to_string();
                    if !instrument_names.insert(name.clone()) {
                        return Err(format!(
                            "line {}: duplicate instrument name '{}'",
                            line_number, name
                        ));
                    }

                    let kind = InstrumentKind::parse(tokens[2]).ok_or_else(|| {
                        format!(
                            "line {}: unknown instrument type '{}'",
                            line_number, tokens[2]
                        )
                    })?;

                    let mut preset: Option<String> = None;
                    for arg in &tokens[3..] {
                        if let Some((key, value)) = arg.split_once('=') {
                            match key.to_ascii_lowercase().as_str() {
                                "preset" => preset = Some(value.to_string()),
                                other => {
                                    return Err(format!(
                                        "line {}: unknown inst argument '{}'",
                                        line_number, other
                                    ));
                                }
                            }
                        } else if preset.is_none() {
                            preset = Some((*arg).to_string());
                        } else {
                            return Err(format!(
                                "line {}: too many inst arguments (unexpected '{}')",
                                line_number, arg
                            ));
                        }
                    }

                    program
                        .instruments
                        .push(InstrumentDef { name, kind, preset });
                }
                "seq" | "s" => {
                    if tokens.len() < 3 {
                        return Err(format!(
                            "line {}: seq expects: seq <instrument> <pattern> [start|stop]",
                            line_number
                        ));
                    }

                    let instrument = tokens[1].to_string();
                    let mut remainder_tokens: Vec<&str> = tokens[2..].to_vec();

                    // Optional trailing flags.
                    let mut start = true;
                    while let Some(last) = remainder_tokens.last().copied() {
                        match last.to_ascii_lowercase().as_str() {
                            "start" | "on" => {
                                start = true;
                                remainder_tokens.pop();
                            }
                            "stop" | "stopped" | "off" => {
                                start = false;
                                remainder_tokens.pop();
                            }
                            _ => break,
                        }
                    }

                    if remainder_tokens.is_empty() {
                        return Err(format!(
                            "line {}: seq expects a non-empty pattern string",
                            line_number
                        ));
                    }

                    let pattern_str = remainder_tokens.join(" ");
                    let pattern = parse_pattern(line_number, &pattern_str)?;
                    program.sequencers.push(SequencerDef {
                        instrument,
                        pattern,
                        start,
                    });
                }
                "lfo" | "l" => {
                    if tokens.len() < 3 {
                        return Err(format!(
                            "line {}: lfo expects: lfo <rate> <inst.param> [amt=..] [offset=..]",
                            line_number
                        ));
                    }

                    let mut index = 1;
                    let rate = parse_lfo_rate(line_number, &tokens, &mut index)?;

                    // Optional arrow token.
                    if tokens.get(index).copied() == Some("->") {
                        index += 1;
                    }

                    let target = tokens.get(index).copied().ok_or_else(|| {
                        format!(
                            "line {}: lfo expects target like 'kick.pitch_drop'",
                            line_number
                        )
                    })?;
                    index += 1;

                    let (target_instrument, target_parameter) = parse_target(line_number, target)?;

                    let mut amount = 1.0;
                    let mut offset = 0.0;

                    for arg in &tokens[index..] {
                        if let Some(rest) = arg.strip_prefix('*') {
                            amount = parse_f32(line_number, "lfo amount", rest)?;
                            continue;
                        }
                        if let Some(rest) = arg.strip_prefix('@') {
                            offset = parse_f32(line_number, "lfo offset", rest)?;
                            continue;
                        }
                        if let Some((key, value)) = arg.split_once('=') {
                            match key.to_ascii_lowercase().as_str() {
                                "amt" | "amount" => {
                                    amount = parse_f32(line_number, "lfo amount", value)?
                                }
                                "off" | "offset" => {
                                    offset = parse_f32(line_number, "lfo offset", value)?
                                }
                                other => {
                                    return Err(format!(
                                        "line {}: unknown lfo argument '{}'",
                                        line_number, other
                                    ));
                                }
                            }
                            continue;
                        }

                        return Err(format!(
                            "line {}: unrecognized lfo argument '{}'",
                            line_number, arg
                        ));
                    }

                    program.lfos.push(LfoDef {
                        rate,
                        target_instrument,
                        target_parameter,
                        amount,
                        offset,
                    });
                }
                "fx" | "effect" => {
                    if tokens.len() < 2 {
                        return Err(format!("line {}: fx expects: fx <type> [...]", line_number));
                    }

                    let fx_type = tokens[1].to_ascii_lowercase();
                    if fx_type == "clear" {
                        program.clear_effects = true;
                        program.effects.clear();
                        continue;
                    }

                    let def = EffectDef::parse(line_number, &tokens[1..])?;
                    program.effects.push(def);
                }
                other => {
                    return Err(format!(
                        "line {}: unknown statement '{}'",
                        line_number, other
                    ));
                }
            }
        }

        Ok(program)
    }

    pub fn build_engine(&self, sample_rate: f32) -> Result<Engine, String> {
        let mut engine = Engine::new(sample_rate);

        if let Some(bpm) = self.bpm {
            engine.set_bpm(bpm);
        }
        if let Some(master_gain) = self.master_gain {
            engine.set_master_gain(master_gain);
        }
        if self.clear_effects {
            engine.clear_global_effects();
        }

        for instrument in &self.instruments {
            let built = instrument.build(sample_rate)?;
            engine.add_instrument(instrument.name.as_str(), built);
        }

        let instrument_kinds = self
            .instruments
            .iter()
            .map(|i| (i.name.as_str(), i.kind))
            .collect::<std::collections::HashMap<_, _>>();

        for effect in &self.effects {
            engine.add_global_effect(effect.build(sample_rate)?);
        }

        // Sequencers often imply "play"; default to started unless explicitly stopped.
        for sequencer in &self.sequencers {
            let mut seq = Sequencer::with_velocity_pattern(
                engine.bpm(),
                sample_rate,
                sequencer.pattern.clone(),
                sequencer.instrument.as_str(),
            );
            if sequencer.start {
                seq.start();
            }
            engine.add_sequencer(seq);
        }

        for lfo in &self.lfos {
            let l = match lfo.rate {
                LfoRate::Hz(freq) => Lfo::new(freq, sample_rate),
                LfoRate::BpmSync(division) => Lfo::new_synced(division, engine.bpm(), sample_rate),
            };
            let idx = engine.add_lfo(l);

            let resolved_parameter = resolve_parameter_alias(
                instrument_kinds
                    .get(lfo.target_instrument.as_str())
                    .copied(),
                lfo.target_parameter.as_str(),
            );
            engine.map_lfo_to_parameter(
                idx,
                lfo.target_instrument.as_str(),
                resolved_parameter.as_str(),
                lfo.amount,
            )?;
            if let Some(lfo_mut) = engine.lfo_mut(idx) {
                lfo_mut.offset = lfo.offset;
            }
        }

        Ok(engine)
    }

    pub fn bpm(&self) -> Option<f32> {
        self.bpm
    }
}

#[derive(Clone, Debug)]
struct InstrumentDef {
    name: String,
    kind: InstrumentKind,
    preset: Option<String>,
}

impl InstrumentDef {
    fn build(&self, sample_rate: f32) -> Result<Box<dyn Instrument>, String> {
        let preset = self
            .preset
            .as_deref()
            .unwrap_or("default")
            .to_ascii_lowercase();

        match self.kind {
            InstrumentKind::Kick => match preset.as_str() {
                "default" => Ok(Box::new(KickDrum::new(sample_rate))),
                "tight" => Ok(Box::new(KickDrum::with_config(
                    sample_rate,
                    KickConfig::tight(),
                ))),
                "punch" => Ok(Box::new(KickDrum::with_config(
                    sample_rate,
                    KickConfig::punch(),
                ))),
                "loose" => Ok(Box::new(KickDrum::with_config(
                    sample_rate,
                    KickConfig::loose(),
                ))),
                "dirt" | "dirty" => Ok(Box::new(KickDrum::with_config(
                    sample_rate,
                    KickConfig::dirt(),
                ))),
                other => Err(format!(
                    "unknown kick preset '{}'. Try: default, tight, punch, loose, dirt",
                    other
                )),
            },
            InstrumentKind::Snare => match preset.as_str() {
                "default" | "tight" => Ok(Box::new(SnareDrum::with_config(
                    sample_rate,
                    SnareConfig::tight(),
                ))),
                "loose" => Ok(Box::new(SnareDrum::with_config(
                    sample_rate,
                    SnareConfig::loose(),
                ))),
                "hiss" => Ok(Box::new(SnareDrum::with_config(
                    sample_rate,
                    SnareConfig::hiss(),
                ))),
                "smack" => Ok(Box::new(SnareDrum::with_config(
                    sample_rate,
                    SnareConfig::smack(),
                ))),
                other => Err(format!(
                    "unknown snare preset '{}'. Try: default, tight, loose, hiss, smack",
                    other
                )),
            },
            InstrumentKind::HiHat => match preset.as_str() {
                "default" | "short" => Ok(Box::new(HiHat::with_config(
                    sample_rate,
                    HiHatConfig::short(),
                ))),
                "loose" => Ok(Box::new(HiHat::with_config(
                    sample_rate,
                    HiHatConfig::loose(),
                ))),
                "dark" => Ok(Box::new(HiHat::with_config(
                    sample_rate,
                    HiHatConfig::dark(),
                ))),
                "soft" => Ok(Box::new(HiHat::with_config(
                    sample_rate,
                    HiHatConfig::soft(),
                ))),
                other => Err(format!(
                    "unknown hihat preset '{}'. Try: short, loose, dark, soft",
                    other
                )),
            },
            InstrumentKind::Tom => match preset.as_str() {
                "default" | "mid" | "mid_tom" => Ok(Box::new(TomDrum::with_config(
                    sample_rate,
                    TomConfig::mid_tom(),
                ))),
                "high" | "high_tom" => Ok(Box::new(TomDrum::with_config(
                    sample_rate,
                    TomConfig::high_tom(),
                ))),
                "low" | "low_tom" => Ok(Box::new(TomDrum::with_config(
                    sample_rate,
                    TomConfig::low_tom(),
                ))),
                "floor" | "floor_tom" => Ok(Box::new(TomDrum::with_config(
                    sample_rate,
                    TomConfig::floor_tom(),
                ))),
                other => Err(format!(
                    "unknown tom preset '{}'. Try: default, high, mid, low, floor",
                    other
                )),
            },
            InstrumentKind::Tom2 => match preset.as_str() {
                "default" => Ok(Box::new(Tom2::new(sample_rate))),
                "derp" => {
                    let mut tom = Tom2::new(sample_rate);
                    tom.set_config(Tom2Config::derp());
                    Ok(Box::new(tom))
                }
                "ring" => {
                    let mut tom = Tom2::new(sample_rate);
                    tom.set_config(Tom2Config::ring());
                    Ok(Box::new(tom))
                }
                "brush" => {
                    let mut tom = Tom2::new(sample_rate);
                    tom.set_config(Tom2Config::brush());
                    Ok(Box::new(tom))
                }
                "void" | "void_preset" => {
                    let mut tom = Tom2::new(sample_rate);
                    tom.set_config(Tom2Config::void_preset());
                    Ok(Box::new(tom))
                }
                other => Err(format!(
                    "unknown tom2 preset '{}'. Try: default, derp, ring, brush, void",
                    other
                )),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum InstrumentKind {
    Kick,
    Snare,
    HiHat,
    Tom,
    Tom2,
}

impl InstrumentKind {
    fn parse(token: &str) -> Option<Self> {
        match token.to_ascii_lowercase().as_str() {
            "kick" | "kickdrum" => Some(Self::Kick),
            "snare" | "snaredrum" => Some(Self::Snare),
            "hihat" | "hat" => Some(Self::HiHat),
            "tom" | "tomdrum" => Some(Self::Tom),
            "tom2" => Some(Self::Tom2),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
struct SequencerDef {
    instrument: String,
    pattern: Vec<SequencerStep>,
    start: bool,
}

#[derive(Clone, Debug)]
struct LfoDef {
    rate: LfoRate,
    target_instrument: String,
    target_parameter: String,
    amount: f32,
    offset: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum LfoRate {
    Hz(f32),
    BpmSync(MusicalDivision),
}

#[derive(Clone, Debug, PartialEq)]
enum EffectDef {
    Lowpass {
        cutoff_hz: f32,
        resonance: f32,
    },
    Delay {
        time_s: f32,
        feedback: f32,
        mix: f32,
    },
    Saturation {
        drive: f32,
        warmth: f32,
        mix: f32,
    },
    Limiter {
        threshold: f32,
    },
}

impl EffectDef {
    fn parse(line_number: usize, tokens: &[&str]) -> Result<Self, String> {
        let fx_type = tokens[0].to_ascii_lowercase();
        match fx_type.as_str() {
            "lowpass" | "lp" => {
                let (cutoff_hz, resonance) =
                    parse_two_f32_args_named(line_number, &tokens[1..], "cutoff", "res")?;
                Ok(Self::Lowpass {
                    cutoff_hz,
                    resonance,
                })
            }
            "delay" => {
                let (time_s, feedback, mix) =
                    parse_three_f32_args_named(line_number, &tokens[1..], "time", "fb", "mix")?;
                Ok(Self::Delay {
                    time_s,
                    feedback,
                    mix,
                })
            }
            "saturation" | "sat" => {
                let (drive, warmth, mix) = parse_three_f32_args_named(
                    line_number,
                    &tokens[1..],
                    "drive",
                    "warmth",
                    "mix",
                )?;
                Ok(Self::Saturation { drive, warmth, mix })
            }
            "limiter" | "limit" => {
                let threshold = parse_one_f32_arg_named(line_number, &tokens[1..], "threshold")?;
                Ok(Self::Limiter { threshold })
            }
            other => Err(format!(
                "line {}: unknown effect type '{}'",
                line_number, other
            )),
        }
    }

    fn build(&self, sample_rate: f32) -> Result<Box<dyn Effect>, String> {
        match *self {
            Self::Lowpass {
                cutoff_hz,
                resonance,
            } => Ok(Box::new(LowpassFilterEffect::new(
                sample_rate,
                cutoff_hz,
                resonance,
            ))),
            Self::Delay {
                time_s,
                feedback,
                mix,
            } => Ok(Box::new(DelayEffect::new(
                sample_rate,
                time_s,
                feedback,
                mix,
            ))),
            Self::Saturation { drive, warmth, mix } => Ok(Box::new(TubeSaturation::new(
                sample_rate,
                drive,
                warmth,
                mix,
            ))),
            Self::Limiter { threshold } => Ok(Box::new(BrickWallLimiter::new(threshold))),
        }
    }
}

fn resolve_parameter_alias(kind: Option<InstrumentKind>, parameter: &str) -> String {
    let parameter = parameter.to_ascii_lowercase();
    match kind {
        Some(InstrumentKind::Kick) => match parameter.as_str() {
            // Historical / example-friendly aliases.
            "pitch_drop" | "pitch_env_amt" => "pitch_envelope_amount".to_string(),
            "pitch_env_crv" => "pitch_envelope_curve".to_string(),
            "pitch_ratio" => "pitch_start_ratio".to_string(),
            "osc_decay" => "oscillator_decay".to_string(),
            "phase_mod_amt" => "phase_mod_amount".to_string(),
            "noise_res" => "noise_resonance".to_string(),
            _ => parameter,
        },
        Some(InstrumentKind::Snare) => match parameter.as_str() {
            "pitch_drop" => "pitch_drop".to_string(),
            _ => parameter,
        },
        Some(InstrumentKind::HiHat) => match parameter.as_str() {
            _ => parameter,
        },
        Some(InstrumentKind::Tom) => match parameter.as_str() {
            _ => parameter,
        },
        Some(InstrumentKind::Tom2) | None => parameter,
    }
}

fn strip_comment(line: &str) -> &str {
    line.split_once('#').map(|(head, _)| head).unwrap_or(line)
}

fn parse_single_f32_arg(
    statement: &str,
    line_number: usize,
    tokens: &[&str],
) -> Result<f32, String> {
    match tokens.len() {
        2 => parse_f32(line_number, statement, tokens[1]),
        3 if tokens[1] == "=" => parse_f32(line_number, statement, tokens[2]),
        _ => Err(format!(
            "line {}: {} expects a single number (e.g. '{} 120')",
            line_number, statement, statement
        )),
    }
}

fn parse_f32(line_number: usize, what: &str, token: &str) -> Result<f32, String> {
    token.parse::<f32>().map_err(|_| {
        format!(
            "line {}: expected a number for {}, got '{}'",
            line_number, what, token
        )
    })
}

fn parse_pattern(line_number: usize, pattern: &str) -> Result<Vec<SequencerStep>, String> {
    let mut steps: Vec<SequencerStep> = Vec::new();

    for ch in pattern.chars() {
        match ch {
            ' ' | '\t' | '|' => continue,
            '.' | '-' | '_' | '0' => steps.push(SequencerStep::new(false)),
            'x' | 'X' => steps.push(SequencerStep::with_velocity(true, 1.0)),
            'o' | 'O' => steps.push(SequencerStep::with_velocity(true, 0.5)),
            '1'..='9' => {
                let digit = ch.to_digit(10).unwrap() as f32;
                let velocity = (digit / 9.0).clamp(0.0, 1.0);
                steps.push(SequencerStep::with_velocity(true, velocity));
            }
            other => {
                return Err(format!(
                    "line {}: invalid pattern character '{}'. Use x . - _ | digits 1-9",
                    line_number, other
                ));
            }
        }
    }

    if steps.is_empty() {
        return Err(format!("line {}: pattern has no steps", line_number));
    }

    Ok(steps)
}

fn parse_lfo_rate(
    line_number: usize,
    tokens: &[&str],
    index: &mut usize,
) -> Result<LfoRate, String> {
    let token = tokens.get(*index).copied().ok_or_else(|| {
        format!(
            "line {}: lfo expects a rate (e.g. '1bar' or 'hz 0.5')",
            line_number
        )
    })?;

    let token_lc = token.to_ascii_lowercase();
    if token_lc == "hz" {
        *index += 1;
        let freq_token = tokens
            .get(*index)
            .copied()
            .ok_or_else(|| format!("line {}: lfo hz expects a frequency number", line_number))?;
        *index += 1;
        let freq = parse_f32(line_number, "lfo frequency", freq_token)?;
        return Ok(LfoRate::Hz(freq));
    }

    if let Some(freq_str) = token_lc.strip_suffix("hz") {
        *index += 1;
        let freq = parse_f32(line_number, "lfo frequency", freq_str)?;
        return Ok(LfoRate::Hz(freq));
    }

    *index += 1;
    let division = parse_division(line_number, &token_lc)?;
    Ok(LfoRate::BpmSync(division))
}

fn parse_division(line_number: usize, token_lc: &str) -> Result<MusicalDivision, String> {
    match token_lc {
        "4bars" | "4bar" => Ok(MusicalDivision::FourBars),
        "2bars" | "2bar" => Ok(MusicalDivision::TwoBars),
        "1bar" | "bar" => Ok(MusicalDivision::OneBar),
        "half" | "1/2" | "1/2note" => Ok(MusicalDivision::Half),
        "quarter" | "1/4" | "1/4note" => Ok(MusicalDivision::Quarter),
        "eighth" | "1/8" | "1/8note" => Ok(MusicalDivision::Eighth),
        "sixteenth" | "1/16" | "1/16note" => Ok(MusicalDivision::Sixteenth),
        "thirtysecond" | "thirty_second" | "1/32" | "1/32note" => Ok(MusicalDivision::ThirtySecond),
        _ => Err(format!(
            "line {}: unknown lfo division '{}'. Try: 1bar, 2bars, 4bars, 1/2, 1/4, 1/8, 1/16, 1/32",
            line_number, token_lc
        )),
    }
}

fn parse_target(line_number: usize, token: &str) -> Result<(String, String), String> {
    let (instrument, parameter) = token.split_once('.').ok_or_else(|| {
        format!(
            "line {}: expected target like 'kick.pitch_drop', got '{}'",
            line_number, token
        )
    })?;
    if instrument.is_empty() || parameter.is_empty() {
        return Err(format!(
            "line {}: expected target like 'kick.pitch_drop', got '{}'",
            line_number, token
        ));
    }
    Ok((instrument.to_string(), parameter.to_string()))
}

fn parse_one_f32_arg_named(line_number: usize, args: &[&str], key: &str) -> Result<f32, String> {
    let mut positional: Vec<&str> = Vec::new();
    let mut value: Option<f32> = None;

    for arg in args {
        if let Some((k, v)) = arg.split_once('=') {
            match k.to_ascii_lowercase().as_str() {
                "thresh" | "threshold" => value = Some(parse_f32(line_number, key, v)?),
                other => {
                    return Err(format!(
                        "line {}: unknown limiter argument '{}'",
                        line_number, other
                    ));
                }
            }
        } else {
            positional.push(*arg);
        }
    }

    if value.is_none() && positional.len() == 1 {
        value = Some(parse_f32(line_number, key, positional[0])?);
    }

    value.ok_or_else(|| {
        format!(
            "line {}: expected {} value (e.g. 'fx limiter 1.0' or 'fx limiter threshold=1.0')",
            line_number, key
        )
    })
}

fn parse_two_f32_args_named(
    line_number: usize,
    args: &[&str],
    key1: &str,
    key2: &str,
) -> Result<(f32, f32), String> {
    let mut positional: Vec<&str> = Vec::new();
    let mut v1: Option<f32> = None;
    let mut v2: Option<f32> = None;

    for arg in args {
        if let Some((k, v)) = arg.split_once('=') {
            match k.to_ascii_lowercase().as_str() {
                "cutoff" | "cutoff_hz" => v1 = Some(parse_f32(line_number, key1, v)?),
                "res" | "resonance" => v2 = Some(parse_f32(line_number, key2, v)?),
                other => {
                    return Err(format!(
                        "line {}: unknown lowpass argument '{}'",
                        line_number, other
                    ));
                }
            }
        } else {
            positional.push(*arg);
        }
    }

    if v1.is_none() && !positional.is_empty() {
        v1 = Some(parse_f32(line_number, key1, positional[0])?);
    }
    if v2.is_none() && positional.len() >= 2 {
        v2 = Some(parse_f32(line_number, key2, positional[1])?);
    }

    match (v1, v2) {
        (Some(a), Some(b)) => Ok((a, b)),
        _ => Err(format!(
            "line {}: expected {} and {} (e.g. 'fx lowpass 2000 0.3')",
            line_number, key1, key2
        )),
    }
}

fn parse_three_f32_args_named(
    line_number: usize,
    args: &[&str],
    key1: &str,
    key2: &str,
    key3: &str,
) -> Result<(f32, f32, f32), String> {
    let mut positional: Vec<&str> = Vec::new();
    let mut v1: Option<f32> = None;
    let mut v2: Option<f32> = None;
    let mut v3: Option<f32> = None;

    for arg in args {
        if let Some((k, v)) = arg.split_once('=') {
            match k.to_ascii_lowercase().as_str() {
                "time" | "t" => v1 = Some(parse_f32(line_number, key1, v)?),
                "fb" | "feedback" => v2 = Some(parse_f32(line_number, key2, v)?),
                "mix" => v3 = Some(parse_f32(line_number, key3, v)?),
                "drive" => v1 = Some(parse_f32(line_number, key1, v)?),
                "warmth" => v2 = Some(parse_f32(line_number, key2, v)?),
                other => {
                    return Err(format!(
                        "line {}: unknown effect argument '{}'",
                        line_number, other
                    ));
                }
            }
        } else {
            positional.push(*arg);
        }
    }

    if v1.is_none() && !positional.is_empty() {
        v1 = Some(parse_f32(line_number, key1, positional[0])?);
    }
    if v2.is_none() && positional.len() >= 2 {
        v2 = Some(parse_f32(line_number, key2, positional[1])?);
    }
    if v3.is_none() && positional.len() >= 3 {
        v3 = Some(parse_f32(line_number, key3, positional[2])?);
    }

    match (v1, v2, v3) {
        (Some(a), Some(b), Some(c)) => Ok((a, b, c)),
        _ => Err(format!(
            "line {}: expected {}, {}, {} (positional or key=value)",
            line_number, key1, key2, key3
        )),
    }
}
