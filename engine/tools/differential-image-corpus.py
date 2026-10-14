#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Complete R23 page classification, frozen R22 hash preservation and original native diagnostics."""
import argparse,collections,gzip,hashlib,importlib.util,json,subprocess,time
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path
from image_page_model import resolve
MODULE=importlib.util.spec_from_file_location('image_pages',Path(__file__).with_name('differential-image-pages.py'));P=importlib.util.module_from_spec(MODULE);MODULE.loader.exec_module(P)
ENGINE=Path(__file__).resolve().parent.parent
sha=lambda b:hashlib.sha256(b).hexdigest()
def save(row,directory,key,filename,data):
    path=directory/filename;path.write_bytes(gzip.compress(data,mtime=0));row['evidence_files'][key]=dict(path=str(path.resolve()),sha256=sha(data))
def native_trace(binary,data,page):
    result=subprocess.run([str(binary),str(page)],input=data,capture_output=True,timeout=60)
    if result.returncode or result.stderr or len(result.stdout)>64*1024*1024:raise ValueError('native trace process failure '+result.stderr.decode(errors='replace'))
    value=json.loads(result.stdout)
    if value.get('repaired') is not False or not isinstance(value.get('pages'),list) or len(value['pages'])!=1:raise ValueError('native trace envelope')
    item=value['pages'][0]
    if item.get('page')!=page or item.get('status') not in ('accepted','refused'):raise ValueError('native trace page identity/status')
    if item['status']=='refused' and (not isinstance(item.get('error'),str) or not item['error'].strip()):raise ValueError('native trace empty refusal')
    if item['status']=='accepted' and not isinstance(item.get('events'),list):raise ValueError('native trace missing events')
    return item,result.stdout

def run(payload):
    a,name,page,dpi,previous=payload;data=previous.get('_data')
    if data is None:data=(ENGINE.parent/'fixtures'/name).read_bytes()
    row=dict(file=name,page=page,dpi=dpi,input_sha256=sha(data),previous_status=previous.get('status','supplementary_new_image'),evidence_files={});dst=a.evidence_dir/name;dst.mkdir(parents=True,exist_ok=True)
    if '_data' in previous:
      row['label']=name.removeprefix('generated/');save(row,dst,'pdf',f'{page}-{dpi}.pdf.gz',data)
    try:
      r=subprocess.run([str(a.rust),f'--page-render={page}@{dpi}','-'],input=data,capture_output=True,timeout=60)
      preserved=previous.get('status')=='native_diagnostic_matched';trace=None
      if not preserved:
        trace,trace_bytes=native_trace(a.trace,data,page);row.update(native_trace_status=trace['status'],native_trace_sha256=sha(trace_bytes));save(row,dst,'native_trace',f'{page}-{dpi}.native-trace.json.gz',trace_bytes)
        if trace['status']=='refused':row['native_trace_refusal']=trace['error']
      if r.returncode:
        if r.returncode!=2 or r.stdout or not r.stderr.startswith(b'pdf-inspect: ') or not r.stderr[len(b'pdf-inspect: '):].strip():raise ValueError('unexpected Rust process failure')
        if preserved:raise ValueError('previously accepted page now refuses')
        row.update(status='refused',refusal=r.stderr.decode(errors='replace'));return row
      if r.stderr:raise ValueError('Rust success with diagnostic')
      w,h,pixels=P.ppm(r.stdout);row.update(width=w,height=h,rust_ppm_sha256=sha(r.stdout));save(row,dst,'rust',f'{page}-{dpi}.rust.ppm.gz',r.stdout)
      if preserved and row['rust_ppm_sha256']!=previous['rust_ppm_sha256']:raise ValueError('R22 accepted pixel hash changed')
      if not preserved:
        if trace['status']!='accepted':raise ValueError('native trace refused newly accepted Rust page')
        from image_corpus_model import certify
        expected,boundary,stats=certify(trace,w,h,dpi,source_pdf=data);bounds=stats.pop('_channel_bounds')
        if len(expected)!=w*h*3 or len(bounds)!=len(expected) or len(boundary)!=w*h:raise ValueError('model certificate extent')
        exact=violations=maximum=0
        for actual,want,interval in zip(pixels,expected,bounds):
          if len(interval)!=2 or any(not isinstance(v,int) for v in interval):raise ValueError('model interval shape')
          lo,hi=interval
          if not 0<=lo<=want<=hi<=255:raise ValueError('model interval range')
          exact+=lo==hi;delta=max(lo-actual,actual-hi,0);violations+=delta!=0;maximum=max(maximum,delta)
        images=stats['images'];checks=stats['source_ctm_checks']
        if images<1 or checks!=images:raise ValueError('model source image CTM coverage')
        row['model_certificate']=dict(pixels=w*h,exact_channels=exact,bounded_channels=len(expected)-exact,violating_channels=violations,max_violation=maximum,images=images,source_ctm_checks=checks)
        row['model_stats']=stats
        if violations:raise ValueError('new image pixels violate independent certificate')
        bounds_bytes=(json.dumps(bounds,separators=(',',':'))+'\n').encode();row.update(model_rgb_sha256=sha(expected),model_bounds_sha256=sha(bounds_bytes));save(row,dst,'model_rgb',f'{page}-{dpi}.model.rgb.gz',expected);save(row,dst,'model_bounds',f'{page}-{dpi}.model.bounds.json.gz',bounds_bytes)
      n=subprocess.run([str(a.native),str(page),str(dpi*4)],input=data,capture_output=True,timeout=60)
      if n.returncode or n.stderr:raise ValueError('native original-page failure '+n.stderr.decode(errors='replace'))
      nw,nh,native=P.ppm(n.stdout)
      if (nw,nh)!=(w*4,h*4):raise ValueError('native/Rust original-page dimensions differ')
      resolved=resolve(native,nw,nh);row.update(native_4x_ppm_sha256=sha(n.stdout),native_resolved_sha256=sha(resolved),native_raw_comparison=P.compare(pixels,resolved),status='preserved_r22' if preserved else 'new_image_render');save(row,dst,'native4x',f'{page}-{dpi}.native4x.ppm.gz',n.stdout)
    except (ValueError,KeyError,IndexError,subprocess.TimeoutExpired) as e:row.update(status='failure',error=str(e))
    return row

