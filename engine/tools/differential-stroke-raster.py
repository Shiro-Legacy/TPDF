#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""R21 exact/certified true stroke samples and complete bounded native corpus diagnostics."""
import argparse,collections,gzip,hashlib,json,math,runpy,subprocess,time
from pathlib import Path
from fractions import Fraction as F
from concurrent.futures import ProcessPoolExecutor
from stroke_source_color import certificate as color_certificate,compare as color_compare
from stroke_raster_cases import generated
from stroke_raster_model import render as modeled
from stroke_raster_reference import parse_ppm,render_native,diagnostic_band,native_width_policy,differences
ENGINE=Path(__file__).resolve().parent.parent
D=runpy.run_path(str(ENGINE/'tools/differential-page-raster.py'))
def sha(data):return hashlib.sha256(data).hexdigest()
def dimensions(width,height,page,dpi):
    b=page['initial_clip'];expected=(math.ceil((b[2]-b[0])*dpi/72),math.ceil((b[3]-b[1])*dpi/72))
    if (width,height)!=expected:raise ValueError('Rust canvas differs from independent native page bounds')

def compare_model(pixels,expected,w,h,uncertain,bounds):
    stats=differences(pixels,expected,w,h,uncertain)
    if len(bounds)!=len(pixels):raise ValueError('model interval extent')
    violations=sum(not lo<=v<=hi for v,(lo,hi) in zip(pixels,bounds));stats['channel_interval_violations']=violations
    stats['certified_nonwhite_channel_constraints']=sum(hi<255 for lo,hi in bounds)
    if violations:raise ValueError('pixel outside independent certified sample-color interval: '+str(stats))
    return stats

def save(root,label,kind,data):D['save_evidence'](root,label,kind,data)

def generated_case(payload):
    a,case=payload;label=case['label'];w=case['width'];h=case['height'];row=dict(label=label,scope=case['scope'],width=w,height=h,dpi=72,input_sha256=sha(case['pdf']))
    try:
        ppm,error=D['rust_render'](a.rust,case['pdf'])
        if error:raise ValueError('generated Rust refusal: '+error)
        rw,rh,pixels=parse_ppm(ppm)
        if (rw,rh)!=(w,h):raise ValueError('generated Rust extent')
        expected,uncertain,counts=modeled(case['events'],w,h);bounds=counts.pop('_channel_bounds');row['model_counts']=counts;row['model_comparison']=compare_model(pixels,expected,w,h,uncertain,bounds)
        if not counts['certified_inside_stroke_samples']:raise ValueError('generated case lacks certified ink samples')
        page,raw=D['native_trace'](a.trace,case['pdf']);dimensions(w,h,page,72)
        native,source=render_native(a.native,page,w,h);aa,_=render_native(a.native,page,w,h,aa=8,factor=1);band,band_counts=diagnostic_band(page,w,h);row['native_band_model_counts']=band_counts
        row['native_4x']=differences(pixels,native,w,h,band);row['native_aa8']=differences(pixels,aa,w,h,band);row['native_width_4x']=native_width_policy(page);row['native_width_aa8']=native_width_policy(page,aa=8,factor=1)
        if row['native_4x']['different_pixels_outside_boundary'] or row['native_aa8']['different_pixels_outside_boundary']:raise ValueError('native difference outside predeclared diagnostic band')
        if case['scope']=='controlled-exact-native' and (pixels!=expected or native!=expected or any(uncertain)):raise ValueError('controlled exact native/model equality')
        row.update(status='matched' if pixels==expected else 'certified_channel_intervals',rust_ppm_sha256=sha(ppm),model_pixel_sha256=sha(expected),model_bounds_sha256=sha(json.dumps(bounds,separators=(',',':')).encode()),native_trace_sha256=sha(raw),native_4x_sha256=sha(native),native_aa8_sha256=sha(aa))
        for kind,value in [('source.pdf',case['pdf']),('rust.ppm',ppm),('model.rgb',expected),('model-bounds.json',json.dumps(bounds,separators=(',',':')).encode()),('native-trace.json',raw),('native-geometry.content',source),('native-4x.rgb',native),('native-aa8.rgb',aa)]:save(a.evidence_dir,'generated/'+label,kind,value)
    except (ValueError,KeyError,IndexError,subprocess.TimeoutExpired) as error:row.update(status='failure',error=str(error))
    return row

