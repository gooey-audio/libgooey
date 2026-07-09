//! Multi-channel Submix - small CLI for verifying the mixer graph FFI.
//!
//! Track 1 is a basic drum beat (kick/snare/hihat/tom summed as `SOURCE_DRUMKIT`).
//! Track 2 is a looping bass-synth pattern (`SOURCE_BASS`). The terminal controls
//! let you submix those two graph tracks live with gain, mute, solo, and a small
//! track-effect rack.
//!
//! Run with:
//! cargo run --example multi_channel_submix --features native,crossterm

#[cfg(feature = "native")]
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    FromSample, Sample, SizedSample, Stream, StreamConfig,
};
#[cfg(feature = "native")]
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
#[cfg(feature = "native")]
use gooey::ffi::*;
#[cfg(feature = "native")]
use std::ffi::CString;
#[cfg(feature = "native")]
use std::io::{self, Write};
#[cfg(feature = "native")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "native")]
use std::time::{Duration, Instant};

#[cfg(feature = "native")]
const TRACK_DRUMS: u32 = 0;
#[cfg(feature = "native")]
const TRACK_BASS: u32 = 1;
#[cfg(feature = "native")]
const TRACK_COUNT: usize = 2;
#[cfg(feature = "native")]
const DEFAULT_BPM: f32 = 116.0;

#[cfg(feature = "native")]
struct FfiEngine {
    ptr: *mut GooeyEngine,
}

#[cfg(feature = "native")]
unsafe impl Send for FfiEngine {}

#[cfg(feature = "native")]
impl FfiEngine {
    fn new(sample_rate: f32) -> Self {
        Self {
            ptr: gooey_engine_new(sample_rate),
        }
    }

    fn ptr(&self) -> *mut GooeyEngine {
        self.ptr
    }
}

#[cfg(feature = "native")]
impl Drop for FfiEngine {
    fn drop(&mut self) {
        unsafe {
            gooey_engine_free(self.ptr);
        }
    }
}

#[cfg(feature = "native")]
#[derive(Clone)]
struct TrackUi {
    name: &'static str,
    gain: f32,
    muted: bool,
    soloed: bool,
    peak: f32,
    fx_knob: f32,
}

#[cfg(feature = "native")]
struct AppState {
    selected: usize,
    bpm: f32,
    running: bool,
    tracks: [TrackUi; TRACK_COUNT],
}

#[cfg(feature = "native")]
fn cstr(s: &str) -> CString {
    CString::new(s).expect("track names do not contain null bytes")
}

#[cfg(feature = "native")]
fn bool_pattern(steps: &[usize]) -> [bool; 16] {
    let mut pattern = [false; 16];
    for &step in steps {
        pattern[step % 16] = true;
    }
    pattern
}

