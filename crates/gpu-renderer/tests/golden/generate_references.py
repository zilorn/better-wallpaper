#!/usr/bin/env python3
"""Independent analytical references, never copied from renderer output.

All rectangles lie on pixel boundaries. Scene coordinates are Y-up; PNG rows
are top-down. A 32x32 source canvas covers the viewport without stretching.
Translucent layers use straight-alpha source-over; additive layers saturate RGB.
The tests construct the same documented authored scene, through PKG/TEX parsing.
"""
from pathlib import Path
import struct
import zlib

ROOT = Path(__file__).parent

def save(name, width, height, rgba):
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    rows = b''.join(b'\0' + bytes(rgba[y*width*4:(y+1)*width*4]) for y in range(height))
    png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0))
    png += chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b'')
    (ROOT / (name + '.png')).write_bytes(png)

def layers(name, width=32, height=32, parallax=False, opacity=0.5, zoom=1):
    pixels = bytearray([24, 40, 80, 255] * width * height)
    scale = max(width / 32, height / 32) * zoom
    crop_x, crop_y = (32*scale-width)/2, (32*scale-height)/2
    # A parent translated by (4,0) and a child centered at (8,20),
    # rotated by pi/2: size (8,12) becomes bounds (6,16)-(18,24).
    fg = (6 - (4 if parallax else 0), 16, 18 - (4 if parallax else 0), 24)
    add = (20, 4 - (4 if parallax else 0), 28, 12 - (4 if parallax else 0))
    for bounds, color, alpha, additive in [(fg, (200,80,32), 128/255*opacity, False), (add, (32,48,16), 0.5, True)]:
        x0,y0,x1,y1=bounds
        for y in range(height):
            for x in range(width):
                sx=(x+0.5+crop_x)/scale
                sy=32-(y+0.5+crop_y)/scale
                if x0 <= sx < x1 and y0 <= sy < y1:
                    i=(y*width+x)*4
                    for c in range(3):
                        pixels[i+c]=min(255, round(color[c]*alpha + pixels[i+c]*(1 if additive else 1-alpha)))
                    # Source-over an opaque background remains opaque.
                    pixels[i+3]=255
    save(name,width,height,pixels)

layers('layers')
layers('cover',64,32)
layers('parallax',parallax=True)
layers('opacity-end',opacity=1)
layers('zoom-end',zoom=2)
pixels=bytearray()
for y in range(32):
    for x in range(32):
        # Deliberately asymmetric channel layout proves texture orientation.
        color=([180,20,40,255] if x<16 else [20,160,60,255]) if y<16 else ([30,50,170,255] if x<16 else [150,120,10,255])
        pixels.extend(color)
save('uv-padding',32,32,pixels)
for name,color in [('video-red',[255,0,0,255]),('video-green',[0,255,0,255])]:
    save(name,32,32,color*1024)

for name,color in [('sprite-red',[255,0,0,255]),('sprite-green',[0,255,0,255])]:
    save(name,32,32,color*1024)
