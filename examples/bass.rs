/* Bass Synth Lab - Interactive CLI for bass synth parameter experimentation.
Supports real-time parameter adjustment, presets, velocity control, and blend mode.
*/

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use std::io::{self, Write};

use gooey::engine::{Engine, EngineOutput, Instrument};
use gooey::instruments::{BassConfig, BassSynth};
use gooey::utils::PresetBlender;
use std::sync::{Arc, Mutex};

struct ParamInfo {
    name: &'static str,
    coarse_step: f32,
    fine_step: f32,
    unit: &'static str,
}

const PARAM_INFO: [ParamInfo; 15] = [
    ParamInfo {
        name: "amp_decay",
        coarse_step: 0.1,
        fine_step: 0.02,
        unit: "",
    },
    ParamInfo {
        name: "amp_dcy_crv",
        coarse_step: 0.1,
        fine_step: 0.02,
        unit: "",
    },
    ParamInfo {
        name: "detune_amt",
        coarse_step: 0.1,
        fine_step: 0.02,
        unit: "",
    },
    ParamInfo {
        name: "detune_lvl",
        coarse_step: 0.1,
        fine_step: 0.02,
        unit: "",
    },
    ParamInfo {
        name: "filt_cutoff",
        coarse_step: 0.1,
        fine_step: 0.02,
        unit: "",
    },
    ParamInfo {
        name: "filt_env_amt",
        coarse_step: 0.1,
        fine_step: 0.02,
        unit: "",
    },
    ParamInfo {
        name: "filt_env_crv",
        coarse_step: 0.1,
        fine_step: 0.02,
        unit: "",
    },
    ParamInfo {
        name: "filt_env_dcy",
        coarse_step: 0.1,
        fine_step: 0.02,
        unit: "",
    },
    ParamInfo {
        name: "filt_reso",
        coarse_step: 0.1,
        fine_step: 0.02,
        unit: "",
    },
    ParamInfo {
        name: "frequency",
        coarse_step: 0.1,
        fine_step: 0.02,
        unit: "",
    },
    ParamInfo {
        name: "osc_level",
        coarse_step: 0.1,
        fine_step: 0.02,
        unit: "",
    },
    ParamInfo {
        name: "osc_shape",
        coarse_step: 0.1,
        fine_step: 0.02,
        unit: "",
    },
    ParamInfo {
        name: "overdrive",
        coarse_step: 0.1,
        fine_step: 0.02,
        unit: "",
    },
    ParamInfo {
        name: "sub_level",
        coarse_step: 0.1,
        fine_step: 0.02,
        unit: "",
    },
    ParamInfo {
        name: "volume",
        coarse_step: 0.1,
        fine_step: 0.02,
        unit: "",
    },
];

// Wrapper to share BassSynth between audio thread and main thread
struct SharedBass(Arc<Mutex<BassSynth>>);

