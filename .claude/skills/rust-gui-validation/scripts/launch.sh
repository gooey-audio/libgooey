#!/bin/bash
# Start (or restart) a GUI binary on a virtual display.
#
#   launch.sh <binary> [display] [WxH]
#
# Starts Xvfb on the display if it is not running, kills any previous copy of
# the binary (by exact process name, so this script never kills itself),
# launches it with software OpenGL and waits for the first frames.
# The app's stdout/stderr go to /tmp/<binary-name>.log.
set -uo pipefail
binary=${1:?usage: launch.sh <binary> [display] [WxH]}
display=${2:-:99}
size=${3:-1600x1000}
name=$(basename "$binary")

display_up() {
    DISPLAY=$display python3 -c 'from Xlib import display; display.Display()' 2>/dev/null
}

if ! display_up; then
    # Xvfb needs this socket directory; it disappears when /tmp is cleaned.
    if [ ! -d /tmp/.X11-unix ]; then
        sudo mkdir -p /tmp/.X11-unix && sudo chmod 1777 /tmp/.X11-unix
    fi
    Xvfb "$display" -screen 0 "${size}x24" >/tmp/xvfb.log 2>&1 &
    for _ in $(seq 20); do display_up && break; sleep 0.25; done
    display_up || { echo "Xvfb failed to start; see /tmp/xvfb.log" >&2; exit 1; }
fi
pkill -x "$name" 2>/dev/null
sleep 0.5
DISPLAY=$display LIBGL_ALWAYS_SOFTWARE=1 nohup "$binary" >"/tmp/$name.log" 2>&1 &
sleep "${LAUNCH_WAIT:-4}"
if ! pgrep -x "$name" >/dev/null; then
    echo "$name exited; log follows" >&2
    cat "/tmp/$name.log" >&2
    exit 1
fi
echo "$name running on $display (log: /tmp/$name.log)"
