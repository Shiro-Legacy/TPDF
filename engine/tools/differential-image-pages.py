#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Full original-page image placement: exact model, native parity controls and policy diagnostics."""
import argparse,collections,gzip,hashlib,json,re,subprocess,time
from pathlib import Path
from image_page_cases import generated
from image_page_model import render,resolve
from image_native_policy import render as native_witness,FROZEN_CLIP
sha=lambda b:hashlib.sha256(b).hexdigest()
def ppm(output):
    m=re.match(rb'P6\n([1-9][0-9]*) ([1-9][0-9]*)\n255\n',output)
    if not m:raise ValueError('PPM header')
    w,h=map(int,m.groups())
    if w>4096 or h>4096 or len(output)!=m.end()+w*h*3:raise ValueError('PPM size')
    return w,h,output[m.end():]
def compare(actual,want):
    if len(actual)!=len(want):raise ValueError('pixel lengths')
    return dict(different_channels=sum(a!=b for a,b in zip(actual,want)),max_channel_delta=max(abs(a-b) for a,b in zip(actual,want)))
def main():
    ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--rust',type=Path,required=True);ap.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-image-page-oracle'));ap.add_argument('--report',type=Path,required=True);ap.add_argument('--evidence-dir',type=Path,required=True);a=ap.parse_args();rows=[];start=time.monotonic();a.evidence_dir.mkdir(parents=True,exist_ok=True)
    paths=[a.rust,a.native,Path(__file__),*[Path(__file__).with_name(p) for p in ['image_page_cases.py','image_page_model.py','image_decode_cases.py','mupdf-image-page-oracle.c','build-image-reference.sh','image_native_policy.py'] ],Path(FROZEN_CLIP),Path(__file__).parent.parent/'reference/image-native-pins.sha256',Path(__file__).parent.parent/'reference/image-reference-policy-r23.json',Path(__file__).parent.parent/'reference/image-build-r23.json',Path(__file__).with_name('differential-lzw.py')];hashes={str(p):sha(p.read_bytes()) for p in paths}
    for case in generated():
      row=dict(label=case['label'],native_parity_required=case['native_parity'],input_sha256=sha(case['pdf']),evidence_files={})
      try:
        native=subprocess.run([str(a.native),'1','288'],input=case['pdf'],capture_output=True,timeout=50);rust=subprocess.run([str(a.rust),'--page-render=1@72','-'],input=case['pdf'],capture_output=True,timeout=50)
        if native.returncode or native.stderr:raise ValueError('native process '+native.stderr.decode(errors='replace'))
        if rust.returncode or rust.stderr:raise ValueError('Rust process '+rust.stderr.decode(errors='replace'))
        nw,nh,n=ppm(native.stdout);w,h,r=ppm(rust.stdout)
        expected_w,expected_h=case.get('width',16),case.get('height',16)
        if (w,h,nw,nh)!=(expected_w,expected_h,expected_w*4,expected_h*4):raise ValueError('dimensions')
        samples,expected=render(case['events'],w,h);native_resolved=resolve(n,nw,nh)
        witness,witness_steps=native_witness(case['events'],w,h);row['native_source_witness']=compare(n,witness);row['native_source_steps']=witness_steps
        if row['native_source_witness']['different_channels']:raise ValueError('native pixels not explained by pinned source-policy witness')
        row['rust_model']=compare(r,expected);row['native_samples_model']=compare(n,samples);row['native_resolved_model']=compare(native_resolved,expected)
        if row['rust_model']['different_channels']:raise ValueError('Rust differs from exact independent sample model')
        if case['native_parity'] and row['native_samples_model']['different_channels']:raise ValueError('native parity control differs')
        row['status']='exact_native_model_match' if not row['native_samples_model']['different_channels'] else 'exact_model_native_policy_difference'
        for kind,data in [('pdf',case['pdf']),('rust.ppm',rust.stdout),('native.ppm',native.stdout),('model.samples.rgb',samples),('model.rgb',expected),('native-source-witness.rgb',witness)]:
          row[kind+'_sha256']=sha(data);path=a.evidence_dir/(case['label']+'.'+kind+'.gz');path.write_bytes(gzip.compress(data,mtime=0));row['evidence_files'][kind]=dict(path=str(path.resolve()),sha256=sha(data))
      except (ValueError,subprocess.TimeoutExpired) as e:row.update(status='failure',error=str(e))
      rows.append(row)
    if hashes!={str(p):sha(p.read_bytes()) for p in paths}:raise ValueError('gate inputs changed during execution')
    failures=[r for r in rows if r['status']=='failure'];report=dict(status='failed' if failures else 'passed',elapsed_seconds=time.monotonic()-start,evidence_dir=str(a.evidence_dir.resolve()),evidence_sha256=hashes,counts=dict(collections.Counter(r['status'] for r in rows)),rows=rows,failures=failures);a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:report[k] for k in ['status','counts','elapsed_seconds']}));raise SystemExit(bool(failures))
if __name__=='__main__':main()
