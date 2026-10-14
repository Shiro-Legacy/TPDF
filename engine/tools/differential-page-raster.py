#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""R19 independent fixed-sample pixels and explicitly scoped native diagnostics."""
import argparse,collections,gzip,hashlib,json,math,subprocess,runpy
from pathlib import Path
from fractions import Fraction as F
from page_raster_cases import controlled,lines,curves
from page_raster_reference import parse_ppm,render_native,differences,boundary_mask,events_from_native
from page_raster_review_model import render as modeled
ENGINE=Path(__file__).resolve().parent.parent

def sha(data):return hashlib.sha256(data).hexdigest()
def save_evidence(root,label,kind,data):
    if root is None:return
    p=root/label/(kind+'.gz');p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(gzip.compress(data,mtime=0))

def native_trace(binary,data,page=1):
    p=subprocess.run([str(binary),str(page)],input=data,capture_output=True,timeout=50)
    if p.returncode or p.stderr:raise ValueError('native trace: '+p.stderr.decode(errors='replace')[:1000])
    result=json.loads(p.stdout)
    if result['repaired'] or len(result['pages'])!=1 or result['pages'][0]['page']!=page:raise ValueError('native trace envelope')
    return result['pages'][0],p.stdout

def rust_render(binary,data,page=1,dpi=72):
    p=subprocess.run([str(binary),f'--page-render={page}@{dpi}','-'],input=data,capture_output=True,timeout=60)
    if p.returncode:
        if p.stdout:raise ValueError('Rust failed render published partial output')
        if p.returncode!=2 or not p.stderr.startswith(b'pdf-inspect: ') or not p.stderr[len(b'pdf-inspect: '):].strip():raise ValueError(f'unexpected Rust failure exit{p.returncode}: '+p.stderr.decode(errors='replace')[:1000])
        return None,p.stderr.decode(errors='replace')[:3000]
    if p.stderr:raise ValueError('Rust successful render diagnostic: '+p.stderr.decode(errors='replace')[:1000])
    return p.stdout,None

def compare_pixels(actual,expected,w,h,uncertain=None):
    stats=differences(actual,expected,w,h,uncertain)
    if stats['different_pixels'] and (uncertain is None or stats['different_pixels_outside_boundary']):raise ValueError('certified/model pixel mismatch: '+str(stats))
    return stats

def model_events_from_native(page,scale=1):
    result=[]
    for e in events_from_native(page):
        if e['kind']=='pop':result.append(dict(kind='pop'));continue
        value={k:e[k] for k in ('kind','path','matrix','rule')};value['matrix']=[v*scale for v in e['matrix']]
        if e['kind']=='fill':
            values=e['color']['values'];values=values*3 if e['color']['space']=='gray' else values;value['color']=[round(v*255) for v in values]
        result.append(value)
    return result

def verify_baseline(rows,expected):
    accepted={(r['file'],r['page']) for r in rows if r['status']=='native_boundary_diagnostic'}
    if not expected<=accepted:raise ValueError('previously eligible R18 pages disappeared from raster gate: '+str(sorted(expected-accepted)))

