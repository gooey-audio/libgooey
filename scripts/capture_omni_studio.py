#!/usr/bin/env python3
"""Actual central-app studio capture; 1600x1100 X11, initial demo layout."""
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
    parser.add_argument('--output', type=Path, default=Path('/tmp/opencode/omni-studio-evidence'))
    args = parser.parse_args()
    os.environ['DISPLAY'] = args.display
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    for name in ['song.json', 'mix.wav']:
        if (out/name).exists():
            raise FileExistsError('Use a fresh evidence directory')

    def xd(*args):
        subprocess.run(['xdotool', *map(str, args)], check=True)

    def click(x, y):
        xd('mousemove', x, y)
        xd('click', 1)
        time.sleep(.4)

    def route():
        inputs = json.loads(subprocess.check_output(['pactl', '-f', 'json', 'list', 'sink-inputs']))
        streams = [i for i in inputs if '[omni_gui]' in i['properties'].get('application.name', '')]
        assert len(streams) == 1, f'Expected one shared stream, got {len(streams)}'
        subprocess.run(['pactl', 'move-sink-input', str(streams[0]['index']), args.sink], check=True)

    def drag(x, y, xx):
        xd('mousemove', x, y)
        xd('mousedown', 1)
        for n in range(1, 31):
            xd('mousemove', round(x+(xx-x)*n/30), y)
            time.sleep(.04)
        xd('mouseup', 1)
        time.sleep(.3)

    def text(x, y, value):
        click(x, y)
        xd('key', 'ctrl+a')
        xd('type', '--clearmodifiers', '--delay', 15, value)
        time.sleep(.5)

    events = []
    start = time.monotonic()

    def shot(name, delay=1):
        time.sleep(delay)
        subprocess.run(['import', '-window', 'root', str(out/(name+'.png'))], check=True)
        events.append({'seconds': round(time.monotonic()-start, 2), 'stage': name})

    ids = subprocess.check_output(['xdotool', 'search', '--name', '^Gooey Omni Lab$'], text=True).split()
    assert len(ids) == 1
    xd('windowactivate', '--sync', ids[0])
    route()
    log = open(out/'capture.log', 'w')
    video = subprocess.Popen([
        'ffmpeg', '-hide_banner', '-loglevel', 'warning', '-y',
        '-f', 'x11grab', '-video_size', '1600x1100', '-framerate', '15', '-i', args.display,
        '-f', 'pulse', '-i', args.sink+'.monitor', '-c:v', 'libx264', '-preset', 'veryfast',
        '-crf', '23', '-pix_fmt', 'yuv420p', '-c:a', 'aac', '-b:a', '160k',
        '-movflags', '+faststart', str(out/'interactions.mp4')
    ], stdin=subprocess.PIPE, stdout=log, stderr=log)
    try:
        click(112, 118)
        shot('01-shared-studio-playing', 3)
        drag(421, 466, 401)
        drag(711, 487, 748)
        click(962, 423)
        shot('02-track-mixing')
        click(962, 423)
        click(438, 423)
        shot('03-bass-solo')
        click(438, 423)
        drag(679, 529, 710)
        drag(687, 550, 720)
        drag(1240, 466, 1270)
        shot('04-shared-track-master-effects')
        click(226, 118)
        time.sleep(3)
        xd('keydown', '1')
        time.sleep(.65)
        xd('keyup', '1')
        xd('key', 'z')
        xd('keydown', '5')
        time.sleep(.6)
        xd('keyup', '5')
        xd('key', 'b')
        drag(401, 466, 465)
        drag(465, 466, 415)
        drag(753, 508, 683)
        drag(683, 508, 740)
        drag(1278, 423, 1254)
        shot('05-captured-performance')
        click(226, 118)
        xd('mousemove', 1150, 745)
        xd('click', '--repeat', 4, '--delay', 80, 5)  # Reveal automation below fold.
        shot('06-automation-shared-diagnostics', 5)
        # Return editor to top for repeatable positions.
        xd('click', '--repeat', 15, '--delay', 40, 4)
        click(226, 118)
        xd('keydown', '3')
        time.sleep(.5)
        click(254, 70)  # Switch with active recording and held chord.
        route()
        xd('keyup', '3')
        shot('07-switch-away-finalized')
        click(490, 70)
        route()
        shot('08-return-song-stopped')
        text(240, 828, str(out/'song.json'))
        click(427, 828)
        shot('09-shared-session-saved', 3)
        text(220, 850, str(out/'mix.wav'))
        click(498, 850)
        shot('10-shared-mixdown', 10)
        click(470, 828)
        shot('11-reloaded-shared-session', 3)
        click(111, 118)
        shot('12-reloaded-performance', 4)
        click(111, 118)
    finally:
        xd('keyup', '1', '3', '5')
        video.stdin.write(b'q\n')
        video.stdin.flush()
        video.wait(timeout=30)
        log.close()
        (out/'timeline.json').write_text(json.dumps(events, indent=2)+'\n')
    assert video.returncode == 0
    song = json.loads((out/'song.json').read_text())
    assert song['chords'] and song['hits'] and len(song['automation']) >= 3
    assert (out/'mix.wav').stat().st_size > 48000*2*4
    print(f'Actual consolidated GUI/audio + persisted song/mixdown verified at {out}')


if __name__ == '__main__':
    main()
