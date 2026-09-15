//! Interactive lab for auditioning the dual-resonator kick voice.

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use gooey::engine::{Engine, EngineOutput, Instrument};
use gooey::instruments::{ResoKick, ResoKickConfig};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

struct ParamInfo {
    name: &'static str,
    coarse_step: f32,
    fine_step: f32,
}

const PARAM_INFO: [ParamInfo; 10] = [
    ParamInfo {
        name: "frequency",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
    ParamInfo {
        name: "depth",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
    ParamInfo {
        name: "pitch_decay",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
    ParamInfo {
        name: "resonate",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
    ParamInfo {
        name: "punch",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
    ParamInfo {
        name: "character",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
    ParamInfo {
        name: "ripple",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
    ParamInfo {
        name: "exciter_noise",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
    ParamInfo {
        name: "volume",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
    ParamInfo {
        name: "tuning",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
];

struct SharedResoKick(Arc<Mutex<ResoKick>>);

impl Instrument for SharedResoKick {
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

fn get_param_value(kick: &ResoKick, index: usize) -> f32 {
    let config = kick.config();
    match index {
        0 => config.frequency,
        1 => config.depth,
        2 => config.pitch_decay,
        3 => config.resonate,
        4 => config.punch,
        5 => config.character,
        6 => config.ripple,
        7 => config.exciter_noise,
        8 => config.volume,
        9 => kick.tuning(),
        _ => 0.0,
    }
}

fn set_param_value(kick: &mut ResoKick, index: usize, value: f32) {
    match index {
        0 => kick.set_frequency(value),
        1 => kick.set_depth(value),
        2 => kick.set_pitch_decay(value),
        3 => kick.set_resonate(value),
        4 => kick.set_punch(value),
        5 => kick.set_character(value),
        6 => kick.set_ripple(value),
        7 => kick.set_exciter_noise(value),
        8 => kick.set_volume(value),
        9 => kick.set_tuning(value),
        _ => {}
    }
}

fn adjust_param(kick: &mut ResoKick, index: usize, delta: f32) {
    set_param_value(
        kick,
        index,
        (get_param_value(kick, index) + delta).clamp(0.0, 1.0),
    );
}

fn make_bar(value: f32, width: usize) -> String {
    let filled = (value.clamp(0.0, 1.0) * width as f32).round() as usize;
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

fn detail(kick: &ResoKick, index: usize) -> String {
    match index {
        0 => format!("{:>6.1} Hz", kick.frequency_hz()),
        1 => format!("{:>6.2} x", kick.pitch_start_multiplier()),
        2 => format!("{:>6.1} ms", kick.pitch_decay_ms()),
        3 => format!("{:>6.2} s", kick.resonate_t60_seconds()),
        4 => format!("{:>6.2} x", kick.punch_gain()),
        5 => format!("{:>6.1} Hz", kick.character_hz()),
        6 => format!("{:>6.2} oct", kick.ripple_octaves()),
        7 => format!("{:>6.2} x", kick.exciter_noise_gain()),
        9 => format!("{:+6.1} st", kick.tuning_semitones()),
        _ => String::new(),
    }
}

fn render_display(
    kick: &ResoKick,
    selected: usize,
    trigger_count: u32,
    velocity: f32,
    preset_name: &str,
) {
    print!("\x1b[2J\x1b[H\x1b[?7l");
    print!("=== Reso Kick Lab ===\r\n");
    print!("SPACE=hit  Q=quit  ↑↓=select  ←→=adjust  []=fine\r\n");
    print!("V=cycle velocity  1-6=presets\r\n");
    print!("Preset: {preset_name}\r\n\r\n");

    for (index, info) in PARAM_INFO.iter().enumerate() {
        let value = get_param_value(kick, index);
        let indicator = if index == selected { ">" } else { " " };
        print!(
            "{} {:<15} [{}] {:>4.2}  {}\r\n",
            indicator,
            info.name,
            make_bar(value, 12),
            value,
            detail(kick, index)
        );
    }

    print!(
        "\r\nHits: {trigger_count} | Velocity: {:.0}%\r\n",
        velocity * 100.0
    );
    print!("Signal: struck body resonator → Punch → character resonator\r\n");
    io::stdout().flush().unwrap();
}

fn preset(number: char) -> Option<(&'static str, ResoKickConfig)> {
    match number {
        '1' => Some(("Classic 808", ResoKickConfig::classic808())),
        '2' => Some(("Punch 909", ResoKickConfig::punch909())),
        '3' => Some(("Soft Bounce", ResoKickConfig::soft_bounce())),
        '4' => Some(("Tom", ResoKickConfig::tom())),
        '5' => Some(("Laser", ResoKickConfig::laser())),
        '6' => Some(("Sub Drone", ResoKickConfig::sub_drone())),
        _ => None,
    }
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44_100.0;
    let kick = Arc::new(Mutex::new(ResoKick::new(sample_rate)));

    let mut engine = Engine::new(sample_rate);
    engine.set_master_gain(0.85);
    engine.add_instrument("reso_kick", Box::new(SharedResoKick(kick.clone())));
    let audio_engine = Arc::new(Mutex::new(engine));

    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;
    engine_output.create_stream_with_engine(audio_engine.clone())?;
    engine_output.start()?;

    let mut selected = 0;
    let mut trigger_count = 0;
    let velocities = [0.25, 0.5, 0.75, 1.0];
    let mut velocity_index = 2;
    let mut preset_name = "Classic 808";
    let mut needs_redraw = true;

    execute!(io::stdout(), Clear(ClearType::All), cursor::Hide)?;
    enable_raw_mode()?;

    let result = loop {
        if needs_redraw {
            let voice = kick.lock().unwrap();
            render_display(
                &voice,
                selected,
                trigger_count,
                velocities[velocity_index],
                preset_name,
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
                        selected = (selected + 1).min(PARAM_INFO.len() - 1);
                        needs_redraw = true;
                    }
                    KeyCode::Left | KeyCode::Right | KeyCode::Char('[') | KeyCode::Char(']') => {
                        let info = &PARAM_INFO[selected];
                        let delta = match code {
                            KeyCode::Left => -info.coarse_step,
                            KeyCode::Right => info.coarse_step,
                            KeyCode::Char('[') => -info.fine_step,
                            KeyCode::Char(']') => info.fine_step,
                            _ => unreachable!(),
                        };
                        adjust_param(&mut kick.lock().unwrap(), selected, delta);
                        preset_name = "Custom";
                        needs_redraw = true;
                    }
                    KeyCode::Char(' ') => {
                        audio_engine
                            .lock()
                            .unwrap()
                            .trigger_instrument_with_velocity(
                                "reso_kick",
                                velocities[velocity_index],
                            );
                        trigger_count += 1;
                        needs_redraw = true;
                    }
                    KeyCode::Char('v') | KeyCode::Char('V') => {
                        velocity_index = (velocity_index + 1) % velocities.len();
                        needs_redraw = true;
                    }
                    KeyCode::Char(number @ '1'..='6') => {
                        if let Some((name, config)) = preset(number) {
                            kick.lock().unwrap().set_config(config);
                            preset_name = name;
                            needs_redraw = true;
                        }
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
        "This example requires native audio and crossterm. Run: cargo run --example reso_kick --features native,crossterm"
    );
}
