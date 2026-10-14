#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""R22 original Form execution, exact scope/provenance models and pixel evidence."""
import argparse,collections,gzip,hashlib,json,runpy,subprocess,time
from pathlib import Path
from concurrent.futures import ProcessPoolExecutor
from form_cases import generated
from form_reference import ENGINE,OUT,native_model,rust_model,provenance,sha
from form_raster_reference import read_native,generated_pixels,corpus_page,S

def save(root,kind,label,data):
    if root is not None:
        p=root/kind/(label+'.gz');p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(gzip.compress(data,mtime=0))
def invoke_native(args,data,page,label):
    run=subprocess.run([str(args.trace),str(page)],input=data,capture_output=True,timeout=50);row=read_native(run,page);save(args.evidence_dir,'native-trace',label,run.stdout);return row,sha(run.stdout)
def invoke_rust(args,data,page,label):
    run=subprocess.run([str(args.rust),f'--page-outlines={page}','-'],input=data,capture_output=True,timeout=50)
    if run.returncode:
        if run.returncode!=2 or run.stdout or not run.stderr.startswith(b'pdf-inspect: ') or not run.stderr[len(b'pdf-inspect: '):].strip():raise ValueError('unexpected Rust process failure '+str(run.returncode))
        return dict(page=page,status='refused',error=run.stderr.decode()),sha(run.stderr)
    if run.stderr or len(run.stdout)>64*1024*1024:raise ValueError('Rust success diagnostic/output bound')
    value=json.loads(run.stdout)
    if not isinstance(value.get('pages'),list) or len(value['pages'])!=1 or value['pages'][0].get('page')!=page or value['pages'][0].get('status')!='accepted':raise ValueError('Rust single page envelope')
    save(args.evidence_dir,'rust-trace',label,run.stdout);return value['pages'][0],sha(run.stdout)
def generated_case(payload):
    a,c=payload;row=dict(label=c['label'],scope=c['scope'],input_sha256=sha(c['pdf']),expected_acceptance=c['accepted']);assets=OUT['load_assets'](a.asset_evidence)
    try:
        n,row['native_sha256']=invoke_native(a,c['pdf'],1,'generated/'+c['label']);row['native_status']=n['status']
        if c['accepted']:row.update(native_model=native_model(n,c))
        elif c['scope']=='strict-scope-native-fallback':
            if n['status']!='accepted':raise ValueError('native fallback no longer accepted')
        elif n['status']!='refused':raise ValueError('malformed native fixture accepted')
        if n['status']=='refused':row['native_refusal']=n['error']
        if a.native_only:row['status']='native_model_matched' if c['accepted'] else 'classified';return row
        r,row['rust_sha256']=invoke_rust(a,c['pdf'],1,'generated/'+c['label']);row['rust_status']=r['status']
        if c['accepted']:
            if r['status']!='accepted':raise ValueError('positive Rust Form refusal '+r['error'])
            row.update(status='matched',geometry=OUT['compare_page'](r,n,assets),rational_model=rust_model(r,c),provenance=provenance(r,n,c))
            row['pixels']=[generated_pixels(a,c,n,assets,dpi=18)]
            if c['scope']=='clip-restore-stroke':row['pixels'].append(generated_pixels(a,c,n,assets,dpi=72))
        else:
            if r['status']!='refused':raise ValueError('expected strict/malformed Rust refusal accepted')
            row.update(status='strict_scope_refusal' if c['scope']=='strict-scope-native-fallback' else 'structural_refusal',rust_refusal=r['error'])
    except (ValueError,KeyError,IndexError,subprocess.TimeoutExpired) as e:row.update(status='failure',error=str(e))
    return row

def census_case(payload):
    a,filename,index,data=payload;row=dict(file=filename,page=index,input_sha256=sha(data));label='census/'+filename+'/'+str(index)
    try:
        n,row['native_sha256']=invoke_native(a,data,index,label);row['native_status']=n['status']
        if n['status']=='refused':row['native_refusal']=n['error']
        if a.native_only:row['status']='native_accepted' if n['status']=='accepted' else 'native_refused';return row
        r,row['rust_sha256']=invoke_rust(a,data,index,label);row['rust_status']=r['status']
        expected=filename=='redact-shared-only.pdf'
        if expected:
            if r['status']!='accepted' or n['status']!='accepted':raise ValueError('actual Form-only page not accepted')
            row.update(status='matched',geometry=OUT['compare_page'](r,n,OUT['load_assets'](a.asset_evidence)),provenance=provenance(r,n))
        else:
            if r['status']!='refused':raise ValueError('unexpected newly accepted old refusal requires review')
            row.update(status='refused',rust_refusal=r['error'])
    except (ValueError,KeyError,IndexError,subprocess.TimeoutExpired) as e:row.update(status='failure',error=str(e))
    return row

