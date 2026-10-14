#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Exact orthogonal raster masks; native parity only for endpoint RGB colors."""
import argparse
import hashlib
import importlib.util
import json
import random
import subprocess
from pathlib import Path


def reference_module():
    spec=importlib.util.spec_from_file_location('raster_reference',Path(__file__).with_name('check-raster-reference.py'));m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m


def rendered(exe,program,width,height,native=False):
    args=[str(exe),str(width),str(height)] if native else [str(exe),f'--raster={width}x{height}','-']
    result=subprocess.run(args,input=program,capture_output=True,timeout=20)
    if result.returncode:raise ValueError(result.stderr.decode(errors='replace')[:2000])
    header=f'P6\n{width} {height}\n255\n'.encode()
    if not result.stdout.startswith(header) or len(result.stdout)!=len(header)+width*height*3:raise ValueError('bad PPM shape')
    return result.stdout[len(header):]


def path_for(points):
    return '\n'.join([f'{points[0][0]} {points[0][1]} m',*[f'{x} {y} l' for x,y in points[1:]],'h'])


def generated(count,seed,mask):
    rng=random.Random(seed);size=32
    for label,program,rectangle in [
        ('scaled-fractional-source',b'2 0 0 2 0 0 cm 0.5 1.5 2 3 re f',(1,3,4,6)),
        ('reflection',b'-1 0 0 1 32 0 cm 3 4 5 6 re f',(24,4,5,6)),
        ('quarter-turn',b'0 1 -1 0 32 0 cm 3 4 5 6 re f',(22,3,6,5)),
        ('noncommuting-transforms',b'1 0 0 1 3 4 cm 2 0 0 3 0 0 cm 0.5 1 2 2 re f',(4,7,4,6)),
    ]:
        coverage=mask([rectangle],size,size,False)
        yield label,program,size,size,bytes(channel for covered in coverage for channel in ([0]*3 if covered else [255]*3))
    for case in range(count):
        pixels=[(255,255,255)]*(size*size);clip=[True]*(size*size);stack=[];commands=[]
        for _ in range(rng.randrange(2,12)):
            if stack and rng.randrange(4)==0:commands.append('Q');clip=stack.pop()
            if rng.randrange(4)==0:commands.append('q');stack.append(list(clip))
            kind=rng.choice(['rect','compound','L','U'])
            if kind in ('rect','compound'):
                rectangles=[]
                for _ in range(1 if kind=='rect' else rng.randrange(2,5)):
                    xa,xb=sorted(rng.sample(range(size+1),2));ya,yb=sorted(rng.sample(range(size+1),2));x,w=(xa,xb-xa) if rng.randrange(2) else (xb,xa-xb);y,h=(ya,yb-ya) if rng.randrange(2) else (yb,ya-yb);rectangles.append((x,y,w,h));commands.append(f'{x} {y} {w} {h} re')
            elif kind=='L':
                x0,x1,x2=sorted(rng.sample(range(size+1),3));y0,y1,y2=sorted(rng.sample(range(size+1),3));points=[(x0,y0),(x2,y0),(x2,y1),(x1,y1),(x1,y2),(x0,y2)];rectangles=[(x0,y0,x2-x0,y1-y0),(x0,y1,x1-x0,y2-y1)];commands.append(path_for(points))
            else:
                x0,xa,xb,x1=sorted(rng.sample(range(size+1),4));y0,y1,y2=sorted(rng.sample(range(size+1),3));points=[(x0,y0),(x1,y0),(x1,y2),(xb,y2),(xb,y1),(xa,y1),(xa,y2),(x0,y2)];rectangles=[(x0,y0,x1-x0,y1-y0),(x0,y1,xa-x0,y2-y1),(xb,y1,x1-xb,y2-y1)];commands.append(path_for(points))
            pending=rng.randrange(3)==0;pending_even=bool(rng.randrange(2));even=bool(rng.randrange(2));color=tuple(rng.choice([0,255]) for _ in range(3))
            if pending:commands.append('W*' if pending_even else 'W')
            if rng.randrange(4):
                commands.append(' '.join(str(v//255) for v in color)+' rg');commands.append('f*' if even else 'f');fill=mask(rectangles,size,size,even);pixels=[color if fill[i] and clip[i] else old for i,old in enumerate(pixels)]
            else:commands.append('n')
            if pending:clip=[a and b for a,b in zip(clip,mask(rectangles,size,size,pending_even))]
        commands.extend('Q' for _ in stack)
        yield case,('\n'.join(commands)+'\n').encode(),size,size,bytes(v for pixel in pixels for v in pixel)


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--rust',type=Path,default=Path('target/raster-gate/debug/pdf-inspect'));p.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-raster-oracle'));p.add_argument('--cases',type=int,default=300);p.add_argument('--seed',type=int,default=2026100312);p.add_argument('--report',type=Path,default=Path('reference/raster-differential-r0.json'));a=p.parse_args()
    if not 1<=a.cases<=2000:p.error('cases1..2000 required')
    rust,native=a.rust.resolve(),a.native.resolve();hashes={str(b):hashlib.sha256(b.read_bytes()).hexdigest() for b in (rust,native)};helper=reference_module();rows=[]
    for case,program,w,h,wanted in generated(a.cases,a.seed,helper.rect_mask):
        row={'case':case,'input_sha256':hashlib.sha256(program).hexdigest()}
        try:
            actual=rendered(rust,program,w,h);reference=rendered(native,program,w,h,True)
            if actual!=wanted or reference!=wanted:raise ValueError(f'pixel differences Rust={sum(x!=y for x,y in zip(actual,wanted))} native={sum(x!=y for x,y in zip(reference,wanted))}')
            row.update(status='matched',pixel_sha256=hashlib.sha256(actual).hexdigest())
        except ValueError as error:row.update(status='failure',error=str(error))
        rows.append(row)
    # Build a single256-pixel strip for exact byte centers, and three255-pixel
    # strips around nominal quantization ties. Exact expected output follows the
    # explicit byte quantization policy, independent of native color conversion.
    colors=[]
    for label,values,wanted in [('byte-centers',[i/255 for i in range(256)],list(range(256))),('below-ties',[(i+0.5)/255-1e-9 for i in range(255)],list(range(255))),('nominal-ties',[(i+0.5)/255 for i in range(255)],list(range(1,256))),('above-ties',[(i+0.5)/255+1e-9 for i in range(255)],list(range(1,256)))]:
        for color_kind in ['g','rg']:
            ops=[]
            for x,value in enumerate(values):ops.append((f'{value:.17g} '+color_kind if color_kind=='g' else ' '.join([f'{value:.17g}']*3)+' rg')+f' {x} 0 1 1 re f')
            program='\n'.join(ops).encode();expected=bytes(v for x in wanted for v in [x]*3);actual=rendered(rust,program,len(values),1);reference=rendered(native,program,len(values),1,True)
            colors.append({'case':label+'-'+color_kind,'passed':actual==expected,'pixels':len(values),'native_different_pixels':sum(actual[i:i+3]!=reference[i:i+3] for i in range(0,len(actual),3)),'rust_sha256':hashlib.sha256(actual).hexdigest(),'native_sha256':hashlib.sha256(reference).hexdigest()})
    refused=[]
    for program in [b'1.5 1 2 2 re f',b'0 0 m 8 0 l 0 4 l f',b'1 1 2 0 re f',b'1 1 0 2 re f',b'-1 0 2 2 re f',b'31 31 2 2 re f',b'0 0 2 2 re S',b'0 0 0 1 k 1 1 2 2 re f',b'0 0 m 1 2 3 4 5 6 c f',b'0 0 2 2 re f 3 3 2 2 re B',b'0 0 2 2 re f BT ET',b'0 0 m 8 0 l 8 8 l 4 8 l 4 0 l 0 0 l f']:
        r=subprocess.run([str(rust),'--raster=32x32','-'],input=program,capture_output=True,timeout=20);refused.append({'input':program.decode(),'passed':r.returncode!=0 and not r.stdout,'diagnostic':r.stderr.decode(errors='replace')[:1000]})
    stable=all(hashlib.sha256(Path(b).read_bytes()).hexdigest()==v for b,v in hashes.items());failures=sum(r['status']!='matched' for r in rows)+sum(not r['passed'] for r in colors+refused)+int(not stable)
    report={'scope':'Strict integer in-canvas orthogonal contours; exact native parity only for endpoint RGB palette','seed':a.seed,'binary_sha256':hashes,'binary_unchanged':stable,'cases':len(rows),'matched':sum(r['status']=='matched' for r in rows),'compared_pixels':1024*len(rows),'color_policy_cases':colors,'strict_refusals':refused,'failures':failures,'results':rows,'limitations':['Native interior Gray/RGB color conversion differs from declared round-half-up Rust policy and is recorded separately, never hidden with pixel tolerances.','Fractional/slanted/cubic/zero-area/self-intersecting/out-of-canvas geometry, strokes and CMYK are explicitly outsideR0.','This validates isolated path programs, not general PDF pages.']}
    a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items() if k not in ('results','strict_refusals','color_policy_cases')},indent=2))
    for row in rows:
        if row['status']!='matched':print(row)
    print('color policy results:',colors)
    raise SystemExit(bool(failures))
if __name__=='__main__':main()
