//! Interactive CLI for macros and motions.
//!
//! A drum loop and a held pad play while a tray of four macros is shown. Each
//! macro drives several parameters; each has a motion that automates it.
//! Nudge a macro by hand, hit "go" to run its motion, or capture a new macro:
//! press `c`, tweak parameters, then `r` to register the change as the macro's
//! end point (the parameters revert until the macro moves).
//!
//! Requires a default system audio output.
//!
//! Run with: `cargo run --example macro_motion --features native,crossterm`

#[cfg(feature = "native")]
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    FromSample, Sample, SizedSample, Stream, StreamConfig,
};
#[cfg(feature = "native")]
use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
#[cfg(feature = "native")]
use gooey::ffi::*;
#[cfg(feature = "native")]
use std::cell::RefCell;
#[cfg(feature = "native")]
use std::io::{self, Write};
#[cfg(feature = "native")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "native")]
use std::time::Duration;

#[cfg(feature = "native")]
struct FfiEngine(*mut GooeyEngine);

#[cfg(feature = "native")]
unsafe impl Send for FfiEngine {}

#[cfg(feature = "native")]
impl Drop for FfiEngine {
    fn drop(&mut self) {
        unsafe { gooey_engine_free(self.0) }
    }
}

#[cfg(feature = "native")]
const TRAY_SIZE: u32 = 4;
#[cfg(feature = "native")]
const DURATIONS_BEATS: [f32; 5] = [1.0, 2.0, 4.0, 8.0, 16.0];

/// Drum loop, a sustained pad, and delay ready to be swept in.
#[cfg(feature = "native")]
fn configure_music(engine: *mut GooeyEngine) {
    unsafe {
        gooey_engine_set_bpm(engine, 112.0);
        for step in [0, 4, 8, 10, 12] {
            gooey_engine_sequencer_set_instrument_step(engine, INSTRUMENT_KICK, step, true);
        }
        for step in [4, 12] {
            gooey_engine_sequencer_set_instrument_step(engine, INSTRUMENT_SNARE, step, true);
        }
        for step in 0..16 {
            gooey_engine_sequencer_set_instrument_step(engine, INSTRUMENT_HIHAT, step, true);
        }
        gooey_engine_set_hihat_param(engine, HIHAT_PARAM_TONE, 0.25);
        gooey_engine_set_hihat_param(engine, HIHAT_PARAM_DECAY, 0.12);

        gooey_engine_set_global_effect_enabled(engine, EFFECT_DELAY, true);
        gooey_engine_set_global_effect_param(engine, EFFECT_DELAY, DELAY_PARAM_TIMING, 3.0);
        gooey_engine_set_global_effect_param(engine, EFFECT_DELAY, DELAY_PARAM_MIX, 0.0);
        gooey_engine_set_global_effect_param(engine, EFFECT_DELAY, DELAY_PARAM_FEEDBACK, 0.3);

        gooey_engine_poly_set_preset(engine, POLY_PRESET_PAD);
        gooey_engine_poly_trigger_chord(
            engine,
            9, // A
            SCALE_MINOR,
            0,
            VOICING_OPEN,
            POLY_PRESET_PAD,
            3,
            0.6,
        );
        gooey_engine_set_master_gain(engine, 0.8);
        gooey_engine_sequencer_start(engine);
    }
}