def corpus_page(payload):
    a,name,page_index,dpi,data,representative=payload;row=dict(file=name,page=page_index,dpi=dpi,stress=name=='large.pdf',representative=representative,input_sha256=sha(data));label='corpus/'+name+'/'+str(page_index)+'-'+str(dpi)
    try:
        ppm,error=D['rust_render'](a.rust,data,page_index,dpi)
        # Native original-page trace is attempted even if Rust refuses. Its explicit
        # refusal remains distinct; crashes cannot hide behind unsupported pages.
        native_run=subprocess.run([str(a.trace),str(page_index)],input=data,capture_output=True,timeout=50)
        if native_run.returncode:
            if native_run.returncode!=2 or native_run.stdout or not native_run.stderr.startswith(b'page-outline oracle: ') or not native_run.stderr[len(b'page-outline oracle: '):].strip():raise ValueError(f'unexpected native trace exit{native_run.returncode}')
            page=dict(status='refused',error=native_run.stderr.decode())
        else:
            if native_run.stderr:raise ValueError('native success diagnostic')
            output=json.loads(native_run.stdout)
            if output.get('repaired') or len(output['pages'])!=1 or output['pages'][0]['page']!=page_index:raise ValueError('native envelope')
            page=output['pages'][0]
        if page.get('status') not in ('accepted','refused'):raise ValueError('invalid native page status')
        if page['status']=='refused' and (not isinstance(page.get('error'),str) or not page['error'].strip()):raise ValueError('invalid native refusal diagnostic')
        row['native_status']=page['status']
        if page['status']=='refused':row['native_refusal']=page['error']
        if error:row.update(status='refused',refusal=error);return row
        if page['status']!='accepted':raise ValueError('native refused accepted Rust render')
        w,h,pixels=parse_ppm(ppm);dimensions(w,h,page,dpi);row.update(width=w,height=h,rust_ppm_sha256=sha(ppm),native_trace_sha256=sha(native_run.stdout));save(a.evidence_dir,label,'rust.ppm',ppm);save(a.evidence_dir,label,'native-trace.json',native_run.stdout)
        native,source=render_native(a.native,page,w,h,dpi=dpi);aa,_=render_native(a.native,page,w,h,dpi=dpi,aa=8,factor=1);band,counts=diagnostic_band(page,w,h,dpi)
        color_mask=None
        if name=='large.pdf':color_mask,row['source_color_certificate']=color_certificate(data,page,w,h,dpi)
        row.update(native_band_model_counts=counts,native_4x=color_compare(pixels,native,w,h,band,color_mask),native_aa8=color_compare(pixels,aa,w,h,band,color_mask),native_4x_sha256=sha(native),native_aa8_sha256=sha(aa),native_width_4x=native_width_policy(page,dpi),native_width_aa8=native_width_policy(page,dpi,aa=8,factor=1))
        for kind,value in [('native-4x.rgb',native),('native-aa8.rgb',aa),('native-geometry.content',source)]:save(a.evidence_dir,label,kind,value)
        if row['native_4x']['unexplained_outside_boundary'] or row['native_aa8']['unexplained_outside_boundary']:raise ValueError('unexplained native fixture/color-certificate difference')
        row['status']='native_diagnostic_matched'
    except (ValueError,KeyError,IndexError,subprocess.TimeoutExpired) as error:row.update(status='failure',error=str(error))
    return row

def total(rows):
    result=dict(pages=len(rows),statuses=dict(collections.Counter(r['status'] for r in rows)),pixels=sum(r.get('width',0)*r.get('height',0) for r in rows))
    for key in ['uncertain_pixels','uncertain_samples','certified_inside_stroke_samples','certified_outside_stroke_samples','stroke_queries','stroke_work','stroke_nodes']:result['model_'+key]=sum(r.get('model_counts',{}).get(key,0) for r in rows)
    for key in ['different_pixels','channel_interval_violations','certified_nonwhite_channel_constraints']:result['model_'+key]=sum(r.get('model_comparison',{}).get(key,0) or 0 for r in rows)
    for kind in ['native_4x','native_aa8']:
        for key in ['different_pixels','different_pixels_outside_boundary','source_color_policy_certified_pixels','source_color_policy_verified_pixels','source_color_proof_violations','unexplained_outside_boundary']:result[kind+'_'+key]=sum(r.get(kind,{}).get(key,0) or 0 for r in rows)
    result['native_4x_clamped_events']=sum(v['clamped'] for r in rows for v in r.get('native_width_4x',[]));result['native_aa8_clamped_events']=sum(v['clamped'] for r in rows for v in r.get('native_width_aa8',[]))
    return result

