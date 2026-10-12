#!/usr/bin/env python3
"""Drive a window on an X display with synthetic input (XTest) and take screenshots.

Usage (each argument is one command, run in order):

    DISPLAY=:99 python3 xdrive.py 'focus Plate Explorer' 'click 740 400' 'key 1' \
        'drag 560 330 920 430 shift' 'sleep 0.5' 'shot out.png'

Commands (coordinates are root-window pixels; without a window manager the
app's window sits at 0,0 with no decorations, so they equal window pixels):

    focus <title words>     give keyboard focus to the window whose title contains the words
                            (required before modifier keys such as shift reach winit)
    move X Y                move the pointer
    click X Y | rclick X Y | dclick X Y
    drag X1 Y1 X2 Y2 [shift]  press, move in 30 steps, release (optionally holding shift)
    scroll up|down N        wheel clicks at the current pointer position
    key NAME                press and release a key (X keysym name: space, 1, Return, Escape, a)
    sleep SECONDS
    shot FILE.png           screenshot of the whole display (ImageMagick `import`)
"""
import subprocess
import sys
import time

from Xlib import X, XK, display
from Xlib.ext import xtest

d = display.Display()


def move(x, y):
    xtest.fake_input(d, X.MotionNotify, x=int(float(x)), y=int(float(y)))
    d.sync()


def button(number, down):
    xtest.fake_input(d, X.ButtonPress if down else X.ButtonRelease, number)
    d.sync()


def key(name, down):
    code = d.keysym_to_keycode(XK.string_to_keysym(name))
    if code == 0:
        sys.exit(f"unknown keysym {name!r}")
    xtest.fake_input(d, X.KeyPress if down else X.KeyRelease, code)
    d.sync()


def find_window(window, needle):
    name = window.get_wm_name()
    if name and needle in str(name):
        return window
    for child in window.query_tree().children:
        found = find_window(child, needle)
        if found:
            return found
    return None


for command in sys.argv[1:]:
    parts = command.split()
    op, args = parts[0], parts[1:]
    if op == "focus":
        window = find_window(d.screen().root, " ".join(args))
        if window is None:
            sys.exit(f"no window titled {' '.join(args)!r}")
        window.set_input_focus(X.RevertToParent, X.CurrentTime)
        d.sync()
        time.sleep(0.2)
    elif op == "move":
        move(*args[:2])
        time.sleep(0.05)
    elif op in ("click", "rclick", "dclick"):
        move(*args[:2])
        time.sleep(0.08)
        number = 3 if op == "rclick" else 1
        for _ in range(2 if op == "dclick" else 1):
            button(number, True)
            time.sleep(0.04)
            button(number, False)
            time.sleep(0.06)
    elif op == "drag":
        x1, y1, x2, y2 = map(float, args[:4])
        shift = "shift" in args[4:]
        if shift:
            key("Shift_L", True)
            time.sleep(0.3)
        move(x1, y1)
        time.sleep(0.08)
        button(1, True)
        time.sleep(0.05)
        for i in range(1, 31):
            move(x1 + (x2 - x1) * i / 30, y1 + (y2 - y1) * i / 30)
            time.sleep(0.03)
        button(1, False)
        time.sleep(0.05)
        if shift:
            key("Shift_L", False)
    elif op == "scroll":
        number = 4 if args[0] == "up" else 5
        for _ in range(int(args[1])):
            button(number, True)
            button(number, False)
            time.sleep(0.03)
    elif op == "key":
        key(args[0], True)
        time.sleep(0.03)
        key(args[0], False)
        time.sleep(0.05)
    elif op == "sleep":
        time.sleep(float(args[0]))
    elif op == "shot":
        subprocess.run(["import", "-window", "root", args[0]], check=True)
    else:
        sys.exit(f"unknown command {op!r}")
