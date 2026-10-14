#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Fail-closed decoder result/parser and actual native wrapper fault checks."""
import argparse,hashlib,json,runpy,subprocess
from pathlib import Path
from image_decode_cases import document,ppm
D=runpy.run_path(str(Path(__file__).with_name('differential-images.py')))
def main():
    ap=argparse.ArgumentParser();ap.add_argument('--rust',type=Path,required=True);ap.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-image-decode-oracle'));ap.add_argument('--report',type=Path,required=True);a=ap.parse_args();rows=[]
    files=[Path(__file__),Path(__file__).with_name('differential-images.py'),Path(__file__).with_name('image_decode_cases.py'),Path(__file__).with_name('differential-lzw.py'),a.native,a.rust];hashes={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in files}
    good=ppm(bytes([13]),1,1,1)
    faults=[('signal',-9,b'',b''),('unexpected-exit',1,b'',b'failure'),('silent-refusal',2,b'',b''),('refusal-stdout',2,good,b'image-decode: bad'),('refusal-prefix',2,b'',b'other'),('prefix-only',2,b'',b'image-decode: '),('prefix-whitespace',2,b'',b'image-decode: \n  '),('prefix-garbage',2,b'',b'garbage image-decode: bad'),('success-warning',0,good,b'warning'),('empty-success',0,b'',b''),('json-success',0,b'{}',b''),('wrong-magic',0,good.replace(b'P6',b'P5'),b''),('negative-width',0,good.replace(b'1 1',b'-1 1'),b''),('zero-width',0,good.replace(b'1 1',b'0 1'),b''),('too-wide',0,good.replace(b'1 1',b'16385 1'),b''),('wrong-depth',0,good.replace(b'255',b'256'),b''),('truncated',0,good[:-1],b''),('extra',0,good+b'x',b''),('partial-header',0,b'P6\n',b'')]
    for label,code,out,err in faults:
      try:D['read_result'](subprocess.CompletedProcess([],code,out,err),b'image-decode: ')
      except ValueError:rows.append(dict(label=label,status='rejected'))
      else:raise ValueError('accepted fault '+label)
    data=document(bytes(range(105)))
    bads=[('not-pdf',b'garbage',['5'],2),('repair',data[:data.index(b'xref')],['5'],2),('missing-object',data,['9999'],2),('non-image',data,['1'],2),('zero-arg',data,['0'],1),('negative-arg',data,['-1'],1),('huge-arg',data,['999999999999999999'],1),('input-bound',bytes((16<<20)+1),['5'],1)]
    for label,blob,args,exit_code in bads:
      r=subprocess.run([str(a.native),*args],input=blob,capture_output=True,timeout=20)
      if r.returncode!=exit_code or r.stdout:raise ValueError('native wrapper failed '+label)
      rows.append(dict(label='native-'+label,status='rejected',exit=r.returncode,stderr=r.stderr.decode(errors='replace')[:4096]))
    for label,blob,args,_ in bads:
      r=subprocess.run([str(a.rust),*args],input=blob,capture_output=True,timeout=20)
      if r.returncode!=2 or r.stdout or not r.stderr.startswith(b'image-decode: '):raise ValueError('Rust wrapper failed '+label)
      rows.append(dict(label='rust-'+label,status='rejected',exit=r.returncode))
    if hashes!={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in files}:raise ValueError('gate inputs changed')
    report=dict(status='passed',evidence_dir=None,evidence_storage='fault cases are self-contained in hashed validation source and report rows',checks=len(rows),rows=rows,evidence_sha256=hashes);a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(dict(status='passed',checks=len(rows))))
if __name__=='__main__':main()
