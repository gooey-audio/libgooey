//! Membrane Resonator Example - Test for membrane effect component
//!
//! Uses the MembraneResonator filter effect with noise input and envelope:
//! noise~ -> *~ (curve envelope) -> MembraneResonator -> output
//!
//! Controls:
//! - SPACE = trigger membrane hit
//! - ↑/↓ = adjust Q scaling factor
//! - ←/→ = adjust gain scaling factor
//! - R = reset scaling to defaults
//! - Q = quit

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use gooey::engine::{Engine, EngineOutput, Instrument};
use gooey::filters::{MembraneResonator, DEFAULT_MEMBRANE_PARAMS};
use gooey::max_curve::MaxCurveEnvelope;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Membrane instrument - uses noise + envelope to excite the resonator effect
pub struct MembraneInstrument {
    envelope: MaxCurveEnvelope,
    resonator: MembraneResonator,
    input_gain: f32,
    is_active: bool,
    // Noise generator state
    sample_counter: u64,
}

impl MembraneInstrument {
    pub fn new(sample_rate: f32) -> Self {
        // Create envelope matching Max patch: "1 5 0.8 0 2000 -0.83"
        let envelope = MaxCurveEnvelope::new(vec![
            (1.0, 5.0, 0.8),      // Attack: rise to 1.0 in 5ms with curve 0.8
            (0.0, 2000.0, -0.83), // Decay: fall to 0.0 in 2000ms with curve -0.83
        ]);

        // Create the membrane resonator effect
        let mut resonator = MembraneResonator::new(sample_rate);
        resonator.set_q_scale(0.01);
        resonator.set_gain_scale(0.001);

        Self {
            envelope,
            resonator,
            input_gain: 0.99,
            is_active: false,
            sample_counter: 0,
        }
    }

    pub fn q_scale(&self) -> f32 {
        self.resonator.q_scale()
    }

    pub fn set_q_scale(&mut self, scale: f32) {
        self.resonator.set_q_scale(scale);
    }

    pub fn gain_scale(&self) -> f32 {
        self.resonator.gain_scale()
    }

    pub fn set_gain_scale(&mut self, scale: f32) {
        self.resonator.set_gain_scale(scale);
    }

    pub fn reset_scaling(&mut self) {
        self.resonator.set_q_scale(0.01);
        self.resonator.set_gain_scale(0.001);
    }

    /// Generate white noise sample using hash-based approach
    #[inline]
    fn noise(&mut self) -> f32 {
        self.sample_counter = self.sample_counter.wrapping_add(1);
        let mut hasher = DefaultHasher::new();
        self.sample_counter.hash(&mut hasher);
        let hash = hasher.finish();
        // Convert hash to float in range [-1.0, 1.0]
        let normalized = (hash as f32) / (u64::MAX as f32);
        (normalized * 2.0) - 1.0
    }
}

impl Instrument for MembraneInstrument {
    fn trigger_with_velocity(&mut self, time: f32, _velocity: f32) {
        self.is_active = true;
        self.envelope.trigger(time);
        self.resonator.reset();
    }

    fn tick(&mut self, current_time: f32) -> f32 {
        if !self.is_active {
            return 0.0;
        }

        // Get envelope value
        let env = self.envelope.get_value(current_time);

        // Check if envelope is done
        if self.envelope.is_complete() {
            self.is_active = false;
            return 0.0;
        }

        // Generate enveloped noise: noise * envelope * input_gain
        let noise = self.noise();
        let input = noise * env * self.input_gain;

        // Process through the membrane resonator effect
        self.resonator.process(input)
    }

    fn is_active(&self) -> bool {
        self.is_active
    }

    fn as_modulatable(&mut self) -> Option<&mut dyn gooey::engine::Modulatable> {
        None
    }
}

// Wrapper for thread-safe sharing
struct SharedMembrane(Arc<Mutex<MembraneInstrument>>);

