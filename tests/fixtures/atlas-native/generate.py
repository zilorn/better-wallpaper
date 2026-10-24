#!/usr/bin/env python3
"""Reproduce owned, constant-frame native atlas probes; never generate captures."""
import argparse
import importlib.util
import json
from pathlib import Path
import struct
import sys
sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('behavior', ROOT.parent / 'scene-behavior/generate.py')
behavior = importlib.util.module_from_spec(spec)
spec.loader.exec_module(behavior)


def unpack(data):
    offset = 0

    def word():
        nonlocal offset
        value = struct.unpack_from('<I', data, offset)[0]
        offset += 4
        return value

    def string():
        nonlocal offset
        size = word()
        value = data[offset:offset + size].decode()
        offset += size
        return value

    assert string() == 'PKGV0001'
    rows = [(string(), word(), word()) for _ in range(word())]
    return {name: data[offset + start:offset + start + size] for name, start, size in rows}


def generate_base():
    for name in ['multi-image', 'rotated', 'skewed', 'flipped-x', 'flipped-y']:
        original = unpack((ROOT.parent / 'scene-behavior' / name / 'scene.pkg').read_bytes())
        for frame in [-1, 0, 1]:
            sample = name + ('-animated' if frame == -1 else str(frame))
            entries = dict(original)
            for key, value in original.items():
                if key.endswith('.tex') and frame != -1:
                    start = value.index(b'TEXS0002\0') + 9
                    entries[key] = value[:start] + behavior.words(1) + value[start + 4 + frame*32:start + 4 + (frame+1)*32]
                elif key.startswith('materials/') and key.endswith('.json'):
                    value = json.loads(value)
                    value['passes'][0]['combos'] = {'SPRITESHEET': 1}
                    entries[key] = behavior.encoded(value)
                elif key.startswith('models/'):
                    value = json.loads(value)
                    value['nopadding'] = True
                    entries[key] = behavior.encoded(value)
                elif key == 'scene.json':
                    value = json.loads(value)
                    value['camera'] = {'center': '0 0 -1', 'eye': '0 0 0', 'up': '0 1 0'}
                    value['general'].update(zoom=1, cameraparallax=False, camerashake=False, camerafade=False, bloom=False)
                    entries[key] = behavior.encoded(value)
            yield f'{sample}/scene.pkg', behavior.package(entries)
            yield f'{sample}/project.json', behavior.encoded({'file': 'scene.json', 'type': 'scene', 'title': sample})



def generate():
    yield from generate_base()
    # Same four colors with native clamp/point flags, and a packed-frame probe
    # with deliberately authored colors in each complete mip level.
    base = dict(generate_base())
    for sample, flags in [('clamped', 6), ('nearest', 7), ('packed-mips', 6), ('packed-point', 7)]:
        entries = unpack(base['rotated0/scene.pkg'])
        if sample.startswith('packed'):
            tex = bytearray(b'TEXV0005\0TEXI0001\0' + behavior.words(0, flags, 64, 32, 64, 32, 0) + b'TEXB0002\0' + behavior.words(1, 7))
            width, height = 64, 32
            for level in range(7):
                a, b = ([240, 10, 20, 255], [20, 220, 30, 255]) if level == 0 else ([30, 40, 200, 255], [180, 130, 10, 255])
                rgba = bytes(sum((a if x < width//2 else b for y in range(height) for x in range(width)), []))
                tex += behavior.words(width, height, 0, len(rgba), len(rgba)) + rgba
                width, height = max(1, width//2), max(1, height//2)
            tex += b'TEXS0002\0' + behavior.words(1, 0) + struct.pack('<7f', .5, 0, 0, 32, 0, 0, 32)
            scene = json.loads(entries['scene.json'])
            scene['objects'][0]['size'] = '4 4'
            entries['scene.json'] = behavior.encoded(scene)
        else:
            tex = bytearray(entries['materials/atlas.tex'])
            tex[22:26] = behavior.words(flags)
        entries['materials/atlas.tex'] = bytes(tex)
        yield f'{sample}/scene.pkg', behavior.package(entries)
        yield f'{sample}/project.json', behavior.encoded({'file': 'scene.json', 'type': 'scene', 'title': sample})


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    for name, data in generate():
        path = ROOT / name
        if args.write:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        elif path.read_bytes() != data:
            raise SystemExit(f'Fixture differs: {path}')
    print('Verified or wrote 38 native probe inputs; captures unchanged')
