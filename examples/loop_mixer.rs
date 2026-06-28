//! Loop Mixer - interactive CLI for the 4-channel stereo loop mixer.
//!
//! Loads up to four stereo loops, plays them simultaneously, and lets you submix
//! them live: per-channel gain, mute/solo, loop window, varispeed, and arbitrary
//! per-channel effects (filter / delay / reverb). This drives the same core
//! `Mixer` that the `gooey_engine_loop_*` FFI controls.
//!
//! Run with (any subset of paths; missing channels get a generated demo loop):
//! cargo run --example loop_mixer --features native,crossterm,bounce -- a.wav b.wav c.wav d.wav

#[cfg(feature = "native")]
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
#[cfg(feature = "native")]
use std::io::{self, Write};
#[cfg(feature = "native")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "native")]
use std::time::{Duration, Instant};

#[cfg(feature = "native")]
use gooey::engine::{Engine, EngineOutput};
#[cfg(feature = "native")]
use gooey::ffi::{
    DELAY_PARAM_MIX, EFFECT_DELAY, EFFECT_LOWPASS_FILTER, EFFECT_REVERB, FILTER_PARAM_CUTOFF,
    REVERB_PARAM_MIX,
};
#[cfg(feature = "native")]
use gooey::mixer::{StereoSampleBuffer, LOOP_CHANNEL_COUNT};

#[cfg(feature = "native")]
const SAMPLE_RATE: f32 = 44_100.0;

/// Build a distinct 2-second stereo demo loop so the example runs with no args.
/// Each channel gets a different root note and a gentle L/R detune so the stereo
/// image is audible.
#[cfg(feature = "native")]
fn demo_loop(index: usize) -> StereoSampleBuffer {
    // A minor-ish spread of roots across the four channels.
    let roots = [110.0_f32, 146.83, 164.81, 220.0];
    let root = roots[index % roots.len()];
    let len = (SAMPLE_RATE * 2.0) as usize;
    let mut left = Vec::with_capacity(len);
    let mut right = Vec::with_capacity(len);
    for i in 0..len {
        let t = i as f32 / SAMPLE_RATE;
        // Simple two-note arpeggio with an 8th-note amplitude pulse.
        let note = if (t * 2.0).fract() < 0.5 {
            root
        } else {
            root * 1.5
        };
        let pulse_phase = (t * 4.0).fract();
        let env = if pulse_phase < 0.5 {
            (std::f32::consts::PI * pulse_phase / 0.5).sin()
        } else {
            0.0
        };
        let detune = 1.003; // ~5 cents, for a wide stereo image
        let l = (std::f32::consts::TAU * note * t).sin();
        let r = (std::f32::consts::TAU * note * detune * t).sin();
        left.push(l * env * 0.6);
        right.push(r * env * 0.6);
    }
    StereoSampleBuffer::from_channels(left, right, SAMPLE_RATE)
        .expect("generated demo loop is valid")
}

/// Load channel `index`'s loop from the CLI args, falling back to a demo loop.
#[cfg(feature = "native")]
fn load_channel(index: usize, paths: &[String]) -> (StereoSampleBuffer, String) {
    if let Some(path) = paths.get(index) {
        match StereoSampleBuffer::from_wav(path) {
            Ok(buffer) => return (buffer, path.clone()),
            Err(e) => eprintln!("ch{index}: failed to load {path}: {e} — using demo loop"),
        }
    }
    (demo_loop(index), format!("demo loop {}", index + 1))
}

#[cfg(feature = "native")]
fn effect_name(id: u32) -> &'static str {
    match id {
        EFFECT_LOWPASS_FILTER => "filter",
        EFFECT_DELAY => "delay",
        EFFECT_REVERB => "reverb",
        _ => "fx",
    }
}

