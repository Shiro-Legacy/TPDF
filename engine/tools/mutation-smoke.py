#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Bounded deterministic mutation smoke checks; malformed input is not parity evidence."""
import argparse
import hashlib
import json
import random
import subprocess
import time
from pathlib import Path

MAX_INPUT=16*1024
MAX_OUTPUT=4*1024*1024
MODES=['--document','--fonts','--content','--path']
CMAP=b'begincmap 1 begincodespacerange <00><ff> endcodespacerange 1 beginbfchar <41><00660069> endbfchar endcmap'
FONT=b'<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /B 255 /.notdef 0 /Opaque] >> >>'
MAPPED=b'<< /Type /Font /Subtype /TrueType /ToUnicode 6 0 R >>'
PATHS=[b'q 0.5 w [2 3] 1 d 1 0 0 RG 0 0 20 30 re W B Q',b'0 0 m q 10 20 l Q 30 40 50 60 v 70 80 90 100 y h S',b'q 1 0.25 0.5 1 2 3 cm 0 0 10 20 re W* n q 1 2 5 6 re B* Q Q',b'0 0 0 0 re S q W n Q']
CONTENTS=PATHS+[b'BT /F 12 Tf <41424300ff> Tj [(abc) 2 (def)] TJ ET',b'[null true 1 2.0 /Name <00ff> << /A (bytes) >>] Custom']
TOKENS=[b'\0',b'\xff',b'99999999999999999999999',b'-9223372036854775808',b'0',b'null',b'1 0 R',b'999 0 R',b'[',b']',b'<<',b'>>',b'/',b'#00',b'BI',b'Q',b'W',b'n',b'BT',b'%',b'\r\n',b'()',b'<D800>',b'/UseCMap /Identity-H',b'/Filter /FlateDecode',b'/Subtype /Type0',b'/ToUnicode 6 0 R']