impl Instrument for SharedBass {
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        self.0
            .lock()
            .unwrap()
            .trigger_with_velocity(time, velocity);
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

// Parameter accessors (alphabetical order matching PARAM_INFO)
fn get_param_value(bass: &BassSynth, index: usize) -> f32 {
    match index {
        0 => bass.params.amp_decay.get(),
        1 => bass.params.amp_decay_curve.get(),
        2 => bass.params.detune_amount.get(),
        3 => bass.params.detune_level.get(),
        4 => bass.params.filter_cutoff.get(),
        5 => bass.params.filter_env_amount.get(),
        6 => bass.params.filter_env_curve.get(),
        7 => bass.params.filter_env_decay.get(),
        8 => bass.params.filter_resonance.get(),
        9 => bass.params.frequency.get(),
        10 => bass.params.osc_level.get(),
        11 => bass.params.osc_shape.get(),
        12 => bass.params.overdrive.get(),
        13 => bass.params.sub_level.get(),
        14 => bass.params.volume.get(),
        _ => 0.0,
    }
}

fn set_param_value(bass: &mut BassSynth, index: usize, value: f32) {
    match index {
        0 => bass.set_amp_decay(value),
        1 => bass.set_amp_decay_curve(value),
        2 => bass.set_detune_amount(value),
        3 => bass.set_detune_level(value),
        4 => bass.set_filter_cutoff(value),
        5 => bass.set_filter_env_amount(value),
        6 => bass.set_filter_env_curve(value),
        7 => bass.set_filter_env_decay(value),
        8 => bass.set_filter_resonance(value),
        9 => bass.set_frequency(value),
        10 => bass.set_osc_level(value),
        11 => bass.set_osc_shape(value),
        12 => bass.set_overdrive(value),
        13 => bass.set_sub_level(value),
        14 => bass.set_volume(value),
        _ => {}
    }
}

fn adjust_param(bass: &mut BassSynth, index: usize, delta: f32) {
    let current = get_param_value(bass, index);
    let new_value = (current + delta).clamp(0.0, 1.0);
    set_param_value(bass, index, new_value);
}

fn make_bar(normalized: f32, width: usize) -> String {
    let filled = (normalized * width as f32).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

struct BlendState {
    enabled: bool,
    x: f32,
    y: f32,
    blender: PresetBlender<BassConfig>,
}

impl BlendState {
    fn new() -> Self {
        Self {
            enabled: false,
            x: 0.5,
            y: 0.5,
            blender: PresetBlender::new(
                BassConfig::acid(),  // Bottom-left (0,0)
                BassConfig::sub(),   // Bottom-right (1,0)
                BassConfig::reese(), // Top-left (0,1)
                BassConfig::stab(),  // Top-right (1,1)
            ),
        }
    }
}

fn render_display(
    bass: &BassSynth,
    selected: usize,
    trigger_count: u32,
    velocity: f32,
    preset_name: &str,
    blend: &BlendState,
) {
    print!("\x1b[2J\x1b[H\x1b[?7l");

    print!("=== Bass Synth Lab ===\r\n");
    if blend.enabled {
        print!("SPACE=hit Q=quit WASD=blend B=exit blend mode\r\n");
        print!("Z/X/C/V=vel 25/50/75/100%\r\n");
    } else {
        print!("SPACE=hit Q=quit ↑↓=sel ←→=adj []=fine B=blend\r\n");
        print!("Z/X/C/V=vel 25/50/75/100% +/-=adj 1-4=preset\r\n");
    }
    print!("Preset: {}\r\n", preset_name);

    if blend.enabled {
        print!("\r\n");
        print!("  X/Y Blend Pad (WASD to move)\r\n");
        print!("  ┌──────────────────────┐\r\n");

        let pad_w = 20;
        let pad_h = 8;
        let cursor_x = (blend.x * (pad_w - 1) as f32).round() as usize;
        let cursor_y = ((1.0 - blend.y) * (pad_h - 1) as f32).round() as usize;

        for row in 0..pad_h {
            print!("  │");
            for col in 0..pad_w {
                if row == cursor_y && col == cursor_x {
                    print!("●");
                } else if row == 0 && col == 0 {
                    print!("R"); // Reese (top-left)
                } else if row == 0 && col == pad_w - 1 {
                    print!("S"); // Stab (top-right)
                } else if row == pad_h - 1 && col == 0 {
                    print!("A"); // Acid (bottom-left)
                } else if row == pad_h - 1 && col == pad_w - 1 {
                    print!("B"); // suB (bottom-right)
                } else {
                    print!("·");
                }
            }
            print!("│\r\n");
        }
        print!("  └──────────────────────┘\r\n");
        print!("  X: {:.2}  Y: {:.2}\r\n", blend.x, blend.y);
    }

    print!("\r\n");

    for (i, info) in PARAM_INFO.iter().enumerate() {
        let value = get_param_value(bass, i);
        let bar = make_bar(value, 10);

        let indicator = if i == selected { ">" } else { " " };
        print!(
            "{} {:<18} [{}] {:>6.2}{:<3}\r\n",
            indicator, info.name, bar, value, info.unit
        );
    }

    print!("\r\n");
    print!("Hits: {} | Vel: {:.0}%", trigger_count, velocity * 100.0);

    io::stdout().flush().unwrap();
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;

    let bass = Arc::new(Mutex::new(BassSynth::new(sample_rate)));

    let mut engine = Engine::new(sample_rate);
    engine.add_instrument("bass", Box::new(SharedBass(bass.clone())));

    let audio_engine = Arc::new(Mutex::new(engine));

    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;

    #[cfg(feature = "visualization")]
    engine_output.enable_visualization(1200, 400, 2.0)?;

    engine_output.create_stream_with_engine(audio_engine.clone())?;
    engine_output.start()?;

    let mut selected_param: usize = 0;
    let mut trigger_count: u32 = 0;
    let mut current_velocity: f32 = 0.75;
    let mut current_preset = "Acid";
    let mut needs_redraw = true;
    let mut blend = BlendState::new();

    execute!(io::stdout(), Clear(ClearType::All), cursor::Hide)?;
    enable_raw_mode()?;

    let result = loop {
        if engine_output.update_visualization() {
            println!("\rVisualization window closed");
            break Ok(());
        }

        if needs_redraw {
            let b = bass.lock().unwrap();
            render_display(
                &b,
                selected_param,
                trigger_count,
                current_velocity,
                current_preset,
                &blend,
            );
            needs_redraw = false;
        }

        if event::poll(std::time::Duration::from_millis(16))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match code {
                    // Parameter navigation
                    KeyCode::Up if !blend.enabled => {
                        selected_param = selected_param.saturating_sub(1);
                        needs_redraw = true;
                    }
                    KeyCode::Down if !blend.enabled => {
                        selected_param = (selected_param + 1).min(PARAM_INFO.len() - 1);
                        needs_redraw = true;
                    }

                    // Coarse adjustment
                    KeyCode::Left if !blend.enabled => {
                        let step = PARAM_INFO[selected_param].coarse_step;
                        let mut b = bass.lock().unwrap();
                        adjust_param(&mut b, selected_param, -step);
                        current_preset = "Custom";
                        needs_redraw = true;
                    }
                    KeyCode::Right if !blend.enabled => {
                        let step = PARAM_INFO[selected_param].coarse_step;
                        let mut b = bass.lock().unwrap();
                        adjust_param(&mut b, selected_param, step);
                        current_preset = "Custom";
                        needs_redraw = true;
                    }

                    // Fine adjustment
                    KeyCode::Char('[') if !blend.enabled => {
                        let step = PARAM_INFO[selected_param].fine_step;
                        let mut b = bass.lock().unwrap();
                        adjust_param(&mut b, selected_param, -step);
                        current_preset = "Custom";
                        needs_redraw = true;
                    }
                    KeyCode::Char(']') if !blend.enabled => {
                        let step = PARAM_INFO[selected_param].fine_step;
                        let mut b = bass.lock().unwrap();
                        adjust_param(&mut b, selected_param, step);
                        current_preset = "Custom";
                        needs_redraw = true;
                    }

                    // Toggle blend mode
                    KeyCode::Char('b') | KeyCode::Char('B') => {
                        blend.enabled = !blend.enabled;
                        if blend.enabled {
                            let blended = blend.blender.blend(blend.x, blend.y);
                            let mut b = bass.lock().unwrap();
                            b.set_config(blended);
                            current_preset = "Blended";
                        }
                        needs_redraw = true;
                    }

                    // Blend mode WASD
                    KeyCode::Char('w') | KeyCode::Char('W') if blend.enabled => {
                        blend.y = (blend.y + 0.05).min(1.0);
                        let blended = blend.blender.blend(blend.x, blend.y);
                        let mut b = bass.lock().unwrap();
                        b.set_config(blended);
                        current_preset = "Blended";
                        needs_redraw = true;
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') if blend.enabled => {
                        blend.y = (blend.y - 0.05).max(0.0);
                        let blended = blend.blender.blend(blend.x, blend.y);
                        let mut b = bass.lock().unwrap();
                        b.set_config(blended);
                        current_preset = "Blended";
                        needs_redraw = true;
                    }
                    KeyCode::Char('a') | KeyCode::Char('A') if blend.enabled => {
                        blend.x = (blend.x - 0.05).max(0.0);
                        let blended = blend.blender.blend(blend.x, blend.y);
                        let mut b = bass.lock().unwrap();
                        b.set_config(blended);
                        current_preset = "Blended";
                        needs_redraw = true;
                    }
                    KeyCode::Char('d') | KeyCode::Char('D') if blend.enabled => {
                        blend.x = (blend.x + 0.05).min(1.0);
                        let blended = blend.blender.blend(blend.x, blend.y);
                        let mut b = bass.lock().unwrap();
                        b.set_config(blended);
                        current_preset = "Blended";
                        needs_redraw = true;
                    }

                    // Presets
                    KeyCode::Char('1') if !blend.enabled => {
                        let mut b = bass.lock().unwrap();
                        b.set_config(BassConfig::acid());
                        current_preset = "Acid";
                        needs_redraw = true;
                    }
                    KeyCode::Char('2') if !blend.enabled => {
                        let mut b = bass.lock().unwrap();
                        b.set_config(BassConfig::sub());
                        current_preset = "Sub";
                        needs_redraw = true;
                    }
                    KeyCode::Char('3') if !blend.enabled => {
                        let mut b = bass.lock().unwrap();
                        b.set_config(BassConfig::reese());
                        current_preset = "Reese";
                        needs_redraw = true;
                    }
                    KeyCode::Char('4') if !blend.enabled => {
                        let mut b = bass.lock().unwrap();
                        b.set_config(BassConfig::stab());
                        current_preset = "Stab";
                        needs_redraw = true;
                    }

                    // Trigger
                    KeyCode::Char(' ') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("bass", current_velocity);
                        trigger_count += 1;
                        needs_redraw = true;
                    }

                    // Velocity triggers
                    KeyCode::Char('z') | KeyCode::Char('Z') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("bass", 0.25);
                        trigger_count += 1;
                        current_velocity = 0.25;
                        needs_redraw = true;
                    }
                    KeyCode::Char('x') | KeyCode::Char('X') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("bass", 0.50);
                        trigger_count += 1;
                        current_velocity = 0.50;
                        needs_redraw = true;
                    }
                    KeyCode::Char('c') | KeyCode::Char('C') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("bass", 0.75);
                        trigger_count += 1;
                        current_velocity = 0.75;
                        needs_redraw = true;
                    }
                    KeyCode::Char('v') | KeyCode::Char('V') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument_with_velocity("bass", 1.0);
                        trigger_count += 1;
                        current_velocity = 1.0;
                        needs_redraw = true;
                    }

                    // Adjust default velocity
                    KeyCode::Char('+') | KeyCode::Char('=') => {
                        current_velocity = (current_velocity + 0.05).min(1.0);
                        needs_redraw = true;
                    }
                    KeyCode::Char('-') | KeyCode::Char('_') => {
                        current_velocity = (current_velocity - 0.05).max(0.05);
                        needs_redraw = true;
                    }

                    // Quit
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                        break Ok(());
                    }
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
    println!("This example is only available with the 'native' feature enabled.");
}
