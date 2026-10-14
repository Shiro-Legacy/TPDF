# SPDX-License-Identifier: AGPL-3.0-or-later
"""R22 Form pixel checks, preserving frozen R21 sample/native/color policies."""
import json,math,runpy,subprocess
from pathlib import Path
from fractions import Fraction as F
from form_reference import ENGINE,sha,model_semantics
from stroke_source_color import certificate as color_certificate,compare as color_compare
from form_pixel_model import render as modeled
from stroke_raster_reference import parse_ppm,render_native,diagnostic_band,native_width_policy
D=runpy.run_path(str(ENGINE/'tools/differential-page-raster.py'))
S=runpy.run_path(str(ENGINE/'tools/differential-stroke-raster.py'))
read_native=runpy.run_path(str(ENGINE/'tools/test-form-reference.py'))['read_native']
dimensions=S['dimensions'];compare_model=S['compare_model'];save=S['save']
def corpus_page(payload):
    a,name,page_index,dpi,data,representative=payload;row=dict(file=name,page=page_index,dpi=dpi,stress=name=='large.pdf',representative=representative,input_sha256=sha(data));label='corpus/'+name+'/'+str(page_index)+'-'+str(dpi)
    try:
        ppm,error=D['rust_render'](a.rust,data,page_index,dpi)
        # Native original-page trace is attempted even if Rust refuses. Its explicit
        # refusal remains distinct; crashes cannot hide behind unsupported pages.
        native_run=subprocess.run([str(a.trace),str(page_index)],input=data,capture_output=True,timeout=50)
        page=read_native(native_run,page_index)
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

def model_events(case,assets,dpi):
    events=[];model=case['expected'];codes=iter(model['codes']);scale=F(dpi,72)
    def rgb(value):
        space,values=value['space'],value['values'];out=[int(F(v)*255+F(1,2)) for v in values]
        if space=='gray':out*=3
        if len(out)!=3:raise ValueError('model device color')
        return out
    for original in model_semantics(model):
        kind=original['kind']
        if kind=='pop':events.append(dict(kind='pop'));continue
        if kind=='code':
            code=next(codes)
            if code['render']==3:continue
            font=case['graph']['fonts'][code['font']];name='space' if code['code']==32 else chr(code['code']);canonical=assets[font['face']][name]
            path=[[p[0],*[F(v,1000) for v in p[1:]]] for p in canonical['commands']]
            events.append(dict(kind='fill',rule='nonzero',path=path,matrix=[F(v)*scale for v in code['transform']],color=rgb(dict(space=code['color'][0],values=code['color'][1]))));continue
        event=dict(kind=kind,path=original['path'],matrix=[F(str(v))*scale for v in original['matrix']])
        if kind in ('fill','clip'):event['rule']=original['rule']
        if kind in ('fill','stroke'):event['color']=rgb(original['color'])
        if kind=='stroke':
            st=original['style'];event['style']=dict(width=st['width'],cap=st['start_cap'],join=st['join'],miter=st['miter'])
        events.append(event)
    return events

def generated_pixels(args,case,page,assets,dpi=18):
    ppm,error=D['rust_render'](args.rust,case['pdf'],1,dpi)
    if error:raise ValueError('generated pixel refusal: '+error)
    w,h,pixels=parse_ppm(ppm);dimensions(w,h,page,dpi)
    native,source=render_native(args.native,page,w,h,dpi=dpi);aa,_=render_native(args.native,page,w,h,dpi=dpi,aa=8,factor=1);band,counts=diagnostic_band(page,w,h,dpi)
    result=dict(dpi=dpi,width=w,height=h,rust_ppm_sha256=sha(ppm),native_4x_sha256=sha(native),native_aa8_sha256=sha(aa),native_band_model_counts=counts,native_4x=color_compare(pixels,native,w,h,band),native_aa8=color_compare(pixels,aa,w,h,band))
    if result['native_4x']['unexplained_outside_boundary'] or result['native_aa8']['unexplained_outside_boundary']:raise ValueError('generated native pixel difference outside fixed band')
    if case['scope']=='clip-restore-stroke' and dpi==72 or case['label'] in ('affine-0-0','affine-1-1') and dpi==18:
        expected,uncertain,stats=modeled(model_events(case,assets,dpi),w,h)
        bounds=stats.pop('_channel_bounds');result['model_comparison']=compare_model(pixels,expected,w,h,uncertain,bounds);result['model_counts']=stats
    label='generated/'+case['label']+'/'+str(dpi)
    for kind,data in [('rust.ppm',ppm),('native-4x.rgb',native),('native-aa8.rgb',aa),('native-geometry.content',source)]:save(args.evidence_dir,label,kind,data)
    return result
