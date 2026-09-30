# Gooey Omni Lab

Open **one application**, with a single audio owner and common stereo waveform,
spectrum, peak/RMS meters and health counters:

```sh
cargo run --example omni_gui --features gui
cargo run --example omni_gui --features gui -- --panel Resonator --silent
cargo run --no-default-features --example omni_gui --features gui -- --list
cargo run --release --no-default-features --example omni_gui --features gui -- --stress 600
```

Panel names are `PolySynth`, `Resonator`, and `Experiments` (case-insensitive).
`--silent` still synthesizes continuously, but never opens an audio device.
Without `native`, silent rendering is the only backend. Device initialization
failure is shown in the shell and falls back to silent rendering. Stress mode
opens neither a window nor a device; it renders the requested **audio duration
per panel per sample rate**, faster than real time, then draws headless egui
layouts throughout the run. Built-in stress scenarios change presets, route
destinations/sources, inspectors, taps, patterns, all eight instruments and all
seven effect selections once per simulated second. It always covers all mounted
panels. `--soak SECONDS` exercises the
actual silent worker and repeated panel transitions in wall-clock time.

## What is centralized

`src/gui/mod.rs` owns the only eframe `App` and desktop entrypoint, panel
selection, common parameter sliders and playable keyboard, and the shared
debugging view. `src/gui/audio.rs` owns either one CPAL stream **or** one joined
silent worker, preallocated render scratch, sample/channel conversion, finite
sample sanitization and bounded atomic stereo history. `src/gui/poly.rs`,
`src/gui/resonator.rs`, and `src/gui/experiments.rs` are editors, not independent
applications. Switching stops/drops the outgoing stream or joins its worker
before deactivating the old panel and creating the incoming renderer. Inactive
panels have no workers. Stop audio and window close also deactivate the panel.

PolySynth retains all 30 parameters in oscillator, amplitude, pitch, filter and
expression pages; all eight route slots have enable/source/destination/depth/
curve/key-scale controls. Its five factory presets retain independent edits
during the application session and can be reset. Z–M and Q–I play two chromatic
octaves; Space plays a major chord, velocity and octave are editable, and the
on-screen keys work with the pointer. Losing window focus or entering a text
editor releases held notes; switching presets/panels releases all notes.

Resonator retains the clickable topology, six inspectors, all nine routing
gains, lowpass/bandpass taps on both modes, all five factory patches, independent
four-track configurations, trigger velocity, tempo and 16-step pattern editing.
The scope below it now measures **the final stereo mix**, not four interleaved
per-voice samples. Space triggers the selected voice or toggles transport in
the sequencer view. Switching away stops all four sequencers.

Experiments provides Kick, Snare, HiHat2, Tom, Tom2, Bass, ResoKick and a
noise/envelope-excited MembraneResonator. Parameter names come from each real
instrument's modulation descriptors and edits call its modulation API.
Normalized UI edits are mapped to that API's bipolar range. Instruments start
at their actual factory defaults; because the legacy descriptor interface has
no getter, slider markers initially indicate midpoint **pending edits**, not
a claimed reading of those defaults. Dry, tempo-synced Delay (including
triplets/ping-pong), Spring Reverb, Plate Reverb (predelay/width/size), Lowpass,
Saturation and Tilt process real audio. Its editable 16-step transport and LFO
target/rate/depth are engine controls, not a dashboard simulation.

## Inventory and migration of every previous graphical path

