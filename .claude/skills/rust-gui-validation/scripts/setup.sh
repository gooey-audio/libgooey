#!/bin/bash
# One-time setup for headless GUI validation on an Amazon Linux / Fedora VM.
# Installs a virtual X server, the X11 runtime libraries winit/eframe dlopen,
# ImageMagick for screenshots and python-xlib for synthetic input.
set -euo pipefail
sudo dnf install -y xorg-x11-server-Xvfb mesa-dri-drivers mesa-libGL ImageMagick \
    libxkbcommon-x11 libXcursor libXrandr libXi alsa-lib >/tmp/gui-setup.log 2>&1 \
    || { tail -20 /tmp/gui-setup.log; exit 1; }
python3 -c 'import Xlib' 2>/dev/null || pip3 install --user python-xlib >/dev/null
echo "GUI validation tools ready"
