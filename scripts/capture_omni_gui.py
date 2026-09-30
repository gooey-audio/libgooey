#!/usr/bin/env python3
"""Capture actual Omni Lab X11 interactions (1600x1100, default window).

Start the native release omni_gui first. Requires xdotool, ImageMagick,
ffmpeg and a PulseAudio-compatible sink monitor. Coordinate-based evidence,
not a portable GUI regression framework.
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
    parser.add_argument('--sink', default='gooey_omni')
    parser.add_argument('--output', type=Path, default=Path('/tmp/opencode/omni-evidence'))
    args = parser.parse_args()
    os.environ['DISPLAY'] = args.display
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)

    def xd(*words):
        subprocess.run(['xdotool', *map(str, words)], check=True)

    def route():
        inputs = json.loads(subprocess.check_output(['pactl', '-f', 'json', 'list', 'sink-inputs']))
        matches = [i for i in inputs if '[omni_gui]' in i['properties'].get('application.name', '')]
        if len(matches) != 1:
            raise RuntimeError(f'Expected one native Omni stream, found {len(matches)}')
        subprocess.run(['pactl', 'move-sink-input', str(matches[0]['index']), args.sink], check=True)

    def click(x, y):
        xd('mousemove', x, y)
        xd('click', 1)
        time.sleep(.4)

    def panel(x):
        click(x, 70)
        route()

    def drag(x, y, xx):
        xd('mousemove', x, y)
        xd('mousedown', 1)
        for n in range(1, 21):
            xd('mousemove', round(x+(xx-x)*n/20), y)
            time.sleep(.04)
        xd('mouseup', 1)

    events = []
    start = time.monotonic()

    def shot(name):
        events.append({'seconds': round(time.monotonic()-start, 2), 'stage': name})
        subprocess.run(['import', '-window', 'root', str(out/(name+'.png'))], check=True)

    ids = subprocess.check_output(['xdotool', 'search', '--name', '^Gooey Omni Lab$'], text=True).split()
    if len(ids) != 1:
        raise RuntimeError('Start exactly one native Gooey Omni Lab')
    xd('windowactivate', '--sync', ids[0])
    xd('windowmove', ids[0], 80, 38)
    panel(254)
    log = open(out/'capture.log', 'w')
    video = subprocess.Popen([
        'ffmpeg', '-hide_banner', '-loglevel', 'warning', '-y',
        '-f', 'x11grab', '-video_size', '1600x1100', '-framerate', '15', '-i', args.display,
        '-f', 'pulse', '-i', args.sink+'.monitor', '-c:v', 'libx264', '-preset', 'veryfast',
        '-crf', '23', '-pix_fmt', 'yuv420p', '-c:a', 'aac', '-b:a', '160k',
        '-movflags', '+faststart', str(out/'interactions.mp4')
    ], stdin=subprocess.PIPE, stdout=log, stderr=log)
    try:
        click(116, 166)
        xd('keydown', 'space')
        time.sleep(1)
        drag(151, 187, 124)
        shot('01-poly-waveform')
        xd('keyup', 'space')
        click(511, 166)
        drag(152, 213, 180)
        xd('keydown', 'z')
        time.sleep(1)
        shot('02-poly-modulation')
        panel(328)  # Deliberately switch while synth key held.
        xd('keyup', 'z')
        xd('key', 'space')
        click(791, 508)
        xd('key', 'space')
        shot('03-resonator-inspector')
        click(590, 741)
        shot('04-resonator-routing')
        click(460, 96)
        click(738, 315)
        click(308, 127)
        time.sleep(3)
        shot('05-resonator-sequencer')
        panel(405)  # Stop previous active sequencer on panel switch.
        click(95, 528)
        drag(138, 183, 162)
        click(851, 166)
        click(834, 213)  # Select Delay explicitly in its popup.
        time.sleep(2)
        shot('06-experiment-effects')
        click(812, 337)
        drag(829, 378, 856)
        time.sleep(2)
        shot('07-experiment-lfo')
        # Repeat active output switches; each route() asserts one stream only.
        for _ in range(8):
            for x in (254, 328, 405):
                panel(x)
        click(488, 70)
        shot('08-host-stopped')
        click(488, 70)
        route()
        shot('09-host-resumed')
    finally:
        xd('keyup', 'space', 'z')
        video.stdin.write(b'q\n')
        video.stdin.flush()
        video.wait(timeout=30)
        log.close()
        (out/'timeline.json').write_text(json.dumps(events, indent=2)+'\n')
    if video.returncode:
        raise RuntimeError('ffmpeg failed; see capture.log')
    print(f'Actual GUI/audio evidence: {out}')


if __name__ == '__main__':
    main()
