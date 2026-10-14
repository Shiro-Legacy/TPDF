#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Validate test-only native line-fill raster output against integer rectangle masks.

Fractional AA0 behavior is recorded separately; it is not claimed to implement
pixel-center inclusion or a prospective Rust rasterizer's sampling convention.
"""
import argparse
import hashlib
import json
import random
import subprocess
from pathlib import Path


def render(exe,width,height,content):
    run=subprocess.run([str(exe),str(width),str(height)],input=content,capture_output=True,timeout=20)
    if run.returncode:raise ValueError(run.stderr.decode(errors='replace'))
    header=f'P6\n{width} {height}\n255\n'.encode()
    if not run.stdout.startswith(header) or len(run.stdout)!=len(header)+3*width*height:raise ValueError('invalid PPM shape')
    return run.stdout[len(header):]


def rect_mask(rectangles,width,height,evenodd):
    values=[]
    for y in range(height):
        for x in range(width):
            winding=0
            for x0,y0,w,h in rectangles:
                x1,y1=x0+w,y0+h
                if min(x0,x1)<=x+0.5<max(x0,x1) and min(y0,y1)<=y+0.5<max(y0,y1):winding+=1 if w*h>0 else -1
            values.append(bool(winding%2) if evenodd else winding!=0)
    return values


def generated(rng):
    width=height=32;pixels=[(255,255,255)]*(width*height);clip=[True]*(width*height);stack=[];program=[]
    for block in range(rng.randrange(2,15)):
        if rng.randrange(4)==0 and stack:program.append('Q');clip=stack.pop()
        if rng.randrange(4)==0:program.append('q');stack.append(list(clip))
        rectangles=[(rng.randrange(-8,30),rng.randrange(-8,30),rng.choice([v for v in range(-20,25) if v]),rng.choice([v for v in range(-20,25) if v])) for _ in range(rng.randrange(1,5))]
        for rect in rectangles:program.append(' '.join(map(str,rect))+' re')
        pending=rng.randrange(3)==0;evenodd=bool(rng.randrange(2));pending_even=bool(rng.randrange(2))
        if pending:program.append('W*' if pending_even else 'W')
        paint=rng.randrange(4)!=0;color=tuple(rng.choice([0,255]) for _ in range(3))
        if paint:
            program.append(' '.join(str(c//255) for c in color)+' rg');program.append('f*' if evenodd else 'f')
            mask=rect_mask(rectangles,width,height,evenodd)
            pixels=[color if mask[i] and clip[i] else old for i,old in enumerate(pixels)]
        else:program.append('n')
        if pending:
            mask=rect_mask(rectangles,width,height,pending_even);clip=[a and b for a,b in zip(clip,mask)]
    program.extend('Q' for _ in stack)
    return width,height,('\n'.join(program)+'\n').encode(),bytes(c for pixel in pixels for c in pixel)


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-raster-oracle'));p.add_argument('--cases',type=int,default=200);p.add_argument('--seed',type=int,default=2026100310);p.add_argument('--report',type=Path,default=Path('reference/raster-oracle-validation.json'));a=p.parse_args()
    if not 1<=a.cases<=2000:p.error('cases1..2000 required')
    exe=a.native.resolve();digest=hashlib.sha256(exe.read_bytes()).hexdigest();rng=random.Random(a.seed);rows=[]
    for case in range(a.cases):
        width,height,program,wanted=generated(rng);actual=render(exe,width,height,program)
        rows.append({'case':case,'input_sha256':hashlib.sha256(program).hexdigest(),'pixel_sha256':hashlib.sha256(actual).hexdigest(),'matched':actual==wanted,'different_bytes':sum(x!=y for x,y in zip(actual,wanted))})
    fractional=[]
    for origin in [-0.6,0.1,0.49,0.5,0.51,0.9,1.1,1.6]:
        data=f'{origin} {origin} 3 2 re f'.encode();actual=render(exe,8,8,data);center=rect_mask([(origin,origin,3,2)],8,8,False)
        native_mask=[actual[i:i+3]==b'\0\0\0' for i in range(0,len(actual),3)]
        fractional.append({'origin':origin,'native_pixels':[[i%8,i//8] for i,v in enumerate(native_mask) if v],'matches_pixel_center':native_mask==center})
    degenerate=[]
    for rect in [(1,2,3,0),(1,2,0,3),(1,2,0,0),(4,4,-3,0),(4,4,0,-3),(4,4,-3,-3)]:
        program=(' '.join(map(str,rect))+' re f').encode();actual=render(exe,8,8,program)
        degenerate.append({'rectangle':list(rect),'native_pixels':[[i//3%8,i//3//8] for i in range(0,len(actual),3) if actual[i:i+3]==b'\0\0\0']})
    slanted=[]
    for label,program in [('slope-two',b'0 0 m 8 0 l 0 4 l f'),('slope-one-half',b'0 0 m 4 0 l 0 8 l f'),('offset-slants',b'1 1 m 7 2 l 3 7 l f')]:
        pixels=render(exe,10,10,program)
        slanted.append({'case':label,'content':program.decode(),'rows':[''.join('#' if pixels[(y*10+x)*3:(y*10+x+1)*3]==b'\0'*3 else '.' for x in range(10)) for y in range(10)]})
    refusals=[]
    for dimensions,program in [(['0','8'],b''),(['4097','1'],b''),(['4096','4096'],b''),(['-1','8'],b''),(['8','8'],b'0 0 m 1 1 l S'),(['8','8'],b'0 0 m 1 2 3 4 5 6 c f'),(['8','8'],b'BT ET'),(['8','8'],b'/Image Do'),(['8','8'],b'0 0 l f'),(['8','8'],b'1000001 0 m n'),(['8','8'],b'0 0 0 1 k'),(['8','8'],b' '*(1024*1024+1))]:
        r=subprocess.run([str(exe),*dimensions],input=program,capture_output=True,timeout=20);refusals.append({'args':dimensions,'input_sha256':hashlib.sha256(program).hexdigest(),'passed':r.returncode!=0 and not r.stdout})
    stable=hashlib.sha256(exe.read_bytes()).hexdigest()==digest
    report={'purpose':'Native oracle self-check, not production Rust rasterizer validation','binary_sha256':digest,'binary_unchanged':stable,'seed':a.seed,'integer_rectangle_cases':len(rows),'matched':sum(r['matched'] for r in rows),'compared_pixels':32*32*len(rows),'failures':sum(not r['matched'] for r in rows)+sum(not r['passed'] for r in refusals)+int(not stable),'fractional_native_behavior':fractional,'degenerate_native_behavior':degenerate,'slanted_integer_native_observations':slanted,'strict_refusals':refusals,'results':rows,'limitations':['Integer axis-aligned compound rectangles only; both fill rules, signed nonzero extents, clipping, restoration and primary RGB colors.','Native zero-area rectangle coverage is recorded separately and is not a mathematical empty-fill guarantee.','Pixel-center expectation coincides with integer rectangle coverage only; native fractional AA0 differs and is recorded separately.','No strokes, curves, text, resources, transparency, antialiasing or production renderer claim.']}
    a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items() if k not in ('results','fractional_native_behavior','degenerate_native_behavior','slanted_integer_native_observations','strict_refusals')},indent=2))
    for row in rows:
        if not row['matched']:print(row)
    raise SystemExit(bool(report['failures']))
if __name__=='__main__':main()