impl Instrument for SharedMembrane {
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

fn render_display(membrane: &MembraneInstrument, trigger_count: u32) {
    print!("\x1b[2J\x1b[H\x1b[?7l");

    print!("=== Membrane Resonator Test ===\r\n");
    print!("SPACE=hit  Q=quit  R=reset\r\n");
    print!("up/dn=Q scale  lt/rt=gain scale\r\n");
    print!("\r\n");

    print!("Q Scale:    {:.4}\r\n", membrane.q_scale());
    print!("Gain Scale: {:.4}\r\n", membrane.gain_scale());
    print!("\r\n");

    print!("Filter Configuration:\r\n");
    print!("  #  Freq(Hz)  Q(raw)  Q(scaled)  Gain(raw)  Gain(scaled)\r\n");
    for (i, (gain, freq, q)) in DEFAULT_MEMBRANE_PARAMS.iter().enumerate() {
        let scaled_q = q * membrane.q_scale();
        let scaled_gain = gain * membrane.gain_scale();
        print!(
            "  {}  {:>7.0}  {:>6.0}  {:>9.2}  {:>9.0}  {:>11.4}\r\n",
            i + 1,
            freq,
            q,
            scaled_q,
            gain,
            scaled_gain
        );
    }

    print!("\r\n");
    print!("Hits: {}\r\n", trigger_count);
    print!("\r\n");
    print!("Envelope: attack 5ms (curve 0.8), decay 2000ms (curve -0.83)\r\n");

    io::stdout().flush().unwrap();
}

#[cfg(feature = "native")]
fn main() -> anyhow::Result<()> {
    let sample_rate = 44100.0;

    // Create membrane instrument (noise + envelope + resonator)
    let membrane = Arc::new(Mutex::new(MembraneInstrument::new(sample_rate)));

    // Create shared wrapper for the engine
    let shared_membrane = SharedMembrane(membrane.clone());

    // Create the audio engine
    let mut engine = Engine::new(sample_rate);
    engine.add_instrument("membrane", Box::new(shared_membrane));

    // Wrap engine in Arc<Mutex> for thread-safe access
    let audio_engine = Arc::new(Mutex::new(engine));

    // Create and configure the Engine output
    let mut engine_output = EngineOutput::new();
    engine_output.initialize(sample_rate)?;

    #[cfg(feature = "visualization")]
    engine_output.enable_visualization(1200, 400, 2.0)?;

    engine_output.create_stream_with_engine(audio_engine.clone())?;

    // Start the audio stream
    engine_output.start()?;

    // UI state
    let mut trigger_count: u32 = 0;
    let mut needs_redraw = true;

    // Clear screen and enable raw mode
    execute!(io::stdout(), Clear(ClearType::All), cursor::Hide)?;
    enable_raw_mode()?;

    // Main input loop
    let result = loop {
        #[cfg(feature = "visualization")]
        if engine_output.update_visualization() {
            break Ok(());
        }

        // Render display if needed
        if needs_redraw {
            let m = membrane.lock().unwrap();
            render_display(&m, trigger_count);
            needs_redraw = false;
        }

        // Poll for key events (non-blocking with short timeout)
        if event::poll(std::time::Duration::from_millis(16))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match code {
                    // Trigger
                    KeyCode::Char(' ') => {
                        let mut engine = audio_engine.lock().unwrap();
                        engine.trigger_instrument("membrane");
                        trigger_count += 1;
                        needs_redraw = true;
                    }

                    // Adjust Q scale
                    KeyCode::Up => {
                        let mut m = membrane.lock().unwrap();
                        let current = m.q_scale();
                        m.set_q_scale(current * 1.1);
                        needs_redraw = true;
                    }
                    KeyCode::Down => {
                        let mut m = membrane.lock().unwrap();
                        let current = m.q_scale();
                        m.set_q_scale(current / 1.1);
                        needs_redraw = true;
                    }

                    // Adjust gain scale
                    KeyCode::Right => {
                        let mut m = membrane.lock().unwrap();
                        let current = m.gain_scale();
                        m.set_gain_scale(current * 1.1);
                        needs_redraw = true;
                    }
                    KeyCode::Left => {
                        let mut m = membrane.lock().unwrap();
                        let current = m.gain_scale();
                        m.set_gain_scale(current / 1.1);
                        needs_redraw = true;
                    }

                    // Reset
                    KeyCode::Char('r') | KeyCode::Char('R') => {
                        let mut m = membrane.lock().unwrap();
                        m.reset_scaling();
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

    // Restore terminal to normal mode
    print!("\x1b[?7h");
    execute!(io::stdout(), cursor::Show)?;
    disable_raw_mode()?;
    println!("\nQuitting...");

    result
}

#[cfg(not(feature = "native"))]
fn main() {
    println!("This example requires the 'native' feature. Run with: cargo run --example membrane --features native");
}
