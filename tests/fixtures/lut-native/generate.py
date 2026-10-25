#!/usr/bin/env python3
"""Owned LUT probes. This generator never creates original-program captures."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import sys
sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('behavior', ROOT.parent / 'scene-behavior/generate.py')
b = importlib.util.module_from_spec(spec)
spec.loader.exec_module(b)
spec = importlib.util.spec_from_file_location('volume', ROOT.parent / 'volume-textures/generate.py')
v = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v)


def generate():
    values = [0, 1, 4, 8, 16, 31, 63, 95, 127, 128, 159, 191, 223, 239, 254, 255]
    pixels = bytes(c for y in range(128) for x in range(128)
                   for c in (values[x // 8], values[y // 8], values[(x // 8 + y // 8 * 3) % 16], 255))
    source = bytearray(b.texture(128, 128, 128, pixels))
    source[22:26] = b.words(3)  # clamp, nearest; exact input bytes at every pixel
    for name, kind, amount, raw, flags in [
        ('bypass', 'neutral', 0, False, 0x42),
        ('neutral', 'neutral', 1, False, 0x42),
        ('invert', 'invert', 1, False, 0x42),
        ('mixed', 'nonlinear', .375, False, 0x42),
        ('nonlinear', 'nonlinear', 1, False, 0x42),
        ('raw', 'invert', 1, True, 0x42),
        ('nearest', 'nonlinear', 1, False, 0x43),
        ('repeat', 'nonlinear', 1, False, 0x40),
        ('alpha', 'invert', 1, False, 0x42),
        ('noncubic', 'nonlinear', 1, False, 0x42),
    ]:
        w, h, d = (4, 3, 2) if name == 'noncubic' else (32, 32, 32)
        rgba = bytes(c for z in range(d) for y in range(h) for x in range(w)
                     for c in ((round(255*x/(w-1)), round(255*y/(h-1)), round(255*z/(d-1)), 255)
                               if kind == 'neutral' else
                               (255-round(255*z/(d-1)), round(255*x/(w-1)), 255-round(255*y/(h-1)), 255)
                               if kind == 'invert' else
                               (round(255*(x/(w-1))*(y/(h-1))), round(255*(z/(d-1))**2), round(255*(1-x/(w-1))*(z/(d-1))), 255)))
        payload = rgba if raw else v.png(w, h*d, rgba)
        tex = bytearray(v.header(w, h, d, 1 if raw else 4, 1, None if raw else 13))
        tex[22:26] = b.words(flags)
        tex += b.words(w, h, d, len(payload)) if raw else b.words(w, h, d, 0, 0, len(payload))
        tex += payload
        scene = {'camera': {'center': '0 0 -1', 'eye': '0 0 0', 'up': '0 1 0'},
                 'general': {'orthogonalprojection': {'width': 128, 'height': 128}, 'zoom': 1,
                             'cameraparallax': False, 'camerashake': False, 'camerafade': False, 'bloom': False},
                 'objects': [{'id': 1, 'image': 'models/probe.json', 'origin': '64 64 0', 'size': '128 128',
                              'effects': [{'file': 'effects/probe-lut/effect.json', 'passes': [{'constantshadervalues': {'lutparams': amount}, 'textures': [None, 'probe-lut']}]}]}]}
        material = {'passes': [{'shader': 'ccsimple', 'blending': 'opaque',
                               'combos': {'LUT': 1, 'COL': 0, 'HDR': 0},
                               'constantshadervalues': {'lutparams': amount},
                               'textures': ['_rt_FullFrameBuffer', 'probe-lut']}]}
        sample_source = bytearray(source)
        if name == 'alpha':
            for index in range(len(sample_source)-len(pixels)+3, len(sample_source), 4):
                sample_source[index] = 128
        entries = {'scene.json': b.encoded(scene),
                   'models/probe.json': b.encoded({'material': 'materials/probe.json', 'nopadding': True}),
                   'materials/probe.json': b.encoded({'passes': [{'shader': 'genericimage4', 'blending': 'translucent' if name == 'alpha' else 'opaque', 'textures': ['source']}]}),
                   'effects/probe-lut/effect.json': b.encoded({'version': 2, 'passes': [{'material': 'materials/effects/probe-lut.json'}]}),
                   'materials/effects/probe-lut.json': b.encoded(material),
                   'materials/source.tex': bytes(sample_source), 'materials/probe-lut.tex': bytes(tex),
                   'shaders/ccsimple.vert': b'attribute vec3 a_Position;\nuniform mat4 g_ModelViewProjectionMatrix;\nattribute vec2 a_TexCoord;\nvarying vec2 v_TexCoord;\nvoid main() { gl_Position = mul(vec4(a_Position, 1.0), g_ModelViewProjectionMatrix); v_TexCoord = a_TexCoord; }\n'}
        if name == 'bypass':
            control = dict(entries)
            control_scene = json.loads(control['scene.json'])
            control_scene['objects'][0].pop('effects')
            control['scene.json'] = b.encoded(control_scene)
            yield 'baseline/scene.pkg', b.package(control)
            yield 'baseline/project.json', b.encoded({'file': 'scene.json', 'type': 'scene', 'title': 'A2 ungraded control'})
        yield f'{name}/scene.pkg', b.package(entries)
        yield f'{name}/project.json', b.encoded({'file': 'scene.json', 'type': 'scene', 'title': f'A2 {name}'})


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
            raise SystemExit(f'Fixture differs: {name}')
    print('Verified or wrote LUT probe inputs; captures unchanged')
