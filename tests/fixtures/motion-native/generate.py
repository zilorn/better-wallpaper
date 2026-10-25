#!/usr/bin/env python3
"""Owned fixed A4 motion/parallax projects, without reference generation."""
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
b = importlib.util.module_from_spec(spec); spec.loader.exec_module(b)


def animated(base, x, y, relative=False, mode='single'):
    return {'value':base, 'animation':{'relative':relative,
        'c0':[{'frame':0,'value':x[0]},{'frame':30,'value':x[1]}],
        'c1':[{'frame':0,'value':y[0]},{'frame':30,'value':y[1]}], 'c2':[],
        'options':{'fps':30,'length':30,'mode':mode,'wraploop':False}}}


def generate():
    texture = bytearray(b.texture(16,16,16,bytes([255,64,16,255])*256));texture[22:26] = b.words(3)
    base = {'camera':{'center':'0 0 -1','eye':'0 0 0','up':'0 1 0'},
        'general':{'orthogonalprojection':{'width':128,'height':128},'zoom':1,'cameraparallax':False,
                   'camerashake':False,'bloom':False,'camerafade':False,'clearcolor':'0 0 0'},
        'objects':[{'id':1,'image':'models/probe.json','origin':'64 64 0','size':'16 16','parallaxDepth':'1 1'}]}
    cases = {}
    cases['baseline'] = copy.deepcopy(base)
    for name, relative in [('origin',False),('relative',True)]:
        scene=copy.deepcopy(base)
        scene['objects'][0]['origin'] = animated('64 64 0' if relative else '0 0 0', (0,16) if relative else (48,80), (0,-16) if relative else (64,48),relative)
        cases[name] = scene
    scene=copy.deepcopy(base);scene['objects'][0]['scale']=animated('1 1 1',(1,2),(1,.5));cases['scale']=scene
    scene=copy.deepcopy(base);scene['objects'][0]['scale']=animated('1 1 1',(0,1),(0,-.5),True);cases['scale-relative']=scene
    for name, amount, depth, zoom, influence in [
        ('parallax-center',.25,'1 1',1,1), ('parallax-right',.25,'1 1',1,1),
        ('parallax-up',.25,'1 1',1,1), ('parallax-negative',.25,'-1 0',1,1),
        ('parallax-zoom',.25,'1 1',1.5,1), ('parallax-influence',.25,'1 1',1,.5),
        ('parallax-amount',.5,'1 1',1,1), ('parallax-off',0,'1 1',1,1),
        ('parallax-smooth',.5,'1 1',1,1), ('parallax-aspect',.25,'1 1',1,1),
    ]:
        scene=copy.deepcopy(base);scene['general'].update(cameraparallax=True,cameraparallaxamount=amount,cameraparallaxdelay=0,cameraparallaxmouseinfluence=influence,zoom=zoom)
        scene['objects'][0]['parallaxDepth']=depth
        if name == 'parallax-smooth': scene['general']['cameraparallaxdelay']=.25
        if name == 'parallax-aspect':
            scene['general']['orthogonalprojection']['width']=256
            scene['objects'][0]['origin']='128 64 0'
        cases[name]=scene
    scene=copy.deepcopy(base);scene['general']['cameraparallax']=True;cases['parallax-defaults']=scene
    scene=copy.deepcopy(base);scene['objects'][0]['alpha']={'value':.25,'animation':{'c0':[{'frame':0,'value':0},{'frame':30,'value':.25}],'options':{'fps':30,'length':30,'mode':'single'}}};cases['layer-alpha']=scene
    scene=copy.deepcopy(base);scene['general']['zoom']={'value':1,'animation':{'c0':[{'frame':0,'value':0},{'frame':30,'value':.5}],'options':{'fps':30,'length':30,'mode':'single'}}};cases['zoom']=scene
    for name,scene in cases.items():
        entries={'scene.json':b.encoded(scene),'materials/source.tex':bytes(texture),
                 'models/probe.json':b.encoded({'material':'materials/probe.json','nopadding':True}),
                 'materials/probe.json':b.encoded({'passes':[{'shader':'genericimage4','blending':'translucent' if name=='layer-alpha' else 'opaque','textures':['source']}]})}
        yield f'{name}/scene.pkg', b.package(entries)
        yield f'{name}/project.json', b.encoded({'title':f'A4 {name}','type':'scene','file':'scene.json'})


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--write',action='store_true');args=parser.parse_args()
    for name,data in generate():
        p=ROOT/name
        if args.write:p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(data)
        elif p.read_bytes()!=data:raise SystemExit('Fixture differs: '+name)
    print('Verified or wrote A4 motion inputs; captures unchanged')
