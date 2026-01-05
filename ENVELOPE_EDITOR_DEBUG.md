# Envelope Editor Debug Guide

## Fixed Issues

### 1. Envelope Line Continuity
**Problem**: There was a discontinuity in the envelope line - it didn't start at (0, 0).

**Fix**: Modified `draw_envelope_curve()` to explicitly start at the origin (0, 0) before generating the rest of the curve points.

### 2. Retina Display / Framebuffer Size
**Problem**: On macOS with Retina displays, the window size (800x600) differs from the framebuffer size (1600x1200), causing mouse coordinates to be wrong.

**Fix**: Use `window.get_framebuffer_size()` instead of the window size parameters for coordinate calculations.

### 3. Control Point Click Detection
**Problem**: The threshold for detecting clicks on control points was too small (0.05 in normalized coordinates), making them hard to click. The control points themselves were also very small (radius 0.02).

**Fix**: 
- Increased click threshold from 0.05 to 0.15
- Increased control point radius from 0.02 to 0.04

### 4. Added Debug Output
Added extensive debug output to track mouse events:
- Window and framebuffer sizes
- Mouse press/release events
- Mouse position (screen and normalized coordinates)
- Control point positions and distances
- Which point (if any) is selected for dragging

## How to Test

Run the example from your terminal (not through the bash tool):

```bash
cd /Users/pretzel/code/gooey/libgooey
cargo run --example envelope_editor --features native,visualization,crossterm
```

You should see output like this at startup:
```
Window size: 800x600, Framebuffer size: 1600x1200
```

On Retina displays, the framebuffer is 2x the window size. This is normal and now handled correctly.

### Expected Behavior

1. **Window should open** showing:
   - A green envelope curve starting at (0,0)
   - Four white control points:
     - Attack point (left, at top)
     - Decay point (middle-left, lower)
     - Sustain point (middle-right, same height as decay)
     - Release point (right, at bottom)
   - Grid lines in the background

2. **Envelope line should be continuous** with no breaks

3. **When you click near a control point**, you should see debug output like:
   ```
   Mouse press at (123.45, 234.56)
   Mouse normalized: (0.12, 0.34)
     Point 0: (-0.85, 0.88) time=0.01 amp=1.0 dist=1.23
     Point 1: (-0.42, 0.45) time=0.31 amp=0.7 dist=0.03
     -> Selected point 1
   Dragging point: Some(1)
   ```

4. **If dragging doesn't work**, the debug output will help identify why:
   - If you don't see "Mouse press" messages → mouse events aren't being captured
   - If distances are all large → threshold might be too small or coordinate conversion is wrong
   - If "Dragging point: None" → point detection logic isn't working

## Understanding the Debug Output

When you click, you'll see output like:
```
Mouse press at (234.234375, 234.078125)
Mouse normalized: (-0.707207, 0.6098698)
  Point 0: (-0.7898219, 0.8) time=0.01 amp=1 dist=0.20730346
  Point 1: (-0.48447838, 0.32) time=0.31 amp=0.7 dist=0.36555785
  Point 2: (0.024427533, 0.32) time=0.81 amp=0.7 dist=0.7869648
  Point 3: (0.53333324, -0.8) time=1.31 amp=0 dist=1.8779438
  -> Selected point 0
Dragging point: Some(0)
```

This shows:
- **Screen coordinates**: Raw pixel position where you clicked
- **Normalized coordinates**: Converted to -1..1 range for OpenGL
- **Point info**: Each control point's position and distance from click
- **dist**: Distance in normalized coordinates (threshold is 0.15)

If a point is within 0.15 units of your click, it will be selected.

## Debugging Mouse Issues

If you can't drag the points, check the debug output:

### Scenario 1: No mouse events at all
```
(no output when clicking)
```
**Problem**: GLFW isn't receiving mouse events.
**Possible causes**:
- Window not in focus
- Mouse button polling not enabled (but it is in the code)

### Scenario 2: Mouse events but no point selected
```
Mouse press at (400.0, 300.0)
Mouse normalized: (0.0, 0.0)
  Point 0: (-0.85, 0.88) time=0.01 amp=1.0 dist=1.23
  Point 1: (-0.42, 0.45) time=0.31 amp=0.7 dist=0.89
  Point 2: (0.12, 0.45) time=0.81 amp=0.7 dist=0.56
  Point 3: (0.65, -0.88) time=1.31 amp=0.0 dist=1.78
  -> No point selected
```
**Problem**: All distances are > 0.05 (the threshold).
**Possible solutions**:
- Increase the threshold value
- Check if coordinate conversion is working correctly

### Scenario 3: Point selected but not moving
```
Mouse press at (400.0, 300.0)
Dragging point: Some(1)
(point doesn't move when you drag)
```
**Problem**: `CursorPos` events aren't being received while dragging.
**Possible cause**: Need to check if cursor position polling is enabled (it should be).

## Control Point Behavior

Each point has different constraints:

1. **Attack Point (0)**: Can move horizontally (time), amplitude stays at 1.0
2. **Decay Point (1)**: Can move both horizontally (time) and vertically (sustain level)
3. **Sustain Point (2)**: Can only move horizontally (duration), amplitude follows decay point
4. **Release Point (3)**: Can only move horizontally (time), amplitude stays at 0.0

## Next Steps

After testing, please report:
1. Does the envelope line look continuous now?
2. What debug output do you see when clicking?
3. Can you drag any of the points?
4. If not, share the debug output so we can diagnose the issue
