//! Granulator Lab - interactive CLI for frozen-scan granular playback.
//!
//! Run with:
//! cargo run --example granulator --features native,crossterm,bounce -- path/to/file.wav

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gooey::engine::{Engine, EngineOutput, Instrument};
use gooey::instruments::{Granulator, SampleBuffer};

struct ParamInfo {
    name: &'static str,
    coarse_step: f32,
    fine_step: f32,
}

const PARAM_INFO: [ParamInfo; 12] = [
    ParamInfo {
        name: "scan_position",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
    ParamInfo {
        name: "grain_length",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
    ParamInfo {
        name: "spray",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
    ParamInfo {
        name: "pitch",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
    ParamInfo {
        name: "density",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
    ParamInfo {
        name: "texture",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
    ParamInfo {
        name: "direction",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
    ParamInfo {
        name: "cloud_duration",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
    ParamInfo {
        name: "volume",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
    ParamInfo {
        name: "random_timing",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
    ParamInfo {
        name: "random_amp",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
    ParamInfo {
        name: "drive",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
];

struct SharedGranulator(Arc<Mutex<Granulator>>);

impl Instrument for SharedGranulator {
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        self.0.lock().unwrap().trigger_with_velocity(time, velocity);
    }

    fn tick(&mut self, current_time: f64) -> f32 {
        self.0.lock().unwrap().tick(current_time)
    }

    fn is_active(&self) -> bool {
        self.0.lock().unwrap().is_active()
    }

    fn as_modulatable(&mut self) -> Option<&mut dyn gooey::engine::Modulatable> {
        None
    }
}

fn get_param_value(granulator: &Granulator, index: usize) -> f32 {
    match index {
        0 => granulator.scan_position(),
        1 => granulator.grain_length(),
        2 => granulator.spray(),
        3 => granulator.pitch(),
        4 => granulator.density(),
        5 => granulator.texture(),
        6 => granulator.direction(),
        7 => granulator.cloud_duration(),
        8 => granulator.volume(),
        9 => granulator.random_timing(),
        10 => granulator.random_amp(),
        11 => granulator.drive(),
        _ => 0.0,
    }
}

fn set_param_value(granulator: &mut Granulator, index: usize, value: f32) {
    match index {
        0 => granulator.set_scan_position(value),
        1 => granulator.set_grain_length(value),
        2 => granulator.set_spray(value),
        3 => granulator.set_pitch(value),
        4 => granulator.set_density(value),
        5 => granulator.set_texture(value),
        6 => granulator.set_direction(value),
        7 => granulator.set_cloud_duration(value),
        8 => granulator.set_volume(value),
        9 => granulator.set_random_timing(value),
        10 => granulator.set_random_amp(value),
        11 => granulator.set_drive(value),
        _ => {}
    }
}

fn adjust_param(granulator: &mut Granulator, index: usize, delta: f32) {
    let current = get_param_value(granulator, index);
    set_param_value(granulator, index, (current + delta).clamp(0.0, 1.0));
}

fn make_bar(normalized: f32, width: usize) -> String {
    let filled = (normalized.clamp(0.0, 1.0) * width as f32).round() as usize;
    let filled = filled.min(width);
    format!("{}{}", "#".repeat(filled), "-".repeat(width - filled))
}

fn render_display(
    granulator: &Granulator,
    selected: usize,
    trigger_count: u32,
    velocity: f32,
    auto_trigger: bool,
    source_name: &str,
    seed: u32,
) {
    print!("\x1b[2J\x1b[H\x1b[?7l");
    print!("=== Granulator Lab ===\r\n");
    print!("SPACE=cloud A=auto Q=quit UP/DOWN=sel LEFT/RIGHT=adj []=fine\r\n");
    print!("Z/X/C/V=vel 25/50/75/100% R=reseed\r\n");
    print!("Source: {}\r\n", source_name);
    print!("\r\n");

    for (i, info) in PARAM_INFO.iter().enumerate() {
        let value = get_param_value(granulator, i);
        let indicator = if i == selected { ">" } else { " " };
        let detail = match i {
            1 => format!("{:.0} ms", granulator.grain_length_ms()),
            2 => format!("{:.0} ms", granulator.spray_ms()),
            3 => format!("{:.2}x", granulator.pitch_ratio()),
            4 => format!("{:.1}/s", granulator.density_grains_per_second()),
            7 => format!("{:.0} ms", granulator.cloud_duration_ms()),
            _ => String::new(),
        };

        if detail.is_empty() {
            print!(
                "{} {:<15} [{}] {:>5.2}\r\n",
                indicator,
                info.name,
                make_bar(value, 14),
                value
            );
        } else {
            print!(
                "{} {:<15} [{}] {:>5.2} ({})\r\n",
                indicator,
                info.name,
                make_bar(value, 14),
                value,
                detail
            );
        }
    }

    let auto = if auto_trigger { "ON" } else { "OFF" };
    print!("\r\n");
    print!(
        "Clouds: {} | Vel: {:.0}% | Auto: {} | Seed: {} | Active grains: {}\r\n",
        trigger_count,
        velocity * 100.0,
        auto,
        seed,
        granulator.active_grain_count()
    );
    io::stdout().flush().unwrap();
}

fn demo_buffer(sample_rate: f32) -> SampleBuffer {
    let len = (sample_rate * 3.0) as usize;
    let mut samples = Vec::with_capacity(len);
    for i in 0..len {
        let t = i as f32 / sample_rate;
        let sweep = 110.0 + 330.0 * (t / 3.0);
        let carrier = (std::f32::consts::TAU * sweep * t).sin();
        let overtone = (std::f32::consts::TAU * 660.0 * t).sin() * 0.25;
        let pulse_phase = (t * 4.0).fract();
        let pulse = if pulse_phase < 0.08 {
            (std::f32::consts::PI * pulse_phase / 0.08).sin() * 0.25
        } else {
            0.0
        };
        samples.push((carrier * 0.5 + overtone + pulse) * 0.7);
    }
    SampleBuffer::from_mono(samples, sample_rate).unwrap()
}

fn load_source(sample_rate: f32) -> anyhow::Result<(SampleBuffer, String)> {
    let path = std::env::args_os().nth(1).map(PathBuf::from);
    if let Some(path) = path {
        let buffer = SampleBuffer::from_wav_mono(&path).map_err(anyhow::Error::msg)?;
        Ok((buffer, path.display().to_string()))
    } else {
        Ok((
            demo_buffer(sample_rate),
            "generated demo buffer".to_string(),
        ))
    }
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;
    let (buffer, source_name) = load_source(sample_rate)?;

    let granulator = Arc::new(Mutex::new(Granulator::new(sample_rate, buffer)));
    granulator.lock().unwrap().snap_params();

    let mut engine = Engine::new(sample_rate);
    engine.set_master_gain(0.9);
    engine.add_instrument("granulator", Box::new(SharedGranulator(granulator.clone())));

    let audio_engine = Arc::new(Mutex::new(engine));
    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;
    engine_output.create_stream_with_engine(audio_engine.clone())?;
    engine_output.start()?;

    let mut selected_param = 0;
    let mut trigger_count = 0;
    let mut current_velocity = 0.75;
    let mut auto_trigger = false;
    let mut next_auto_trigger = Instant::now();
    let mut seed = 0x1234_abcd;
    let mut needs_redraw = true;
    let mut last_redraw = Instant::now();

    execute!(io::stdout(), Clear(ClearType::All), cursor::Hide)?;
    enable_raw_mode()?;

    let result = loop {
        let now = Instant::now();
        if auto_trigger && now >= next_auto_trigger {
            audio_engine
                .lock()
                .unwrap()
                .trigger_instrument_with_velocity("granulator", current_velocity);
            trigger_count += 1;

            let cloud_ms = granulator.lock().unwrap().cloud_duration_ms();
            let interval = Duration::from_millis((cloud_ms.max(100.0) * 0.75) as u64);
            next_auto_trigger = now + interval;
            needs_redraw = true;
        }

        if needs_redraw || last_redraw.elapsed() > Duration::from_millis(100) {
            let g = granulator.lock().unwrap();
            render_display(
                &g,
                selected_param,
                trigger_count,
                current_velocity,
                auto_trigger,
                &source_name,
                seed,
            );
            needs_redraw = false;
            last_redraw = Instant::now();
        }

        if event::poll(Duration::from_millis(16))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match code {
                    KeyCode::Up => {
                        selected_param = selected_param.saturating_sub(1);
                        needs_redraw = true;
                    }
                    KeyCode::Down => {
                        selected_param = (selected_param + 1).min(PARAM_INFO.len() - 1);
                        needs_redraw = true;
                    }
                    KeyCode::Left => {
                        let step = PARAM_INFO[selected_param].coarse_step;
                        adjust_param(&mut granulator.lock().unwrap(), selected_param, -step);
                        needs_redraw = true;
                    }
                    KeyCode::Right => {
                        let step = PARAM_INFO[selected_param].coarse_step;
                        adjust_param(&mut granulator.lock().unwrap(), selected_param, step);
                        needs_redraw = true;
                    }
                    KeyCode::Char('[') => {
                        let step = PARAM_INFO[selected_param].fine_step;
                        adjust_param(&mut granulator.lock().unwrap(), selected_param, -step);
                        needs_redraw = true;
                    }
                    KeyCode::Char(']') => {
                        let step = PARAM_INFO[selected_param].fine_step;
                        adjust_param(&mut granulator.lock().unwrap(), selected_param, step);
                        needs_redraw = true;
                    }
                    KeyCode::Char(' ') => {
                        audio_engine
                            .lock()
                            .unwrap()
                            .trigger_instrument_with_velocity("granulator", current_velocity);
                        trigger_count += 1;
                        needs_redraw = true;
                    }
                    KeyCode::Char('a') | KeyCode::Char('A') => {
                        auto_trigger = !auto_trigger;
                        next_auto_trigger = Instant::now();
                        needs_redraw = true;
                    }
                    KeyCode::Char('z') | KeyCode::Char('Z') => {
                        current_velocity = 0.25;
                        audio_engine
                            .lock()
                            .unwrap()
                            .trigger_instrument_with_velocity("granulator", current_velocity);
                        trigger_count += 1;
                        needs_redraw = true;
                    }
                    KeyCode::Char('x') | KeyCode::Char('X') => {
                        current_velocity = 0.50;
                        audio_engine
                            .lock()
                            .unwrap()
                            .trigger_instrument_with_velocity("granulator", current_velocity);
                        trigger_count += 1;
                        needs_redraw = true;
                    }
                    KeyCode::Char('c') | KeyCode::Char('C') => {
                        current_velocity = 0.75;
                        audio_engine
                            .lock()
                            .unwrap()
                            .trigger_instrument_with_velocity("granulator", current_velocity);
                        trigger_count += 1;
                        needs_redraw = true;
                    }
                    KeyCode::Char('v') | KeyCode::Char('V') => {
                        current_velocity = 1.0;
                        audio_engine
                            .lock()
                            .unwrap()
                            .trigger_instrument_with_velocity("granulator", current_velocity);
                        trigger_count += 1;
                        needs_redraw = true;
                    }
                    KeyCode::Char('r') | KeyCode::Char('R') => {
                        seed = seed.wrapping_add(0x9e37_79b9);
                        granulator.lock().unwrap().set_seed(seed);
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
    println!("This example requires the 'native' feature.");
}