/// Seed three macros with motions; leave the fourth empty for capture.
#[cfg(feature = "native")]
fn configure_macros(engine: *mut GooeyEngine) {
    unsafe {
        let map = |macro_index, kind, index, param, from, to| {
            assert!(gooey_engine_macro_add_mapping(
                engine,
                macro_index,
                kind,
                index,
                param,
                from,
                to
            ));
        };
        // 0: open the hats up — brighter and longer.
        map(
            0,
            PARAM_TARGET_DRUM,
            INSTRUMENT_HIHAT,
            HIHAT_PARAM_TONE,
            0.25,
            0.95,
        );
        map(
            0,
            PARAM_TARGET_DRUM,
            INSTRUMENT_HIHAT,
            HIHAT_PARAM_DECAY,
            0.12,
            0.55,
        );
        // 1: close the pad filter and add bite.
        let cutoff = gooey_engine_poly_get_param(engine, POLY_PARAM_FILTER_CUTOFF);
        let resonance = gooey_engine_poly_get_param(engine, POLY_PARAM_FILTER_RESONANCE);
        map(
            1,
            PARAM_TARGET_POLY,
            0,
            POLY_PARAM_FILTER_CUTOFF,
            cutoff,
            0.08,
        );
        map(
            1,
            PARAM_TARGET_POLY,
            0,
            POLY_PARAM_FILTER_RESONANCE,
            resonance,
            0.7,
        );
        // 2: wash everything in delay.
        map(
            2,
            PARAM_TARGET_GLOBAL_EFFECT,
            EFFECT_DELAY,
            DELAY_PARAM_MIX,
            0.0,
            0.45,
        );
        map(
            2,
            PARAM_TARGET_GLOBAL_EFFECT,
            EFFECT_DELAY,
            DELAY_PARAM_FEEDBACK,
            0.3,
            0.8,
        );

        // Motion slot N drives macro N.
        let settings = [
            (
                8.0,
                MOTION_CURVE_EASE_IN,
                MOTION_END_HOLD,
                MOTION_QUANTIZE_BAR,
            ),
            (
                4.0,
                MOTION_CURVE_S_CURVE,
                MOTION_END_RETURN,
                MOTION_QUANTIZE_BEAT,
            ),
            (
                2.0,
                MOTION_CURVE_EASE_OUT,
                MOTION_END_SNAP_BACK,
                MOTION_QUANTIZE_BEAT,
            ),
            (
                4.0,
                MOTION_CURVE_LINEAR,
                MOTION_END_HOLD,
                MOTION_QUANTIZE_NONE,
            ),
        ];
        for (slot, (beats, curve, end_mode, quantize)) in settings.into_iter().enumerate() {
            let slot = slot as u32;
            gooey_engine_motion_configure(engine, slot, slot, 1.0);
            gooey_engine_motion_set_duration(engine, slot, MOTION_DURATION_BEATS, beats);
            gooey_engine_motion_set_curve(engine, slot, curve);
            gooey_engine_motion_set_end_mode(engine, slot, end_mode);
            gooey_engine_motion_set_quantize(engine, slot, quantize);
        }
    }
}

#[cfg(feature = "native")]
thread_local! { static RENDER_BUFFER: RefCell<Vec<f32>> = RefCell::new(vec![0.0; 8192]); }

#[cfg(feature = "native")]
fn render_audio<T: Sample + FromSample<f32>>(
    output: &mut [T],
    channels: usize,
    engine: &Arc<Mutex<FfiEngine>>,
) {
    let frames = output.len() / channels;
    RENDER_BUFFER.with(|cell| {
        let mut input = cell.borrow_mut();
        input.resize(frames * 2, 0.0);
        let guard = engine.lock().unwrap();
        unsafe { gooey_engine_render(guard.0, input.as_mut_ptr(), frames as u32) };
        for (frame_index, frame) in output.chunks_mut(channels).enumerate() {
            let left = input[frame_index * 2];
            let right = input[frame_index * 2 + 1];
            frame[0] = T::from_sample(left);
            if channels > 1 {
                frame[1] = T::from_sample(right);
                for sample in &mut frame[2..] {
                    *sample = T::from_sample(0.5 * (left + right));
                }
            }
        }
    });
}

#[cfg(feature = "native")]
fn make_stream<T: SizedSample + FromSample<f32>>(
    engine: Arc<Mutex<FfiEngine>>,
    device: &cpal::Device,
    config: &StreamConfig,
) -> anyhow::Result<Stream> {
    let channels = config.channels as usize;
    Ok(device.build_output_stream(
        config,
        move |output: &mut [T], _| render_audio(output, channels, &engine),
        |error| eprintln!("audio stream error: {error}"),
        None,
    )?)
}

#[cfg(feature = "native")]
fn build_stream(
    engine: Arc<Mutex<FfiEngine>>,
    device: &cpal::Device,
    config: &StreamConfig,
    format: cpal::SampleFormat,
) -> anyhow::Result<Stream> {
    match format {
        cpal::SampleFormat::I8 => make_stream::<i8>(engine, device, config),
        cpal::SampleFormat::I16 => make_stream::<i16>(engine, device, config),
        cpal::SampleFormat::I32 => make_stream::<i32>(engine, device, config),
        cpal::SampleFormat::I64 => make_stream::<i64>(engine, device, config),
        cpal::SampleFormat::U8 => make_stream::<u8>(engine, device, config),
        cpal::SampleFormat::U16 => make_stream::<u16>(engine, device, config),
        cpal::SampleFormat::U32 => make_stream::<u32>(engine, device, config),
        cpal::SampleFormat::U64 => make_stream::<u64>(engine, device, config),
        cpal::SampleFormat::F32 => make_stream::<f32>(engine, device, config),
        cpal::SampleFormat::F64 => make_stream::<f64>(engine, device, config),
        other => Err(anyhow::anyhow!("unsupported output sample format {other}")),
    }
}