def main():
    ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--rust',type=Path,required=True);ap.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-image-page-oracle'));ap.add_argument('--trace',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-image-trace-oracle'));ap.add_argument('--report',type=Path,required=True);ap.add_argument('--evidence-dir',type=Path,required=True);ap.add_argument('--workers',type=int,default=4);a=ap.parse_args();start=time.monotonic()
    baseline=ENGINE/'reference/form-differential-f0.json';inventory=ENGINE/'reference/fixture-inventory.json';prior=json.loads(baseline.read_text());files=[Path(__file__),a.rust,a.native,a.trace,baseline,inventory,*[ENGINE/'tools'/p for p in ['image_page_model.py','differential-image-pages.py','mupdf-image-page-oracle.c','build-image-reference.sh','mupdf-image-trace-oracle.c','build-image-trace-reference.sh','image_corpus_model.py','page_raster_review_model.py','stroke_review_model.py','image_decode_cases.py','differential-lzw.py','image_native_policy.py','image_page_cases.py']],ENGINE/'reference/image-native-pins.sha256',ENGINE/'reference/image-reference-policy-r23.json',ENGINE/'reference/image-build-r23.json'];hashes={str(p):sha(p.read_bytes()) for p in files}
    for doc in json.loads(inventory.read_text())['documents']:
      if not doc.get('passwordRequired') and sha((ENGINE.parent/'fixtures'/doc['file']).read_bytes())!=doc['sha256']:raise ValueError('fixture hash')
    tasks=[(a,r['file'],r['page'],r['dpi'],r) for r in prior['pixel_rows']]
    if len(tasks)!=620 or sum(p[3]==18 for p in tasks)!=572:raise ValueError('R22 baseline completeness')
    with ProcessPoolExecutor(max_workers=a.workers) as pool:
      rows=list(pool.map(run,tasks));extra=[(a,r['file'],r['page'],72,{}) for r in rows if r['dpi']==18 and r['status']=='new_image_render'];rows.extend(pool.map(run,extra))
      from image_corpus_model import mixed_cases
      generated_rows=list(pool.map(run,[(a,'generated/'+c['label'],1,72,{'_data':c['pdf']}) for c in mixed_cases()]))
    failures=[r for r in rows+generated_rows if r['status']=='failure'];preserved=[r for r in rows if r['status']=='preserved_r22'];census=[r for r in rows if r['dpi']==18 and r['previous_status']=='refused']
    if len(preserved)!=593:failures.append(dict(error='593 R22 successful render hashes not all preserved'))
    if len(census)!=27:failures.append(dict(error='27 old refusal census incomplete'))
    if hashes!={str(p):sha(p.read_bytes()) for p in files}:raise ValueError('gate inputs changed during execution')
    count=lambda dpi:dict(collections.Counter(r['status'] for r in rows if r['dpi']==dpi))
    report=dict(status='failed' if failures else 'passed',scope='complete original-page classification, R22 hash preservation, mandatory independent new-image pixel certificates and raw native diagnostics',elapsed_seconds=time.monotonic()-start,evidence_dir=str(a.evidence_dir.resolve()),evidence_sha256=hashes,preserved_r22_hashes=len(preserved),corpus_18dpi=count(18),corpus_72dpi=count(72),prior_refusal_census=census,generated_model_rows=generated_rows,pixel_rows=rows,failures=failures)
    a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:report[k] for k in ['status','elapsed_seconds','preserved_r22_hashes','corpus_18dpi','corpus_72dpi']}));raise SystemExit(bool(failures))
if __name__=='__main__':main()
