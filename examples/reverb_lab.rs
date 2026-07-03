/* Reverb Lab - Interactive CLI for comparing reverb algorithms.

Hosts every reverb the engine ships (spring, plate) as global effects and lets
you A/B them one at a time: TAB switches the active algorithm (the inactive
one's mix is forced to 0), the parameter rows adapt to the selected algorithm.
Playback is true stereo via the engine's tick_stereo path, so the plate's
cross-branch tap image and the spring's decorrelated tanks are audible.

Run: cargo run --example reverb_lab --features native,crossterm
*/

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use gooey::effects::{PlateReverbEffect, SpringReverbEffect};
use gooey::engine::{Engine, EngineOutput, Instrument, Sequencer};
use gooey::instruments::{KickDrum, SnareDrum};

// Lock-free control handles for the reverbs.
// SAFETY: both effects' setters take &self and use atomics internally. The raw
// pointers are valid as long as the engine (which owns the Box<dyn Effect>s)
// is alive.
struct ReverbControls {
    spring: *const SpringReverbEffect,
    plate: *const PlateReverbEffect,
}

unsafe impl Send for ReverbControls {}

impl ReverbControls {
    fn spring(&self) -> &SpringReverbEffect {
        unsafe { &*self.spring }
    }
    fn plate(&self) -> &PlateReverbEffect {
        unsafe { &*self.plate }
    }
}

// Wrappers to share drum voices between the audio thread and main thread.
struct SharedSnare(Arc<Mutex<SnareDrum>>);

impl Instrument for SharedSnare {
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

struct SharedKick(Arc<Mutex<KickDrum>>);

impl Instrument for SharedKick {
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

#[derive(Clone, Copy, PartialEq)]
enum Algo {
    Spring,
    Plate,
}

impl Algo {
    fn name(self) -> &'static str {
        match self {
            Algo::Spring => "SPRING",
            Algo::Plate => "PLATE",
        }
    }
}

/// One adjustable row in the UI: label + normalized value + how to push it to
/// the engine. BPM is the odd one out (60-200 range).
#[derive(Clone, Copy, PartialEq)]
enum Param {
    SpringDecay,
    SpringMix,
    SpringDamping,
    PlateDecay,
    PlateMix,
    PlateDamping,
    PlatePredelay,
    PlateWidth,
    PlateSize,
    Bpm,
}

impl Param {
    fn label(self) -> &'static str {
        match self {
            Param::SpringDecay | Param::PlateDecay => "decay",
            Param::SpringMix | Param::PlateMix => "mix",
            Param::SpringDamping | Param::PlateDamping => "damping",
            Param::PlatePredelay => "predelay (0-200ms)",
            Param::PlateWidth => "width",
            Param::PlateSize => "size (0.25x-2x)",
            Param::Bpm => "bpm",
        }
    }
}

fn params_for(algo: Algo) -> Vec<Param> {
    match algo {
        Algo::Spring => vec![
            Param::SpringDecay,
            Param::SpringMix,
            Param::SpringDamping,
            Param::Bpm,
        ],
        Algo::Plate => vec![
            Param::PlateDecay,
            Param::PlateMix,
            Param::PlateDamping,
            Param::PlatePredelay,
            Param::PlateWidth,
            Param::PlateSize,
            Param::Bpm,
        ],
    }
}

struct LabState {
    algo: Algo,
    spring_decay: f32,
    spring_mix: f32,
    spring_damping: f32,
    plate_decay: f32,
    plate_mix: f32,
    plate_damping: f32,
    plate_predelay: f32,
    plate_width: f32,
    plate_size: f32,
    bpm: f32,
    running: bool,
    kick_on: bool,
    enabled: bool,
}

impl LabState {
    fn value(&self, param: Param) -> f32 {
        match param {
            Param::SpringDecay => self.spring_decay,
            Param::SpringMix => self.spring_mix,
            Param::SpringDamping => self.spring_damping,
            Param::PlateDecay => self.plate_decay,
            Param::PlateMix => self.plate_mix,
            Param::PlateDamping => self.plate_damping,
            Param::PlatePredelay => self.plate_predelay,
            Param::PlateWidth => self.plate_width,
            Param::PlateSize => self.plate_size,
            Param::Bpm => self.bpm,
        }
    }
}

/// Push the wet mixes so only the active algorithm is audible (bypass mutes
/// both). Every other parameter is pushed directly when adjusted.
fn apply_mixes(controls: &ReverbControls, state: &LabState) {
    let spring_mix = if state.enabled && state.algo == Algo::Spring {
        state.spring_mix
    } else {
        0.0
    };
    let plate_mix = if state.enabled && state.algo == Algo::Plate {
        state.plate_mix
    } else {
        0.0
    };
    controls.spring().set_mix(spring_mix);
    controls.plate().set_mix(plate_mix);
}

