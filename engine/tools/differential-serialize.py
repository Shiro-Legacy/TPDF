#!/usr/bin/env python3
"""Check canonical object output independently with the pinned MuPDF lexer."""
import argparse
import hashlib
import json
import random
import subprocess
from pathlib import Path
from differential import invoke, compare


def object_bytes(rng, depth=0):
    kind = rng.randrange(8 if depth < 4 else 6)
    if kind == 0: return rng.choice([b'null', b'true', b'false'])
    if kind == 1: return str(rng.randint(-2**40, 2**40)).encode()
    if kind == 2: return f'{rng.randint(-9999,9999)}.{rng.randrange(100000):05d}'.encode()
    if kind == 3:
        data = bytes(rng.randrange(256) for _ in range(rng.randrange(50)))
        return b'(' + b''.join(f'\\{byte:03o}'.encode() for byte in data) + b')'
    if kind == 4: return b'/' + b''.join(f'#{rng.randrange(1,256):02x}'.encode() for _ in range(rng.randrange(20)))
    if kind == 5: return f'{rng.randint(1,2**31-1)} {rng.randrange(65536)} R'.encode()
    if kind == 6: return b'[ ' + b'\n'.join(object_bytes(rng,depth+1) for _ in range(rng.randrange(6))) + b' ]'
    return b'<< ' + b' '.join(f'/K{i} '.encode() + object_bytes(rng,depth+1) for i in range(rng.randrange(6))) + b' >>'


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--rust',type=Path,default=Path('target/debug/pdf-inspect'))
    p.add_argument('--reference',type=Path,required=True)
    p.add_argument('--cases',type=int,default=1000)
    p.add_argument('--seed',type=int,default=20261003)
    p.add_argument('--report',type=Path,default=Path('reference/serialize-differential-r5a.json'))
    a = p.parse_args()
    if not 1 <= a.cases <= 100000: p.error('--cases must be 1..100000')
    rng=random.Random(a.seed); failures=[]; corpus=hashlib.sha256(); checked=0
    for i in range(a.cases):
        source=object_bytes(rng); corpus.update(len(source).to_bytes(8,'big')); corpus.update(source)
        canonical=subprocess.run([str(a.rust.resolve()),'--canonical'],input=source,capture_output=True,timeout=5)
        if canonical.returncode:
            failures.append({'case':i,'source':source.hex(),'error':canonical.stderr.decode('utf8','replace')}); break
        before=invoke(a.reference.resolve(),source); after=invoke(a.reference.resolve(),canonical.stdout)
        equal,_=compare(before[1],after[1])
        again=subprocess.run([str(a.rust.resolve()),'--canonical'],input=canonical.stdout,capture_output=True,timeout=5)
        checked+=1
        if before[0] or after[0] or not equal or again.returncode or again.stdout!=canonical.stdout:
            failures.append({'case':i,'source_hex':source.hex(),'canonical_hex':canonical.stdout.hex(),
                'before':before,'after':after,'idempotent':again.stdout==canonical.stdout})
            if len(failures)>=10: break
    report={'seed':a.seed,'cases':checked,'corpus_sha256':corpus.hexdigest(),
        'comparison':'MuPDF 1.28.1 lexical values before/after; small-magnitude reals with f32 rounding tolerance; byte-identical canonical idempotence',
        'passed':not failures,'failures':failures}
    a.report.parent.mkdir(parents=True,exist_ok=True);a.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2))
    return 0 if report['passed'] else 1


if __name__=='__main__': raise SystemExit(main())
