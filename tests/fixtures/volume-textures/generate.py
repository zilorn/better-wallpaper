#!/usr/bin/env python3
"""Generate owned volume probes using only Python's standard library."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import zlib


def words(*values):
    return struct.pack('<' + 'I' * len(values), *values)


def chunk(name, payload):
    return struct.pack('>I', len(payload)) + name + payload + struct.pack('>I', zlib.crc32(name + payload))


def png(width, height, pixels):
    rows = b''.join(b'\0' + pixels[y * width * 4:(y + 1) * width * 4] for y in range(height))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(rows, 9)) + chunk(b'IEND', b''))


def header(width, height, depth, version, levels, codec=None):
    data = b'TEXV0005\0TEXI0001\0' + words(0, 0x42, width, height, width * depth, height, depth, 0xFF123456)
    data += f'TEXB{version:04d}\0'.encode() + words(1)
    if version >= 3:
        data += words(codec)
    if version == 4:
        data += words(0)
    return data + words(levels)


def generate(output):
    output.mkdir(parents=True, exist_ok=True)
    pixels = bytes(channel for z in range(4) for y in range(4) for x in range(4)
                   for channel in (x * 85, y * 85, z * 85, 255))
    encoded = png(4, 16, pixels)
    neutral = header(4, 4, 4, 4, 1, 13) + words(4, 4, 4, 0, 0, len(encoded)) + encoded
    raw = header(5, 3, 2, 1, 3)
    for level, (width, height, depth) in enumerate(((5, 3, 2), (2, 1, 1), (1, 1, 1))):
        # Distinct byte channels expose row/slice swaps without asserting a raw
        # ARGB8888-to-RGBA conversion. Raw storage is intentionally preserved.
        payload = bytes(channel for z in range(depth) for y in range(height) for x in range(width)
                        for channel in (10 + x, 40 + y, 70 + z, 200 + level))
        raw += words(width, height, depth, len(payload)) + payload
    records = []
    for name, data in [('neutral-png.tex', neutral), ('noncubic-raw.tex', raw)]:
        (output / name).write_bytes(data)
        records.append({'path': name, 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()})
    (output / 'manifest.json').write_text(json.dumps({'schema_version': 1, 'assets': records}, indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=Path(__file__).parent)
    generate(parser.parse_args().output)
