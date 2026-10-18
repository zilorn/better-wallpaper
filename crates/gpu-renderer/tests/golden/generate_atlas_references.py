#!/usr/bin/env python3
"""Analytical A1 references: bilinear interpolation of four authored colors.

No renderer output or native screenshots are read. Pixel centers are (x+.5)/32.
The identity/quarter-turn/flip equations are explicit permutations of those
coordinates. The skew probe samples a line after its one-texel horizontal axis
is inset to its center. This specifies our sampling convention, not native parity.
Default checks existing bytes; --write writes only the seven A1 PNGs.
"""
import argparse
import math
from pathlib import Path
import struct
import zlib

ROOT = Path(__file__).parent
COLORS = [(240, 10, 20, 255), (20, 220, 30, 255),
          (30, 40, 200, 255), (180, 130, 10, 255)]


def png(rgba):
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    rows = b''.join(b'\0' + bytes(rgba[y*128:(y+1)*128]) for y in range(32))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 32, 32, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b''))


def references():
    for name, color in [('atlas-image0', [200, 40, 10, 255]), ('atlas-image1', [20, 180, 60, 255])]:
        yield name, png(color * 1024)
    for name in ['identity', 'rotated', 'flipped-x', 'flipped-y', 'skewed']:
        pixels = []
        for y in range(32):
            for x in range(32):
                u, v = (x + .5) / 32, (y + .5) / 32
                if name == 'rotated':
                    u, v = 1-v, u
                elif name == 'flipped-x':
                    u = 1-u
                elif name == 'flipped-y':
                    v = 1-v
                elif name == 'skewed':
                    u, v = .5/math.sqrt(5) + v*(1-1/math.sqrt(5)), 1/math.sqrt(5)-.5 + v*(2-2/math.sqrt(5))
                u, v = max(0, min(1, u)), max(0, min(1, v))
                weights = [(1-u)*(1-v), u*(1-v), (1-u)*v, u*v]
                pixels.extend(round(sum(weight*color[c] for weight, color in zip(weights, COLORS))) for c in range(4))
        yield f'atlas-{name}', png(pixels)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    for name, data in references():
        path = ROOT / f'{name}.png'
        if args.write:
            path.write_bytes(data)
        elif path.read_bytes() != data:
            raise SystemExit(f'Analytical reference differs: {path}')
    print('Verified or wrote seven analytical A1 references')
