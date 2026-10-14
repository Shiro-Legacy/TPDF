#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Native source-policy certificate and page envelope/process mutation tests."""
import argparse,copy,hashlib,importlib.util,json,subprocess
from pathlib import Path
from image_page_cases import generated
from image_native_policy import render,FROZEN_CLIP
MODULE=importlib.util.spec_from_file_location('image_pages',Path(__file__).with_name('differential-image-pages.py'));P=importlib.util.module_from_spec(MODULE);MODULE.loader.exec_module(P)
def main():
    ap=argparse.ArgumentParser();ap.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-image-page-oracle'));ap.add_argument('--report',type=Path,required=True);a=ap.parse_args();rows=[]
    paths=[Path(__file__),a.native,Path(FROZEN_CLIP),*[Path(__file__).with_name(p) for p in ['image_page_cases.py','image_decode_cases.py','differential-lzw.py','image_native_policy.py','image_page_model.py','differential-image-pages.py']],Path(__file__).parent.parent/'reference/image-native-pins.sha256'];sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();hashes={str(p):sha(p) for p in paths}
    for case in generated():
      w,h=case.get('width',16),case.get('height',16);witness,_=render(case['events'],w,h);r=subprocess.run([str(a.native),'1','288'],input=case['pdf'],capture_output=True,timeout=50)
      if r.returncode or r.stderr:raise ValueError('native process')
      nw,nh,raw=P.ppm(r.stdout)
      if (nw,nh)!=(w*4,h*4) or raw!=witness:raise ValueError('source witness positive')
      for offset in (0,len(raw)//2,len(raw)-1):
        changed=bytearray(raw);changed[offset]^=1
        if not P.compare(changed,witness)['different_channels']:raise ValueError('native corruption hidden as policy')
        rows.append(dict(label=case['label']+'-native-byte-'+str(offset),status='rejected'))
    case=next(c for c in generated() if c['label']=='axis');data=case['pdf'];good=subprocess.run([str(a.native),'1','288'],input=data,capture_output=True,timeout=50).stdout
    for label,value in [('empty',b''),('wrong-magic',good.replace(b'P6',b'P5',1)),('truncated',good[:-1]),('extra',good+b'x'),('negative-dimension',good.replace(b'64 64',b'-64 64',1)),('dimension-overflow',good.replace(b'64 64',b'4097 64',1))]:
      try:P.ppm(value)
      except ValueError:rows.append(dict(label=label,status='rejected'))
      else:raise ValueError('bad PPM admitted')
    for label,args,blob,code in [('missing-page',['2','72'],data,2),('zero-page',['0','72'],data,1),('dpi-bound',['1','289'],data,1),('bad-pdf',['1','72'],b'bad',2),('repaired',['1','72'],data[:data.index(b'xref')],2),('input-bound',['1','72'],bytes((16<<20)+1),1)]:
      r=subprocess.run([str(a.native),*args],input=blob,capture_output=True,timeout=50)
      if r.returncode!=code or r.stdout:raise ValueError('native fault not atomic '+label)
      rows.append(dict(label=label,status='rejected',exit=code))
    if hashes!={str(p):sha(p) for p in paths}:raise ValueError('gate inputs changed')
    report=dict(status='passed',evidence_dir=None,evidence_storage='fault cases are self-contained in hashed validation source and report rows',checks=len(rows),rows=rows,evidence_sha256=hashes);a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(dict(status='passed',checks=len(rows))))
if __name__=='__main__':main()