def verify_pixel_baseline(rows,baseline):
    observed={(r['file'],r['page'],r['dpi']):r for r in rows}
    if len(observed)!=len(rows):raise ValueError('duplicate corpus page/DPI')
    for key,before in baseline.items():
        after=observed.get(key,{})
        if after.get('status')!='native_diagnostic_matched' or after.get('rust_ppm_sha256')!=before['rust_ppm_sha256']:raise ValueError('R21 accepted pixels changed: '+str(key))
    if sum(r['dpi']==18 and r['status']=='native_diagnostic_matched' for r in rows)!=545:raise ValueError('expected 545 accepted 18DPI pages')
    if not all(observed.get(('redact-shared-only.pdf',p,d),{}).get('status')=='native_diagnostic_matched' for p in (1,2) for d in (18,72)):raise ValueError('actual Form pixel evidence missing')

def totals(rows):return dict(cases=len(rows),statuses=dict(collections.Counter(r['status'] for r in rows)))
def main():
    ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--rust',type=Path);ap.add_argument('--native-only',action='store_true');ap.add_argument('--trace',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-form-page-oracle'));ap.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-stroke-raster-oracle'));ap.add_argument('--report',type=Path,required=True);ap.add_argument('--evidence-dir',type=Path);ap.add_argument('--asset-evidence',type=Path,default=Path('/private/tmp/tandem-r20-builtin-evidence'));ap.add_argument('--workers',type=int,default=4);ap.add_argument('--skip-corpus',action='store_true');a=ap.parse_args();start=time.monotonic()
    if not a.native_only and a.rust is None:ap.error('--rust required for Rust-inclusive gate')
    files=[Path(__file__),a.trace,a.native,ENGINE/'reference/stroke-raster-differential-s1.json',ENGINE/'reference/stroke-raster-policy-s0.json',ENGINE/'reference/stroke-color-policy-s1.json',ENGINE/'reference/fixture-inventory.json',ENGINE/'reference/form-review-model-selfcheck.json',ENGINE/'reference/form-reference-policy-f0.json',ENGINE/'reference/form-reference-policy-notes-f0.md',ENGINE/'reference/builtin-outlines-manifest.json']+[ENGINE/'tools'/s for s in ['form_cases.py','form_pixel_model.py','form_review_model.py','form_reference.py','form_raster_reference.py','test-form-reference.py','mupdf-form-page-oracle.c','build-form-page-reference.sh','differential-page-outlines.py','page_outlines_model.py','differential-page-program.py','differential-program.py','program_model.py','text_position_model.py','page_glyph_model.py','stroke_raster_reference.py','stroke_raster_model.py','stroke_review_model.py','stroke_source_color.py','page_raster_review_model.py','page_raster_reference.py','differential-stroke-raster.py','differential-page-raster.py']]+[a.asset_evidence/(face+'-native.json.gz') for face in ('Helvetica','Times-Roman')]
    if a.rust:files.append(a.rust)
    hashes={str(p):sha(p.read_bytes()) for p in files};prior=json.loads((ENGINE/'reference/stroke-raster-differential-s1.json').read_text());old_refusals=[r for r in prior['rows'] if r['dpi']==18 and r['status']=='refused'];assert len(old_refusals)==29
    census=[(a,r['file'],r['page'],(ENGINE.parent/'fixtures'/r['file']).read_bytes()) for r in old_refusals]
    baseline={(r['file'],r['page'],r['dpi']):r for r in prior['rows'] if r['status']=='native_diagnostic_matched'};tasks=[]
    if not a.native_only and not a.skip_corpus:
        for doc in json.loads((ENGINE/'reference/fixture-inventory.json').read_text())['documents']:
            if doc.get('passwordRequired'):continue
            data=(ENGINE.parent/'fixtures'/doc['file']).read_bytes()
            if sha(data)!=doc['sha256']:raise ValueError('input fixture hash')
            for p in range(1,doc['pages']+1):
                tasks.append((a,doc['file'],p,18,data,False))
                if (doc['file'],p,72) in baseline or doc['file']=='redact-shared-only.pdf':tasks.append((a,doc['file'],p,72,data,True))
    with ProcessPoolExecutor(max_workers=a.workers) as pool:
        gen=list(pool.map(generated_case,[(a,c) for c in generated()]));old=list(pool.map(census_case,census));pixels=list(pool.map(corpus_page,tasks))
    failures=[r for r in gen+old+pixels if r['status']=='failure']
    if pixels:
        try:verify_pixel_baseline(pixels,baseline)
        except ValueError as e:failures.append(dict(status='failure',error=str(e)))
    if hashes!={str(p):sha(p.read_bytes()) for p in files}:raise ValueError('R22 gate sources changed during run')
    report=dict(status='passed' if not failures else 'failed',native_only=a.native_only,elapsed_seconds=time.monotonic()-start,evidence_sha256=hashes,generated=totals(gen),old_refusal_census=totals(old),corpus_18dpi=S['total']([r for r in pixels if r['dpi']==18]),corpus_72dpi=S['total']([r for r in pixels if r['dpi']==72]),generated_rows=gen,census_rows=old,pixel_rows=pixels,failures=failures)
    a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:report[k] for k in ('status','native_only','elapsed_seconds','generated','old_refusal_census','corpus_18dpi','corpus_72dpi')}))
    if failures:raise SystemExit(1)
if __name__=='__main__':main()
