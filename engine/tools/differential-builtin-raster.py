#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""R20 corpus render coverage and small independently certified pinned CFF sample cases."""
import argparse,collections,hashlib,json,runpy,sys,subprocess
from pathlib import Path
from fractions import Fraction as F
from page_outlines_model import pdf
from page_raster_reference import parse_ppm,render_native,boundary_mask,differences
from page_raster_review_model import render as modeled
ENGINE=Path(__file__).resolve().parent.parent
D=runpy.run_path(str(ENGINE/'tools/differential-page-raster.py'))
O=runpy.run_path(str(ENGINE/'tools/differential-page-outlines.py'))
def sha(b):return hashlib.sha256(b).hexdigest()
def cases(assets):
    for i in range(16):
        face=['Helvetica','Times-Roman'][i%2];name=['A','B','C','D','O','S','space','j'][i//2];glyph=assets[face][name];size=[F(4),F(6)][i%2];x=F(9,8);y=F(9,8);color=[0,0,0] if i%3==0 else [255,0,0]
        font=dict(face=face,names={65:name},widths={65:250},mapping={65:'\U0001f642'})
        raw=f'{color[0]/255:g} {color[1]/255:g} {color[2]/255:g} rg BT /F0 {size} Tf 1 0 0 1 {float(x)} {float(y)} Tm (A) Tj ET'.encode()
        events=[dict(kind='fill',path=glyph['commands'],matrix=[size/1000,0,0,-size/1000,x,8-y],rule='nonzero',color=color)]
        yield dict(index=i,face=face,name=name,width=8,height=8,pdf=pdf(raw,{'F0':font},crop=(0,0,8,8)),events=events)

def corpus_page(payload):
    a,name,index,data=payload;row=dict(file=name,page=index,stress=name=='large.pdf');label='corpus/'+name+'/'+str(index)
    try:
        ppm,error=D['rust_render'](a.rust,data,index)
        if error:row.update(status='refused',refusal=error);return row
        w,h,pixels=parse_ppm(ppm);row.update(width=w,height=h,rust_ppm_sha256=sha(ppm));D['save_evidence'](a.evidence_dir,label,'rust.ppm',ppm)
        page,raw=D['native_trace'](a.trace,data,index)
        if page['status']!='accepted':raise ValueError('native refused accepted raster page')
        row['native_trace_sha256']=sha(raw);D['save_evidence'](a.evidence_dir,label,'native-trace.json',raw)
        native,source=render_native(a.native,page,w,h);D['save_evidence'](a.evidence_dir,label,'native-4x.rgb',native);row['native_4x_sha256']=sha(native)
        events=D['model_events_from_native'](page);band=boundary_mask(events,w,h,initial_bounds=page['initial_clip']);row['native_4x']=differences(pixels,native,w,h,band)
        aa,_=render_native(a.native,page,w,h,aa=8,factor=1);D['save_evidence'](a.evidence_dir,label,'native-aa8.rgb',aa);row['native_aa8_sha256']=sha(aa);row['native_aa8']=differences(pixels,aa,w,h,band)
        if row['native_4x']['different_pixels_outside_boundary'] or row['native_aa8']['different_pixels_outside_boundary']:raise ValueError('native fixture differences outside boundary band')
        row['status']='native_boundary_diagnostic'
    except (ValueError,KeyError,IndexError,subprocess.TimeoutExpired) as error:row.update(status='failure',error=str(error))
    return row

def corpus_gate(a):
    from concurrent.futures import ProcessPoolExecutor
    tasks=[];docs=[]
    for doc in json.loads((ENGINE/'reference/fixture-inventory.json').read_text())['documents']:
        if doc.get('passwordRequired'):continue
        data=(ENGINE.parent/'fixtures'/doc['file']).read_bytes()
        if sha(data)!=doc['sha256']:raise ValueError('fixture hash')
        docs.append(dict(file=doc['file'],sha256=doc['sha256']));tasks.extend((a,doc['file'],i+1,data) for i in range(doc['pages']))
    with ProcessPoolExecutor(max_workers=4) as pool:rows=list(pool.map(corpus_page,tasks))
    return rows,docs

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--rust',type=Path,required=True);p.add_argument('--trace',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-page-outlines-oracle'));p.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-page-raster-oracle'));p.add_argument('--asset-evidence',type=Path,default=Path('/private/tmp/tandem-r20-builtin-evidence'));p.add_argument('--evidence-dir',type=Path);p.add_argument('--report',type=Path,required=True);p.add_argument('--skip-corpus',action='store_true');a=p.parse_args()
    paths=[Path(__file__),a.rust,a.trace,a.native]+[ENGINE/'tools'/v for v in ['page_outlines_model.py','differential-page-raster.py','page_raster_reference.py','page_raster_review_model.py','mupdf-page-outlines-oracle.c','mupdf-page-raster-oracle.c']]+[a.asset_evidence/(face+'-native.json.gz') for face in ('Helvetica','Times-Roman')];hashes={str(v):sha(v.read_bytes()) for v in paths};rows=[]
    for case in cases(O['load_assets'](a.asset_evidence)):
        row={k:case[k] for k in ('index','face','name','width','height')};w=h=8;label='cff/'+str(case['index'])
        try:
            ppm,error=D['rust_render'](a.rust,case['pdf'])
            if error:raise ValueError(error)
            rw,rh,pixels=parse_ppm(ppm)
            if (rw,rh)!=(w,h):raise ValueError('dimensions')
            expected,uncertain,counts=modeled(case['events'],w,h,initial_bounds=[0,0,w,h]);row['model_counts']=counts;row['model_comparison']=D['compare_pixels'](pixels,expected,w,h,uncertain)
            page,raw=D['native_trace'](a.trace,case['pdf']);band=boundary_mask(case['events'],w,h,initial_bounds=[0,0,w,h]);native,source=render_native(a.native,page,w,h);aa,_=render_native(a.native,page,w,h,aa=8,factor=1)
            row['native_4x']=differences(pixels,native,w,h,band);row['native_aa8']=differences(pixels,aa,w,h,band)
            if row['native_4x']['different_pixels_outside_boundary'] or row['native_aa8']['different_pixels_outside_boundary']:raise ValueError('native pixels outside declared band')
            row.update(status='certified_outside_approximation_band' if row['model_comparison']['different_pixels'] else 'matched',input_sha256=sha(case['pdf']),rust_ppm_sha256=sha(ppm),model_pixel_sha256=sha(expected),native_trace_sha256=sha(raw),native_4x_sha256=sha(native),native_aa8_sha256=sha(aa))
            for kind,b in [('rust.ppm',ppm),('model.rgb',expected),('native-trace.json',raw),('native-4x.rgb',native),('native-aa8.rgb',aa)]:D['save_evidence'](a.evidence_dir,label,kind,b)
        except (ValueError,KeyError,IndexError) as e:row.update(status='failure',error=str(e))
        rows.append(row)
    corpus,docs=([],[]) if a.skip_corpus else corpus_gate(a)
    if corpus:
        previous=json.loads((ENGINE/'reference/page-raster-differential-r1.json').read_text());baseline={(r['file'],r['page']) for r in previous['rows'] if r['status']=='native_boundary_diagnostic'};D['verify_baseline'](corpus,baseline)
    def total(pop):return dict(pages=len(pop),statuses=dict(collections.Counter(r['status'] for r in pop)),pixels=sum(r.get('width',0)*r.get('height',0) for r in pop),model_uncertain_pixels=sum(r.get('model_counts',{}).get('uncertain_pixels',0) for r in pop),model_different_pixels=sum(r.get('model_comparison',{}).get('different_pixels',0) for r in pop),model_outside_band_differences=sum(r.get('model_comparison',{}).get('different_pixels_outside_boundary',0) or 0 for r in pop),native_4x_different_pixels=sum(r.get('native_4x',{}).get('different_pixels',0) for r in pop),native_aa8_different_pixels=sum(r.get('native_aa8',{}).get('different_pixels',0) for r in pop),native_outside_band_differences=sum((r.get(k,{}).get('different_pixels_outside_boundary',0) or 0) for r in pop for k in ('native_4x','native_aa8')))
    if hashes!={str(v):sha(v.read_bytes()) for v in paths}:raise ValueError('raster source changed')
    report=dict(evidence_sha256=hashes,generated=total(rows),corpus=total(corpus),stress=total([r for r in corpus if r['stress']]),generated_rows=rows,rows=corpus,documents=docs,limitations=['No native hinted text pixel parity: native raster consumes independently extracted unhinted drawing-origin geometry.','The independent Bernstein model certifies pixels outside its predeclared1/32px curve and1/256px mixed-line sample boundary uncertainty.','Native4xAA0 and nativeAA8 are diagnostics: differences allowed only in predeclared1px L-infinity boundary band with1/4096px curve enclosure.','Corpus reports all572readable pages and every atomic refusal at72DPI; accepted fixtures receive native unhinted geometry diagnostics, not the expensive per-sample Bernstein model.','R19 controlled orthogonal exact native raster gate remains separate and unchanged.'])
    a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:report[k] for k in ('generated','corpus','stress')}))
    if any(r['status']=='failure' for r in rows+corpus):raise SystemExit(1)
if __name__=='__main__':main()
