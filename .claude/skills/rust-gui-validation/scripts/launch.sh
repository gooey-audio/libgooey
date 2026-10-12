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

if ! pgrep -f "Xvfb $display" >/dev/null; then
    Xvfb "$display" -screen 0 "${size}x24" >/tmp/xvfb.log 2>&1 &
    sleep 1
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
