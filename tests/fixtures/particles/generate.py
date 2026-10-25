#!/usr/bin/env python3
"""Generate owned A6 particle inputs and hashes; never generate native references."""
import argparse
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('behavior', ROOT.parent / 'scene-behavior/generate.py')
b = importlib.util.module_from_spec(spec)
spec.loader.exec_module(b)


def definition():
    return {'material':'materials/particle.json', 'maxcount':100, 'starttime':0,
            'emitter':[{'name':'boxrandom','rate':0,'instantaneous':1,
                        'distancemin':'0 0 0','distancemax':'0 0 0'}],
            'initializer':[{'name':'lifetimerandom','min':3600,'max':3600},
                           {'name':'sizerandom','min':32,'max':32},
                           {'name':'velocityrandom','min':'0 0 0','max':'0 0 0'},
                           {'name':'colorrandom','min':'128 255 64','max':'128 255 64'}],
            'operator':[{'name':'movement','gravity':'0 0 0','drag':0}],
            'renderer':[{'name':'sprite'}]}


def generate():
    base = {'camera':{'center':'0 0 -1','eye':'0 0 0','up':'0 1 0'},
            'general':{'orthogonalprojection':{'width':128,'height':128},'clearcolor':'0 0 0',
                       'bloom':False,'camerafade':False,'cameraparallax':False,'zoom':1},
            'objects':[{'id':17,'particle':'particles/probe.json','origin':'64 64 0'}]}
    texture = bytearray(b.texture(8,4,8,bytes([255,128,64,128])*32))
    texture[22:26] = b.words(3)
    material = {'passes':[{'shader':'genericparticle','blending':'translucent',
                           'textures':['particle'],'cullmode':'nocull','depthtest':'disabled','depthwrite':'disabled'}]}
    cases = {'static':(copy.deepcopy(base),definition(),copy.deepcopy(material))}
    for name,scale in [('scale-one','1 1 1'),('scale-two','2 2 2'),('stretched','2 1 1')]:
        scene=copy.deepcopy(base);scene['objects'][0]['scale']=scale
        cases[name]=(scene,definition(),copy.deepcopy(material))
    scene=copy.deepcopy(base);scene['objects'][0]['angles']='0 0 1.5707963267948966'
    cases['rotated']=(scene,definition(),copy.deepcopy(material))
    cases['pattern']=(copy.deepcopy(base),definition(),copy.deepcopy(material))
    scene=copy.deepcopy(base);particle=definition();particle['starttime']=60
    cases['starttime-probe']=(scene,particle,copy.deepcopy(material))
    scene=copy.deepcopy(base);particle=definition()
    particle['emitter'][0].update(rate=10,instantaneous=0,duration=1)
    particle['initializer'][3].update(min='1 1 1',max='1 1 1')
    mat=copy.deepcopy(material);mat['passes'][0]['blending']='additive'
    cases['finite']=(scene,particle,mat)
    particle=definition();particle['maxcount']=100000;particle['emitter'][0]['instantaneous']=100000
    particle['initializer'][1].update(min=0,max=0)
    cases['budget']=(copy.deepcopy(base),particle,copy.deepcopy(material))
    for name in ['additive','overlap','continuous','random','delayed','unsupported']:
        scene, particle, mat = copy.deepcopy(base), definition(), copy.deepcopy(material)
        if name in ['additive','overlap']: particle['emitter'][0]['instantaneous']=2
        if name == 'additive': mat['passes'][0]['blending']='additive'
        if name in ['continuous','random','delayed']:
            particle['emitter'][0].update(rate=10,instantaneous=0)
            particle['initializer'][0].update(min=2,max=2)
            particle['initializer'][2].update(min='8 0 0',max='8 0 0')
        if name == 'delayed':
            particle['starttime']=2
            particle['emitter'][0]['duration']=1
        if name == 'random':
            particle['emitter'][0].update(distancemin='-32 -16 0',distancemax='32 16 0')
            particle['initializer'][1].update(min=4,max=16)
            particle['initializer'][2].update(min='-8 -4 0',max='8 4 0')
            particle['initializer'][3].update(min='0 255 16',max='255 0 240')
        if name == 'unsupported': particle['operator']=[{'name':'turbulence'}]
        cases[name]=(scene,particle,mat)
    files={}
    for name,(scene,particle,mat) in cases.items():
        pixels=bytes(texture)
        if name in ['pattern','rotated','stretched']:
            pattern=bytearray(b.texture(8,4,8,bytes(c for y in range(4) for x in range(8)
                for c in (255 if x<4 else 32,255 if y<2 else 32,32 if x<4 else 255,128))))
            pattern[22:26]=b.words(3);pixels=bytes(pattern)
        if name=='finite':
            pixels=bytearray(b.texture(8,4,8,bytes([255,255,255,255])*32));pixels[22:26]=b.words(3);pixels=bytes(pixels)
        entries={'scene.json':b.encoded(scene),'particles/probe.json':b.encoded(particle),
                 'materials/particle.json':b.encoded(mat),'materials/particle.tex':pixels}
        files[f'{name}/scene.pkg']=b.package(entries)
        files[f'{name}/project.json']=b.encoded({'title':f'A6 {name}','type':'scene','file':'scene.json'})
    manifest={'program':{'exe_sha256':'40e2ce021e9352324fadb3b8f72b8ba2a7ee95b71cc571d5b9f84be75cd993b0'},
              'viewport':[128,128],'pointer':{},'random_seed':'FNV-1a node 17 / SplitMix64 counters',
              'times':[0,.1,.5,1,2,3,4,5,3600,1000000000],
              'native_cases':['static','overlap','additive','scale-one','scale-two','pattern','rotated','stretched','finite'],
              'native_rejected_probes':['starttime-probe'],
              'native_capture_sha256':{p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted((ROOT/'captures').glob('*.png'))},
              'sha256':{p:hashlib.sha256(d).hexdigest() for p,d in files.items()}}
    files['manifest.json']=(json.dumps(manifest,indent=2,sort_keys=True)+'\n').encode()
    return files


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write',action='store_true')
    args=parser.parse_args()
    for name,data in generate().items():
        path=ROOT/name
        if args.write: path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(data)
        elif path.read_bytes()!=data: raise SystemExit('Fixture differs: '+name)
    print('Verified A6 particle fixture bytes and hashes')
