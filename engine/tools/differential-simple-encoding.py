#!/usr/bin/env python3
"""Compare explicit PDF byte encodings/Differences against pinned source and native tables.

Python codecs are deliberately not used as the expected mapping.
"""
import argparse
import hashlib
import json
import random
import re
import subprocess
from pathlib import Path

TABLES={'WinAnsiEncoding':'win_ansi','MacRomanEncoding':'mac_roman','StandardEncoding':'adobe_standard','MacExpertEncoding':'mac_expert'}


def reference_tables(source,oracle,evidence):
    for relative,digest in evidence['sourceSha256'].items():
        if hashlib.sha256((source/relative).read_bytes()).hexdigest()!=digest:raise ValueError('source hash mismatch: '+relative)
    header=(source/'source/fitz/encodings.h').read_text()
    tables={}
    for name,symbol in TABLES.items():
        body=re.search(r'fz_glyph_name_from_'+symbol+r'\[256\]\s*=\s*\{(.*?)\};',header,re.S).group(1)
        entries=re.findall(r'"([^"]+)"|(_notdef)',body)
        tables[name]=[glyph.encode('ascii') if glyph else None for glyph,undefined in entries]
        if len(tables[name])!=256:raise ValueError('expected256 source slots')
    glyph_header=(source/'source/fitz/glyphlist.h').read_text()
    names=re.findall(r'"([^"]+)"',re.search(r'single_name_list\[\]\s*=\s*\{(.*?)\};',glyph_header,re.S).group(1))
    values=[int(value,16) for value in re.findall(r'0x([0-9a-fA-F]+)',re.search(r'single_code_list\[\]\s*=\s*\{(.*?)\};',glyph_header,re.S).group(1))]
    if len(names)!=len(values):raise ValueError('glyph source array sizes differ')
    source_registry={name.encode('ascii'):[value] for name,value in zip(names,values)}
    used={glyph for slots in tables.values() for glyph in slots if glyph is not None}
    registry={glyph:source_registry[glyph] for glyph in used}
    if len(registry)!=378:raise ValueError('expected378 named glyphs')
    run=subprocess.run([str(oracle)],capture_output=True,check=True,timeout=10)
    rows=run.stdout.decode('ascii').splitlines()
    if len(rows)!=1024:raise ValueError('native oracle row count differs')
    for line in rows:
        encoding,byte,glyph,strict,_=line.split('\t');byte=int(byte);strict=int(strict)
        expected=tables[encoding][byte]
        if (None if glyph=='-' else glyph.encode('ascii'))!=expected:raise ValueError('native/source glyph mismatch')
        if strict!=(registry[expected][0] if expected else 0):raise ValueError('native/source Unicode mismatch')
    return tables,registry


def pdf_name(data):
    return b'/'+b''.join(bytes([byte]) if 33<=byte<=126 and byte not in b'()<>[]{}/%#' else f'#{byte:02x}'.encode() for byte in data)


def expected(base,slots,registry):
    rows=[]
    for code,glyph in enumerate(slots):
        if glyph is None or glyph==b'.notdef':kind,values='undefined',None
        elif glyph in registry:kind,values='unicode',registry[glyph]
        else:kind,values='unknown',None
        rows.append({'code':code,'glyph_name_hex':None if glyph is None else glyph.hex(),'kind':kind,'scalars':values})
    return {'base':base,'slots':rows}


def apply(slots,differences):
    output=list(slots);cursor=None
    for value in differences:
        if isinstance(value,int):cursor=value
        else:
            if cursor is None or cursor>255:raise ValueError('bad generator cursor')
            output[cursor]=value;cursor+=1
    return output


def input_program(base,differences=None):
    if differences is None:return pdf_name(base.encode())
    data=b' '.join(str(value).encode() if isinstance(value,int) else pdf_name(value) for value in differences)
    return b'<< /Type /Encoding /BaseEncoding '+pdf_name(base.encode())+b' /Differences ['+data+b'] >>'