| Previous path | Central capability / compatibility behavior |
| --- | --- |
| `examples/polysynth_gui.rs` | Thin selected-PolySynth launcher; no GLFW renderer or audio wrapper remains in this example. |
| `examples/resonator_voice_gui.rs` | Thin selected-Resonator launcher; topology, inspectors, routing, taps, patches and four-track editor moved into `src/gui/resonator.rs`. |
| `examples/kick.rs` | `visualization` invokes Experiments / Kick, shared final-output scope. Without that feature, original terminal client remains. |
| `examples/snare.rs` | Experiments / Snare. Same terminal compatibility rule. |
| `examples/hihat.rs`, `examples/hihat2.rs` | Experiments / HiHat2. `HiHat` is already an engine alias for `HiHat2` on main. The old hihat terminal source referenced removed closed/open APIs and did not compile; its name now shares hihat2's functioning terminal controls too. |
| `examples/tom.rs` | Experiments / Tom. |
| `examples/tom2.rs` | Experiments / Tom2, including membrane-related modulation descriptors. |
| `examples/bass.rs` | Experiments / Bass. |
| `examples/bass_sequencer.rs` | Experiments / Bass with transport started. The old terminal note-editing client remains available without visualization; the new general pattern is an audition pattern rather than importing that terminal session. |
| `examples/membrane.rs` | Experiments / Membrane: noise through the original MaxCurveEnvelope shape and MembraneResonator; Q/gain scaling controls. Noise uses a deterministic xorshift generator, not the terminal's hash implementation, so this is capability-equivalent, not sample-identical. |
| `examples/sequencer.rs` | Resonator four-track sequencer, with common final mix diagnostics. The original terminal preset-blending sequencer is unchanged without visualization; its terminal command surface is not duplicated. |
| `examples/lfo_test.rs` | Experiments / Kick, transport and LFO enabled; target/rate/depth editable. |
| `examples/delay.rs` | Experiments / Kick, Delay and transport enabled. |
| `examples/reverb.rs` | Experiments / Kick, Spring Reverb and transport enabled. |
| `examples/reverb_lab.rs` | Experiments / Snare, Spring Reverb and transport enabled; Plate is selectable in the same effect editor. |
| `src/visualization.rs` / `AudioBuffer` | Exported buffer and snapshots preserved for existing consumers; not the new audio callback path. |
| `src/visualization/spectrogram.rs` / `SpectrogramAnalyzer` | Exported analysis API retained. The shell's shared spectrum is a current Hann-windowed FFT, not a scrolling historical spectrogram. |
| `src/visualization/waveform_display.rs` / `WaveformDisplay` | Retired for all example entrypoints; only compiled when external clients explicitly opt into `legacy-visualization`. |
| `EngineOutput::enable_visualization` / `update_visualization` | Signatures retained. With `visualization` alone, enabling returns an actionable migration error before opening any window; update without an enabled display remains a no-op. Explicit `legacy-visualization` restores the manually pumped window for external compatibility clients. |

The `visualization` feature is now an alias of `gui`. Thus old graphical example
commands (with their existing `native,crossterm,visualization` features) enter
the unified shell. Dedicated GUI example commands using `visualization` also
continue to work. GLFW/OpenGL dependencies and the old backend remain only as
an **explicit external compatibility exception**; no example selects them,
even with that compatibility feature enabled. This is a deliberate migration
of the old window API, not a silent promise that its pumping semantics can be
implemented by eframe. Code using `WaveformDisplay::glfw()` must opt into
`legacy-visualization` as well. The default native engine, iOS engine, C ABI and
non-graphical clients are not rerouted or made dependent on a GUI toolkit.

## Mounting an extension in this shell

Public interfaces live under `gooey::gui`. `GuiPanel::ui` draws the editor in
the remaining egui area (the shell already reserves navigation and diagnostics).
`GuiPanel::renderer(rate, health)` returns a `Box<dyn BlockRenderer>` owned by
the host. `GuiPanel::deactivate` must release notes and stop transport;
the host has already stopped rendering before it calls this method.
`PanelFactory` constructs rate-dependent DSP at the negotiated device sample
rate. An extension must not call `run_native`, create a CPAL stream, or spawn a
scratch/silent audio worker.

For example, a later safe Rust Studio wrapper can hold a shared engine in its
panel, draw its controls with `parameter`/`Keyboard`, and return an
`InterleavedAdapter` for its block API:

```rust,ignore
use gooey::gui::{self, AudioHealth, BlockRenderer, GuiPanel, InterleavedAdapter};
use std::sync::{Arc, Mutex};

struct MyPanel { engine: Arc<Mutex<MySafeEngine>> }
impl GuiPanel for MyPanel {
    fn name(&self) -> &str { "My panel" }
    fn ui(&mut self, ctx: &gui::egui::Context) {
        gui::egui::CentralPanel::default().show(ctx, |ui| {
            // Draw actual engine controls; use gui::parameter and gui::Keyboard.
        });
    }
    fn renderer(&mut self, _rate: f32, health: Arc<AudioHealth>) -> Box<dyn BlockRenderer> {
        let engine = Arc::clone(&self.engine);
        Box::new(InterleavedAdapter::new(move |output: &mut [f32], rate| {
            if let Ok(mut engine) = engine.try_lock() {
                engine.render_interleaved(output, rate);
            } else {
                output.fill(0.0);
                health.contention.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }))
    }
    fn deactivate(&mut self) {
        // On the GUI thread: release notes and stop the engine transport.
    }
}
fn main() -> anyhow::Result<()> {
    let mut factories = gui::builtin_factories();
    factories.push(Box::new(|rate| Box::new(MyPanel {
        engine: Arc::new(Mutex::new(MySafeEngine::new(rate)))
    })));
    gui::run_with(factories, Some("My panel"))
}
```