def generated_gate(a):
    rows=[]
    for case in [*controlled(a.controlled),*lines(a.lines),*curves()]:
        label=case['label'];w=case['width'];h=case['height'];dpi=case.get('dpi',72);row=dict(label=label,scope=case['scope'],width=w,height=h,dpi=dpi,input_sha256=sha(case['pdf']))
        try:
            ppm,error=rust_render(a.rust,case['pdf'],dpi=dpi);
            if error:raise ValueError('generated Rust refusal: '+error)
            rw,rh,pixels=parse_ppm(ppm)
            if (rw,rh)!=(w,h):raise ValueError('generated Rust dimensions')
            row['rust_ppm_sha256']=sha(ppm);save_evidence(a.evidence_dir,label,'rust.ppm',ppm)
            if case['events'] is None:
                expected=case['expected'];uncertain=None;counts=dict(uncertain_pixels=0,pixels=w*h,samples=16*w*h)
            else:expected,uncertain,counts=modeled(case['events'],w,h,snap_lines=case['scope']=='rational-lines')
            if 'analytical' in case and (expected!=case['analytical'] or any(uncertain)):raise ValueError('analytical curve certification failed')
            row['model_counts']=counts;row['model_comparison']=compare_pixels(pixels,expected,w,h,uncertain);row['model_pixel_sha256']=sha(expected)
            page,trace_bytes=native_trace(a.trace,case['pdf']);save_evidence(a.evidence_dir,label,'native-trace.json',trace_bytes);row['native_trace_sha256']=sha(trace_bytes)
            native,source=render_native(a.native,page,w,h,dpi=dpi);save_evidence(a.evidence_dir,label,'native-4x.rgb',native);save_evidence(a.evidence_dir,label,'native-geometry.content',source)
            if case['scope']=='controlled-exact-native':
                compare_pixels(native,expected,w,h);band=boundary_mask(case['events'],w,h,initial_bounds=[0,0,w,h]) if case['events'] else boundary_mask(model_events_from_native(page,dpi/72),w,h,initial_bounds=[v*dpi/72 for v in page['initial_clip']])
            else:band=boundary_mask(case['events'],w,h,initial_bounds=[0,0,w,h])
            row['native_4x']=differences(pixels,native,w,h,band);row['native_4x_sha256']=sha(native)
            aa,source=render_native(a.native,page,w,h,dpi=dpi,aa=8,factor=1);save_evidence(a.evidence_dir,label,'native-aa8.rgb',aa)
            row['native_aa8']=differences(pixels,aa,w,h,band);row['native_aa8_sha256']=sha(aa)
            if band is not None and (row['native_4x']['different_pixels_outside_boundary'] or row['native_aa8']['different_pixels_outside_boundary']):raise ValueError('native diagnostic difference outside declared boundary band')
            row['status']='matched' if not row['model_comparison']['different_pixels'] else 'certified_outside_approximation_band'
        except (ValueError,KeyError,IndexError,subprocess.TimeoutExpired) as e:row.update(status='failure',error=str(e))
        rows.append(row)
    return rows

