# AGENTS.md - Coding Agent Guidelines for libgooey

This document provides guidelines for AI coding agents working on the libgooey codebase.

## Project Overview

libgooey is a Rust audio synthesis engine supporting native (desktop), iOS, and WASM targets.
It provides drum synthesizers, sequencing, LFO modulation, and effects processing.

## Build Commands

```bash
# Build
cargo build                          # Debug build (default features: native)
cargo build --release                # Release build
cargo build --features ios           # iOS target (no native audio output)
cargo build --features web           # WASM target

# Run examples (require native + crossterm features)
cargo run --example kick --features "native,crossterm"
cargo run --example snare --features "native,crossterm"
cargo run --example sequencer --features "native,crossterm"
```

## Testing

```bash
# Run all tests
cargo test --verbose

# Run tests with native features
cargo test --features native --verbose

# Run a single test by name
cargo test test_engine_creation --verbose
cargo test test_kick_drum_modulation --verbose

# Run tests in a specific file
cargo test --test engine_basics --verbose
cargo test --test lfo_modulation --verbose

# Run tests matching a pattern
cargo test modulation --verbose
```

## Linting and Formatting

```bash
# Check formatting (CI runs this)
cargo fmt --all -- --check

# Apply formatting
cargo fmt --all

# Run clippy (CI runs with all features)
cargo clippy --all-targets --all-features

# Fix clippy warnings automatically
cargo clippy --fix --all-targets --all-features
```

## Feature Flags

| Feature | Description |
|---------|-------------|
| `native` | Desktop audio output via cpal (default) |
| `ios` | iOS target - engine only, no audio output |
| `web` | WASM/WebAssembly bindings |
| `crossterm` | Terminal UI for examples |
| `visualization` | Waveform display (requires glfw, gl, rustfft) |

## Code Style Guidelines

### Imports

Group imports in this order, separated by blank lines:
1. Standard library (`std::`)
2. External crates
3. Local crate modules (`crate::`, `super::`)

```rust
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::engine::{Engine, Instrument};
use crate::instruments::KickDrum;
```

### Types and Naming

- Use `f32` for all audio values (sample rate, time, amplitude, frequency)
- Structs: PascalCase (`KickDrum`, `ADSRConfig`)
- Functions/methods: snake_case (`trigger_instrument`, `set_bpm`)
- Constants: SCREAMING_SNAKE_CASE (`KICK_PARAM_FREQUENCY`)
- Type aliases: PascalCase (`type ParamSmoother = SmoothedParam`)

### Struct Patterns

**Config structs** - Static presets with validation in constructor:
```rust
#[derive(Clone, Copy, Debug)]
pub struct KickConfig {
    pub kick_frequency: f32,
    pub decay_time: f32,
    // ...
}

impl KickConfig {
    pub fn new(...) -> Self { /* validate and clamp values */ }
    pub fn default() -> Self { /* sensible defaults */ }
    pub fn punchy() -> Self { /* named preset */ }
}
```

**Params structs** - Runtime smoothed parameters:
```rust
pub struct KickParams {
    pub frequency: SmoothedParam,
    pub decay: SmoothedParam,
}
```

### Traits

Core traits that instruments/effects must implement:

```rust
/// All instruments must be Send for audio thread safety
pub trait Instrument: Send {
    fn trigger(&mut self, time: f32);
    fn tick(&mut self, current_time: f32) -> f32;
    fn is_active(&self) -> bool;
}

pub trait Effect: Send {
    fn process(&self, input: f32) -> f32;
}

pub trait Modulatable {
    fn modulatable_parameters(&self) -> Vec<&'static str>;
    fn apply_modulation(&mut self, parameter: &str, value: f32) -> Result<(), String>;
}
```

### Documentation

- Module-level docs: `//!` at top of file
- Public items: `///` doc comments
- FFI functions: Include `# Safety` section for unsafe functions
- Inline comments: `//` for implementation notes

```rust
//! C FFI bindings for the gooey audio engine

/// Create a new gooey engine
///
/// # Safety
/// The returned pointer must be freed with `gooey_engine_free`.
#[no_mangle]
pub extern "C" fn gooey_engine_new(sample_rate: f32) -> *mut GooeyEngine {
```

### Error Handling

- Use `Result<T, String>` for recoverable errors in library code
- Use `anyhow::Result` in examples and binaries
- Validate parameters with `.clamp()` or `.max()/.min()` in constructors

### FFI Code

For C-compatible functions in `src/ffi.rs`:
- Use `#[no_mangle]` and `extern "C"`
- Check for null pointers at function entry
- Document safety requirements
- Use `Box::into_raw` / `Box::from_raw` for heap allocation

### Conditional Compilation

Use feature gates for platform-specific code:
```rust
#[cfg(feature = "native")]
pub mod engine_output;

#[cfg(feature = "web")]
pub mod web { /* WASM bindings */ }
```

## Testing Patterns

**Integration tests** in `tests/` directory:
```rust
use gooey::engine::{Engine, Sequencer};
use gooey::instruments::KickDrum;

#[test]
fn test_engine_creation() {
    let engine = Engine::new(44100.0);
    assert_eq!(engine.sample_rate(), 44100.0);
}
```

**Unit tests** within modules:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_smoother_reaches_target() {
        // ...
    }
}
```

## Architecture Notes

- `Engine` is the central audio coordinator, owns instruments and effects
- Instruments generate audio via `tick(current_time)` returning a single sample
- Sequencer provides sample-accurate timing for triggering instruments
- Parameter smoothing prevents clicks/pops during real-time control
- Thread safety: wrap `Engine` in `Arc<Mutex<>>` for audio thread access
