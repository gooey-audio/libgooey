#!/usr/bin/env python3
"""Independently inspect a studio PCM/float WAV export without Rust/hound."""
import argparse
import json
import math
import struct


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('file')
    parser.add_argument('--minimum-seconds', type=float, default=1)
    args = parser.parse_args()
    with open(args.file, 'rb') as file:
        assert file.read(4) == b'RIFF', 'expected RIFF WAV'
        file.read(4)
        assert file.read(4) == b'WAVE'
        fmt = None
        payload = None
        while header := file.read(8):
            kind, length = struct.unpack('<4sI', header)
            if kind == b'fmt ':
                fmt = file.read(length)
            elif kind == b'data':
                payload = (file.tell(), length)
                file.seek(length, 1)
            else:
                file.seek(length, 1)
            if length % 2:
                file.seek(1, 1)
        assert fmt and payload, 'missing format or audio data'
        encoding, channels, rate, _, alignment, bits = struct.unpack('<HHIIHH', fmt[:16])
        if encoding == 65534:
            encoding = struct.unpack('<H', fmt[24:26])[0]
        assert channels == 2, 'mixdown must be stereo'
        assert rate in (22050, 44100, 48000, 96000)
        assert encoding in (1, 3), f'unsupported WAV encoding {encoding}'
        assert bits in ((32, 64) if encoding == 3 else (8, 16, 24, 32))
        assert alignment == channels * bits // 8
        offset, length = payload
        assert length % alignment == 0
        frames = length // alignment
        assert frames / rate >= args.minimum_seconds
        file.seek(offset)
        width = bits // 8
        peak = [0.0, 0.0]
        squares = [0.0, 0.0]
        nonzero = 0
        consumed = 0
        remaining = length
        while remaining:
            block = file.read(min(remaining, alignment * 8192))
            assert block and len(block) % alignment == 0, 'truncated audio'
            remaining -= len(block)
            for index in range(0, len(block), width):
                raw = block[index:index+width]
                if encoding == 3:
                    value = struct.unpack('<f' if bits == 32 else '<d', raw)[0]
                elif bits == 8:
                    value = (raw[0]-128)/128
                else:
                    value = int.from_bytes(raw, 'little', signed=True) / (2**(bits-1))
                assert math.isfinite(value), f'nonfinite sample {consumed}'
                channel = consumed % channels
                peak[channel] = max(peak[channel], abs(value))
                squares[channel] += value * value
                nonzero += abs(value) > 1e-7
                consumed += 1
        assert nonzero, 'silent mixdown'
        assert max(peak) <= 1.00001, f'over-full-scale mixdown {peak}'
        print(json.dumps(dict(file=args.file, encoding=encoding, channels=channels,
                             sample_rate=rate, frames=frames, seconds=frames/rate,
                             peak=peak, rms=[math.sqrt(s/frames) for s in squares],
                             nonzero_samples=nonzero), indent=2))


if __name__ == '__main__':
    main()