This example intentionally uses an illustrative `MySafeEngine`, not a
prerequisite dependency on Studio. `RenderAdapter(F)` adapts stereo-frame
closures directly; `InterleavedAdapter::new(F)` adapts interleaved f32 stereo
and owns fixed scratch storage. `EngineRenderer::new` adapts the existing Rust
Engine. Custom block renderers must fill every output frame, honor the rate,
and not wait for GUI locks. The host may request partial blocks; the
interleaved adapter chunks requests at 256 frames. The shell automatically
scopes all adapters, so extensions need no telemetry implementation.

`ParameterDescriptor` centralizes labels, IDs and ranges; `poly_parameters()`
publishes the 30 built-in normalized synth descriptors. `stereo_spectrum`
combines channel powers, so opposite-phase stereo signals do not disappear
from the shared spectrum. Both helpers are public for extension test code.

The optional `BlockRenderer::set_time(seconds)` hook receives the one
host-owned cumulative audio timestamp before each block. Built-in timestamped
engines use it so returning to a panel never moves envelope time backward.
Engines with their own sample-count API can ignore it. Health counters remain
cumulative across panel switches; scope history is cleared on activation.
`GuiPanel::exercise_step(step)` is an optional control-side harness hook for
custom dynamic stress scenarios; it defaults to the panel's basic audition.

## Debugging and POC constraints

Health shows processed blocks, non-finite samples replaced with zero, engine
lock misses (silenced blocks), elapsed render time over the block budget,
maximum block time, device errors, recovered GUI control-lock errors, and
renderer failures. A caught unwinding renderer panic is counted and its block
silenced; this is a troubleshooting fallback, not proof the renderer can safely
continue in all states. Peak/RMS and scope are sampled from a bounded
2048-frame ring; snapshots may contain a stale frame without blocking audio.
Scope and FFT allocations happen on the GUI thread. Native mono uses the
stereo average, stereo uses L/R, and channels beyond two are silent. CPAL's
f32/f64 and signed/unsigned 8/16/32/64-bit output formats are supported. Native
output clamps to [-1,1]; telemetry intentionally shows pre-clamp levels.

This is a graphical **test** host, not a hard-real-time production engine.
The host's successful render path uses fixed scratch and atomic publication,
but the legacy Rust Engine itself can allocate while advancing sequencers or
LFOs. Its UI adapter uses a nonblocking engine try-lock per block; edits may
therefore produce audible silent gaps, counted in diagnostics. Resonator GUI
edits take the engine lock before voice locks to avoid waiting inside the
callback. Effect edits currently construct and swap an effect on the GUI
thread and clear its tail; they are not glitch-free automation. No MIDI device,
session persistence, scrolling spectrogram, offline export or Loop Studio
features are added by this prerequisite.

## Verification

Shutdown joins the silent worker after its current block. Rust cannot safely
interrupt a custom renderer stuck in an infinite loop or blocking call, so an
extension violating the prompt-return contract can stall shutdown. Built-in
audio adapters never wait for GUI locks; this is not a claim that arbitrary
extension DSP can be forcibly time-bounded by the shell.

Run `cargo test --features gui --lib --tests` and the corresponding
`--no-default-features --features gui` command. GUI tests cover all 30 PolySynth
parameters and destinations, both modulation sources and eight slots, factory
presets, every parameter page, resonator patches/taps/routing/inspectors and
patterns, every instrument/effect combination, finite audio at 44.1/48 kHz,
bounded telemetry, mono/stereo/multichannel conversion, focus/chord/octave
keyboard transitions, engine lock contention, repeated worker switching and
headless shell layout at 900×700 and 1440×1000. Parent review owns independent
Xvfb interaction captures and physical/device audio validation.

An additional allocator integration probe renders/publishes 10,000 blocks
through the generic interleaved adapter with zero allocations or frees inside
that seam. This deliberately does **not** make that claim about the legacy
Engine itself.

The prerequisite's final verification passed **533 library + 353 integration
tests** in both native and no-native GUI configurations. A 600-second dynamic
run of every panel at both sample rates passed (3600 synthesized seconds total),
as did a 180-second wall-clock worker/control soak with 180 panel auditions and
30137 rendered blocks. Native binaries for the main application and both thin
lab aliases build; all examples build with
`gui,visualization,crossterm,bounce,midi`, and `--no-default-features --features
ios` builds without the GUI. Format/diff checks pass. Normal Clippy succeeds
with 99 existing engine/FFI warnings and no warnings in `src/gui`; strict
`-D warnings` remains blocked by that baseline. The optional retired GLFW
backend could not be built in this environment because `cmake` is missing;
unrelated `plots` builds require missing `fontconfig.pc`. Neither dependency is
needed by the unified graphical app. Independent GUI capture and device audio
validation remain parent-review responsibilities.
