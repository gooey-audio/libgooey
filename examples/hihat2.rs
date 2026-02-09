/* HiHat2 Lab - Interactive CLI for hihat2 parameter experimentation.
Supports real-time parameter adjustment, noise/slope toggles, and velocity control.
*/

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use gooey::engine::{Engine, EngineOutput, Instrument};
use gooey::instruments::{FilterSlope, HiHat2, HiHat2Config, NoiseColor};

struct ParamInfo {
    name: &'static str,
    coarse_step: f32,
    fine_step: f32,
    unit: &'static str,
}

const PARAM_INFO: [ParamInfo; 5] = [
    ParamInfo { name: "Pitch", coarse_step: 0.05, fine_step: 0.01, unit: "Hz" },
    ParamInfo { name: "Decay", coarse_step: 0.05, fine_step: 0.01, unit: "ms" },
    ParamInfo { name: "Attack", coarse_step: 0.05, fine_step: 0.01, unit: "ms" },
    ParamInfo { name: "Tone", coarse_step: 0.05, fine_step: 0.01, unit: "Hz" },
    ParamInfo { name: "Volume", coarse_step: 0.05, fine_step: 0.01, unit: "" },
];

struct SharedHiHat2(Arc<Mutex<HiHat2>>);

impl Instrument for SharedHiHat2 {
    fn trigger_with_velocity(&mut self, time: f32, velocity: f32) {
        self.0.lock().unwrap().trigger_with_velocity(time, velocity);
    }

    fn tick(&mut self, current_time: f32) -> f32 {
        self.0.lock().unwrap().tick(current_time)
    }

    fn is_active(&self) -> bool {
        self.0.lock().unwrap().is_active()
    }

    fn as_modulatable(&mut self) -> Option<&mut dyn gooey::engine::Modulatable> {
        None
    }
}

fn get_param_value(hihat: &HiHat2, index: usize) -> f32 {
    match index {
        0 => hihat.params.pitch.get(),
        1 => hihat.params.decay.get(),
        2 => hihat.params.attack.get(),
        3 => hihat.params.tone.get(),
        4 => hihat.params.volume.get(),
        _ => 0.0,
    }
}

fn set_param_value(hihat: &mut HiHat2, index: usize, value: f32) {
    match index {
        0 => hihat.set_pitch(value),
        1 => hihat.set_decay(value),
        2 => hihat.set_attack(value),
        3 => hihat.set_tone(value),
        4 => hihat.set_volume(value),
        _ => {}
    }
}

fn adjust_param(hihat: &mut HiHat2, index: usize, delta: f32) {
    let current = get_param_value(hihat, index);
    let new_value = (current + delta).clamp(0.0, 1.0);
    set_param_value(hihat, index, new_value);
}

fn make_bar(normalized: f32, width: usize) -> String {
    let filled = (normalized * width as f32).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    format!("{}{}", "=".repeat(filled), "-".repeat(empty))
}

fn display_value(index: usize, normalized: f32) -> f32 {
    match index {
        0 => {
            let curved = normalized * normalized;
            3500.0 + curved * (10000.0 - 3500.0)
        }
        1 => 0.5 + normalized * (4000.0 - 0.5),
        2 => 0.5 + normalized * (200.0 - 0.5),
        3 => 500.0 + normalized * (10000.0 - 500.0),
        4 => normalized,
        _ => normalized,
    }
}