#[cfg(feature = "native")]
fn configure_engine(engine: *mut GooeyEngine) {
    unsafe {
        gooey_engine_set_bpm(engine, DEFAULT_BPM);
        gooey_engine_set_master_gain(engine, 0.85);

        gooey_engine_mixer_clear_layout(engine);
        let drums = cstr("Track 1 - Drum Beat");
        let bass = cstr("Track 2 - Bass Loop");
        assert_eq!(gooey_engine_mixer_add_track(engine, drums.as_ptr()), 0);
        assert_eq!(gooey_engine_mixer_add_track(engine, bass.as_ptr()), 1);
        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_DRUMKIT,
            TRACK_DRUMS
        ));
        assert!(gooey_engine_mixer_route_source(
            engine,
            SOURCE_BASS,
            TRACK_BASS
        ));
        gooey_engine_mixer_set_track_gain(engine, TRACK_DRUMS, 0.85);
        gooey_engine_mixer_set_track_gain(engine, TRACK_BASS, 0.75);

        // Kit pattern: a compact four-on-floor beat with hats and a tom pickup.
        let kick = bool_pattern(&[0, 4, 8, 12]);
        let snare = bool_pattern(&[4, 12]);
        let hat = bool_pattern(&[0, 2, 4, 6, 8, 10, 12, 14]);
        let tom = bool_pattern(&[14]);
        gooey_engine_sequencer_set_instrument_pattern(engine, INSTRUMENT_KICK, kick.as_ptr());
        gooey_engine_sequencer_set_instrument_pattern(engine, INSTRUMENT_SNARE, snare.as_ptr());
        gooey_engine_sequencer_set_instrument_pattern(engine, INSTRUMENT_HIHAT, hat.as_ptr());
        gooey_engine_sequencer_set_instrument_pattern(engine, INSTRUMENT_TOM, tom.as_ptr());
        for step in 0..16 {
            gooey_engine_sequencer_set_instrument_step_velocity(
                engine,
                INSTRUMENT_HIHAT,
                step,
                if step % 4 == 0 { 0.55 } else { 0.35 },
            );
        }

        // Bass synth loop: six triggered notes over one bar. Per-step MIDI notes
        // make the bass line loop without external audio files.
        gooey_engine_load_bass_preset(engine, BASS_PRESET_SUB);
        gooey_engine_set_bass_param(engine, BASS_PARAM_VOLUME, 0.78);
        gooey_engine_set_bass_param(engine, BASS_PARAM_AMP_DECAY, 0.42);
        gooey_engine_set_bass_param(engine, BASS_PARAM_FILTER_CUTOFF, 0.38);
        let bass_pattern = bool_pattern(&[0, 3, 6, 8, 10, 13]);
        let bass_notes: [u8; 16] = [
            36,
            STEP_NOTE_NONE,
            STEP_NOTE_NONE,
            36,
            STEP_NOTE_NONE,
            STEP_NOTE_NONE,
            43,
            STEP_NOTE_NONE,
            34,
            STEP_NOTE_NONE,
            36,
            STEP_NOTE_NONE,
            STEP_NOTE_NONE,
            31,
            STEP_NOTE_NONE,
            STEP_NOTE_NONE,
        ];
        gooey_engine_sequencer_set_instrument_pattern(
            engine,
            INSTRUMENT_BASS,
            bass_pattern.as_ptr(),
        );
        gooey_engine_sequencer_set_instrument_note_pattern(
            engine,
            INSTRUMENT_BASS,
            bass_notes.as_ptr(),
        );
        for step in [0_u32, 3, 6, 8, 10, 13] {
            gooey_engine_sequencer_set_instrument_step_velocity(
                engine,
                INSTRUMENT_BASS,
                step,
                if step == 0 { 0.95 } else { 0.75 },
            );
        }

        gooey_engine_sequencer_start(engine);
    }
}

#[cfg(feature = "native")]
fn bar(value: f32, width: usize) -> String {
    let filled = (value.clamp(0.0, 1.0) * width as f32).round() as usize;
    let mut s = String::with_capacity(width);
    for i in 0..width {
        s.push(if i < filled { '#' } else { '-' });
    }
    s
}

#[cfg(feature = "native")]
fn effect_name(id: i32) -> &'static str {
    match id as u32 {
        EFFECT_LOWPASS_FILTER => "filter",
        EFFECT_DELAY => "delay",
        EFFECT_REVERB => "reverb",
        _ => "fx",
    }
}

#[cfg(feature = "native")]
fn sync_track_state(engine: *mut GooeyEngine, state: &mut AppState) {
    unsafe {
        state.bpm = gooey_engine_get_bpm(engine);
        state.running = gooey_engine_sequencer_get_current_step(engine) >= 0;
        for (i, track) in state.tracks.iter_mut().enumerate() {
            let track_idx = i as u32;
            track.gain = gooey_engine_mixer_get_track_gain(engine, track_idx);
            track.muted = gooey_engine_mixer_get_track_mute(engine, track_idx);
            track.soloed = gooey_engine_mixer_get_track_solo(engine, track_idx);
            track.peak = gooey_engine_mixer_get_track_peak(engine, track_idx);
        }
    }
}

