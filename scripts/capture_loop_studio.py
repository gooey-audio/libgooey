#!/usr/bin/env python3
"""Historical standalone Loop Studio capture recipe (pre-Omni-Lab layout).

Retained to explain the original evidence. Its coordinates/window selectors do
not target the current central shell; parent review owns new central captures.

Run the native release GUI with --demo on a 1440x1000 X display first.
Requires xdotool, ImageMagick, ffmpeg and an audio monitor (see --help).
This is coordinate-based POC evidence, not a portable GUI regression framework.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--display', default=':99')
    parser.add_argument('--monitor', default='gooey_studio.monitor')
    parser.add_argument('--output', type=Path, default=Path('/tmp/opencode/studio-evidence'))
    args = parser.parse_args()
    os.environ['DISPLAY'] = args.display
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    for name in ['recorded-session.json', 'recorded-mix.wav']:
        if (out/name).exists():
            raise FileExistsError(f'Use a fresh output directory: {out/name} already exists')
    events = []
    start = time.monotonic()

    def xd(*words):
        subprocess.run(['xdotool', *map(str, words)], check=True)

    def click(x, y):
        xd('mousemove', x, y)
        xd('click', '1')
        time.sleep(0.3)

    def drag(x, y, xx, yy):
        xd('mousemove', x, y)
        xd('mousedown', 1)
        for step in range(1, 31):
            xd('mousemove', round(x+(xx-x)*step/30), round(y+(yy-y)*step/30))
            time.sleep(0.04)
        xd('mouseup', 1)
        time.sleep(0.3)

    def text(x, y, value):
        click(x, y)
        xd('key', 'ctrl+a')
        xd('type', '--clearmodifiers', '--delay', '15', value)
        # Let the GUI consume the final text events before clicking a button.
        time.sleep(0.5)

    def stage(name, delay=1):
        time.sleep(delay)
        events.append({'seconds': round(time.monotonic()-start, 2), 'stage': name})
        subprocess.run(['import', '-window', 'root', str(out/(name+'.png'))], check=True)

    windows = subprocess.check_output(['xdotool', 'search', '--name', '^Gooey Loop Studio$'], text=True).split()
    if len(windows) != 1:
        raise RuntimeError(f'Expected exactly one Gooey Loop Studio window, got {windows}')
    xd('windowactivate', '--sync', windows[0])
    xd('windowmove', windows[0], 30, 38)
    xd('windowsize', windows[0], 1380, 940)
    time.sleep(1)
    log = open(out/'capture.log', 'w')
    capture = subprocess.Popen([
        'ffmpeg', '-hide_banner', '-loglevel', 'warning', '-y',
        '-f', 'x11grab', '-video_size', '1440x1000', '-framerate', '20', '-i', args.display,
        '-f', 'pulse', '-i', args.monitor,
        '-c:v', 'libx264', '-preset', 'veryfast', '-crf', '22', '-pix_fmt', 'yuv420p',
        '-c:a', 'aac', '-b:a', '160k', '-movflags', '+faststart', str(out/'loop-studio-interactions.mp4')
    ], stdin=subprocess.PIPE, stdout=log, stderr=log)
    try:
        stage('01-demo-ready')
        click(60, 73)  # Play
        stage('02-four-tracks-playing', 3)
        drag(359, 421, 332, 421)  # Bass gain
        drag(637, 443, 672, 443)  # Chord pan
        click(879, 379)  # Audio loop mute
        stage('03-track-mixing')
        click(879, 379)
        click(373, 379)  # Bass solo
        stage('04-bass-solo')
        click(373, 379)
        drag(604, 485, 646, 485)  # Chord delay
        drag(614, 506, 649, 506)  # Chord reverb
        drag(1143, 421, 1167, 421)  # Master reverb
        stage('05-track-and-master-effects', 2)
        click(215, 73)  # Record
        stage('06-record-armed', 3)  # Wait a genuine bar for chord arming
        xd('keydown', '1')
        time.sleep(0.65)
        xd('keyup', '1')
        xd('key', 'z')
        time.sleep(0.4)
        xd('keydown', '5')
        time.sleep(0.55)
        xd('keyup', '5')
        xd('key', 'b')
        drag(331, 421, 401, 421)  # Capture bass gain automation
        drag(401, 421, 345, 421)
        drag(681, 463, 605, 463)  # Capture chord filter automation
        drag(605, 463, 672, 463)
        drag(1180, 379, 1152, 379)  # Capture master gain automation
        stage('07-performance-and-automation-recording')
        click(215, 73)  # Stop recording, not transport
        stage('08-recorded-automation-replay', 5)
        click(60, 73)  # Stop
        stage('09-stopped-recorded-song')
        text(210, 928, str(out/'recorded-session.json'))
        click(377, 928)
        stage('10-session-saved', 3)
        text(200, 950, str(out/'recorded-mix.wav'))
        click(447, 950)
        stage('11-final-mixdown', 9)
        click(419, 928)  # Load saved session
        stage('12-saved-session-reloaded', 3)
        click(122, 73)  # Rewind
        click(60, 73)
        stage('13-reloaded-performance-playing', 5)
        click(60, 73)
    finally:
        if capture.stdin:
            capture.stdin.write(b'q\n')
            capture.stdin.flush()
        capture.wait(timeout=30)
        log.close()
        (out/'interaction-timeline.json').write_text(json.dumps(events, indent=2)+'\n')
    if capture.returncode:
        raise RuntimeError(f'Capture failed: inspect {out / "capture.log"}')
    saved = json.loads((out/'recorded-session.json').read_text())
    if not saved['automation'] or not saved['hits'] or not saved['chords']:
        raise RuntimeError('GUI did not save recorded automation, hits and chords')
    if (out/'recorded-mix.wav').stat().st_size < 48000*2*4:
        raise RuntimeError('GUI did not produce a substantial final mixdown')
    print(f'Actual GUI/audio capture saved under {out}')


if __name__ == '__main__':
    main()
