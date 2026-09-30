# libgooey

`libgooey` is a Rust-based audio engine which supports sound generation, processing, modulation and beyond. It's intended to be embedded as a library in other applications wherever general sound synthesis applies.

## Features

- Kick, Snare, Hihat and Tom drum synthesizer with comprehensive parameter control
- Parameter smoothing
- 16-step sequencer with sample-accurate timing
- Optional metronome / monitor click, locked to the transport and excluded from offline exports
- C FFI for integration with Swift/iOS and other languages
- Cross-platform support (native, iOS)
- One extensible graphical test application: PolySynth, resonator graph/sequencer,
  instrument/effects auditions, shared stereo scopes and audio diagnostics

## Building

### Loop Studio desktop and headless mixdown

The optional [Loop Studio](docs/loop-studio.md) panel in the same Omni Lab is a working four-track editor:
drums, bass, Nebula/poly-synth chords, and an audio loop, with mixing, effects,
performance/automation recording, JSON save/load, and stereo WAV export.

```bash
cargo run --release --features studio-gui --example omni_gui -- --panel "Loop Studio" --demo
# The loop_studio example is a thin compatibility alias for that panel.
# No audio device required (also works under virtual X):
cargo run --release --no-default-features --features studio-gui --example loop_studio -- --silent --demo
# No window or audio device required:
cargo run --release --no-default-features --features studio --example studio_render -- --export mix.wav --bars 4
```

`studio-gui` enables `studio` + the shared `gui` host (including X11/Wayland),
not a separate application or GLFW visualization.
The reusable `gooey::studio::{Session, Studio}` API is available with `studio`.

### Local Development

```bash
# Build for native (desktop)
cargo build --release

# Open the unified graphical laboratory
cargo run --example omni_gui --features gui

# Run terminal examples (without a GUI)
cargo run --example kick --features crossterm
cargo run --example snare --features crossterm
cargo run --example hihat --features crossterm
cargo run --example sampler_rack --features native,crossterm
```

See [Omni GUI usage, migration inventory, extension API and verification](docs/omni-gui.md).
Use `-- --silent` to render without an audio device, or build with
`--no-default-features --features gui`. Existing graphical example names select
panels in this same application; `visualization` is now an alias for `gui`.

### iOS

Build for iOS devices and simulator:

```bash
./scripts/build-ios.sh
```

This produces:
- `target/aarch64-apple-ios/release/libgooey.a` (device)
- `target/aarch64-apple-ios-sim/release/libgooey.a` (simulator)
- `include/gooey.h` (C header)

## Using Pre-built iOS Binaries

iOS developers can download pre-built static libraries from [GitHub Releases](../../releases) instead of building from source.

### Download Latest Release

```bash
# Download a specific version
VERSION=v0.1.0
curl -L -o libgooey-ios.tar.gz \
  https://github.com/your-org/libgooey/releases/download/$VERSION/libgooey-ios-$VERSION.tar.gz

# Extract
tar -xzf libgooey-ios.tar.gz
```

The archive contains:
- `device/libgooey.a` - iOS device library (aarch64-apple-ios)
- `simulator/libgooey.a` - iOS simulator library (aarch64-apple-ios-sim)
- `include/gooey.h` - C header with FFI bindings
- `README.md` - Integration instructions

### Xcode Integration

1. Add both libraries to your Xcode project
2. Configure Library Search Paths to include the library directories
3. Configure Header Search Paths to include the `include/` directory
4. Add to your bridging header (for Swift):
   ```c
   #include "gooey.h"
   ```

See the extracted `README.md` for detailed integration instructions.

## Releasing (for Maintainers)

To create a new iOS release:

1. Tag the release:
   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```

2. The GitHub Actions workflow will automatically:
   - Build iOS libraries (device + simulator)
   - Generate C headers
   - Create a GitHub Release
   - Upload the tarball as a release asset

3. The release will be available at:
   ```
   https://github.com/your-org/libgooey/releases/download/v0.1.0/libgooey-ios-v0.1.0.tar.gz
   ```

You can also trigger builds manually via GitHub Actions UI (workflow_dispatch).

## API Overview

```c
// Create engine
GooeyEngine* engine = gooey_engine_new(44100.0);

// Set parameters
gooey_engine_set_kick_param(engine, KICK_PARAM_FREQUENCY, 60.0);
gooey_engine_set_bpm(engine, 120.0);

// Configure sequencer
gooey_engine_sequencer_set_step(engine, 0, true);  // Step 0: on
gooey_engine_sequencer_set_step(engine, 4, true);  // Step 4: on
gooey_engine_sequencer_start(engine);

// Render audio
float buffer[512];
gooey_engine_render(engine, buffer, 512);

// Cleanup
gooey_engine_free(engine);
```

See `include/gooey.h` for complete API documentation.

For apps migrating from the old flat instrument mix assumption to mixer graph
submixing, see [docs/mixer-graph-migration.md](docs/mixer-graph-migration.md).

## License

MIT