/// The primary parameter (and its range) tweaked by the `k`/`l` knob for each
/// effect type the example can add.
#[cfg(feature = "native")]
fn primary_param(effect_id: u32) -> (u32, f32, f32) {
    match effect_id {
        EFFECT_LOWPASS_FILTER => (FILTER_PARAM_CUTOFF, 200.0, 18_000.0),
        EFFECT_DELAY => (DELAY_PARAM_MIX, 0.0, 1.0),
        EFFECT_REVERB => (REVERB_PARAM_MIX, 0.0, 1.0),
        _ => (0, 0.0, 1.0),
    }
}

#[cfg(feature = "native")]
fn bar(value: f32, width: usize) -> String {
    let filled = ((value.clamp(0.0, 1.0)) * width as f32).round() as usize;
    let mut s = String::with_capacity(width);
    for i in 0..width {
        s.push(if i < filled { '#' } else { '-' });
    }
    s
}

#[cfg(feature = "native")]
fn render_display(engine: &Engine, names: &[String], selected: usize, knob: &[f32]) {
    execute!(io::stdout(), Clear(ClearType::All), cursor::MoveTo(0, 0)).unwrap();
    let mixer = engine.mixer();
    print!(
        "=== Loop Mixer — {} stereo channels ===\r\n\r\n",
        LOOP_CHANNEL_COUNT
    );

    for (ch, &knob_value) in knob.iter().enumerate().take(mixer.channel_count()) {
        let c = mixer.channel(ch).unwrap();
        let marker = if ch == selected { '>' } else { ' ' };
        let play = if c.is_playing() { "PLAY" } else { "stop" };
        let mute = if c.is_muted() { "M" } else { "." };
        let solo = if c.is_soloed() { "S" } else { "." };
        print!(
            "{marker} ch{ch} [{play}] {mute}{solo}  {}\r\n",
            names.get(ch).map(String::as_str).unwrap_or("")
        );
        print!(
            "    gain [{}] {:.2}  speed {:+.2}  loop {:.2}-{:.2}  pos [{}]\r\n",
            bar(c.gain() / 2.0, 16),
            c.gain(),
            c.speed(),
            c.loop_start(),
            c.loop_end(),
            bar(c.position_normalized(), 16),
        );
        // Effect chain.
        let fx_count = c.effects().len();
        if fx_count == 0 {
            print!("    fx: (none)\r\n");
        } else {
            let mut chain = String::new();
            for slot in 0..fx_count {
                if let Some(id) = c.effects().effect_type_at(slot) {
                    if slot > 0 {
                        chain.push_str(" -> ");
                    }
                    chain.push_str(effect_name(id));
                }
            }
            print!("    fx: {chain}  (knob {:.0}%)\r\n", knob_value * 100.0);
        }
        print!("\r\n");
    }

    print!("Up/Down select  Left/Right gain  -/= speed  ,/. loop-start  ;/' loop-end\r\n");
    print!("space play/stop  r restart  m mute  s solo\r\n");
    print!("f +filter  d +delay  v +reverb  c clear-fx  k/l tweak last fx  q quit\r\n");
    io::stdout().flush().unwrap();
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let paths: Vec<String> = std::env::args().skip(1).collect();

    let mut engine = Engine::new(SAMPLE_RATE);
    engine.set_master_gain(0.8);

    let mut names = Vec::new();
    for ch in 0..LOOP_CHANNEL_COUNT {
        let (buffer, name) = load_channel(ch, &paths);
        let mixer = engine.mixer_mut();
        mixer.load(ch, buffer);
        mixer.set_gain(ch, 0.6); // headroom for four simultaneous loops
        mixer.set_playing(ch, true);
        names.push(name);
    }

    let audio_engine = Arc::new(Mutex::new(engine));
    let mut engine_output = EngineOutput::new();
    engine_output.initialize(SAMPLE_RATE)?;
    engine_output.create_stream_with_engine(audio_engine.clone())?;
    engine_output.start()?;

    let mut selected = 0usize;
    let mut knob = vec![0.5f32; LOOP_CHANNEL_COUNT];
    let mut needs_redraw = true;
    let mut last_redraw = Instant::now();

    execute!(io::stdout(), Clear(ClearType::All), cursor::Hide)?;
    enable_raw_mode()?;

    let result = loop {
        if needs_redraw || last_redraw.elapsed() > Duration::from_millis(80) {
            let engine = audio_engine.lock().unwrap();
            render_display(&engine, &names, selected, &knob);
            drop(engine);
            needs_redraw = false;
            last_redraw = Instant::now();
        }

        if event::poll(Duration::from_millis(16))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                let mut engine = audio_engine.lock().unwrap();
                let mixer = engine.mixer_mut();
                match code {
                    KeyCode::Up => selected = selected.saturating_sub(1),
                    KeyCode::Down => selected = (selected + 1).min(LOOP_CHANNEL_COUNT - 1),
                    KeyCode::Left | KeyCode::Right => {
                        let delta = if code == KeyCode::Left { -0.05 } else { 0.05 };
                        let g = mixer.channel(selected).unwrap().gain();
                        mixer.set_gain(selected, g + delta);
                    }
                    KeyCode::Char('-') | KeyCode::Char('=') => {
                        let delta = if code == KeyCode::Char('-') {
                            -0.05
                        } else {
                            0.05
                        };
                        let s = mixer.channel(selected).unwrap().speed();
                        mixer.set_speed(selected, s + delta);
                    }
                    KeyCode::Char(',') | KeyCode::Char('.') => {
                        let delta = if code == KeyCode::Char(',') {
                            -0.02
                        } else {
                            0.02
                        };
                        let v = mixer.channel(selected).unwrap().loop_start();
                        mixer.set_loop_start(selected, v + delta);
                    }
                    KeyCode::Char(';') | KeyCode::Char('\'') => {
                        let delta = if code == KeyCode::Char(';') {
                            -0.02
                        } else {
                            0.02
                        };
                        let v = mixer.channel(selected).unwrap().loop_end();
                        mixer.set_loop_end(selected, v + delta);
                    }
                    KeyCode::Char(' ') => {
                        let playing = mixer.channel(selected).unwrap().is_playing();
                        mixer.set_playing(selected, !playing);
                    }
                    KeyCode::Char('r') => mixer.restart(selected),
                    KeyCode::Char('m') => {
                        let muted = mixer.channel(selected).unwrap().is_muted();
                        mixer.set_muted(selected, !muted);
                    }
                    KeyCode::Char('s') => {
                        let soloed = mixer.channel(selected).unwrap().is_soloed();
                        mixer.set_soloed(selected, !soloed);
                    }
                    KeyCode::Char('f') => {
                        mixer.effect_add(selected, EFFECT_LOWPASS_FILTER);
                    }
                    KeyCode::Char('d') => {
                        mixer.effect_add(selected, EFFECT_DELAY);
                    }
                    KeyCode::Char('v') => {
                        mixer.effect_add(selected, EFFECT_REVERB);
                    }
                    KeyCode::Char('c') => mixer.effect_clear(selected),
                    KeyCode::Char('k') | KeyCode::Char('l') => {
                        let delta = if code == KeyCode::Char('k') {
                            -0.05
                        } else {
                            0.05
                        };
                        knob[selected] = (knob[selected] + delta).clamp(0.0, 1.0);
                        let count = mixer.effect_count(selected);
                        if count > 0 {
                            let slot = count - 1;
                            if let Some(id) = mixer.effect_type_at(selected, slot) {
                                let (param, min, max) = primary_param(id);
                                let value = min + knob[selected] * (max - min);
                                mixer.effect_set_param(selected, slot, param, value);
                            }
                        }
                    }
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => break Ok(()),
                    _ => {}
                }
                needs_redraw = true;
            }
        }
    };

    execute!(io::stdout(), cursor::Show)?;
    disable_raw_mode()?;
    println!("\nQuitting...");
    result
}

#[cfg(not(feature = "native"))]
fn main() {
    println!("This example requires the 'native' and 'crossterm' features.");
}
