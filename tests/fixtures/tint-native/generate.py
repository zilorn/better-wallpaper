#!/usr/bin/env python3
"""Owned A4 inputs; never generates native reference pixels."""
import argparse
import importlib.util
import json
from pathlib import Path
import sys
sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('behavior', ROOT.parent / 'scene-behavior/generate.py')
b = importlib.util.module_from_spec(spec)
spec.loader.exec_module(b)


def generate():
    values = [0, 16, 64, 96, 128, 160, 224, 255]
    for name, mode, amount, masked, alpha in [
        ('baseline', 30, 0, False, 255), ('tint', 30, 1, False, 255),
        ('bypass', 30, 0, False, 255), ('partial', 30, .375, False, 255),
        ('multiply', 2, .75, False, 255), ('screen', 7, .5, False, 255),
        ('normal', 0, .5, False, 128), ('alpha', 30, .75, False, 128),
        ('mask', 30, .75, True, 255), ('mask-linear', 7, 1, True, 255),
        ('disabled', 30, 1, False, 255), ('animated', 30, 1, False, 255),
    ]:
        pixels = bytes(c for y in range(128) for x in range(128)
                       for c in (values[x//16], values[y//16], values[(x//16+y//16*3)%8], alpha))
        source = bytearray(b.texture(128, 128, 128, pixels)); source[22:26] = b.words(3)
        # Logical mask width 96, storage 128: padding must never enter the image.
        mask = bytes(c for y in range(64) for x in range(128)
                     for c in (([0, 64, 128, 255][x//24], 255, 0, 17) if x<96 else (255, 0, 255, 255)))
        masktex = bytearray(b.texture(96, 64, 128, mask)); masktex[22:26] = b.words(2 if name == 'mask-linear' else 3)
        effect = {'file':'effects/tint/effect.json', 'visible':name != 'disabled',
                  'passes':[{'combos':{'BLENDMODE':mode, 'MASK':int(masked)},
                             'constantshadervalues':{'color':{'user':'tint', 'value':'0.25 0.75 0.5'},
                                                     'alpha':{'user':'amount','value':amount}},
                             'textures':[None, 'mask'] if masked else [None]}]}
        if name == 'animated':
            effect['passes'][0]['constantshadervalues']['alpha']['animation'] = {
                'c0':[{'frame':0,'value':0},{'frame':30,'value':1}],
                'options':{'fps':30,'length':30,'mode':'single'}}
        scene = {'camera':{'center':'0 0 -1','eye':'0 0 0','up':'0 1 0'},
                 'general':{'orthogonalprojection':{'width':128,'height':128}, 'zoom':1,
                            'cameraparallax':False,'camerashake':False,'camerafade':False,'bloom':False},
                 'objects':[{'id':1,'image':'models/probe.json','origin':'64 64 0','size':'128 128'}]}
        if name != 'baseline': scene['objects'][0]['effects'] = [effect]
        entries = {'scene.json':b.encoded(scene), 'materials/source.tex':bytes(source),
                   'models/probe.json':b.encoded({'material':'materials/probe.json','nopadding':True}),
                   'materials/probe.json':b.encoded({'passes':[{'shader':'genericimage4','blending':'translucent' if alpha<255 else 'opaque','textures':['source']}]}),
                   'effects/tint/effect.json':b.encoded({'version':1,'passes':[{'material':'materials/effects/tint.json'}]}),
                   'materials/effects/tint.json':b.encoded({'passes':[{'shader':'effects/tint','blending':'normal','depthtest':'disabled','depthwrite':'disabled','cullmode':'nocull'}]})}
        if masked: entries['materials/mask.tex'] = bytes(masktex)
        yield f'{name}/scene.pkg', b.package(entries)
        yield f'{name}/project.json', b.encoded({'type':'scene','file':'scene.json','title':f'A4 Tint {name}'})


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    for name, data in generate():
        path = ROOT / name
        if args.write:
            path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(data)
        elif path.read_bytes() != data: raise SystemExit(f'Fixture differs: {name}')
    print('Verified or wrote A4 Tint inputs; native captures unchanged')
