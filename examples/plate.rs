//! Interactive lab for auditioning the modal plate voice.
//!
//! Run with:
//! `cargo run --release --example plate --features native,crossterm`
//!
//! Controls:
//! - SPACE = hit at the current velocity
//! - ↑/↓ = select parameter, ←/→ = adjust (coarse), [ / ] = adjust (fine)
//! - 1-5 = Ride, Crash, Bell, Timbale, Gong presets (32 modes)
//! - 6-8 = China (128 modes), Tam-tam (128), Gamelan (64)
//! - V = cycle velocity (hard hits glide and shimmer, soft hits don't)
//! - M = cycle material (latched on the next hit)
//! - N = cycle mode count 8 → 128 (latched on the next hit)
//! - P = play / stop a 16-step pattern for the current preset
//! - Q = quit

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use gooey::effects::SoftLimiter;
use gooey::engine::{Engine, EngineOutput, Instrument, Sequencer, SequencerStep};
use gooey::instruments::{
    PlateConfig, PlateMaterial, PlateVoice, PLATE_MODE_COUNTS, PLATE_PARAM_MATERIAL_INDEX,
    PLATE_PARAM_MODES_INDEX,
};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

const INSTRUMENT: &str = "plate";
const BPM: f32 = 120.0;

const PARAM_NAMES: [&str; 13] = [
    "pitch",
    "geometry",
    "strike pos",
    "pickup pos",
    "hardness",
    "decay",
    "hf damping",
    "tension",
    "cascade",
    "deplete",
    "brightness",
    "material",
    "modes",
];

struct SharedPlateVoice(Arc<Mutex<PlateVoice>>);

impl Instrument for SharedPlateVoice {
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        self.0.lock().unwrap().trigger_with_velocity(time, velocity);
    }

    fn tick(&mut self, current_time: f64) -> f32 {
        self.0.lock().unwrap().tick(current_time)
    }

    fn is_active(&self) -> bool {
        self.0.lock().unwrap().is_active()
    }
}

#[derive(Clone, Copy)]
struct Preset {
    name: &'static str,
    config: fn() -> PlateConfig,
    /// 16 steps of accent velocity; 0 is a rest.
    pattern: [f32; 16],
}

const PRESETS: [Preset; 8] = [
    Preset {
        name: "Ride",
        config: PlateConfig::ride,
        pattern: [
            1.0, 0.0, 0.5, 0.0, 0.8, 0.0, 0.5, 0.4, 1.0, 0.0, 0.5, 0.0, 0.8, 0.0, 0.5, 0.4,
        ],
    },
    Preset {
        name: "Crash",
        config: PlateConfig::crash,
        pattern: [
            1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.3, 0.0,
        ],
    },
    Preset {
        name: "Bell",
        config: PlateConfig::bell,
        pattern: [
            1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.6, 0.0, 0.0, 0.0, 0.8, 0.0, 0.0, 0.0, 0.0, 0.0,
        ],
    },
    Preset {
        name: "Timbale",
        config: PlateConfig::timbale,
        pattern: [
            1.0, 0.0, 0.4, 0.7, 0.0, 0.4, 0.9, 0.0, 0.4, 0.7, 0.0, 0.4, 1.0, 0.0, 0.6, 0.8,
        ],
    },
    Preset {
        name: "Gong",
        config: PlateConfig::gong,
        pattern: [
            1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        ],
    },
    Preset {
        name: "China",
        config: PlateConfig::china,
        pattern: [
            1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.7, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        ],
    },
    Preset {
        name: "Tam-tam",
        config: PlateConfig::tam_tam,
        pattern: [
            1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        ],
    },
    Preset {
        name: "Gamelan",
        config: PlateConfig::gamelan,
        pattern: [
            1.0, 0.0, 0.0, 0.6, 0.0, 0.0, 0.8, 0.0, 0.0, 0.0, 0.7, 0.0, 0.0, 0.5, 0.0, 0.0,
        ],
    },
];

