# Envelope Editor

An interactive visual tool for debugging and experimenting with ADSR envelope shapes in libgooey.

## Overview

The envelope editor provides a graphical interface for adjusting envelope parameters in real-time. This is a development/debugging tool and is **not** part of the core library export - it's only available when the `visualization` feature is enabled.

## Features

- **Visual Envelope Display**: See the complete ADSR envelope curve rendered in real-time
- **Interactive Control Points**: Drag control points to adjust envelope parameters:
  - **Attack Point**: Controls attack time (how quickly the sound reaches peak amplitude)
  - **Decay Point**: Controls both decay time and sustain level
  - **Sustain Point**: Visual-only, shows sustain duration
  - **Release Point**: Controls release time (how quickly the sound fades after note release)
- **Grid Background**: Helps visualize timing and amplitude
- **Real-time Audio Feedback**: Test your envelope changes immediately with instrument triggers

## Usage

### Running the Example

```bash
cargo run --example envelope_editor --features native,visualization,crossterm
```

### Using in Your Code

```rust
use libgooey::envelope::ADSRConfig;
use libgooey::visualization::EnvelopeEditor;

// Create an editor with initial configuration
let initial_config = ADSRConfig::default();
let mut editor = EnvelopeEditor::new(800, 600, initial_config)?;

// Main loop
while !editor.should_close() {
    editor.process_events();
    editor.render();

    // Get the current configuration
    let config = editor.get_config();

    // Apply to your instruments...
}
```

### Controls

- **Mouse Drag**: Click and drag control points to adjust envelope parameters
- **ESC Key**: Close the editor window

## Implementation Details

### Architecture

The envelope editor is implemented as part of the `visualization` module:

```
src/
  visualization/
    envelope_editor.rs  <- Envelope editor implementation
  visualization.rs      <- Module exports (feature-gated)
```

### Feature Flags

The envelope editor requires the `visualization` feature flag:

```toml
[dependencies]
libgooey = { version = "0.1", features = ["visualization"] }
```

This ensures the editor code is not included in production builds of the core library.

### OpenGL Rendering

The editor uses OpenGL 3.3 Core Profile for rendering:
- Simple vertex/fragment shaders for 2D graphics
- Line strips for envelope curves
- Triangle fans for control point circles
- Grid lines for reference

### Thread Safety

The example demonstrates how to use the editor in a separate thread while sharing envelope configuration through `Arc<Mutex<ADSRConfig>>` for thread-safe updates.

## Integration with Instruments

The example shows how to connect the editor to an instrument (KickDrum). To use with your own instruments:

1. Get the current config from the editor: `editor.get_config()`
2. Apply it to your instrument's envelope using the appropriate setter methods
3. Trigger the instrument to hear the result

## Limitations

- The sustain point in the editor is for visualization only - ADSR sustain is a level, not a duration
- The editor shows a fixed time scale - very long envelopes may extend beyond the visible area
- Currently only supports ADSR envelopes (not arbitrary multi-point envelopes)

## Future Enhancements

Potential improvements for the envelope editor:

- [ ] Support for arbitrary multi-point envelopes
- [ ] Curve shape controls (linear, exponential, logarithmic)
- [ ] Time scale zoom controls
- [ ] Preset save/load functionality
- [ ] Visual feedback of currently playing envelope
- [ ] Support for modulation envelopes (pitch, filter, etc.)