#[cfg(feature = "native")]
fn bar(value: f32, width: usize) -> String {
    let filled = ((value.clamp(0.0, 1.0) * width as f32).round() as usize).min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

#[cfg(feature = "native")]
fn curve_label(curve: u32) -> &'static str {
    match curve {
        MOTION_CURVE_LINEAR => "linear",
        MOTION_CURVE_EASE_IN => "ease-in",
        MOTION_CURVE_EASE_OUT => "ease-out",
        MOTION_CURVE_S_CURVE => "s-curve",
        _ => "?",
    }
}

#[cfg(feature = "native")]
fn end_label(end_mode: u32) -> &'static str {
    match end_mode {
        MOTION_END_HOLD => "hold",
        MOTION_END_RETURN => "return",
        MOTION_END_SNAP_BACK => "snap back",
        _ => "?",
    }
}

#[cfg(feature = "native")]
fn quantize_label(quantize: u32) -> &'static str {
    match quantize {
        MOTION_QUANTIZE_NONE => "now",
        MOTION_QUANTIZE_BEAT => "next beat",
        MOTION_QUANTIZE_BAR => "next bar",
        _ => "?",
    }
}

#[cfg(feature = "native")]
fn state_label(state: u32, progress: f32) -> String {
    match state {
        MOTION_STATE_PENDING => "waiting…".to_string(),
        MOTION_STATE_RUNNING => format!("▶ {:>3.0}%", progress * 100.0),
        MOTION_STATE_RETURNING => format!("◀ {:>3.0}%", progress * 100.0),
        _ => "idle".to_string(),
    }
}

#[cfg(feature = "native")]
fn target_label(kind: u32, index: u32, param: u32) -> String {
    match (kind, index, param) {
        (PARAM_TARGET_POLY, _, POLY_PARAM_FILTER_CUTOFF) => "pad cutoff".into(),
        (PARAM_TARGET_POLY, _, POLY_PARAM_FILTER_RESONANCE) => "pad resonance".into(),
        (PARAM_TARGET_POLY, _, param) => format!("poly #{param}"),
        (PARAM_TARGET_DRUM, INSTRUMENT_HIHAT, HIHAT_PARAM_TONE) => "hat tone".into(),
        (PARAM_TARGET_DRUM, INSTRUMENT_HIHAT, HIHAT_PARAM_DECAY) => "hat decay".into(),
        (PARAM_TARGET_DRUM, channel, param) => format!("drum {channel} #{param}"),
        (PARAM_TARGET_GLOBAL_EFFECT, EFFECT_DELAY, DELAY_PARAM_MIX) => "delay mix".into(),
        (PARAM_TARGET_GLOBAL_EFFECT, EFFECT_DELAY, DELAY_PARAM_FEEDBACK) => "delay fb".into(),
        (PARAM_TARGET_GLOBAL_EFFECT, effect, param) => format!("fx {effect} #{param}"),
        _ => "?".into(),
    }
}

