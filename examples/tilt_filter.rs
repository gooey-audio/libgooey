/* Tilt Filter Demo - Interactive CLI for the unified lowpass/highpass tilt filter.
Uses a kick drum loop as a sound source so you can hear the filter sweep.

Controls:
  Left/Right = adjust cutoff (center = passthrough, left = lowpass, right = highpass)
  Up/Down    = adjust resonance
  [/]        = fine cutoff adjust
  SPACE      = trigger kick
  Q          = quit
*/

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use gooey::effects::TiltFilterEffect;
use gooey::engine::{Engine, EngineOutput, Instrument, Sequencer};
use gooey::instruments::{KickConfig, KickDrum};

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

/// Wrapper to share the TiltFilterEffect via Arc while implementing Effect
struct SharedTilt(Arc<TiltFilterEffect>);

impl gooey::effects::Effect for SharedTilt {
    fn process(&self, input: f32) -> f32 {
        self.0.process(input)
    }
}

unsafe impl Send for SharedTilt {}

fn make_tilt_bar(cutoff: f32, width: usize) -> String {
    let center = width / 2;
    let pos = (cutoff * width as f32).round() as usize;
    let pos = pos.min(width);

    let mut bar: Vec<char> = vec!['░'; width];

    if pos < center {
        for i in pos..center {
            bar[i] = '█';
        }
    } else if pos > center {
        for i in center..pos {
            bar[i] = '█';
        }
    }

    bar[center] = '│';
    bar.iter().collect()
}

fn make_bar(value: f32, width: usize) -> String {
    let filled = (value * width as f32).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

fn render_display(cutoff: f32, resonance: f32, selected: usize, loop_on: bool) {
    print!("\x1b[2J\x1b[H\x1b[?7l");

    print!("=== Tilt Filter Demo ===\r\n");
    print!("Up/Down=select  Left/Right=adjust  []=fine cutoff\r\n");
    print!("SPACE=toggle loop  Q=quit\r\n");
    print!("\r\n");

    let label = if cutoff < 0.48 {
        "LOWPASS"
    } else if cutoff > 0.52 {
        "HIGHPASS"
    } else {
        "BYPASS"
    };

    let sel_cutoff = if selected == 0 { ">" } else { " " };
    let sel_res = if selected == 1 { ">" } else { " " };

    print!(
        "{} Cutoff     [{}] {:.2}  {}\r\n",
        sel_cutoff,
        make_tilt_bar(cutoff, 30),
        cutoff,
        label
    );
    print!(
        "{} Resonance  [{}] {:.2}\r\n",
        sel_res,
        make_bar(resonance, 30),
        resonance
    );

    print!("\r\n");
    print!("Loop: {}\r\n", if loop_on { "ON" } else { "OFF" });

    io::stdout().flush().unwrap();
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;
    let bpm = 120.0;

    let kick = Arc::new(Mutex::new(KickDrum::new(sample_rate)));
    {
        let mut k = kick.lock().unwrap();
        k.set_config(KickConfig::punch());
    }

    let tilt = Arc::new(TiltFilterEffect::new(sample_rate));

    let mut engine = Engine::new(sample_rate);
    engine.add_instrument("kick", Box::new(SharedKick(kick.clone())));

    // Add a 4-on-the-floor kick pattern via sequencer
    let mut seq = Sequencer::new(bpm, sample_rate, 16, "kick");
    for step in [0, 4, 8, 12] {
        seq.set_step(step, true);
        seq.set_step_velocity(step, 0.8);
    }
    seq.start();
    engine.add_sequencer(seq);

    // Add the tilt filter to the global effects chain
    engine.add_global_effect(Box::new(SharedTilt(tilt.clone())));

    let audio_engine = Arc::new(Mutex::new(engine));

    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;
    engine_output.create_stream_with_engine(audio_engine.clone())?;
    engine_output.start()?;

    let mut cutoff: f32 = 0.5;
    let mut resonance: f32 = 0.0;
    let mut selected: usize = 0;
    let mut loop_on = true;
    let mut needs_redraw = true;

    execute!(io::stdout(), Clear(ClearType::All), cursor::Hide)?;
    enable_raw_mode()?;

    let result = loop {
        if needs_redraw {
            render_display(cutoff, resonance, selected, loop_on);
            needs_redraw = false;
        }

        if event::poll(std::time::Duration::from_millis(16))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match code {
                    KeyCode::Up => {
                        selected = 0;
                        needs_redraw = true;
                    }
                    KeyCode::Down => {
                        selected = 1;
                        needs_redraw = true;
                    }

                    KeyCode::Left => {
                        if selected == 0 {
                            cutoff = (cutoff - 0.05).max(0.0);
                            tilt.set_cutoff(cutoff);
                        } else {
                            resonance = (resonance - 0.05).max(0.0);
                            tilt.set_resonance(resonance);
                        }
                        needs_redraw = true;
                    }
                    KeyCode::Right => {
                        if selected == 0 {
                            cutoff = (cutoff + 0.05).min(1.0);
                            tilt.set_cutoff(cutoff);
                        } else {
                            resonance = (resonance + 0.05).min(1.0);
                            tilt.set_resonance(resonance);
                        }
                        needs_redraw = true;
                    }

                    KeyCode::Char('[') => {
                        cutoff = (cutoff - 0.01).max(0.0);
                        tilt.set_cutoff(cutoff);
                        needs_redraw = true;
                    }
                    KeyCode::Char(']') => {
                        cutoff = (cutoff + 0.01).min(1.0);
                        tilt.set_cutoff(cutoff);
                        needs_redraw = true;
                    }

                    KeyCode::Char(' ') => {
                        loop_on = !loop_on;
                        let mut eng = audio_engine.lock().unwrap();
                        if let Some(seq) = eng.sequencer_mut(0) {
                            if loop_on {
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

#[cfg(not(feature = "native"))]
fn main() {
    println!("This example requires the 'native' feature.");
}
