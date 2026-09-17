/* FM Percussion Live - hear the monophonic FM percussion voice sequenced
live, with per-step note and velocity variation plus periodic preset swaps
across the four factory sounds (Sub Kick, Metal Hat, Zap, Industrial). */

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use gooey::engine::{Engine, EngineOutput, Instrument, Modulatable, Sequencer};
use gooey::instruments::{FmPercussion, FmPercussionConfig};

// A 16-step "noir percussion" groove. 255 = step disabled.
const NOTES: [u8; 16] = [
    36, 255, 43, 255, 38, 50, 255, 55, 36, 255, 45, 60, 34, 255, 48, 63,
];
const VELOCITIES: [f32; 16] = [
    1.00, 0.0, 0.55, 0.0, 0.85, 0.35, 0.0, 0.70, 0.95, 0.0, 0.50, 0.40, 0.80, 0.0, 0.65, 0.75,
];

// Preset corners the groove cycles through every two bars for timbral variety.
const PRESET_NAMES: [&str; 4] = ["Sub Kick", "Metal Hat", "Zap", "Industrial"];
const BARS_PER_PRESET: u32 = 2;

fn preset_config(index: usize) -> FmPercussionConfig {
    match index {
        0 => FmPercussionConfig::sub_kick(),
        1 => FmPercussionConfig::metal_hat(),
        2 => FmPercussionConfig::zap(),
        _ => FmPercussionConfig::industrial(),
    }
}

// Wrapper to share FmPercussion between the audio thread and the main thread
struct SharedFm(Arc<Mutex<FmPercussion>>);

impl Instrument for SharedFm {
    fn trigger_with_velocity(&mut self, time: f64, velocity: f32) {
        self.0.lock().unwrap().trigger_with_velocity(time, velocity);
    }

    fn tick(&mut self, current_time: f64) -> f32 {
        self.0.lock().unwrap().tick(current_time)
    }

    fn is_active(&self) -> bool {
        self.0.lock().unwrap().is_active()
    }

    fn set_midi_note(&mut self, note: u8) {
        self.0.lock().unwrap().set_midi_note(note);
    }

    fn set_frequency_normalized(&mut self, value: f32) {
        self.0.lock().unwrap().set_frequency_normalized(value);
    }

    fn get_frequency(&self) -> Option<f32> {
        self.0.lock().unwrap().get_frequency()
    }

    fn as_modulatable(&mut self) -> Option<&mut dyn Modulatable> {
        None
    }
}

fn render_display(
    running: bool,
    playhead: usize,
    bpm: f32,
    preset_name: &str,
    bars_completed: u32,
) {
    print!("\x1b[2J\x1b[H\x1b[?7l");

    print!("=== FM Percussion Live ===\r\n");
    print!("SPACE=play/stop  ←→=BPM  P=next preset  Q=quit\r\n");
    let status = if running { "PLAYING" } else { "STOPPED" };
    print!(
        "Status: {}  BPM: {:.0}  Preset: {}  Bars: {}\r\n",
        status, bpm, preset_name, bars_completed
    );
    print!("\r\n");

    print!("  Step: ");
    for i in 0..16 {
        if i == playhead && running {
            print!(" \x1b[7m{:>2}\x1b[0m", i + 1);
        } else {
            print!(" {:>2}", i + 1);
        }
    }
    print!("\r\n");

    print!("  Note: ");
    for note in NOTES {
        if note == 255 {
            print!("  -");
        } else {
            print!(" {:>2}", note);
        }
    }
    print!("\r\n");

    print!("   Vel: ");
    for vel in VELOCITIES {
        if vel <= 0.0 {
            print!("  -");
        } else {
            print!(" {:>2.0}", vel * 100.0);
        }
    }
    print!("\r\n");

    io::stdout().flush().unwrap();
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;

    let fm = Arc::new(Mutex::new(FmPercussion::new(sample_rate)));

    let mut engine = Engine::new(sample_rate);
    engine.add_instrument("fm", Box::new(SharedFm(fm.clone())));
    engine.set_master_gain(1.0);

    let bpm = 132.0;
    engine.set_bpm(bpm);

    let pattern: Vec<bool> = NOTES.iter().map(|&note| note != 255).collect();
    let mut sequencer = Sequencer::with_pattern(bpm, sample_rate, pattern, "fm");
    for (step, velocity) in VELOCITIES.iter().enumerate() {
        sequencer.set_step_velocity(step, *velocity);
    }
    sequencer.set_note_pattern(&NOTES);
    engine.add_sequencer(sequencer);

    let audio_engine = Arc::new(Mutex::new(engine));

    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;

    #[cfg(feature = "visualization")]
    engine_output.enable_visualization(1200, 400, 2.0)?;

    engine_output.create_stream_with_engine(audio_engine.clone())?;
    engine_output.start()?;

    let mut bpm = bpm;
    let mut running = false;
    let mut preset_index = 0usize;
    let mut bars_completed: u32 = 0;
    let mut last_step: usize = 0;
    let mut needs_redraw = true;

    execute!(io::stdout(), Clear(ClearType::All), cursor::Hide)?;
    enable_raw_mode()?;

    let result = loop {
        if engine_output.update_visualization() {
            break Ok(());
        }

        let playhead = {
            let engine = audio_engine.lock().unwrap();
            engine.sequencer(0).map(|s| s.current_step()).unwrap_or(0)
        };

        // Every time the pattern wraps back to step 0, count a bar and swap the
        // preset every BARS_PER_PRESET bars for ongoing timbral variation.
        if running && playhead == 0 && last_step != 0 {
            bars_completed += 1;
            if bars_completed.is_multiple_of(BARS_PER_PRESET) {
                preset_index = (preset_index + 1) % PRESET_NAMES.len();
                fm.lock().unwrap().set_config(preset_config(preset_index));
            }
            needs_redraw = true;
        }
        last_step = playhead;

        if needs_redraw || running {
            render_display(
                running,
                playhead,
                bpm,
                PRESET_NAMES[preset_index],
                bars_completed,
            );
            needs_redraw = false;
        }

        if event::poll(std::time::Duration::from_millis(16))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match code {
                    KeyCode::Char(' ') => {
                        let mut engine = audio_engine.lock().unwrap();
                        if let Some(seq) = engine.sequencer_mut(0) {
                            if seq.is_running() {
                                seq.stop();
                                running = false;
                            } else {
                                seq.start();
                                running = true;
                            }
                        }
                        needs_redraw = true;
                    }

                    KeyCode::Left => {
                        bpm = (bpm - 5.0).max(60.0);
                        let mut engine = audio_engine.lock().unwrap();
                        engine.set_bpm(bpm);
                        if let Some(seq) = engine.sequencer_mut(0) {
                            seq.set_bpm(bpm);
                        }
                        needs_redraw = true;
                    }
                    KeyCode::Right => {
                        bpm = (bpm + 5.0).min(200.0);
                        let mut engine = audio_engine.lock().unwrap();
                        engine.set_bpm(bpm);
                        if let Some(seq) = engine.sequencer_mut(0) {
                            seq.set_bpm(bpm);
                        }
                        needs_redraw = true;
                    }

                    KeyCode::Char('p') | KeyCode::Char('P') => {
                        preset_index = (preset_index + 1) % PRESET_NAMES.len();
                        fm.lock().unwrap().set_config(preset_config(preset_index));
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

#[cfg(not(feature = "native"))]
fn main() {
    println!("This example is only available with the 'native' feature enabled.");
}
