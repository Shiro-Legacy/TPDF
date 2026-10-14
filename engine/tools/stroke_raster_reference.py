#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""R21 native user-space strokes + full saved affine transform; no Rust outlines."""
import math,struct,subprocess
from page_raster_reference import parse_ppm,box4,num,point,commands_pdf,differences,boundary_mask as fill_boundary_mask

def events_from_native(page):
    if page['status']!='accepted':raise ValueError(page['error'])
    events=[]
    for event in page['events']:
        kind=event['kind']
        if kind=='glyph':
            if event['mode']==3:continue
            if event['mode']!=0:raise ValueError('unsupported native text mode')
            event=dict(kind='fill',path=event['unhinted_commands'],matrix=[1,0,0,1,0,0],color=page['contexts'][event['context']]['color'],rule='nonzero')
        if event['kind'] in ('fill','stroke') and event['color']['space'] not in ('gray','rgb'):raise ValueError('painted native color outside raster gate')
        if event['kind']=='stroke':
            s=event['style']
            if s['width']<=0 or s['dash'] or not(s['start_cap']==s['end_cap']==s['dash_cap']) or s['start_cap'] not in (0,1,2) or s['join'] not in (0,1,2) or s['miter']<1:raise ValueError('native stroke style outside solid positive-width gate')
        events.append(event)
    return events

def geometry(page,scale=1):
    events=events_from_native(page);target=[scale,0,0,scale,0,0];x0,y0,x1,y1=page['initial_clip'];clip=[['m',x0,y0],['l',x1,y0],['l',x1,y1],['l',x0,y1],['h']]
    out=['q',*commands_pdf(clip,target),'W n'];depth=1
    for event in events:
        kind=event['kind']
        if kind=='pop':
            if depth<=1:raise ValueError('native clip stack underflow')
            depth-=1;out.append('Q');continue
        if kind=='clip':out.append('q');depth+=1
        matrix=[scale*v for v in event['matrix']]
        if kind in ('fill','stroke'):
            color=event['color'];op=('g' if color['space']=='gray' else 'rg');op=op.upper() if kind=='stroke' else op
            out.append(' '.join([*(num(v) for v in color['values']),op]))
        if kind=='stroke':
            s=event['style'];out.extend(['q',' '.join([*(num(v) for v in matrix),'cm']),num(s['width'])+' w',str(s['start_cap'])+' J',str(s['join'])+' j',num(s['miter'])+' M',*commands_pdf(event['path']),'S','Q'])
        else:
            out.extend(commands_pdf(event['path'],matrix))
            if kind=='clip':out.append('W* n' if event['rule']=='evenodd' else 'W n')
            elif kind=='fill':out.append('f*' if event['rule']=='evenodd' else 'f')
            else:raise ValueError('unexpected native event')
    if depth-1!=page['cleanup_pops']:raise ValueError('native cleanup depth differs')
    out.extend('Q' for _ in range(depth));return ('\n'.join(out)+'\n').encode()

def f32(value):return struct.unpack('f',struct.pack('f',value))[0]
def native_width_policy(page,dpi=72,aa=0,factor=4):
    rows=[]
    for index,event in enumerate(events_from_native(page)):
        if event['kind']!='stroke':continue
        scale=dpi/72*factor;a,b,c,d,_,_=map(lambda v:f32(v*scale),event['matrix']);det=f32(f32(a*d)-f32(b*c));expansion=f32(math.sqrt(abs(det)))
        if expansion<2**-23:expansion=1
        threshold=f32(2/(aa+2));width=f32(event['style']['width']);effective=width
        if f32(width*expansion)<threshold:effective=f32(threshold/expansion)
        flatness=max(f32(.001),f32(f32(.3)/expansion))
        rows.append(dict(event=index,declared_user_width=width,native_effective_user_width=effective,clamped=effective!=width,native_expansion=expansion,minimum_native_device_width=threshold,native_user_flatness=flatness,aa=aa,factor=factor))
    return rows