#[cfg(feature = "native")]
fn render_display(engine: *mut GooeyEngine, state: &AppState) {
    execute!(io::stdout(), Clear(ClearType::All), cursor::MoveTo(0, 0)).unwrap();

    let step = unsafe { gooey_engine_sequencer_get_current_step(engine) };
    print!("=== Multi-channel Submix (MixerGraph FFI) ===\r\n");
    print!(
        "BPM {:.0}  sequencer {}  current step {}\r\n\r\n",
        state.bpm,
        if state.running { "RUNNING" } else { "STOPPED" },
        if step >= 0 {
            format!("{:02}", step + 1)
        } else {
            "--".to_string()
        }
    );

    for (idx, track) in state.tracks.iter().enumerate() {
        let marker = if idx == state.selected { '>' } else { ' ' };
        let mute = if track.muted { "M" } else { "." };
        let solo = if track.soloed { "S" } else { "." };
        let source = if idx == TRACK_DRUMS as usize {
            "SOURCE_DRUMKIT"
        } else {
            "SOURCE_BASS"
        };
        let fx_count = unsafe { gooey_engine_track_effect_count(engine, idx as u32) };

        print!("{marker} {}  {}{}  {source}\r\n", track.name, mute, solo);
        print!(
            "    gain [{}] {:.2}   peak [{}] {:.2}\r\n",
            bar(track.gain / 2.0, 18),
            track.gain,
            bar(track.peak, 18),
            track.peak,
        );
        if fx_count == 0 {
            print!("    fx: (none)\r\n");
        } else {
            let mut chain = String::new();
            for slot in 0..fx_count {
                let id = unsafe { gooey_engine_track_effect_type_at(engine, idx as u32, slot) };
                if slot > 0 {
                    chain.push_str(" -> ");
                }
                chain.push_str(effect_name(id));
            }
            print!("    fx: {chain}  knob {:.0}%\r\n", track.fx_knob * 100.0);
        }
        print!("\r\n");
    }

    print!("Up/Down select track    Left/Right track gain    m mute    s solo\r\n");
    print!(
        "f add filter    d add delay    v add reverb    c clear track fx    k/l tweak last fx\r\n"
    );
    print!("Space start/stop sequencer    b/B BPM -/+5    q quit\r\n");
    io::stdout().flush().unwrap();
}

#[cfg(feature = "native")]
fn apply_last_fx_knob(engine: *mut GooeyEngine, track: u32, knob: f32) {
    unsafe {
        let count = gooey_engine_track_effect_count(engine, track);
        if count == 0 {
            return;
        }
        let slot = count - 1;
        let effect_id = gooey_engine_track_effect_type_at(engine, track, slot);
        match effect_id as u32 {
            EFFECT_LOWPASS_FILTER => {
                let cutoff = 200.0 + knob * (18_000.0 - 200.0);
                gooey_engine_track_effect_set_param(
                    engine,
                    track,
                    slot,
                    FILTER_PARAM_CUTOFF,
                    cutoff,
                );
            }
            EFFECT_DELAY => {
                gooey_engine_track_effect_set_param(engine, track, slot, DELAY_PARAM_MIX, knob);
                gooey_engine_track_effect_set_param(
                    engine,
                    track,
                    slot,
                    DELAY_PARAM_FEEDBACK,
                    knob * 0.65,
                );
            }
            EFFECT_REVERB => {
                gooey_engine_track_effect_set_param(engine, track, slot, REVERB_PARAM_MIX, knob);
            }
            _ => {}
        }
    }
}

#[cfg(feature = "native")]
fn build_output_stream(
    engine: Arc<Mutex<FfiEngine>>,
    device: &cpal::Device,
    config: &StreamConfig,
    sample_format: cpal::SampleFormat,
) -> anyhow::Result<Stream> {
    match sample_format {
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
        format => Err(anyhow::anyhow!("unsupported sample format {format}")),
    }
}