fn make_bar(value: f32, width: usize) -> String {
    let filled = (value.clamp(0.0, 1.0) * width as f32).round() as usize;
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

fn detail(config: &PlateConfig, index: usize) -> String {
    match index {
        0 => format!("{:>8.1} Hz", config.pitch_hz),
        1 => format!("{:>+8.2}", config.geometry),
        2 => format!("{:>8.2} (0 centre, 1 edge)", config.strike_pos),
        3 => format!("{:>8.2} (0 centre, 1 edge)", config.pickup_pos),
        4 => {
            let kind = if config.hardness < 1.0 / 3.0 {
                "pulse"
            } else if config.hardness < 2.0 / 3.0 {
                "click"
            } else {
                "noise"
            };
            let width_ms = 8.0 * (0.25_f32 / 8.0).powf(config.hardness);
            format!("{:>8.2} ms {kind}", width_ms)
        }
        5 => format!("{:>8.2} s", config.decay_seconds),
        6 => format!("{:>8.2} x", config.hf_damping),
        7..=9 => format!(
            "{:>8.0}%",
            [config.tension, config.cascade, config.deplete][index - 7] * 100.0
        ),
        10 => format!("{:>8.0} Hz", config.brightness_hz),
        11 => format!("{:>8}", config.material.name()),
        12 => format!("{:>8} modes", config.mode_count),
        _ => String::new(),
    }
}

fn render_display(
    voice: &PlateVoice,
    selected: usize,
    preset_name: &str,
    velocity: f32,
    playing: bool,
) {
    let config = voice.config_targets();
    print!("\x1b[2J\x1b[H\x1b[?7l");
    print!("=== Plate Lab / MODAL PLATE ENGINE ===\r\n");
    print!("SPACE=hit  V=velocity  M=material  P=pattern  Q=quit  arrows/[]=adjust\r\n");
    print!("1=ride 2=crash 3=bell 4=timbale 5=gong 6=china 7=tam-tam 8=gamelan  N=modes\r\n");
    print!(
        "Preset: {preset_name} | Velocity: {:.0}% | Pattern: {} @ {BPM:.0} BPM\r\n\r\n",
        velocity * 100.0,
        if playing { "playing" } else { "stopped" }
    );
    for (index, name) in PARAM_NAMES.iter().enumerate() {
        let value = voice.parameter_normalized(index).unwrap_or(0.0);
        let indicator = if index == selected { ">" } else { " " };
        print!(
            "{} {:<12} [{}] {:>4.2}  {}\r\n",
            indicator,
            name,
            make_bar(value, 16),
            value,
            detail(&config, index)
        );
    }
    print!(
        "\r\nSignal: exciter × strike weights → N modes × pickup weights → Σ → LP → soft clip\r\n"
    );
    print!(
        "        modal energy → tension glide;  band k → cubic → excites band k+1 (cascade)\r\n"
    );
    io::stdout().flush().unwrap();
}

fn pattern_steps(preset: &Preset, velocity: f32) -> Vec<SequencerStep> {
    preset
        .pattern
        .iter()
        .map(|&accent| SequencerStep::with_velocity(accent > 0.0, accent * velocity))
        .collect()
}

fn load_pattern(engine: &Arc<Mutex<Engine>>, preset: &Preset, velocity: f32) {
    let mut engine = engine.lock().unwrap();
    if let Some(sequencer) = engine.sequencer_mut(0) {
        for (step, settings) in pattern_steps(preset, velocity).into_iter().enumerate() {
            sequencer.set_step_with_velocity(step, settings.enabled, settings.velocity);
        }
    }
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44_100.0;
    let mut preset = PRESETS[0];
    let voice = Arc::new(Mutex::new(PlateVoice::with_config(
        sample_rate,
        (preset.config)(),
    )));

    let velocities = [0.25, 0.5, 0.75, 1.0];
    let mut velocity_index = 3;

    let mut engine = Engine::new(sample_rate);
    engine.set_bpm(BPM);
    engine.set_master_gain(0.9);
    engine.add_instrument(INSTRUMENT, Box::new(SharedPlateVoice(voice.clone())));
    engine.add_sequencer(Sequencer::with_velocity_pattern(
        BPM,
        sample_rate,
        pattern_steps(&preset, velocities[velocity_index]),
        INSTRUMENT,
    ));
    engine.clear_global_effects();
    engine.add_global_effect(Box::new(SoftLimiter::new(1.0)));
    let audio_engine = Arc::new(Mutex::new(engine));

    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;
    engine_output.create_stream_with_engine(audio_engine.clone())?;
    engine_output.start()?;

    let mut selected = 0;
    let mut preset_name = preset.name;
    let mut playing = false;
    let mut needs_redraw = true;

    execute!(io::stdout(), Clear(ClearType::All), cursor::Hide)?;
    enable_raw_mode()?;

    let result = loop {
        if needs_redraw {
            render_display(
                &voice.lock().unwrap(),
                selected,
                preset_name,
                velocities[velocity_index],
                playing,
            );
            needs_redraw = false;
        }

        if event::poll(std::time::Duration::from_millis(16))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match code {
                    KeyCode::Up => {
                        selected = selected.saturating_sub(1);
                        needs_redraw = true;
                    }
                    KeyCode::Down => {
                        selected = (selected + 1).min(PARAM_NAMES.len() - 1);
                        needs_redraw = true;
                    }
                    KeyCode::Left | KeyCode::Right | KeyCode::Char('[') | KeyCode::Char(']') => {
                        // Material and modes step through their discrete positions.
                        let (coarse, fine) = match selected {
                            PLATE_PARAM_MATERIAL_INDEX => (0.25, 0.25),
                            PLATE_PARAM_MODES_INDEX => {
                                let step = 1.0 / (PLATE_MODE_COUNTS.len() - 1) as f32;
                                (step, step)
                            }
                            _ => (0.05, 0.01),
                        };
                        let delta = match code {
                            KeyCode::Left => -coarse,
                            KeyCode::Right => coarse,
                            KeyCode::Char('[') => -fine,
                            KeyCode::Char(']') => fine,
                            _ => unreachable!(),
                        };
                        let mut voice = voice.lock().unwrap();
                        let value = voice.parameter_normalized(selected).unwrap_or(0.0) + delta;
                        voice.set_parameter_normalized(selected, value.clamp(0.0, 1.0));
                        preset_name = "Custom";
                        needs_redraw = true;
                    }
                    KeyCode::Char(' ') => {
                        audio_engine
                            .lock()
                            .unwrap()
                            .trigger_instrument_with_velocity(
                                INSTRUMENT,
                                velocities[velocity_index],
                            );
                    }
                    KeyCode::Char('v') | KeyCode::Char('V') => {
                        velocity_index = (velocity_index + 1) % velocities.len();
                        load_pattern(&audio_engine, &preset, velocities[velocity_index]);
                        needs_redraw = true;
                    }
                    KeyCode::Char('m') | KeyCode::Char('M') => {
                        let mut voice = voice.lock().unwrap();
                        let next = (voice.material().index() + 1) % PlateMaterial::ALL.len();
                        voice.set_material(PlateMaterial::from_index(next));
                        preset_name = "Custom";
                        needs_redraw = true;
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') => {
                        let mut voice = voice.lock().unwrap();
                        let current = PLATE_MODE_COUNTS
                            .iter()
                            .position(|count| *count == voice.mode_count())
                            .unwrap_or(0);
                        let next = PLATE_MODE_COUNTS[(current + 1) % PLATE_MODE_COUNTS.len()];
                        voice.set_mode_count(next);
                        preset_name = "Custom";
                        needs_redraw = true;
                    }
                    KeyCode::Char(number @ '1'..='8') => {
                        preset = PRESETS[number as usize - '1' as usize];
                        voice.lock().unwrap().set_config((preset.config)());
                        load_pattern(&audio_engine, &preset, velocities[velocity_index]);
                        preset_name = preset.name;
                        needs_redraw = true;
                    }
                    KeyCode::Char('p') | KeyCode::Char('P') => {
                        let mut engine = audio_engine.lock().unwrap();
                        if let Some(sequencer) = engine.sequencer_mut(0) {
                            if playing {
                                sequencer.stop();
                            } else {
                                sequencer.reset();
                                sequencer.start();
                            }
                            playing = !playing;
                        }
                        needs_redraw = true;
                    }
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => break Ok(()),
                    _ => {}
                }
            }
        }
    };

    print!("\x1b[?7h");
    execute!(io::stdout(), cursor::Show)?;
    disable_raw_mode()?;
    println!("\nQuitting...");
    result
}

#[cfg(not(feature = "native"))]
fn main() {
    println!(
        "This example requires native audio and crossterm. Run: cargo run --example plate --features native,crossterm"
    );
}