def render_native(binary,page,w,h,dpi=72,aa=0,factor=4):
    source=geometry(page,dpi/72*factor);p=subprocess.run([str(binary),str(w*factor),str(h*factor),str(aa)],input=source,capture_output=True,timeout=50)
    if p.returncode or p.stderr:raise ValueError(f'native stroke render exit{p.returncode}: '+p.stderr.decode(errors='replace')[:1500])
    nw,nh,rgb=parse_ppm(p.stdout)
    if (nw,nh)!=(w*factor,h*factor):raise ValueError('native raster dimensions')
    return box4(rgb,w,h) if factor==4 else rgb,source

# The immutable certified masks amortize the identical stress paths without
# accepting a pixel or bypassing a per-page output comparison.
from functools import lru_cache
from fractions import Fraction as F
from stroke_review_model import StrokeModel,Budget

@lru_cache(maxsize=16)
def stroke_band(path,matrix,width,cap,join,miter,w,h,margin):
    if w*h>2_000_000:raise ValueError('stroke reference mask budget')
    model=StrokeModel(path,width,cap=['butt','round','square'][cap],join=['miter','round','bevel'][join],miter_limit=miter,matrix=matrix,budget=Budget(max_work=64_000_000,max_nodes=200_000,max_queries=200_000))
    # Conservative hull + full miter radius bounds query work; False outside
    # this envelope follows directly from affine transformation of that hull.
    xs=[];ys=[]
    for command in path:
        for i in range(1,len(command),2):
            x,y=point(command[i],command[i+1],matrix);xs.append(x);ys.append(y)
    norm=max(abs(matrix[0])+abs(matrix[2]),abs(matrix[1])+abs(matrix[3]));r=float(width)/2*max(2,float(miter) if join==0 else 2)*norm+float(margin)
    x0=max(0,math.floor(min(xs)-r-1));x1=min(w,math.ceil(max(xs)+r+1));y0=max(0,math.floor(min(ys)-r-1));y1=min(h,math.ceil(max(ys)+r+1));mask=bytearray(w*h)
    for y in range(y0,y1):
        for x in range(x0,x1):
            if model.contains((F(2*x+1,2),F(2*y+1,2)),F(margin)) is None:mask[y*w+x]=1
    return bytes(mask),(model.b.work,model.b.nodes,model.b.queries)

def diagnostic_band(page,w,h,dpi=72):
    events=events_from_native(page);scale=dpi/72;fills=[];stroke_masks=[];counts=dict(stroke_uncertain_pixels=0,stroke_model_work=0,stroke_model_nodes=0,stroke_model_queries=0)
    policies={row['event']:row for row in native_width_policy(page,dpi=dpi)};aa_policies={row['event']:row for row in native_width_policy(page,dpi=dpi,aa=8,factor=1)}
    for index,event in enumerate(events):
        kind=event['kind']
        if kind=='pop':continue
        if kind!='stroke':fills.append(dict(event,matrix=[v*scale for v in event['matrix']]));continue
        s=event['style'];matrix=tuple(v*scale for v in event['matrix']);norm=max(abs(matrix[0])+abs(matrix[2]),abs(matrix[1])+abs(matrix[3]))
        widths=[policies[index],aa_policies[index]];flat=max(v['native_user_flatness'] for v in widths);extra=max(abs(v['native_effective_user_width']-s['width'])/2 for v in widths)
        # 1px diagnostic distance from a pixel box =1.5px from its center,
        # plus explicit native flattening and minimum-width policy terms.
        margin=F(3,2)+F(norm)*F(flat+extra)
        mask,usage=stroke_band(tuple(tuple(c) for c in event['path']),matrix,s['width'],s['start_cap'],s['join'],s['miter'],w,h,margin);stroke_masks.append(mask)
        counts['stroke_model_work']+=usage[0];counts['stroke_model_nodes']+=usage[1];counts['stroke_model_queries']+=usage[2]
    band=fill_boundary_mask(fills,w,h,initial_bounds=[v*scale for v in page['initial_clip']]);combined=bytearray(w*h)
    for mask in stroke_masks:
        for i,value in enumerate(mask):combined[i]|=value
    counts['stroke_uncertain_pixels']=sum(combined)
    return [v or bool(combined[i]) for i,v in enumerate(band)],counts
