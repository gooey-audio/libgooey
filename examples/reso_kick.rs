//! Interactive lab for auditioning the dual-resonator kick voice.

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use gooey::effects::{Effect, EntityDynamics, SoftLimiter};
use gooey::engine::{Engine, EngineOutput, Instrument};
use gooey::frame::StereoFrame;
use gooey::instruments::{
    ResoKick, ResoKickConfig, ResonatorVoice, ResonatorVoiceConfig, UltraPercBodyMode,
    UltraPercConfig, UltraPercNoiseMode, UltraPercVoice,
};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

struct ParamInfo {
    name: &'static str,
    coarse_step: f32,
    fine_step: f32,
}

const PARAM_INFO: [ParamInfo; 13] = [
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
    ParamInfo {
        name: "bass_drive",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
    ParamInfo {
        name: "gain_dist",
        coarse_step: 0.05,
        fine_step: 0.01,
    },
    ParamInfo {
        name: "dynamics",
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

struct SharedResonatorVoice(Arc<Mutex<ResonatorVoice>>);

impl Instrument for SharedResonatorVoice {
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

struct SharedUltraPercVoice(Arc<Mutex<UltraPercVoice>>);

impl Instrument for SharedUltraPercVoice {
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

#[derive(Clone, Copy, Eq, PartialEq)]
enum LabPage {
    Legacy,
    Macros,
    Advanced,
    UltraPerc,
}

const GENERIC_MACROS: [&str; 9] = [
    "pitch",
    "pitch_sweep",
    "sweep_time",
    "decay",
    "body_character",
    "noise",
    "coupling",
    "drive",
    "volume",
];

const ADVANCED_NAMES: [&str; 8] = [
    "mode1 ratio",
    "mode1 decay",
    "mode1 feedback",
    "mode2 ratio",
    "mode2 decay",
    "mode2 feedback",
    "mode1->mode2",
    "noise level",
];

const ULTRA_PERC_NAMES: [&str; 14] = [
    "master tune",
    "detune down",
    "length",
    "body bias",
    "FM decay",
    "bipolar FM depth",
    "body trigger delay",
    "harmonics / fold",
    "noise filter",
    "noise decay",
    "noise bias",
    "volume",
    "body spectral mode",
    "noise routing mode",
];

fn ultra_value(voice: &UltraPercVoice, index: usize) -> f32 {
    if index < 12 {
        voice.parameter_normalized(index).unwrap_or(0.0)
    } else {
        let config = voice.config_targets();
        match (index, config.body_mode, config.noise_mode) {
            (12, UltraPercBodyMode::Low, _) => 0.0,
            (12, UltraPercBodyMode::Mid, _) => 0.5,
            (12, UltraPercBodyMode::High, _) => 1.0,
            (13, _, UltraPercNoiseMode::Lowpass) => 0.0,
            (13, _, UltraPercNoiseMode::Highpass) => 0.5,
            (13, _, UltraPercNoiseMode::Body) => 1.0,
            _ => 0.0,
        }
    }
}

fn set_ultra_value(voice: &mut UltraPercVoice, index: usize, value: f32) {
    if index < 12 {
        voice.set_parameter_normalized(index, value);
    } else if index == 12 {
        voice.set_body_mode(if value < 0.25 {
            UltraPercBodyMode::Low
        } else if value < 0.75 {
            UltraPercBodyMode::Mid
        } else {
            UltraPercBodyMode::High
        });
    } else {
        voice.set_noise_mode(if value < 0.25 {
            UltraPercNoiseMode::Lowpass
        } else if value < 0.75 {
            UltraPercNoiseMode::Highpass
        } else {
            UltraPercNoiseMode::Body
        });
    }
}

fn ultra_detail(voice: &UltraPercVoice, index: usize) -> String {
    let config = voice.config_targets();
    match index {
        0 => format!("{:>7.1} Hz", config.master_tune_hz),
        1 => format!("{:>7.2} oct", config.detune_octaves),
        2 => format!("{:>7.3} s", config.length_seconds),
        3 => format!("{:>+7.2}", config.body_bias),
        4 => format!("{:>7.3} s", config.fm_decay_seconds),
        5 => format!("{:>+7.2} oct", config.fm_depth_octaves),
        6 => format!("{:>7.1} ms", config.trigger_delay_seconds * 1_000.0),
        7 => format!("{:>7.0}%", config.harmonics * 100.0),
        8 => format!("{:>7.0} Hz", config.noise_filter_hz),
        9 => format!("{:>7.3} s", config.noise_decay_seconds),
        10 => format!("{:>+7.2}", config.noise_bias),
        11 => format!("{:>7.2} x", config.volume),
        12 => format!("{:>10?}", config.body_mode),
        13 => format!("{:>10?}", config.noise_mode),
        _ => String::new(),
    }
}

fn render_ultra_display(voice: &UltraPercVoice, selected: usize, preset_name: &str, velocity: f32) {
    print!("\x1b[2J\x1b[H\x1b[?7l");
    print!("=== Resonator Voice Lab / ULTRA-PERC-INSPIRED ENGINE ===\r\n");
    print!("E=compare resonator  TAB=page  SPACE=hit  Q=quit  arrows/[]=adjust\r\n");
    print!("7=kick 8=tom 9=snare 0=clap -=metallic\r\n");
    print!(
        "Preset: {preset_name} | Velocity: {:.0}%\r\n\r\n",
        velocity * 100.0
    );
    for (index, name) in ULTRA_PERC_NAMES.iter().enumerate() {
        let value = ultra_value(voice, index);
        let indicator = if index == selected { ">" } else { " " };
        print!(
            "{} {:<22} [{}] {:>4.2}  {}\r\n",
            indicator,
            name,
            make_bar(value, 12),
            value,
            ultra_detail(voice, index)
        );
    }
    print!(
        "\r\nSignal: delayed strike → serial twin cores → LO/MID/HI → wavefolder → body VCA\r\n"
    );
    print!("        noise → LP/HP direct mix, or LP → body cores; separate noise envelope\r\n");
    io::stdout().flush().unwrap();
}

fn generic_macro_value(voice: &ResonatorVoice, index: usize) -> f32 {
    match index {
        0 => voice.params.pitch.target(),
        1 => voice.params.pitch_sweep.target(),
        2 => voice.params.sweep_time.target(),
        3 => voice.params.decay.target(),
        4 => voice.params.body_character.target(),
        5 => voice.params.noise.target(),
        6 => voice.params.coupling.target(),
        7 => voice.params.drive.target(),
        8 => voice.params.volume.target(),
        _ => 0.0,
    }
}

fn advanced_value(config: &ResonatorVoiceConfig, index: usize) -> f32 {
    match index {
        0 => config.mode1.frequency_ratio / 8.0,
        1 => config.mode1.decay_seconds / 5.0,
        2 => config.mode1.feedback / 0.8,
        3 => config.mode2.frequency_ratio / 8.0,
        4 => config.mode2.decay_seconds / 5.0,
        5 => config.mode2.feedback / 0.8,
        6 => config.routing.mode1_to_mode2,
        7 => config.noise.level / 2.0,
        _ => 0.0,
    }
    .clamp(0.0, 1.0)
}

fn set_advanced(
    voice: &mut ResonatorVoice,
    config: &mut ResonatorVoiceConfig,
    index: usize,
    normalized: f32,
) {
    let value = normalized.clamp(0.0, 1.0);
    match index {
        0 => {
            config.mode1.frequency_ratio = 0.125 + value * 7.875;
            voice.set_mode_frequency_ratio(0, config.mode1.frequency_ratio);
        }
        1 => {
            config.mode1.decay_seconds = 0.005 + value * 4.995;
            voice.set_mode_decay_seconds(0, config.mode1.decay_seconds);
        }
        2 => {
            config.mode1.feedback = value * 0.8;
            voice.set_mode_feedback(0, config.mode1.feedback);
        }
        3 => {
            config.mode2.frequency_ratio = 0.125 + value * 7.875;
            voice.set_mode_frequency_ratio(1, config.mode2.frequency_ratio);
        }
        4 => {
            config.mode2.decay_seconds = 0.005 + value * 4.995;
            voice.set_mode_decay_seconds(1, config.mode2.decay_seconds);
        }
        5 => {
            config.mode2.feedback = value * 0.8;
            voice.set_mode_feedback(1, config.mode2.feedback);
        }
        6 => {
            config.routing.mode1_to_mode2 = value;
            voice.set_routing(config.routing);
        }
        7 => {
            config.noise.level = value * 2.0;
            voice.set_noise_config(config.noise);
        }
        _ => {}
    }
}

fn render_generic_display(
    voice: &ResonatorVoice,
    config: &ResonatorVoiceConfig,
    page: LabPage,
    selected: usize,
    preset_name: &str,
    velocity: f32,
) {
    print!("\x1b[2J\x1b[H\x1b[?7l");
    let page_name = if page == LabPage::Macros {
        "MACROS"
    } else {
        "ADVANCED"
    };
    print!("=== Resonator Voice Lab / {page_name} ===\r\n");
    print!("E=compare Ultra-Perc-inspired engine  TAB=page  SPACE=hit  Q=quit\r\n");
    print!("arrows/[]=adjust  1-6=legacy  7=kick 8=tom 9=snare 0=clap/hybrid -=metallic\r\n");
    print!(
        "Preset: {preset_name} | Velocity: {:.0}%\r\n\r\n",
        velocity * 100.0
    );
    if page == LabPage::Macros {
        for (index, name) in GENERIC_MACROS.iter().enumerate() {
            let value = generic_macro_value(voice, index);
            let indicator = if index == selected { ">" } else { " " };
            print!(
                "{} {:<17} [{}] {:>4.2}\r\n",
                indicator,
                name,
                make_bar(value, 12),
                value
            );
        }
    } else {
        for (index, name) in ADVANCED_NAMES.iter().enumerate() {
            let value = advanced_value(config, index);
            let indicator = if index == selected { ">" } else { " " };
            print!(
                "{} {:<17} [{}] {:>4.2}\r\n",
                indicator,
                name,
                make_bar(value, 12),
                value
            );
        }
    }
    print!("\r\nSignal: transient + noise → two routed modes → driven tap mix\r\n");
    io::stdout().flush().unwrap();
}

struct SharedEntityDynamics(Arc<EntityDynamics>);

impl Effect for SharedEntityDynamics {
    fn process(&self, input: f32) -> f32 {
        self.0.process(input)
    }

    fn process_stereo(&self, input: StereoFrame) -> StereoFrame {
        self.0.process_stereo(input)
    }
}

fn gain_dist_to_normalized(db: f32) -> f32 {
    (db + 12.0) / 36.0
}

fn normalized_to_gain_dist(value: f32) -> f32 {
    -12.0 + value * 36.0
}

fn get_param_value(kick: &ResoKick, dynamics: &EntityDynamics, index: usize) -> f32 {
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
        10 => dynamics.bass_drive(),
        11 => gain_dist_to_normalized(dynamics.gain_dist_db()),
        12 => dynamics.dynamics(),
        _ => 0.0,
    }
}

fn set_param_value(kick: &mut ResoKick, dynamics: &EntityDynamics, index: usize, value: f32) {
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
        10 => dynamics.set_bass_drive(value),
        11 => dynamics.set_gain_dist_db(normalized_to_gain_dist(value)),
        12 => dynamics.set_dynamics(value),
        _ => {}
    }
}

fn adjust_param(kick: &mut ResoKick, dynamics: &EntityDynamics, index: usize, delta: f32) {
    set_param_value(
        kick,
        dynamics,
        index,
        (get_param_value(kick, dynamics, index) + delta).clamp(0.0, 1.0),
    );
}

fn make_bar(value: f32, width: usize) -> String {
    let filled = (value.clamp(0.0, 1.0) * width as f32).round() as usize;
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

fn detail(kick: &ResoKick, dynamics: &EntityDynamics, index: usize) -> String {
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
        10 => format!("{:>6.1} Hz", dynamics.bass_drive_hz()),
        11 => format!("{:+6.1} dB", dynamics.gain_dist_db()),
        12 => format!("{:>6.0}% wet", dynamics.dynamics() * 100.0),
        _ => String::new(),
    }
}

fn render_display(
    kick: &ResoKick,
    dynamics: &EntityDynamics,
    selected: usize,
    trigger_count: u32,
    velocity: f32,
    preset_name: &str,
) {
    print!("\x1b[2J\x1b[H\x1b[?7l");
    print!("=== Reso Kick Lab ===\r\n");
    print!("SPACE=hit  Q=quit  ↑↓=select  ←→=adjust  []=fine  TAB=page\r\n");
    print!("V=velocity  E=Ultra-Perc engine  1-6=legacy  7-9/0/-=engine presets\r\n");
    print!("Preset: {preset_name}\r\n\r\n");

    for (index, info) in PARAM_INFO.iter().enumerate() {
        let value = get_param_value(kick, dynamics, index);
        let indicator = if index == selected { ">" } else { " " };
        print!(
            "{} {:<15} [{}] {:>4.2}  {}\r\n",
            indicator,
            info.name,
            make_bar(value, 12),
            value,
            detail(kick, dynamics, index)
        );
    }

    print!(
        "\r\nHits: {trigger_count} | Velocity: {:.0}%\r\n",
        velocity * 100.0
    );
    print!("Signal: body → Punch ‖ character ping → Entity Dynamics → safety limiter\r\n");
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
    let mut generic_config = ResonatorVoiceConfig::kick();
    let generic = Arc::new(Mutex::new(ResonatorVoice::with_config(
        sample_rate,
        generic_config,
    )));
    let ultra = Arc::new(Mutex::new(UltraPercVoice::with_config(
        sample_rate,
        UltraPercConfig::kick(),
    )));
    let dynamics = Arc::new(EntityDynamics::new(sample_rate));
    dynamics.set_bass_drive(0.55);
    dynamics.set_gain_dist_db(6.0);
    dynamics.set_dynamics(0.75);

    let mut engine = Engine::new(sample_rate);
    engine.set_master_gain(0.85);
    engine.add_instrument("reso_kick", Box::new(SharedResoKick(kick.clone())));
    engine.add_instrument(
        "resonator_voice",
        Box::new(SharedResonatorVoice(generic.clone())),
    );
    engine.add_instrument(
        "ultra_perc_voice",
        Box::new(SharedUltraPercVoice(ultra.clone())),
    );
    engine.clear_global_effects();
    engine.add_global_effect(Box::new(SharedEntityDynamics(dynamics.clone())));
    engine.add_global_effect(Box::new(SoftLimiter::new(1.0)));
    let audio_engine = Arc::new(Mutex::new(engine));

    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;
    engine_output.create_stream_with_engine(audio_engine.clone())?;
    engine_output.start()?;

    let mut selected = 0;
    let mut page = LabPage::Legacy;
    let mut trigger_count = 0;
    let velocities = [0.25, 0.5, 0.75, 1.0];
    let mut velocity_index = 2;
    let mut preset_name = "Classic 808";
    let mut needs_redraw = true;

    execute!(io::stdout(), Clear(ClearType::All), cursor::Hide)?;
    enable_raw_mode()?;

    let result = loop {
        if needs_redraw {
            if page == LabPage::Legacy {
                let voice = kick.lock().unwrap();
                render_display(
                    &voice,
                    &dynamics,
                    selected,
                    trigger_count,
                    velocities[velocity_index],
                    preset_name,
                );
            } else {
                if page == LabPage::UltraPerc {
                    render_ultra_display(
                        &ultra.lock().unwrap(),
                        selected,
                        preset_name,
                        velocities[velocity_index],
                    );
                } else {
                    let voice = generic.lock().unwrap();
                    render_generic_display(
                        &voice,
                        &generic_config,
                        page,
                        selected,
                        preset_name,
                        velocities[velocity_index],
                    );
                }
            }
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
                        let len = match page {
                            LabPage::Legacy => PARAM_INFO.len(),
                            LabPage::Macros => GENERIC_MACROS.len(),
                            LabPage::Advanced => ADVANCED_NAMES.len(),
                            LabPage::UltraPerc => ULTRA_PERC_NAMES.len(),
                        };
                        selected = (selected + 1).min(len - 1);
                        needs_redraw = true;
                    }
                    KeyCode::Left | KeyCode::Right | KeyCode::Char('[') | KeyCode::Char(']') => {
                        let (coarse, fine) = if page == LabPage::Legacy {
                            let info = &PARAM_INFO[selected];
                            (info.coarse_step, info.fine_step)
                        } else {
                            (0.05, 0.01)
                        };
                        let delta = match code {
                            KeyCode::Left => -coarse,
                            KeyCode::Right => coarse,
                            KeyCode::Char('[') => -fine,
                            KeyCode::Char(']') => fine,
                            _ => unreachable!(),
                        };
                        match page {
                            LabPage::Legacy => {
                                adjust_param(&mut kick.lock().unwrap(), &dynamics, selected, delta)
                            }
                            LabPage::Macros => {
                                let mut voice = generic.lock().unwrap();
                                let value =
                                    (generic_macro_value(&voice, selected) + delta).clamp(0.0, 1.0);
                                voice.set_macro(GENERIC_MACROS[selected], value).unwrap();
                            }
                            LabPage::Advanced => {
                                let value = (advanced_value(&generic_config, selected) + delta)
                                    .clamp(0.0, 1.0);
                                set_advanced(
                                    &mut generic.lock().unwrap(),
                                    &mut generic_config,
                                    selected,
                                    value,
                                );
                            }
                            LabPage::UltraPerc => {
                                let mut voice = ultra.lock().unwrap();
                                let value = (ultra_value(&voice, selected) + delta).clamp(0.0, 1.0);
                                set_ultra_value(&mut voice, selected, value);
                            }
                        }
                        preset_name = "Custom";
                        needs_redraw = true;
                    }
                    KeyCode::Char(' ') => {
                        let instrument = match page {
                            LabPage::Legacy => "reso_kick",
                            LabPage::UltraPerc => "ultra_perc_voice",
                            LabPage::Macros | LabPage::Advanced => "resonator_voice",
                        };
                        audio_engine
                            .lock()
                            .unwrap()
                            .trigger_instrument_with_velocity(
                                instrument,
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
                            page = LabPage::Legacy;
                            selected = 0;
                            needs_redraw = true;
                        }
                    }
                    KeyCode::Char(number @ '7'..='9') => {
                        let (name, config, ultra_config) = match number {
                            '7' => (
                                "Kick",
                                ResonatorVoiceConfig::kick(),
                                UltraPercConfig::kick(),
                            ),
                            '8' => ("Tom", ResonatorVoiceConfig::tom(), UltraPercConfig::tom()),
                            _ => (
                                "Snare",
                                ResonatorVoiceConfig::snare(),
                                UltraPercConfig::snare(),
                            ),
                        };
                        generic_config = config;
                        *generic.lock().unwrap() = ResonatorVoice::with_config(sample_rate, config);
                        *ultra.lock().unwrap() =
                            UltraPercVoice::with_config(sample_rate, ultra_config);
                        if page == LabPage::Legacy {
                            page = LabPage::Macros;
                        }
                        selected = 0;
                        preset_name = name;
                        needs_redraw = true;
                    }
                    KeyCode::Char('0') | KeyCode::Char('-') => {
                        let (name, config, ultra_config) = if code == KeyCode::Char('0') {
                            (
                                "Clap / Hybrid",
                                ResonatorVoiceConfig::hybrid(),
                                UltraPercConfig::clap(),
                            )
                        } else {
                            (
                                "Metallic",
                                ResonatorVoiceConfig::metallic_drone(),
                                UltraPercConfig::metallic(),
                            )
                        };
                        generic_config = config;
                        *generic.lock().unwrap() = ResonatorVoice::with_config(sample_rate, config);
                        *ultra.lock().unwrap() =
                            UltraPercVoice::with_config(sample_rate, ultra_config);
                        if page == LabPage::Legacy {
                            page = LabPage::Macros;
                        }
                        selected = 0;
                        preset_name = name;
                        needs_redraw = true;
                    }
                    KeyCode::Tab => {
                        page = match page {
                            LabPage::Legacy => LabPage::Macros,
                            LabPage::Macros => LabPage::Advanced,
                            LabPage::Advanced => LabPage::UltraPerc,
                            LabPage::UltraPerc => LabPage::Legacy,
                        };
                        selected = 0;
                        needs_redraw = true;
                    }
                    KeyCode::Char('e') | KeyCode::Char('E') => {
                        page = if page == LabPage::UltraPerc {
                            LabPage::Macros
                        } else {
                            LabPage::UltraPerc
                        };
                        selected = 0;
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
        "This example requires native audio and crossterm. Run: cargo run --example reso_kick --features native,crossterm"
    );
}
