//! MIDI input handler for drum pad examples
//! This module is example-only and not part of the core library

use midir::{MidiInput, MidiInputConnection};
use std::sync::mpsc::{channel, Receiver};

/// MIDI message types relevant for drums
#[derive(Debug, Clone, Copy)]
pub enum MidiDrumEvent {
    /// Note on with note number and velocity (0-127)
    NoteOn { note: u8, velocity: u8 },
    /// Note off (note field kept for future use with choked hi-hats)
    #[allow(dead_code)]
    NoteOff { note: u8 },
}

/// Standard General MIDI drum note mappings
#[allow(dead_code)]
pub mod drum_notes {
    pub const KICK: u8 = 36;         // Bass Drum 1
    pub const KICK_ALT: u8 = 35;     // Acoustic Bass Drum
    pub const SNARE: u8 = 38;        // Acoustic Snare
    pub const SNARE_ALT: u8 = 40;    // Electric Snare
    pub const HIHAT_CLOSED: u8 = 42; // Closed Hi-Hat
    pub const HIHAT_PEDAL: u8 = 44;  // Pedal Hi-Hat
    pub const HIHAT_OPEN: u8 = 46;   // Open Hi-Hat
    pub const TOM_HIGH: u8 = 50;     // High Tom
    pub const TOM_MID: u8 = 47;      // Low-Mid Tom
    pub const TOM_LOW: u8 = 45;      // Low Tom
    pub const TOM_FLOOR: u8 = 41;    // Low Floor Tom
    pub const CRASH: u8 = 49;        // Crash Cymbal 1
    pub const RIDE: u8 = 51;         // Ride Cymbal 1
}

/// MIDI input handler that runs in a separate thread
pub struct MidiHandler {
    _connection: MidiInputConnection<()>,
    receiver: Receiver<MidiDrumEvent>,
}

impl MidiHandler {
    /// Create a new MIDI handler, connecting to the first available MIDI input
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Self::with_port_filter(None)
    }

    /// Create with optional port name filter
    pub fn with_port_filter(
        port_name_filter: Option<&str>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let midi_in = MidiInput::new("libgooey-example")?;

        let ports = midi_in.ports();
        if ports.is_empty() {
            return Err("No MIDI input devices found".into());
        }

        // Find matching port or use first available
        let port = if let Some(filter) = port_name_filter {
            ports
                .iter()
                .find(|p| {
                    midi_in
                        .port_name(p)
                        .map(|n| n.contains(filter))
                        .unwrap_or(false)
                })
                .unwrap_or(&ports[0])
        } else {
            &ports[0]
        };

        let port_name = midi_in.port_name(port)?;
        println!("Connecting to MIDI input: {}", port_name);

        let (sender, receiver) = channel::<MidiDrumEvent>();

        // Create callback that parses MIDI and sends events
        let connection = midi_in.connect(
            port,
            "libgooey-midi-input",
            move |_timestamp, message, _| {
                if let Some(event) = parse_midi_message(message) {
                    let _ = sender.send(event);
                }
            },
            (),
        )?;

        Ok(Self {
            _connection: connection,
            receiver,
        })
    }

    /// List available MIDI input ports
    pub fn list_ports() -> Vec<String> {
        let midi_in = MidiInput::new("libgooey-list").ok();
        midi_in
            .map(|m| {
                m.ports()
                    .iter()
                    .filter_map(|p| m.port_name(p).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Poll for MIDI events (non-blocking)
    #[allow(dead_code)]
    pub fn poll(&self) -> Option<MidiDrumEvent> {
        self.receiver.try_recv().ok()
    }

    /// Poll all pending events
    pub fn poll_all(&self) -> Vec<MidiDrumEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.receiver.try_recv() {
            events.push(event);
        }
        events
    }
}

/// Parse raw MIDI bytes into drum events
fn parse_midi_message(message: &[u8]) -> Option<MidiDrumEvent> {
    if message.len() < 3 {
        return None;
    }

    let status = message[0];
    let note = message[1];
    let velocity = message[2];

    // Note On (0x90-0x9F) - accept any channel
    if status & 0xF0 == 0x90 {
        if velocity > 0 {
            return Some(MidiDrumEvent::NoteOn { note, velocity });
        } else {
            // Note On with velocity 0 = Note Off
            return Some(MidiDrumEvent::NoteOff { note });
        }
    }

    // Note Off (0x80-0x8F)
    if status & 0xF0 == 0x80 {
        return Some(MidiDrumEvent::NoteOff { note });
    }

    None
}

/// Convert MIDI velocity (0-127) to normalized float (0.0-1.0)
pub fn velocity_to_float(velocity: u8) -> f32 {
    (velocity as f32) / 127.0
}
