#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Exploratory native/tiny-skia coverage comparison, not an acceptance gate."""
import argparse
import hashlib
import json
import random
import subprocess
from pathlib import Path


def pixels(result,width,height):
    if result.returncode:raise ValueError(result.stderr.decode(errors='replace'))
    header=f'P6\n{width} {height}\n255\n'.encode()
    if not result.stdout.startswith(header) or len(result.stdout)!=len(header)+width*height*3:raise ValueError('unexpected PPM')
    return result.stdout[len(header):]


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-raster-oracle'));p.add_argument('--tiny',type=Path,default=Path('/private/tmp/tandem-raster-audit/probe/target/debug/tandem-raster-source-audit'));p.add_argument('--cases',type=int,default=200);p.add_argument('--seed',type=int,default=9301);p.add_argument('--report',type=Path,default=Path('reference/raster-backend-coverage-probe.json'));a=p.parse_args()
    if not 1<=a.cases<=2000:p.error('cases1..2000 required')
    binaries=[a.native.resolve(),a.tiny.resolve()];hashes={str(b):hashlib.sha256(b.read_bytes()).hexdigest() for b in binaries};rng=random.Random(a.seed);cases=[]
    for label,points in [('integer-slope-one',[(0,0),(5,0),(0,5)]),('integer-slope-two',[(0,0),(8,0),(0,4)]),('integer-slope-half',[(0,0),(4,0),(0,8)]),('fractional-rectangle',[(1.6,2.6),(4.6,2.6),(4.6,4.6),(1.6,4.6)]),('zero-height',[(1,2),(4,2),(4,2),(1,2)])]:cases.append((label,points,'nonzero'))
    for i in range(a.cases):
        points=[(rng.randrange(-4,20),rng.randrange(-4,20)) for _ in range(3)];cases.append(('integer-triangle-'+str(i),points,rng.choice(['nonzero','evenodd'])))
    for i in range(50):
        x,y=rng.randrange(-4,20),rng.randrange(-4,20);w,h=rng.choice([-8,-3,2,7]),rng.choice([-7,-2,3,8]);cases.append(('integer-rectangle-'+str(i),[(x,y),(x+w,y),(x+w,y+h),(x,y+h)],rng.choice(['nonzero','evenodd'])))
    rows=[]
    for label,points,rule in cases:
        width=height=24;content=(' '.join([f'{points[0][0]} {points[0][1]} m',*[f'{x} {y} l' for x,y in points[1:]],'h','f*' if rule=='evenodd' else 'f'])).encode()
        native=pixels(subprocess.run([str(binaries[0]),str(width),str(height)],input=content,capture_output=True,timeout=20),width,height)
        tiny=pixels(subprocess.run([str(binaries[1]),str(width),str(height),rule,*[str(v) for point in points for v in point]],capture_output=True,timeout=20),width,height)
        different=sum(native[i:i+3]!=tiny[i:i+3] for i in range(0,len(native),3))
        rows.append({'case':label,'points':points,'rule':rule,'matched':not different,'different_pixels':different,'native_sha256':hashlib.sha256(native).hexdigest(),'tiny_sha256':hashlib.sha256(tiny).hexdigest()})
    stable=all(hashlib.sha256(Path(b).read_bytes()).hexdigest()==h for b,h in hashes.items())
    report={'purpose':'Exploratory renderer coverage semantics; mismatches are recorded differences, not production failures','binary_sha256':hashes,'binary_unchanged':stable,'seed':a.seed,'cases':len(rows),'matched':sum(r['matched'] for r in rows),'different':sum(not r['matched'] for r in rows),'triangle_matches':sum(r['matched'] and r['case'].startswith('integer-triangle') for r in rows),'rectangle_matches':sum(r['matched'] and r['case'].startswith('integer-rectangle') for r in rows),'results':rows}
    a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items() if k!='results'},indent=2))
    if not stable:raise SystemExit('binary changed')
if __name__=='__main__':main()