def make_pdf(font=FONT,mapped=MAPPED,cmap=CMAP,content=PATHS[0],resources=None):
    if resources is None:resources=b'<< /Font << /F 4 0 R /Alias 4 0 R /Mapped 5 0 R >> >>'
    def stream(body):return b'<< /Length '+str(len(body)).encode()+b' >>\nstream\n'+body+b'\nendstream'
    objects=[b'<< /Type /Catalog /Pages 2 0 R >>',b'<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources '+resources+b' >>',b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 7 0 R >>',font,mapped,stream(cmap),stream(content)]
    result=bytearray(b'%PDF-1.7\n');offsets=[0]
    for index,body in enumerate(objects,1):
        offsets.append(len(result));result.extend(str(index).encode()+b' 0 obj\n'+body+b'\nendobj\n')
    xref=len(result);result.extend(f'xref\n0 {len(offsets)}\n0000000000 65535 f \n'.encode())
    for offset in offsets[1:]:result.extend(f'{offset:010} 00000 n \n'.encode())
    result.extend(f'trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode())
    return bytes(result)


def mutate(data,rng):
    data=bytearray(data)
    for _ in range(rng.randrange(1,5)):
        choice=rng.randrange(7);at=rng.randrange(len(data)+1)
        if choice==0 and data:
            i=min(at,len(data)-1);data[i]^=1<<rng.randrange(8)
        elif choice==1:data[at:at]=rng.choice(TOKENS)
        elif choice==2 and data:del data[at:at+rng.randrange(1,17)]
        elif choice==3 and data:
            start=rng.randrange(len(data));data[at:at]=data[start:start+rng.randrange(1,33)]
        elif choice==4:data[at:at+rng.randrange(1,17)]=rng.choice(TOKENS)
        elif choice==5:del data[at:]
        else:data[at:at]=bytes(rng.randrange(256) for _ in range(rng.randrange(1,9)))
        if len(data)>MAX_INPUT:del data[MAX_INPUT:]
    return bytes(data)


def generate(count,seed):
    rng=random.Random(seed)
    for i in range(count):
        mode=MODES[i%len(MODES)];family='raw'
        if mode=='--path':data=mutate(rng.choice(PATHS),rng)
        elif mode=='--content':data=mutate(rng.choice(CONTENTS),rng)
        elif mode=='--document' and rng.randrange(2):data=mutate(make_pdf(),rng)
        else:
            family=rng.choice(['font','mapped','cmap','resources','content'])
            source={'font':FONT,'mapped':MAPPED,'cmap':CMAP,'resources':b'<< /Font << /F 4 0 R /Alias 4 0 R /Mapped 5 0 R >> >>','content':rng.choice(CONTENTS)}[family]
            data=make_pdf(**{family:mutate(source,rng)})
            family='rebuilt-'+family
        assert len(data)<=MAX_INPUT
        yield i,mode,family,data


def inspect(binary,mode,data,timeout):
    # Communicate is acceptable here because every input is <=16KiB and CLI
    # output already has a64MiB hard cap. Also flag unexpectedly large output.
    started=time.monotonic()
    try:result=subprocess.run([str(binary),mode,'-'],input=data,capture_output=True,timeout=timeout)
    except subprocess.TimeoutExpired:return {'status':'failure','reason':'timeout','seconds':timeout}
    reasons=[]
    if result.returncode<0:reasons.append('terminated by signal '+str(-result.returncode))
    if result.returncode==101 or b'panicked at' in result.stderr or b'fatal runtime error' in result.stderr:reasons.append('Rust panic/abort')
    if result.returncode and result.stdout:reasons.append('non-atomic error output')
    if len(result.stdout)>MAX_OUTPUT or len(result.stderr)>65536:reasons.append('unexpected output volume')
    if result.returncode==0 and mode in ('--document','--fonts','--path'):
        try:json.loads(result.stdout)
        except (ValueError,UnicodeDecodeError):reasons.append('successful output is invalid JSON')
    return {'status':'failure' if reasons else 'accepted' if result.returncode==0 else 'refused','exit':result.returncode,'stdout_bytes':len(result.stdout),'stderr_bytes':len(result.stderr),'seconds':round(time.monotonic()-started,6),**({'reason':'; '.join(reasons),'diagnostic':result.stderr[:2000].decode(errors='replace')} if reasons else {})}


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--rust',type=Path,default=Path('target/debug/pdf-inspect'));p.add_argument('--cases',type=int,default=1600);p.add_argument('--seed',type=int,default=2026100309);p.add_argument('--timeout',type=float,default=2);p.add_argument('--report',type=Path,default=Path('reference/mutation-smoke-r9.json'));p.add_argument('--failures-dir',type=Path,default=Path('/private/tmp/tandem-mutation-failures'));a=p.parse_args()
    if not 1<=a.cases<=10000 or not 0.1<=a.timeout<=10:p.error('cases1..10000, timeout0.1..10 required')
    binary=a.rust.resolve();digest=hashlib.sha256(binary.read_bytes()).hexdigest();rows=[];baseline=[]
    for mode,data in [('--document',make_pdf()),('--fonts',make_pdf()),*[( '--path',v) for v in PATHS],*[( '--content',v) for v in CONTENTS]]:
        row=inspect(binary,mode,data,a.timeout);baseline.append({'mode':mode,**row})
        if row['status']!='accepted':raise RuntimeError('baseline not accepted: '+str(baseline[-1]))
    for index,mode,family,data in generate(a.cases,a.seed):
        row={'case':index,'mode':mode,'family':family,'input_bytes':len(data),'input_sha256':hashlib.sha256(data).hexdigest(),**inspect(binary,mode,data,a.timeout)};rows.append(row)
        if row['status']=='failure':
            a.failures_dir.mkdir(parents=True,exist_ok=True);path=a.failures_dir/f'{a.seed}-{index}.bin';path.write_bytes(data);row['reproducer']=str(path)
    stable=hashlib.sha256(binary.read_bytes()).hexdigest()==digest
    counts={mode:{kind:sum(row['mode']==mode and row['status']==kind for row in rows) for kind in ('accepted','refused','failure')} for mode in MODES}
    report={'purpose':'Deterministic bounded crash/timeout/atomic-output smoke; malformed acceptance is not semantic parity','seed':a.seed,'cases':a.cases,'binary':str(binary),'binary_sha256':digest,'binary_unchanged':stable,'max_input_bytes':MAX_INPUT,'subprocess_timeout_seconds':a.timeout,'counts':counts,'failures':sum(row['status']=='failure' for row in rows)+int(not stable),'baseline':baseline,'results':rows}
    a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items() if k not in ('results','baseline')},indent=2));raise SystemExit(bool(report['failures']))
if __name__=='__main__':main()
