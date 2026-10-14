#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Declared simple-encoding text and byte spans versus pinned source/native tables."""
import argparse
import hashlib
import importlib.util
import json
import random
import subprocess
import tempfile
from pathlib import Path


def module():
    spec=importlib.util.spec_from_file_location('encoding_reference',Path(__file__).with_name('differential-simple-encoding.py'));loaded=importlib.util.module_from_spec(spec);spec.loader.exec_module(loaded);return loaded


def expected(raw,slots,registry):
    output=bytearray();codes=[]
    for index,code in enumerate(raw):
        sequence=registry[slots[code]];start=len(output);output.extend(''.join(chr(v) for v in sequence).encode())
        codes.append({'code':bytes([code]).hex(),'source':[index,index+1],'utf8':[start,len(output)]})
    return {'text_utf8_hex':output.hex(),'codes':codes}


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--rust',type=Path,default=Path('target/debug/pdf-inspect'));p.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-encoding-oracle'));p.add_argument('--source',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-1.28.1-source'));p.add_argument('--cases',type=int,default=100);p.add_argument('--seed',type=int,default=2026100311);p.add_argument('--report',type=Path,default=Path('reference/encoding-text-differential-d0.json'));a=p.parse_args()
    if not 1<=a.cases<=2000:p.error('cases1..2000 required')
    helper=module();rust=a.rust.resolve();native=a.native.resolve();hashes={str(b):hashlib.sha256(b.read_bytes()).hexdigest() for b in (rust,native)}
    evidence=json.loads(Path('reference/encoding-table-inventory.json').read_text());tables,registry=helper.reference_tables(a.source,native,evidence);rng=random.Random(a.seed);cases=[];refusals=[];defined=0
    for base,slots in tables.items():
        raw=bytes(i for i,g in enumerate(slots) if g in registry);defined+=len(raw)
        cases.append(('all-defined-'+base,helper.input_program(base),raw,expected(raw,slots,registry)))
        cases.append(('empty-'+base,helper.input_program(base),b'',expected(b'',slots,registry)))
        for code,glyph in enumerate(slots):
            if glyph not in registry:refusals.append(('undefined-'+base+'-'+str(code),helper.input_program(base),bytes([code]),'UndefinedEncodingCode'))
    names=sorted(registry)
    for start in (0,256):
        entries=[0]+names[start:start+256];slots=helper.apply(tables['WinAnsiEncoding'],entries);raw=bytes(range(len(entries)-1));cases.append((f'registry-{start}',helper.input_program('WinAnsiEncoding',entries),raw,expected(raw,slots,registry)))
    for case in range(a.cases):
        base=list(tables)[case%4];entries=[]
        for _ in range(rng.randrange(0,100)):entries.extend([rng.randrange(256),rng.choice(names)])
        slots=helper.apply(tables[base],entries);valid=[i for i,g in enumerate(slots) if g in registry];raw=bytes(rng.choice(valid) for _ in range(rng.randrange(0,1000)))
        cases.append((f'generated-{case}',helper.input_program(base,entries),raw,expected(raw,slots,registry)))
    for glyph in [b'uni0041',b'A.alt',b'A_B',b'altselector',b'\xffOpaque',b'.notdef']:
        refusals.append(('override-'+glyph.hex(),helper.input_program('WinAnsiEncoding',[0,glyph]),b'A\0','UndefinedEncodingCode' if glyph==b'.notdef' else 'UnknownGlyphName'))
    refusals.extend([('input-cap',b'/WinAnsiEncoding',b'A'*(1024*1024+1),None),('code-cap',b'/WinAnsiEncoding',b'A'*262145,'Codes')])
    rows=[];refused=[];codes=0;utf8=0
    with tempfile.TemporaryDirectory(prefix='tandem-encoding-text-',dir='/private/tmp') as directory:
        rawfile=Path(directory)/'raw.bin'
        def run(metadata,raw):
            rawfile.write_bytes(raw);return subprocess.run([str(rust),'--encoding-text='+str(rawfile),'-'],input=metadata,capture_output=True,timeout=10)
        for label,metadata,raw,wanted in cases:
            result=run(metadata,raw);row={'case':label,'input_sha256':hashlib.sha256(raw).hexdigest(),'encoding_sha256':hashlib.sha256(metadata).hexdigest()}
            try:
                if result.returncode:raise ValueError(result.stderr.decode(errors='replace'))
                if json.loads(result.stdout)!=wanted:raise ValueError('text or per-code spans differ')
                row['status']='matched';codes+=len(raw);utf8+=len(wanted['text_utf8_hex'])//2
            except (ValueError,UnicodeDecodeError) as error:row.update(status='failure',error=str(error)[:1500])
            rows.append(row)
        for label,metadata,raw,kind in refusals:
            result=run(metadata,raw);diagnostic=result.stderr.decode(errors='replace');refused.append({'case':label,'passed':result.returncode!=0 and not result.stdout and (kind is None or kind in diagnostic),'diagnostic':diagnostic[:500]})
    stable=all(hashlib.sha256(Path(b).read_bytes()).hexdigest()==digest for b,digest in hashes.items());failures=sum(row['status']!='matched' for row in rows)+sum(not row['passed'] for row in refused)+int(not stable)
    report={'reference':'Pinned native encoding oracle and independently parsed source registry; no Python codec substitution','seed':a.seed,'binary_sha256':hashes,'binary_unchanged':stable,'defined_base_slots':defined,'undefined_base_slots':1024-defined,'registry_names':len(registry),'valid_cases':len(rows),'matched':sum(r['status']=='matched' for r in rows),'decoded_codes':codes,'utf8_bytes':utf8,'refusals':len(refused),'failures':failures,'results':rows,'strict_refusals':refused}
    a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items() if k not in ('results','strict_refusals')},indent=2));raise SystemExit(bool(failures))
if __name__=='__main__':main()