#[cfg(feature = "native")]
fn make_stream<T>(
    engine: Arc<Mutex<FfiEngine>>,
    device: &cpal::Device,
    config: &StreamConfig,
) -> anyhow::Result<Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let err_fn = |err| eprintln!("audio stream error: {err}");
    let stream = device.build_output_stream(
        config,
        move |output: &mut [T], _| {
            render_audio(output, channels, &engine);
        },
        err_fn,
        None,
    )?;
    Ok(stream)
}

#[cfg(feature = "native")]
fn render_audio<T>(output: &mut [T], channels: usize, engine: &Arc<Mutex<FfiEngine>>)
where
    T: Sample + FromSample<f32>,
{
    let frames = output.len() / channels;
    let mut interleaved = vec![0.0_f32; frames * GOOEY_OUTPUT_CHANNELS as usize];

    match engine.try_lock() {
        Ok(engine) => unsafe {
            gooey_engine_render(engine.ptr(), interleaved.as_mut_ptr(), frames as u32);
        },
        Err(_) => {
            for sample in output.iter_mut() {
                *sample = T::from_sample(0.0);
            }
            return;
        }
    }

    for (frame_idx, frame) in output.chunks_mut(channels).enumerate() {
        let l = interleaved[frame_idx * 2];
        let r = interleaved[frame_idx * 2 + 1];
        match frame.len() {
            0 => {}
            1 => frame[0] = T::from_sample(0.5 * (l + r)),
            _ => {
                frame[0] = T::from_sample(l);
                frame[1] = T::from_sample(r);
                if frame.len() > 2 {
                    let mono = T::from_sample(0.5 * (l + r));
                    for sample in &mut frame[2..] {
                        *sample = mono;
                    }
                }
            }
        }
    }
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("no default output device"))?;
    let supported_config = device.default_output_config()?;
    let config: StreamConfig = supported_config.clone().into();
    let sample_rate = config.sample_rate.0 as f32;

    println!("Output device: {}", device.name()?);
    println!("Output config: {:?}", supported_config);

    let ffi_engine = FfiEngine::new(sample_rate);
    configure_engine(ffi_engine.ptr());
    let engine = Arc::new(Mutex::new(ffi_engine));

    let stream = build_output_stream(
        engine.clone(),
        &device,
        &config,
        supported_config.sample_format(),
    )?;
    stream.play()?;

    let mut state = AppState {
        selected: 0,
        bpm: DEFAULT_BPM,
        running: true,
        tracks: [
            TrackUi {
                name: "Track 1 - Drum Beat",
                gain: 0.85,
                muted: false,
                soloed: false,
                peak: 0.0,
                fx_knob: 0.5,
            },
            TrackUi {
                name: "Track 2 - Bass Loop",
                gain: 0.75,
                muted: false,
                soloed: false,
                peak: 0.0,
                fx_knob: 0.5,
            },
        ],
    };

    execute!(io::stdout(), Clear(ClearType::All), cursor::Hide)?;
    enable_raw_mode()?;

    let mut last_redraw = Instant::now();
    let mut needs_redraw = true;

    let result = loop {
        if needs_redraw || last_redraw.elapsed() > Duration::from_millis(80) {
            if let Ok(engine) = engine.lock() {
                sync_track_state(engine.ptr(), &mut state);
                render_display(engine.ptr(), &state);
            }
            needs_redraw = false;
            last_redraw = Instant::now();
        }

        if event::poll(Duration::from_millis(16))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                let selected_track = state.selected as u32;
                if let Ok(engine) = engine.lock() {
                    match code {
                        KeyCode::Up => state.selected = state.selected.saturating_sub(1),
                        KeyCode::Down => state.selected = (state.selected + 1).min(TRACK_COUNT - 1),
                        KeyCode::Left | KeyCode::Right => {
                            let delta = if code == KeyCode::Left { -0.05 } else { 0.05 };
                            let current = unsafe {
                                gooey_engine_mixer_get_track_gain(engine.ptr(), selected_track)
                            };
                            unsafe {
                                gooey_engine_mixer_set_track_gain(
                                    engine.ptr(),
                                    selected_track,
                                    current + delta,
                                );
                            }
                        }
                        KeyCode::Char('m') => {
                            let muted = unsafe {
                                gooey_engine_mixer_get_track_mute(engine.ptr(), selected_track)
                            };
                            unsafe {
                                gooey_engine_mixer_set_track_mute(
                                    engine.ptr(),
                                    selected_track,
                                    !muted,
                                );
                            }
                        }
                        KeyCode::Char('s') => {
                            let soloed = unsafe {
                                gooey_engine_mixer_get_track_solo(engine.ptr(), selected_track)
                            };
                            unsafe {
                                gooey_engine_mixer_set_track_solo(
                                    engine.ptr(),
                                    selected_track,
                                    !soloed,
                                );
                            }
                        }
                        KeyCode::Char('f') => {
                            unsafe {
                                gooey_engine_track_effect_add(
                                    engine.ptr(),
                                    selected_track,
                                    EFFECT_LOWPASS_FILTER,
                                );
                            }
                            apply_last_fx_knob(
                                engine.ptr(),
                                selected_track,
                                state.tracks[state.selected].fx_knob,
                            );
                        }
                        KeyCode::Char('d') => {
                            unsafe {
                                gooey_engine_track_effect_add(
                                    engine.ptr(),
                                    selected_track,
                                    EFFECT_DELAY,
                                );
                            }
                            apply_last_fx_knob(
                                engine.ptr(),
                                selected_track,
                                state.tracks[state.selected].fx_knob,
                            );
                        }
                        KeyCode::Char('v') => {
                            unsafe {
                                gooey_engine_track_effect_add(
                                    engine.ptr(),
                                    selected_track,
                                    EFFECT_REVERB,
                                );
                            }
                            apply_last_fx_knob(
                                engine.ptr(),
                                selected_track,
                                state.tracks[state.selected].fx_knob,
                            );
                        }
                        KeyCode::Char('c') => unsafe {
                            gooey_engine_track_effect_clear(engine.ptr(), selected_track);
                        },
                        KeyCode::Char('k') | KeyCode::Char('l') => {
                            let delta = if code == KeyCode::Char('k') {
                                -0.05
                            } else {
                                0.05
                            };
                            state.tracks[state.selected].fx_knob =
                                (state.tracks[state.selected].fx_knob + delta).clamp(0.0, 1.0);
                            apply_last_fx_knob(
                                engine.ptr(),
                                selected_track,
                                state.tracks[state.selected].fx_knob,
                            );
                        }
                        KeyCode::Char(' ') => unsafe {
                            if state.running {
                                gooey_engine_sequencer_stop(engine.ptr());
                            } else {
                                gooey_engine_sequencer_start(engine.ptr());
                            }
                            state.running = !state.running;
                        },
                        KeyCode::Char('b') | KeyCode::Char('B') => {
                            let delta = if code == KeyCode::Char('b') {
                                -5.0
                            } else {
                                5.0
                            };
                            let bpm = (state.bpm + delta).clamp(50.0, 180.0);
                            unsafe {
                                gooey_engine_set_bpm(engine.ptr(), bpm);
                            }
                        }
                        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => break Ok(()),
                        _ => {}
                    }
                }
                needs_redraw = true;
            }
        }
    };

    disable_raw_mode()?;
    execute!(io::stdout(), cursor::Show)?;
    println!("\nQuitting...");
    result
}

#[cfg(not(feature = "native"))]
fn main() {
    println!("This example requires the 'native' and 'crossterm' features.");
}
