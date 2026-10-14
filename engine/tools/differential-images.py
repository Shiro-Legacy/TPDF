#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Exact independent/native image decoded-byte gate; strict-policy refusals separate."""
import argparse,collections,gzip,hashlib,json,re,subprocess,time
from pathlib import Path
from image_decode_cases import generated
ENGINE=Path(__file__).resolve().parent.parent
sha=lambda b:hashlib.sha256(b).hexdigest()
def read_result(result,prefix):
    if result.returncode==2:
        if result.stdout or not result.stderr.startswith(prefix) or not result.stderr[len(prefix):].strip():raise ValueError('malformed refusal')
        return None
    if result.returncode!=0:raise ValueError('unexpected process exit '+str(result.returncode))
    if result.stderr:raise ValueError('success with diagnostic')
    output=result.stdout
    if len(output)>64*1024*1024:raise ValueError('output cap')
    match=re.match(rb'P6\n([1-9][0-9]*) ([1-9][0-9]*)\n255\n',output)
    if not match:raise ValueError('PPM envelope')
    w,h=map(int,match.groups())
    if w>16384 or h>16384 or w*h>4194304 or len(output)!=match.end()+w*h*3:raise ValueError('PPM dimensions/byte length')
    return output

def main():
    ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--rust',type=Path,required=True);ap.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-image-decode-oracle'));ap.add_argument('--report',type=Path,required=True);ap.add_argument('--evidence-dir',type=Path,required=True);a=ap.parse_args();start=time.monotonic()
    paths=[a.rust,a.native,Path(__file__),ENGINE/'tools/image_decode_cases.py',ENGINE/'tools/mupdf-image-decode-oracle.c',ENGINE/'tools/build-image-reference.sh',ENGINE/'reference/image-native-pins.sha256',ENGINE/'examples/image_decode_probe.rs',ENGINE/'tools/differential-lzw.py',ENGINE/'src/image_resources.rs',ENGINE/'src/stream.rs',ENGINE/'Cargo.lock',ENGINE/'reference/image-build-r23.json',ENGINE/'reference/image-reference-policy-r23.json']
    hashes={str(p):sha(p.read_bytes()) for p in paths};rows=[];a.evidence_dir.mkdir(parents=True,exist_ok=True)
    for case in generated():
      row=dict(label=case['label'],accepted=case['accepted'],input_sha256=sha(case['pdf']),evidence_files={})
      try:
        nr=subprocess.run([str(a.native),'5'],input=case['pdf'],capture_output=True,timeout=20);rr=subprocess.run([str(a.rust),'5'],input=case['pdf'],capture_output=True,timeout=20)
        n=read_result(nr,b'image decode oracle: ');r=read_result(rr,b'image-decode: ')
        for label,result in [('native',nr),('rust',rr)]:
          row[label]=dict(exit=result.returncode,stdout_sha256=sha(result.stdout),stderr=result.stderr.decode(errors='replace')[:4096])
          path=a.evidence_dir/(case['label']+'.'+label+'.gz');path.write_bytes(gzip.compress(result.stdout,mtime=0));row['evidence_files'][label]=dict(path=str(path.resolve()),sha256=sha(result.stdout))
        path=a.evidence_dir/(case['label']+'.pdf.gz');path.write_bytes(gzip.compress(case['pdf'],mtime=0));row['evidence_files']['pdf']=dict(path=str(path.resolve()),sha256=sha(case['pdf']))
        if case['accepted']:
          if n!=case['expected'] or r!=case['expected']:raise ValueError('decoded bytes differ from independently authored samples')
          row['status']='exact_match';row['expected_sha256']=sha(case['expected'])
        else:
          if r is not None:raise ValueError('strict invalid image accepted')
          row['status']='strict_refused_native_accepted' if n is not None else 'both_refused'
      except (ValueError,subprocess.TimeoutExpired) as e:row.update(status='failure',error=str(e))
      rows.append(row)
    if hashes!={str(p):sha(p.read_bytes()) for p in paths}:raise ValueError('gate sources changed')
    failures=[r for r in rows if r['status']=='failure'];report=dict(status='failed' if failures else 'passed',scope='decode-only; page render integration remains separate',evidence_dir=str(a.evidence_dir.resolve()),evidence_sha256=hashes,elapsed_seconds=time.monotonic()-start,counts=dict(collections.Counter(r['status'] for r in rows)),rows=rows,failures=failures)
    a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:report[k] for k in ['status','counts','elapsed_seconds']}));raise SystemExit(bool(failures))
if __name__=='__main__':main()
