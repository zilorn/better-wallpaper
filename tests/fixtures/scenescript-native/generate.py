#!/usr/bin/env python3
"""Owned A7 scripts. Generate inputs only; never manufacture native captures."""
import argparse
import copy
import importlib.util
from pathlib import Path
import sys
sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('motion', ROOT.parent / 'motion-native/generate.py')
motion = importlib.util.module_from_spec(spec)
spec.loader.exec_module(motion)
b = motion.b


def generate():
    base = {'camera': {'center': '0 0 -1', 'eye': '0 0 0', 'up': '0 1 0'},
            'general': {'orthogonalprojection': {'width': 128, 'height': 128}, 'zoom': 1,
                        'cameraparallax': False, 'camerashake': False, 'bloom': False,
                        'camerafade': False, 'clearcolor': '0 0 0'},
            'objects': [{'id': 1, 'name': 'Probe', 'image': 'models/probe.json',
                         'origin': '64 64 0', 'scale': '1 1 1', 'size': '16 16', 'alpha': 1}]}
    scripts = {
        'origin': ('origin', 'value.x = 80; value.y = 48; return value;'),
        'base-value': ('origin', 'value.x = value.x + 16; value.y = value.y - 16; return value;'),
        'scale': ('scale', 'return new Vec3(2, 0.5, 1);'),
        'alpha': ('alpha', 'return Math.min(0.5, Math.abs(-0.5));'),
        'layer': ('alpha', 'return thisLayer.origin.x / 128;'),
        'scene-layer': ('alpha', "return thisScene.getLayer('Probe').size.x / 32;"),
        'property': ('alpha', 'return engine.userProperties.opacity;'),
        'clock': ('origin', 'value.x = 80 + Math.sin(engine.runtime * 0); return value;'),
        'motion': ('origin', 'value.x = 64 + engine.runtime * 16; return value;'),
        'fault': ('alpha', 'return 0.5 / (engine.runtime - 0.5);'),
    }
    texture = bytearray(b.texture(16, 16, 16, bytes([255,64,16,255])*256))
    texture[22:26] = b.words(3)
    for name, (field, body) in scripts.items():
        scene = copy.deepcopy(base)
        scene['objects'][0][field] = {'value': scene['objects'][0][field],
                                      'script': "'use strict';\nexport function update(value) { " + body + ' }'}
        entries = {'scene.json': b.encoded(scene), 'materials/source.tex': bytes(texture),
                   'models/probe.json': b.encoded({'material': 'materials/probe.json', 'nopadding': True}),
                   'materials/probe.json': b.encoded({'passes': [{'shader': 'genericimage4', 'blending': 'translucent', 'textures': ['source']}]})}
        yield f'{name}/scene.pkg', b.package(entries)
        yield f'{name}/project.json', b.encoded({'title': f'A7 {name}', 'type': 'scene', 'file': 'scene.json',
             'general': {'properties': {'opacity': {'type': 'slider', 'value': 0.5, 'min': 0, 'max': 1}}}})


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
            raise SystemExit('Input differs: ' + name)
    print('Verified or wrote 20 A7 owned inputs; native captures unchanged')
