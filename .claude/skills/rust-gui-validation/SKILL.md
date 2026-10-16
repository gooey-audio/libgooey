---
name: rust-gui-validation
description: Run, drive and screenshot a native Rust GUI (eframe/egui, winit, glow) on a headless Linux VM, then attach the screenshots to a PR. Use when building or changing a GUI example (e.g. examples/*_gui.rs, examples/plate_explorer.rs), when asked to "validate the GUI", "show me the app", or "add screenshots to the PR".
---

# Rust GUI validation

Unit tests cannot tell you whether a GUI looks right or responds to input. This
skill runs the real binary on a virtual X display, drives it with synthetic
mouse and keyboard input, and captures screenshots you can inspect with the
Read tool and attach to a PR. Everything lives in `scripts/` next to this file.

## 1. Set up (once per VM)

```bash
.claude/skills/rust-gui-validation/scripts/setup.sh
```

Installs Xvfb, Mesa (software OpenGL), the X11 libraries winit loads at
runtime (`libxkbcommon-x11`, `libXcursor`, `libXrandr`, `libXi`), ImageMagick
and `python-xlib`. `xdotool` is not packaged for Amazon Linux, which is why the
input driver is a small XTest script.

## 2. Build and launch

Build in release so software rendering keeps up, then launch on display `:99`:

```bash
cargo build --release --example plate_explorer --features native,visualization
.claude/skills/rust-gui-validation/scripts/launch.sh target/release/examples/plate_explorer
```

`launch.sh` starts Xvfb (1600x1000) if needed, restarts the binary, sets
`LIBGL_ALWAYS_SOFTWARE=1` and fails loudly with the app log if the process
dies. Re-run it after every rebuild.

The VM has no sound card. Examples that open an `EngineOutput` should fall back
instead of exiting when `initialize`/`start` fails, otherwise there is nothing
to validate. In libgooey, drive the `Engine` from a thread at real-time pace
when the device is missing (see `spawn_silent_clock` in
`examples/plate_explorer.rs`) so meters and animation still move.

## 3. Drive it and take screenshots

```bash
X=".claude/skills/rust-gui-validation/scripts/xdrive.py"
mkdir -p .context/shots
DISPLAY=:99 python3 $X 'focus Plate Explorer' 'click 740 400' 'key 2' 'sleep 0.3' \
    'shot .context/shots/01-hit.png'
```

Commands: `focus <title>`, `move X Y`, `click/rclick/dclick X Y`,
`drag X1 Y1 X2 Y2 [shift]`, `scroll up|down N`, `key NAME`, `sleep S`,
`shot FILE`. Run `head -30 scripts/xdrive.py` for details.

Workflow:

1. Take a first screenshot and Read it to learn the layout. With no window
   manager the window sits at 0,0 without decorations, so pixel coordinates in
   the screenshot are click coordinates.
2. Script one user story per screenshot (tap, hover, drag, switch tab, play)
   and check each image with Read before moving on. Look for clipped text,
   tofu boxes (glyphs missing from egui's default fonts, e.g. `→`, `✕`, `▦`;
   `▶`, `⏹`, `🗙`, `🎲` render), overlapping labels, empty panels and anything
   that should animate but is static.
3. To review several states at once:
   `montage a.png b.png c.png d.png -tile 2x2 -geometry 800x500+2+2 check.png`.
4. When something does not respond, add a temporary `eprintln!` in the
   handler, rebuild, relaunch, repeat the input and read `/tmp/<binary>.log`.
   Remove the debug line afterwards.

## Gotchas

- **Modifiers need focus.** Without a window manager the window never gets
  focus, so winit drops modifier state: shift-drag arrives as a plain drag.
  Digit and letter keys still work. Run `focus <title>` first.
- **Don't `pkill -f` the binary name** from a shell whose command line contains
  that name; it kills your own shell (exit 144). `launch.sh` uses `pkill -x`.
- **Double-clicks right after a layout change can miss.** Sleep about a second
  after switching tabs or bodies before a `dclick`.
- **Timing-dependent visuals** (ripples, decays, flashes) need the `shot` to
  follow the input within a few hundred ms; put them in the same `xdrive.py`
  invocation.
- egui 0.29 warns `float_literal_f32_fallback` for `Stroke::new(1.0, ..)`;
  write `1.0_f32`.

## 4. Attach screenshots to a PR

Do not commit screenshots to the feature branch. Push them to an orphan
assets branch and link the raw URLs (the repository is public):

```bash
branch=$(git branch --show-current)
assets="pr-assets/${branch//\//-}"
tmp=$(mktemp -d)
git worktree add --detach "$tmp"
git -C "$tmp" checkout --orphan "$assets"
git -C "$tmp" rm -rfq .
cp .context/shots/0*.png "$tmp"/
git -C "$tmp" add . && git -C "$tmp" commit -qm "Screenshots for $branch"
git -C "$tmp" push -q origin "$assets"
git worktree remove --force "$tmp"
echo "https://raw.githubusercontent.com/gooey-audio/libgooey/$assets/01-hit.png"
```

Then add a `## Screenshots` section to the PR body with one
`![caption](url)` per state and a sentence on what each shows, for example
with `gh pr edit <n> --body-file body.md`. Mention that the screenshots come
from Xvfb with software rendering and no audio device.