def corpus_gate(a):
    rows=[];docs=[]
    for doc in json.loads((ENGINE/'reference/fixture-inventory.json').read_text())['documents']:
        if doc.get('passwordRequired'):continue
        data=(ENGINE.parent/'fixtures'/doc['file']).read_bytes();assert sha(data)==doc['sha256'];record=dict(file=doc['file'],sha256=doc['sha256']);docs.append(record)
        for page_index in range(1,doc['pages']+1):
            row=dict(file=doc['file'],page=page_index,stress=doc['file']=='large.pdf')
            try:
                ppm,error=rust_render(a.rust,data,page_index)
                if error:row.update(status='refused',refusal=error);rows.append(row);continue
                w,h,pixels=parse_ppm(ppm);row.update(width=w,height=h,rust_ppm_sha256=sha(ppm));label='corpus/'+doc['file']+'/'+str(page_index);save_evidence(a.evidence_dir,label,'rust.ppm',ppm)
                page,trace_bytes=native_trace(a.trace,data,page_index);row['native_trace_sha256']=sha(trace_bytes);save_evidence(a.evidence_dir,label,'native-trace.json',trace_bytes)
                native,source=render_native(a.native,page,w,h);save_evidence(a.evidence_dir,label,'native-4x.rgb',native);row['native_4x_sha256']=sha(native)
                events=model_events_from_native(page);band=boundary_mask(events,w,h,initial_bounds=page['initial_clip'])
                row['native_4x']=differences(pixels,native,w,h,band)
                aa,_=render_native(a.native,page,w,h,aa=8,factor=1);save_evidence(a.evidence_dir,label,'native-aa8.rgb',aa);row['native_aa8_sha256']=sha(aa);row['native_aa8']=differences(pixels,aa,w,h,band)
                if row['native_4x']['different_pixels_outside_boundary'] or row['native_aa8']['different_pixels_outside_boundary']:raise ValueError('native fixture differences outside boundary band')
                row['status']='native_boundary_diagnostic'
            except (ValueError,KeyError,IndexError,subprocess.TimeoutExpired) as e:row.update(status='failure',error=str(e))
            rows.append(row)
    return rows,docs

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--rust',type=Path,required=True);p.add_argument('--trace',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-page-glyph-oracle'));p.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-page-raster-oracle'));p.add_argument('--report',type=Path,required=True);p.add_argument('--evidence-dir',type=Path);p.add_argument('--controlled',type=int,default=300);p.add_argument('--lines',type=int,default=100);p.add_argument('--skip-corpus',action='store_true');a=p.parse_args()
    if not 0<=a.controlled<=1000 or not 0<=a.lines<=1000:p.error('generated case counts must be0..1000')
    sources=[a.rust,a.trace,a.native,Path(__file__),Path(__file__).with_name('page_raster_reference.py'),Path(__file__).with_name('page_raster_cases.py'),Path(__file__).with_name('page_raster_review_model.py'),ENGINE/'tools/page_glyph_model.py',ENGINE/'tools/outline_model.py',ENGINE/'tools/truetype_cmap_model.py',ENGINE/'tools/differential-raster.py',ENGINE/'tools/check-raster-reference.py',ENGINE/'reference/fixture-inventory.json',ENGINE/'reference/page-glyph-differential-g1.json',ENGINE/'PAGE-RASTER-NATIVE-NOTES.md']
    for name in ['mupdf-page-raster-oracle.c','build-page-raster-reference.sh']:
        if (ENGINE/'tools'/name).exists():sources.append(ENGINE/'tools'/name)
    hashes={str(v):sha(v.read_bytes()) for v in sources}
    generated=generated_gate(a);rows,docs=([],[]) if a.skip_corpus else corpus_gate(a)
    baseline={(r['file'],r['page']) for r in json.loads((ENGINE/'reference/page-glyph-differential-g1.json').read_text())['rows'] if r['status']=='matched'}
    if not a.skip_corpus:verify_baseline(rows,baseline)
    def totals(pop):return dict(pages=len(pop),statuses=dict(collections.Counter(v['status'] for v in pop)),pixels=sum(v.get('width',0)*v.get('height',0) for v in pop),native_4x_different_pixels=sum(v.get('native_4x',{}).get('different_pixels',0) for v in pop),native_aa8_different_pixels=sum(v.get('native_aa8',{}).get('different_pixels',0) for v in pop),model_uncertain_pixels=sum(v.get('model_counts',{}).get('uncertain_pixels',0) for v in pop),model_differences=sum(v.get('model_comparison',{}).get('different_pixels',0) for v in pop),model_outside_band_differences=sum(v.get('model_comparison',{}).get('different_pixels_outside_boundary',0) or 0 for v in pop))
    if hashes!={str(v):sha(v.read_bytes()) for v in sources}:raise ValueError('raster evidence changed during run')
    report=dict(r18_eligible_baseline=[dict(file=f,page=p) for f,p in sorted(baseline)],reference='Independent rational sample model + certified Bernstein curves; native unhinted geometry at4xAA0 and nativeAA8 separately',evidence_sha256=hashes,generated=totals(generated),controlled=totals([v for v in generated if v['scope']=='controlled-exact-native']),lines=totals([v for v in generated if v['scope']=='rational-lines']),curves=totals([v for v in generated if v['scope']=='certified-curves']),corpus=totals(rows),stress=totals([v for v in rows if v['stress']]),documents=docs,generated_rows=generated,rows=rows,limitations=['No native hinted text pixel parity. Native uses independently extracted unhinted glyph geometry.','Controlled orthogonal endpoint colors require exact native4x equality; arbitrary native curves and slants are diagnostic, not silently called exact.','Curve comparison certifies only pixels without samples in the independent1/32px curve or1/256px line uncertainty band; counts are explicit.','Native diagnostic band is1devicepixel L-infinity from pixel box to boundary, conservatively enclosed to1/4096px for curves.','Mean channel error>2 and RMS>12 are predeclared investigation flags, not adjustable acceptance tolerances.'])
    a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:report[k] for k in ['generated','controlled','lines','curves','corpus','stress']}))
    for row in generated+rows:
        if row['status']=='failure':print(row)
    if any(v['status']=='failure' for v in generated+rows):raise SystemExit(1)
if __name__=='__main__':main()