fn make_bar(normalized: f32, width: usize) -> String {
    let filled = (normalized * width as f32).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

fn render_display(state: &LabState, selected: usize) {
    print!("\x1b[2J\x1b[H\x1b[?7l");

    print!("=== Reverb Lab ===\r\n");
    print!("TAB=algorithm  SPACE=start/stop  K=kick  B=bypass  Q=quit\r\n");
    print!("↑↓=select  ←→=adjust  []=fine\r\n");
    let status = if state.running { "RUNNING" } else { "STOPPED" };
    let reverb_status = if state.enabled { "ON" } else { "BYPASS" };
    let kick_status = if state.kick_on { "ON" } else { "OFF" };
    print!(
        "Status: {}  Algorithm: {}  Reverb: {}  Kick: {}\r\n",
        status,
        state.algo.name(),
        reverb_status,
        kick_status
    );
    print!("\r\n");

    for (i, param) in params_for(state.algo).iter().enumerate() {
        let ind = if selected == i { ">" } else { " " };
        let value = state.value(*param);
        if *param == Param::Bpm {
            let norm = (value - 60.0) / 140.0;
            print!(
                "{} {:<18} [{}] {:>6.0} BPM\r\n",
                ind,
                param.label(),
                make_bar(norm, 10),
                value
            );
        } else {
            print!(
                "{} {:<18} [{}] {:>6.2}\r\n",
                ind,
                param.label(),
                make_bar(value, 10),
                value
            );
        }
    }

    io::stdout().flush().unwrap();
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;

    let mut engine = Engine::new(sample_rate);

    let snare = Arc::new(Mutex::new(SnareDrum::new(sample_rate)));
    engine.add_instrument("snare", Box::new(SharedSnare(snare.clone())));
    let kick = Arc::new(Mutex::new(KickDrum::new(sample_rate)));
    engine.add_instrument("kick", Box::new(SharedKick(kick.clone())));

    let bpm = 120.0;
    engine.set_bpm(bpm);

    // Sparse snare on 0, 4, 8, 12 so the reverb tail is clearly audible.
    let snare_pattern = vec![
        true, false, false, false, true, false, false, false, true, false, false, false, true,
        false, false, false,
    ];
    engine.add_sequencer(Sequencer::with_pattern(
        bpm,
        sample_rate,
        snare_pattern,
        "snare",
    ));

    // Off-beat kick (toggled with K) to hear the reverb under low end.
    let kick_pattern = vec![
        false, false, true, false, false, false, true, false, false, false, true, false, false,
        false, true, false,
    ];
    engine.add_sequencer(Sequencer::with_pattern(
        bpm,
        sample_rate,
        kick_pattern,
        "kick",
    ));

    engine.set_master_gain(0.8);

    // Create both reverbs dry and get lock-free control handles before moving
    // them into the engine; apply_mixes below un-mutes the active one.
    let spring = Box::new(SpringReverbEffect::new(sample_rate, 0.5, 0.0, 0.5));
    let plate = Box::new(PlateReverbEffect::new(sample_rate, 0.5, 0.0, 0.5));
    let controls = ReverbControls {
        spring: &*spring as *const SpringReverbEffect,
        plate: &*plate as *const PlateReverbEffect,
    };
    engine.add_global_effect(spring);
    engine.add_global_effect(plate);

    let audio_engine = Arc::new(Mutex::new(engine));

    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;

    #[cfg(feature = "visualization")]
    engine_output.enable_visualization(1200, 600, 2.0)?;

    engine_output.create_stream_with_engine(audio_engine.clone())?;
    engine_output.start()?;

    let mut state = LabState {
        algo: Algo::Plate,
        spring_decay: 0.5,
        spring_mix: 0.4,
        spring_damping: 0.5,
        plate_decay: 0.5,
        plate_mix: 0.4,
        plate_damping: 0.5,
        plate_predelay: 0.0,
        plate_width: 1.0,
        plate_size: 0.5,
        bpm,
        running: false,
        kick_on: false,
        enabled: true,
    };
    apply_mixes(&controls, &state);

    let mut selected: usize = 0;
    let mut needs_redraw = true;

    execute!(io::stdout(), Clear(ClearType::All), cursor::Hide)?;
    enable_raw_mode()?;

    let result = loop {
        if engine_output.update_visualization() {
            break Ok(());
        }

        if needs_redraw {
            render_display(&state, selected);
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
                        selected = (selected + 1).min(params_for(state.algo).len() - 1);
                        needs_redraw = true;
                    }

                    // Coarse adjust
                    KeyCode::Right => {
                        adjust_param(&audio_engine, &controls, &mut state, selected, 1.0, false);
                        needs_redraw = true;
                    }
                    KeyCode::Left => {
                        adjust_param(&audio_engine, &controls, &mut state, selected, -1.0, false);
                        needs_redraw = true;
                    }

                    // Fine adjust
                    KeyCode::Char(']') => {
                        adjust_param(&audio_engine, &controls, &mut state, selected, 1.0, true);
                        needs_redraw = true;
                    }
                    KeyCode::Char('[') => {
                        adjust_param(&audio_engine, &controls, &mut state, selected, -1.0, true);
                        needs_redraw = true;
                    }

                    // Switch active reverb algorithm
                    KeyCode::Tab => {
                        state.algo = match state.algo {
                            Algo::Spring => Algo::Plate,
                            Algo::Plate => Algo::Spring,
                        };
                        selected = selected.min(params_for(state.algo).len() - 1);
                        apply_mixes(&controls, &state);
                        needs_redraw = true;
                    }

                    // Toggle reverb bypass
                    KeyCode::Char('b') | KeyCode::Char('B') => {
                        state.enabled = !state.enabled;
                        apply_mixes(&controls, &state);
                        needs_redraw = true;
                    }

                    // Toggle the kick pattern
                    KeyCode::Char('k') | KeyCode::Char('K') => {
                        state.kick_on = !state.kick_on;
                        let mut engine = audio_engine.lock().unwrap();
                        if let Some(seq) = engine.sequencer_mut(1) {
                            if state.kick_on && state.running {
                                seq.start();
                            } else {
                                seq.stop();
                            }
                        }
                        needs_redraw = true;
                    }

                    // Start/stop
                    KeyCode::Char(' ') => {
                        let mut engine = audio_engine.lock().unwrap();
                        state.running = !state.running;
                        if let Some(seq) = engine.sequencer_mut(0) {
                            if state.running {
                                seq.start();
                            } else {
                                seq.stop();
                            }
                        }
                        if let Some(seq) = engine.sequencer_mut(1) {
                            if state.running && state.kick_on {
                                seq.start();
                            } else {
                                seq.stop();
                            }
                        }
                        needs_redraw = true;
                    }

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

#[cfg(feature = "native")]
fn adjust_param(
    audio_engine: &Arc<Mutex<Engine>>,
    controls: &ReverbControls,
    state: &mut LabState,
    selected: usize,
    direction: f32,
    fine: bool,
) {
    let params = params_for(state.algo);
    let Some(&param) = params.get(selected) else {
        return;
    };

    if param == Param::Bpm {
        let step = if fine { 1.0 } else { 5.0 };
        state.bpm = (state.bpm + step * direction).clamp(60.0, 200.0);
        let mut engine = audio_engine.lock().unwrap();
        engine.set_bpm(state.bpm);
        for i in 0..2 {
            if let Some(seq) = engine.sequencer_mut(i) {
                seq.set_bpm(state.bpm);
            }
        }
        return;
    }

    let step = if fine { 0.01 } else { 0.05 };
    match param {
        Param::SpringDecay => {
            state.spring_decay = (state.spring_decay + step * direction).clamp(0.0, 1.0);
            controls.spring().set_decay(state.spring_decay);
        }
        Param::SpringMix => {
            state.spring_mix = (state.spring_mix + step * direction).clamp(0.0, 1.0);
            apply_mixes(controls, state);
        }
        Param::SpringDamping => {
            state.spring_damping = (state.spring_damping + step * direction).clamp(0.0, 1.0);
            controls.spring().set_damping(state.spring_damping);
        }
        Param::PlateDecay => {
            state.plate_decay = (state.plate_decay + step * direction).clamp(0.0, 1.0);
            controls.plate().set_decay(state.plate_decay);
        }
        Param::PlateMix => {
            state.plate_mix = (state.plate_mix + step * direction).clamp(0.0, 1.0);
            apply_mixes(controls, state);
        }
        Param::PlateDamping => {
            state.plate_damping = (state.plate_damping + step * direction).clamp(0.0, 1.0);
            controls.plate().set_damping(state.plate_damping);
        }
        Param::PlatePredelay => {
            state.plate_predelay = (state.plate_predelay + step * direction).clamp(0.0, 1.0);
            controls.plate().set_predelay(state.plate_predelay);
        }
        Param::PlateWidth => {
            state.plate_width = (state.plate_width + step * direction).clamp(0.0, 1.0);
            controls.plate().set_width(state.plate_width);
        }
        Param::PlateSize => {
            state.plate_size = (state.plate_size + step * direction).clamp(0.0, 1.0);
            controls.plate().set_size(state.plate_size);
        }
        Param::Bpm => unreachable!(),
    }
}

#[cfg(not(feature = "native"))]
fn main() {
    println!("This example is only available with the 'native' feature enabled.");
}