def verify_baseline(rows,baseline):
    actual={(r['file'],r['page'],r['dpi']) for r in rows if r['status']=='native_diagnostic_matched'}
    if not {(f,p,72) for f,p in baseline}<=actual:raise ValueError('R20 72DPI baseline disappeared')
    if not {('large.pdf',p,18) for p in range(1,501)}<=actual:raise ValueError('not all500stress pages rendered and compared')
    if not {('large.pdf',p,72) for p in (1,250,500)}<=actual:raise ValueError('stress72DPI representatives missing')

def main():
    ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--rust',type=Path,required=True);ap.add_argument('--trace',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-page-outlines-oracle'));ap.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-stroke-raster-oracle'));ap.add_argument('--report',type=Path,required=True);ap.add_argument('--evidence-dir',type=Path);ap.add_argument('--skip-corpus',action='store_true');ap.add_argument('--workers',type=int,default=4);ap.add_argument('--generated-limit',type=int,default=0);a=ap.parse_args();start=time.monotonic()
    files=[a.rust,a.trace,a.native,Path(__file__),ENGINE/'reference/fixture-inventory.json',ENGINE/'reference/page-outlines-raster-b0.json',ENGINE/'reference/stroke-raster-policy-s0.json',ENGINE/'reference/stroke-color-policy-s1.json']+[ENGINE/'tools'/n for n in ['stroke_source_color.py','stroke_raster_cases.py','stroke_raster_model.py','stroke_review_model.py','stroke_raster_reference.py','mupdf-stroke-raster-oracle.c','build-stroke-raster-reference.sh','differential-page-raster.py','page_raster_reference.py','page_raster_review_model.py']];hashes={str(p):sha(p.read_bytes()) for p in files};prior=json.loads((ENGINE/'reference/page-outlines-raster-b0.json').read_text());baseline={(r['file'],r['page']) for r in prior['rows'] if r['status']=='native_boundary_diagnostic'}
    cases=list(generated());cases=cases[:a.generated_limit] if a.generated_limit else cases;tasks=[]
    if not a.skip_corpus:
        for doc in json.loads((ENGINE/'reference/fixture-inventory.json').read_text())['documents']:
            if doc.get('passwordRequired'):continue
            data=(ENGINE.parent/'fixtures'/doc['file']).read_bytes()
            if sha(data)!=doc['sha256']:raise ValueError('fixture hash')
            for p in range(1,doc['pages']+1):
                tasks.append((a,doc['file'],p,18,data,False))
                if (doc['file'],p) in baseline or doc['file']=='large.pdf' and p in (1,250,500):tasks.append((a,doc['file'],p,72,data,True))
    with ProcessPoolExecutor(max_workers=a.workers) as pool:
        gen=list(pool.map(generated_case,[(a,c) for c in cases]));rows=list(pool.map(corpus_page,tasks))
    failures=[r for r in gen+rows if r['status']=='failure']
    if rows:
        try:verify_baseline(rows,baseline)
        except ValueError as e:failures.append(dict(status='failure',error=str(e)))
    if hashes!={str(p):sha(p.read_bytes()) for p in files}:raise ValueError('source changed during final stroke gate')
    report=dict(status='passed' if not failures else 'failed',elapsed_seconds=time.monotonic()-start,evidence_sha256=hashes,generated=total(gen),generated_scopes={scope:total([r for r in gen if r['scope']==scope]) for scope in sorted({r['scope'] for r in gen})},corpus_18dpi=total([r for r in rows if r['dpi']==18]),preserved_72dpi=total([r for r in rows if (r['file'],r['page']) in baseline and r['dpi']==72]),stress_18dpi=total([r for r in rows if r['stress'] and r['dpi']==18]),stress_72dpi=total([r for r in rows if r['stress'] and r['dpi']==72]),generated_rows=gen,rows=rows,failures=failures,limitations=['Independent true-stroke sample certificates, not production polygons, define resolved channel bounds. Unknown samples alone contribute0..255 uncertainty.','Native4xAA0 andAA8 are diagnostic with fixed independently certified boundary bands and explicit min-width/flattening policy terms.','Raw outside-band native counts are retained. Exact source-derived round-versus-truncate color pairs on independently proven stress-rectangle interiors are classified separately; no blanket channel tolerance.','Native hinted font pixels are not claimed; glyphs use the independently extracted unhinted geometry.','Stroke boundary certificates may conservatively mark internal union seams; counters retain that uncertainty.','Every corpus18DPI image is independently rendered and compared;43prior images and3stress representatives also receive72DPI checks.'])
    a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:report[k] for k in ['status','elapsed_seconds','generated','corpus_18dpi','stress_18dpi','preserved_72dpi','stress_72dpi']}))
    if failures:raise SystemExit(1)
if __name__=='__main__':main()