#[cfg(feature = "native")]
fn mapping_summary(engine: *mut GooeyEngine, macro_index: u32) -> String {
    let count = unsafe { gooey_engine_macro_get_mapping_count(engine, macro_index) };
    if count == 0 {
        return "(empty — press c to capture)".into();
    }
    (0..count)
        .filter_map(|mapping| {
            let (mut kind, mut index, mut param) = (0, 0, 0);
            let null = std::ptr::null_mut();
            unsafe {
                gooey_engine_macro_get_mapping(
                    engine,
                    macro_index,
                    mapping,
                    &mut kind,
                    &mut index,
                    &mut param,
                    null,
                    null,
                )
            }
            .then(|| target_label(kind, index, param))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(feature = "native")]
fn draw(engine: *mut GooeyEngine, selected: u32, status: &str) -> io::Result<()> {
    const NAMES: [&str; TRAY_SIZE as usize] = ["Hat open", "Pad close", "Delay wash", "Captured"];
    unsafe {
        execute!(io::stdout(), cursor::MoveTo(0, 0), Clear(ClearType::All))?;
        println!("=== Macros & Motions ===\r");
        println!(
            "beat {:>6.2}   {}\r",
            gooey_engine_transport_get_beat_position(engine),
            if gooey_engine_macro_is_capturing(engine) {
                format!(
                    "● CAPTURING — {} param(s) changed; r register, R merge, x cancel",
                    gooey_engine_macro_capture_get_change_count(engine)
                )
            } else {
                String::new()
            }
        );
        println!("\r");
        for macro_index in 0..TRAY_SIZE {
            let value = gooey_engine_macro_get_value(engine, macro_index);
            let slot = macro_index;
            println!(
                "{} [{}] {:<11} {} {:.2}   motion → {:.1}: {:<8}\r",
                if macro_index == selected { ">" } else { " " },
                macro_index + 1,
                NAMES[macro_index as usize],
                bar(value, 24),
                value,
                gooey_engine_motion_get_target(engine, slot),
                state_label(
                    gooey_engine_motion_get_state(engine, slot),
                    gooey_engine_motion_get_progress(engine, slot)
                ),
            );
            println!(
                "        {} beats, {}, {}, start {}   drives: {}\r",
                gooey_engine_motion_get_duration_value(engine, slot),
                curve_label(gooey_engine_motion_get_curve(engine, slot)),
                end_label(gooey_engine_motion_get_end_mode(engine, slot)),
                quantize_label(gooey_engine_motion_get_quantize(engine, slot)),
                mapping_summary(engine, macro_index),
            );
        }
        println!("\r");
        println!(
            "pad cutoff {:.2}   hat tone {:.2}   delay mix {:.2}\r",
            gooey_engine_poly_get_param(engine, POLY_PARAM_FILTER_CUTOFF),
            gooey_engine_get_hihat_param(engine, HIHAT_PARAM_TONE),
            gooey_engine_get_global_effect_param(engine, EFFECT_DELAY, DELAY_PARAM_MIX),
        );
        println!("\r");
        println!("1-4 select  ,/. nudge macro  g go  t flip target  s stop all\r");
        println!("b duration  u curve  e end mode  z quantize\r");
        println!(
            "c capture  f/F pad cutoff  h/H hat tone  w/W delay mix  r/R register  x cancel\r"
        );
        println!("space play/stop  q quit\r");
        println!("\r{status}\r");
        io::stdout().flush()
    }
}

#[cfg(feature = "native")]
fn nudge_param(engine: *mut GooeyEngine, key: char) -> Option<String> {
    let (step, label) = match key {
        'f' | 'h' | 'w' => (-0.1, key),
        'F' | 'H' | 'W' => (0.1, key.to_ascii_lowercase()),
        _ => return None,
    };
    unsafe {
        match label {
            'f' => {
                let value = gooey_engine_poly_get_param(engine, POLY_PARAM_FILTER_CUTOFF);
                gooey_engine_poly_set_param(
                    engine,
                    POLY_PARAM_FILTER_CUTOFF,
                    (value + step).clamp(0.0, 1.0),
                );
            }
            'h' => {
                let value = gooey_engine_get_hihat_param(engine, HIHAT_PARAM_TONE);
                gooey_engine_set_hihat_param(
                    engine,
                    HIHAT_PARAM_TONE,
                    (value + step).clamp(0.0, 1.0),
                );
            }
            _ => {
                let value =
                    gooey_engine_get_global_effect_param(engine, EFFECT_DELAY, DELAY_PARAM_MIX);
                gooey_engine_set_global_effect_param(
                    engine,
                    EFFECT_DELAY,
                    DELAY_PARAM_MIX,
                    (value + step).clamp(0.0, 1.0),
                );
            }
        }
    }
    Some("param adjusted".into())
}

#[cfg(feature = "native")]
fn cycle<T: PartialEq + Copy>(options: &[T], current: T) -> T {
    let position = options.iter().position(|&o| o == current).unwrap_or(0);
    options[(position + 1) % options.len()]
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("no default audio output device"))?;
    let supported = device.default_output_config()?;
    let config: StreamConfig = supported.clone().into();
    let engine = Arc::new(Mutex::new(FfiEngine(gooey_engine_new(
        config.sample_rate.0 as f32,
    ))));
    {
        let guard = engine.lock().unwrap();
        configure_music(guard.0);
        configure_macros(guard.0);
    }

    let stream = build_stream(engine.clone(), &device, &config, supported.sample_format())?;
    stream.play()?;

    enable_raw_mode()?;
    let mut running = true;
    let mut selected = 0_u32;
    let mut status = String::from("Try: g on [1] to open the hats over 8 beats.");
    loop {
        draw(engine.lock().unwrap().0, selected, &status)?;
        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != event::KeyEventKind::Press {
            continue;
        }
        let guard = engine.lock().unwrap();
        let e = guard.0;
        let slot = selected;
        unsafe {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Char(' ') => {
                    if running {
                        gooey_engine_sequencer_stop(e);
                    } else {
                        gooey_engine_sequencer_start(e);
                    }
                    running = !running;
                }
                KeyCode::Char(c @ '1'..='4') => selected = c as u32 - '1' as u32,
                KeyCode::Char(c @ (',' | '.')) => {
                    let step = if c == ',' { -0.05 } else { 0.05 };
                    let value = gooey_engine_macro_get_value(e, selected);
                    gooey_engine_macro_set_value(e, selected, value + step);
                    status = "macro moved by hand (stops its motion)".into();
                }
                KeyCode::Char('g') => {
                    status = if gooey_engine_motion_trigger(e, slot) {
                        format!("motion {} go", slot + 1)
                    } else {
                        "motion not triggered".into()
                    };
                }
                KeyCode::Char('t') => {
                    let target = gooey_engine_motion_get_target(e, slot);
                    let flipped = if target > 0.5 { 0.0 } else { 1.0 };
                    gooey_engine_motion_configure(e, slot, selected, flipped);
                }
                KeyCode::Char('s') => {
                    gooey_engine_motion_stop_all(e);
                    status = "all motions stopped".into();
                }
                KeyCode::Char('b') => {
                    let beats = gooey_engine_motion_get_duration_value(e, slot);
                    let next = cycle(&DURATIONS_BEATS, beats);
                    gooey_engine_motion_set_duration(e, slot, MOTION_DURATION_BEATS, next);
                }
                KeyCode::Char('u') => {
                    let curves = [
                        MOTION_CURVE_LINEAR,
                        MOTION_CURVE_EASE_IN,
                        MOTION_CURVE_EASE_OUT,
                        MOTION_CURVE_S_CURVE,
                    ];
                    let next = cycle(&curves, gooey_engine_motion_get_curve(e, slot));
                    gooey_engine_motion_set_curve(e, slot, next);
                }
                KeyCode::Char('e') => {
                    let modes = [MOTION_END_HOLD, MOTION_END_RETURN, MOTION_END_SNAP_BACK];
                    let next = cycle(&modes, gooey_engine_motion_get_end_mode(e, slot));
                    gooey_engine_motion_set_end_mode(e, slot, next);
                }
                KeyCode::Char('z') => {
                    let modes = [
                        MOTION_QUANTIZE_NONE,
                        MOTION_QUANTIZE_BEAT,
                        MOTION_QUANTIZE_BAR,
                    ];
                    let next = cycle(&modes, gooey_engine_motion_get_quantize(e, slot));
                    gooey_engine_motion_set_quantize(e, slot, next);
                }
                KeyCode::Char('c') => {
                    gooey_engine_macro_capture_begin(e);
                    status = "capturing: tweak params, then r (replace) or R (merge)".into();
                }
                KeyCode::Char(c @ ('r' | 'R')) => {
                    let mode = if c == 'r' {
                        MACRO_CAPTURE_REPLACE
                    } else {
                        MACRO_CAPTURE_MERGE
                    };
                    status = match gooey_engine_macro_capture_commit(e, selected, mode) {
                        MACRO_CAPTURE_ERROR_INVALID => "nothing to register (press c first)".into(),
                        MACRO_CAPTURE_ERROR_TOO_MANY => "too many params for one macro".into(),
                        count => format!(
                            "registered into macro {}: {count} param(s), reverted; press g",
                            selected + 1
                        ),
                    };
                }
                KeyCode::Char('x') => {
                    if gooey_engine_macro_capture_cancel(e, true) {
                        status = "capture cancelled, params reverted".into();
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(message) = nudge_param(e, c) {
                        status = message;
                    }
                }
                _ => {}
            }
        }
    }
    disable_raw_mode()?;
    println!("\nBye.");
    Ok(())
}

#[cfg(not(feature = "native"))]
fn main() {
    eprintln!("This example requires --features native,crossterm");
}
