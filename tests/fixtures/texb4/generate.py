#!/usr/bin/env python3
"""Generate owned TEXB4 fixtures from the verified native layout; no native captures."""
import argparse
import hashlib
import json
from pathlib import Path
import struct

ROOT = Path(__file__).resolve().parent

def words(*values):
    return struct.pack('<' + 'I' * len(values), *values)

def texture(payload, video=False, records=()):
    size = 16 if video else 1
    data = b'TEXV0005\0TEXI0001\0' + words(0, 34 if video else 2, size, size, size, size, 0)
    data += b'TEXB0004\0' + words(1, 0xffffffff, len(records))
    for index, text in enumerate(records):
        data += words(index, 100 + index, 200 + index) + text.encode() + b'\0'
    data += words(1, size, size, 0, len(payload), len(payload)) + payload
    if records:
        data += words(0) # No conditional mip variants.
    return data

def generate():
    media = (ROOT.parent / 'scene-video.mp4').read_bytes()
    records = ['{"label":"owned metadata"}', '{"label":"第二条记录"}']
    files = {
        'video.tex': texture(media, True),
        'video-metadata.tex': texture(media, True, records),
        'static-metadata.tex': texture(bytes([23, 67, 119, 255]), records=records),
    }
    manifest = {
        'provenance': 'owned generated inputs; native reader layout, no original-program captures',
        'source_media_sha256': hashlib.sha256(media).hexdigest(),
        'inputs': {'scene_times_seconds': [0, .499, .5, .999, 1, 1.499, 1.5, 1.999, 2, 4.5],
                   'pointer': None, 'properties': {}, 'random_seed': None},
        'sha256': {name: hashlib.sha256(data).hexdigest() for name, data in files.items()},
    }
    files['manifest.json'] = (json.dumps(manifest, indent=2, ensure_ascii=False) + '\n').encode()
    return files

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    for name, data in generate().items():
        path = ROOT / name
        if args.write:
            path.write_bytes(data)
        elif not path.is_file() or path.read_bytes() != data:
            raise SystemExit(f'Fixture differs: {path}')
    print('Wrote fixtures' if args.write else 'Verified fixtures')