def compare(rust,program,wanted):
    result=subprocess.run([str(rust),'--encoding','-'],input=program,capture_output=True,timeout=10)
    if result.returncode:raise ValueError('Rust refusal: '+result.stderr.decode(errors='replace')[:1500])
    actual=json.loads(result.stdout)
    if actual!=wanted:
        for index,(got,want) in enumerate(zip(actual.get('slots',[]),wanted['slots'])):
            if got!=want:raise ValueError(f'slot{index} differs: {got!r} != {want!r}')
        raise ValueError('encoding envelope differs')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rust',type=Path,default=Path('target/debug/pdf-inspect'))
    parser.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-encoding-oracle'))
    parser.add_argument('--source',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-1.28.1-source'))
    parser.add_argument('--evidence',type=Path,default=Path('reference/encoding-table-inventory.json'))
    parser.add_argument('--cases',type=int,default=200)
    parser.add_argument('--seed',type=int,default=20261003)
    parser.add_argument('--report',type=Path,default=Path('reference/simple-encoding-differential-s0.json'))
    args=parser.parse_args()
    if not 1<=args.cases<=10000:parser.error('cases must be1..10000')
    rust,native=args.rust.resolve(),args.native.resolve()
    hashes={str(path):hashlib.sha256(path.read_bytes()).hexdigest() for path in (rust,native)}
    evidence=json.loads(args.evidence.read_text())
    tables,registry=reference_tables(args.source,native,evidence)
    cases=[(name,input_program(name),expected(name,table,registry)) for name,table in tables.items()]
    for suffix in (b'', b' /Differences null', b' /Differences []', b' /Type null /Extra null'):
        program=b'<< /BaseEncoding /WinAnsiEncoding'+suffix+b' >>'
        cases.append(('dictionary-'+suffix.decode(),program,expected('WinAnsiEncoding',tables['WinAnsiEncoding'],registry)))
    names=sorted(registry)
    for start in (0,256):
        differences=[0]+names[start:start+256]
        cases.append((f'registry-{start}',input_program('WinAnsiEncoding',differences),expected('WinAnsiEncoding',apply(tables['WinAnsiEncoding'],differences),registry)))
    rng=random.Random(args.seed)
    unknown=[b'uni0041',b'A.alt',b'A_B',b'altselector',b'\xffOpaque',b'A'*127,b'.notdef',b'line\nbreak',b'#name']
    for case in range(args.cases):
        base=list(TABLES)[case%4];differences=[];cursor=None
        for _ in range(rng.randrange(1,160)):
            if cursor is None or cursor>255 or rng.randrange(4)==0:
                cursor=rng.choice([0,255,rng.randrange(256)]);differences.append(cursor)
            else:
                differences.append(rng.choice(unknown) if rng.randrange(3)==0 else rng.choice(names));cursor+=1
        cases.append((f'generated-{case}',input_program(base,differences),expected(base,apply(tables[base],differences),registry)))
    results=[]
    compare(rust,cases[0][1],cases[0][2])
    for label,program,wanted in cases:
        row={'case':label,'input_sha256':hashlib.sha256(program).hexdigest()}
        try:compare(rust,program,wanted);row['status']='matched'
        except (ValueError,subprocess.TimeoutExpired) as error:row.update(status='failure',error=str(error))
        results.append(row)
    malformed=[b'/Identity-H',b'<<>>',b'<< /BaseEncoding null >>',b'<< /BaseEncoding /WinAnsiEncoding /Extra 1 >>',b'<< /BaseEncoding /WinAnsiEncoding /Type /Font >>',b'<< /BaseEncoding 1 0 R >>']
    for entries in [b'/A',b'-1 /A',b'256 /A',b'9223372036854775807 /A',b'0 1.0',b'0 true',b'0 null',b'0 (A)',b'0 [1]',b'0 <<>>',b'0 1 0 R',b'255 /A /B',b'0 /',b'0 /'+b'A'*128,b'0 /A#00B',b' '.join([b'0']*4097)]:
        malformed.append(b'<< /BaseEncoding /WinAnsiEncoding /Differences ['+entries+b'] >>')
    for value in (b'1',b'/A',b'1 0 R',b'<<>>',b'(A)',b'true'):
        malformed.append(b'<< /BaseEncoding /WinAnsiEncoding /Differences '+value+b' >>')
    malformed.extend([b'<< /Differences [0 /A] >>',b'<< /BaseEncoding /WinAnsiEncoding /Type 1 0 R >>',b'<< /BaseEncoding /WinAnsiEncoding /Extra false >>'])
    refusals=[]
    for number,program in enumerate(malformed):
        result=subprocess.run([str(rust),'--encoding','-'],input=program,capture_output=True,timeout=10)
        refusals.append({'case':number,'passed':result.returncode!=0 and not result.stdout,'diagnostic':result.stderr.decode(errors='replace')[:1000]})
    if any(hashlib.sha256(Path(path).read_bytes()).hexdigest()!=digest for path,digest in hashes.items()):raise RuntimeError('comparison binary changed during run')
    failures=sum(row['status']!='matched' for row in results)+sum(not row['passed'] for row in refusals)
    report={'reference':'Pinned MuPDF native fixed tables plus independently parsed encodings.h/glyphlist.h','seed':args.seed,'binary_sha256':hashes,'source_sha256':evidence['sourceSha256'],'base_slots':1024,'registry_names':len(registry),'cases':len(cases),'matched':sum(row['status']=='matched' for row in results),'compared_slots':256*sum(row['status']=='matched' for row in results),'strict_refusals':refusals,'failures':failures,'results':results,'limitations':['Explicit standalone byte-to-glyph-name encodings only; no implicit base, font loading or text extraction.','Unicode registry is exactly378 predefined names; unknown names remain opaque, including loose Adobe-style aliases.','Manual NUL glyph-name preservation is covered by library tests; PDF name syntax still rejects escaped NUL.','Python codecs are not used as expected mappings.']}
    args.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({key:value for key,value in report.items() if key not in ('results','strict_refusals','source_sha256')},indent=2))
    for row in [row for row in results if row['status']!='matched'][:10]:print(row)
    raise SystemExit(bool(failures))


if __name__=='__main__':main()
