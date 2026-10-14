#!/usr/bin/env python3
"""Compare typed Rust page placement to pinned MuPDF with explicit f32 tolerance."""
import argparse
import hashlib
import json
import math
import random
import subprocess
from pathlib import Path


def pdf(media, crop, unit, rotate):
    def numbers(values):
        return ' '.join(f'{x:.8f}'.rstrip('0').rstrip('.') if x else '0' for x in values)
    values = [b'<< /Type /Catalog /Pages 2 0 R >>', b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
              f'<< /Type /Page /Parent 2 0 R /MediaBox [{numbers(media)}] /CropBox [{numbers(crop)}] /Rotate {rotate} /UserUnit {numbers([unit])} >>'.encode()]
    data = bytearray(b'%PDF-1.7\n')
    offsets = []
    for i, value in enumerate(values, 1):
        offsets.append(len(data))
        data += f'{i} 0 obj\n'.encode() + value + b'\nendobj\n'
    start = len(data)
    data += b'xref\n0 4\n0000000000 65535 f \n'
    for offset in offsets: data += f'{offset:010} 00000 n \n'.encode()
    data += f'trailer\n<< /Root 1 0 R /Size 4 >>\nstartxref\n{start}\n%%EOF\n'.encode()
    return bytes(data)


def run(binary, args, data):
    p = subprocess.run([str(binary), *args], input=data, capture_output=True, timeout=15)
    if p.returncode: raise ValueError(p.stderr.decode(errors='replace')[:2000])
    result = json.loads(p.stdout)
    if result.get('repaired'): raise ValueError('native repaired input')
    if p.stderr: raise ValueError('unexpected diagnostic: ' + p.stderr.decode(errors='replace')[:2000])
    return result['pages']


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--rust',type=Path,default=Path('target/debug/pdf-inspect'))
    p.add_argument('--oracle',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-page-geometry-oracle'))
    p.add_argument('--fixtures',type=Path,default=Path('../fixtures'))
    p.add_argument('--report',type=Path,default=Path('reference/page-geometry-differential.json'))
    args = p.parse_args()
    rust, oracle = args.rust.resolve(), args.oracle.resolve()
    hashes = {str(f):hashlib.sha256(f.read_bytes()).hexdigest() for f in [rust,oracle]}
    rows=[]
    def compare(label,data,source=None):
        row={'case':label,'sha256':hashlib.sha256(data).hexdigest()}
        if source: row['source']=source
        try:
            a=run(rust,['--page-geometry','-'],data); b=run(oracle,[],data)
            if len(a)!=len(b): raise ValueError('page count differs')
            row['pages']=len(a); row['max_absolute_difference']=0.; row['tolerance_events']=0
            for x,y in zip(a,b):
                for field in ['matrix','bounds']:
                    if len(x[field])!=len(y[field]): raise ValueError('field length differs')
                    for u,v in zip(x[field],y[field]):
                        if not math.isfinite(u) or not math.isfinite(v): raise ValueError('nonfinite result')
                        difference=abs(u-v)
                        row['max_absolute_difference']=max(row['max_absolute_difference'],difference)
                        if difference: row['tolerance_events']+=1
                        if not math.isclose(u,v,rel_tol=2e-6,abs_tol=1e-5): raise ValueError(f'{field}: {u} != {v}')
            row['status']='matched'
        except (ValueError,subprocess.TimeoutExpired) as e: row.update(status='failure',error=str(e))
        rows.append(row)
    inventory=json.loads(Path('reference/fixture-inventory.json').read_text())
    for record in inventory['documents']:
        data=(args.fixtures/record['file']).read_bytes()
        if hashlib.sha256(data).hexdigest()!=record['sha256']: raise SystemExit('immutable fixture changed')
        if not record.get('passwordRequired'): compare(record['file'],data,'fixture')
    rng=random.Random(20261003)
    for i in range(256):
        x=rng.randint(-4000,4000)/4; y=rng.randint(-4000,4000)/4
        w=rng.randint(4,8000)/4; h=rng.randint(4,8000)/4
        crop=[x,y,x+w,y+h]; media=[x-20,y-30,x+w+40,y+h+50]
        compare(f'generated-{i}',pdf(media,crop,rng.choice([.125,.5,1.,1.25,2.,16.,100.]),rng.choice([-450,-360,-90,0,90,180,270,450,720])),'generated')
    # Native silently enlarges a valid sub-unit box. Preserve evidence separately.
    small=pdf([0,0,10,10],[0,0,.25,.5],1,0)
    recovery={'case':'positive-subunit-box','rust':run(rust,['--page-geometry','-'],small),'native':run(oracle,[],small),'policy':'Rust preserves positive dimensions; native substitutes a unit rectangle.'}
    if recovery['rust'][0]['bounds']!=[0,0,.25,.5] or recovery['native'][0]['bounds']!=[0,0,1,1]: raise SystemExit('sub-unit policy changed')
    if hashes!={str(f):hashlib.sha256(f.read_bytes()).hexdigest() for f in [rust,oracle]}: raise SystemExit('binary changed during comparison')
    report={'reference':'MuPDF 1.28.1 native pdf_page_transform/pdf_bound_page','binary_sha256':hashes,'tolerance':{'relative':2e-6,'absolute':1e-5,'reason':'native f32 geometry versus Rust f64'},'cases':len(rows),'matched':sum(r['status']=='matched' for r in rows),'pages':sum(r.get('pages',0) for r in rows),'failures':sum(r['status']=='failure' for r in rows),'rows':rows,'documented_native_recovery':[recovery]}
    args.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({k:report[k] for k in ['cases','matched','pages','failures']}))
    if report['failures']: raise SystemExit(1)
if __name__=='__main__': main()