fn render_display(
    hihat: &HiHat2,
    selected: usize,
    trigger_count: u32,
    velocity: f32,
    noise_color: NoiseColor,
    filter_slope: FilterSlope,
) {
    print!("\x1b[2J\x1b[H\x1b[?7l");

    print!("=== HiHat2 Lab ===\r\n");
    print!("SPACE=hit Q=quit arrows=sel/adj []=fine N=noise S=slope V/B=velocity\r\n");
    print!("\r\n");

    print!(
        "Noise: {:?} | Slope: {}dB | Vel: {:.0}% | Hits: {}\r\n",
        noise_color,
        if filter_slope == FilterSlope::Db24 { 24 } else { 12 },
        velocity * 100.0,
        trigger_count
    );
    print!("\r\n");

    for (i, info) in PARAM_INFO.iter().enumerate() {
        let value = get_param_value(hihat, i);
        let bar = make_bar(value, 10);
        let display = display_value(i, value);

        let indicator = if i == selected { ">" } else { " " };

        print!(
            "{} {:<8} [{}] {:>7.2}{}\r\n",
            indicator, info.name, bar, display, info.unit
        );
    }

    print!("\r\n");
    io::stdout().flush().unwrap();
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;

    let hihat = Arc::new(Mutex::new(HiHat2::with_config(
        sample_rate,
        HiHat2Config::default(),
    )));
    let shared_hihat = SharedHiHat2(hihat.clone());

    let mut engine = Engine::new(sample_rate);
    engine.add_instrument("hihat2", Box::new(shared_hihat));

    let audio_engine = Arc::new(Mutex::new(engine));

    enable_raw_mode()?;

    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;

    #[cfg(feature = "visualization")]
    engine_output.enable_visualization(1200, 400, 2.0)?;

    engine_output.create_stream_with_engine(audio_engine.clone())?;
    engine_output.start()?;

    let mut selected_param = 0;
    let mut trigger_count = 0u32;
    let mut velocity = 0.8f32;
    let mut noise_color = NoiseColor::White;
    let mut filter_slope = FilterSlope::Db24;

    {
        let hihat_lock = hihat.lock().unwrap();
        render_display(
            &hihat_lock,
            selected_param,
            trigger_count,
            velocity,
            noise_color,
            filter_slope,
        );
    }

    let result = loop {
        #[cfg(feature = "visualization")]
        if engine_output.update_visualization() {
            break Ok(());
        }

        if event::poll(std::time::Duration::from_millis(16))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                let mut needs_redraw = true;

                match code {
                    KeyCode::Char(' ') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("hihat2", velocity);
                        trigger_count += 1;
                    }
                    KeyCode::Char('q') | KeyCode::Esc => {
                        break Ok(());
                    }
                    KeyCode::Up => {
                        if selected_param > 0 {
                            selected_param -= 1;
                        }
                    }
                    KeyCode::Down => {
                        if selected_param < PARAM_INFO.len() - 1 {
                            selected_param += 1;
                        }
                    }
                    KeyCode::Left => {
                        let mut hihat_lock = hihat.lock().unwrap();
                        adjust_param(&mut hihat_lock, selected_param, -PARAM_INFO[selected_param].coarse_step);
                    }
                    KeyCode::Right => {
                        let mut hihat_lock = hihat.lock().unwrap();
                        adjust_param(&mut hihat_lock, selected_param, PARAM_INFO[selected_param].coarse_step);
                    }
                    KeyCode::Char('[') => {
                        let mut hihat_lock = hihat.lock().unwrap();
                        adjust_param(&mut hihat_lock, selected_param, -PARAM_INFO[selected_param].fine_step);
                    }
                    KeyCode::Char(']') => {
                        let mut hihat_lock = hihat.lock().unwrap();
                        adjust_param(&mut hihat_lock, selected_param, PARAM_INFO[selected_param].fine_step);
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') => {
                        noise_color = if noise_color == NoiseColor::White {
                            NoiseColor::Pink
                        } else {
                            NoiseColor::White
                        };
                        let mut hihat_lock = hihat.lock().unwrap();
                        hihat_lock.set_noise_color(noise_color);
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        filter_slope = if filter_slope == FilterSlope::Db24 {
                            FilterSlope::Db12
                        } else {
                            FilterSlope::Db24
                        };
                        let mut hihat_lock = hihat.lock().unwrap();
                        hihat_lock.set_filter_slope(filter_slope);
                    }
                    KeyCode::Char('v') | KeyCode::Char('V') => {
                        velocity = (velocity - 0.1).max(0.1);
                    }
                    KeyCode::Char('b') | KeyCode::Char('B') => {
                        velocity = (velocity + 0.1).min(1.0);
                    }
                    _ => {
                        needs_redraw = false;
                    }
                }

                if needs_redraw {
                    let hihat_lock = hihat.lock().unwrap();
                    render_display(
                        &hihat_lock,
                        selected_param,
                        trigger_count,
                        velocity,
                        noise_color,
                        filter_slope,
                    );
                }
            }
        }
    };

    execute!(io::stdout(), Clear(ClearType::All), cursor::MoveTo(0, 0))?;
    disable_raw_mode()?;

    result
}

#[cfg(not(feature = "native"))]
fn main() {
    println!("This example is only available with the 'native' feature enabled.");
}
