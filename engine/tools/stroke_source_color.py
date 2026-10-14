#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Narrow exact source-color certificate for the pinned self-authored stress PDF.

This is a structural fixture oracle, not a general PDF parser. It never derives
an allowed color or region from Rust/native mismatching pixel values.
"""
import hashlib,re,struct,math
from fractions import Fraction as F
from functools import lru_cache
from stroke_review_model import StrokeModel,Budget
from stroke_raster_reference import differences,native_width_policy
PIN='9f08ccad4f8b7e511227f7d25928727b150f42d1aa1b378bf33ffd3d379dcde9'
IDENTITY_PAGE=(1,0,0,-1,0,792)
RECT=(200,160,160,36)
COLORS=(F(9,10),F(4,10),F(2,10))
CURVE=(('m',72,60),('c',200,200,400,200,540,60))

def f32(x):return struct.unpack('f',struct.pack('f',float(x)))[0]
def color_pair():return tuple(math.floor(v*255+F(1,2)) for v in COLORS),tuple(int(f32(f32(v)*255)) for v in COLORS)

def operations(data):
    if len(data)>4096:raise ValueError('stress content bound')
    out=[];args=[];i=0
    while i<len(data):
        if data[i] in b' \t\r\n':i+=1;continue
        if data[i]==40:
            start=i;i+=1;depth=1
            while i<len(data) and depth:
                v=data[i];i+=1
                if v==92:
                    if i==len(data):raise ValueError('truncated source escape')
                    i+=1
                elif v==40:depth+=1
                elif v==41:depth-=1
                if depth>8:raise ValueError('source literal depth')
            if depth:raise ValueError('unclosed source literal')
            args.append(('string',data[start:i]));continue
        if data[i]==47:
            m=re.match(rb'/[A-Za-z0-9]+',data[i:])
            if not m:raise ValueError('source font name')
            args.append(('name',m[0]));i+=len(m[0]);continue
        m=re.match(rb'[+-]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)',data[i:])
        if m:args.append(F(m[0].decode()));i+=len(m[0]);continue
        m=re.match(rb'[A-Za-z]+',data[i:])
        if not m:raise ValueError('outside exact source grammar')
        out.append((m[0].decode(),tuple(args)));args=[];i+=len(m[0])
        if len(out)>128:raise ValueError('source operation bound')
    if args:raise ValueError('source trailing operands')
    return out

def parse_content(data):
    ops=operations(data);cut=next((i for i,(op,args) in enumerate(ops) if op=='RG'),None)
    if cut is None:raise ValueError('missing stress tail')
    inside=False
    for op,args in ops[:cut]:
        if op=='BT' and not args and not inside:inside=True
        elif op=='ET' and not args and inside:inside=False
        elif op=='Tf' and inside and len(args)==2 and args[0]==('name',b'/F1') and isinstance(args[1],F):pass
        elif op in ('Tm','Td') and inside and len(args)==(6 if op=='Tm' else 2) and all(isinstance(v,F) for v in args):pass
        elif op=='Tj' and inside and len(args)==1 and isinstance(args[0],tuple) and args[0][0]=='string':pass
        else:raise ValueError('non-text prefix or invalid text state')
    if inside:raise ValueError('unclosed text before stress tail')
    expected=[('RG',(F(2,10),F(4,10),F(9,10))),('w',(F(3,2),)),('m',(F(72),F(120))),('l',(F(540),F(120))),('S',()),('rg',COLORS),('re',tuple(map(F,RECT))),('f',()),('RG',(F(1,10),F(6,10),F(3,10))),('w',(F(1),)),('m',(F(72),F(60))),('c',tuple(map(F,[200,200,400,200,540,60]))),('S',())]
    if ops[cut:]!=expected:raise ValueError('source color/rectangle/stroke/order differs')
    return dict(source_content_sha256=hashlib.sha256(data).hexdigest(),rectangle=list(RECT),source_color=[str(v) for v in COLORS],rust_color=list(color_pair()[0]),native_color=list(color_pair()[1]),prefix_operations=cut,tail_operations=len(expected))

@lru_cache(maxsize=1)
def contents(data):
    if hashlib.sha256(data).hexdigest()!=PIN:raise ValueError('stress source input hash')
    pairs=re.findall(rb'(?m)^([0-9]+) 0 obj\n(.*?)\nendobj\n',data,re.S)
    objects={int(key):body for key,body in pairs}
    if len(objects)!=len(pairs) or len(objects)!=1003:raise ValueError('stress indirect object structure')
    kids=re.fullmatch(rb'<< /Type /Pages /Kids \[(.*?)\] /Count 500 >>',objects[2])
    if not kids:raise ValueError('stress page tree structure')
    refs=re.findall(rb'([0-9]+) 0 R',kids[1])
    if len(refs)!=500 or b' '.join(v+b' 0 R' for v in refs)!=kids[1]:raise ValueError('stress page ordering')
    out=[]
    for ref in refs:
        page=re.fullmatch(rb'<< /Type /Page /Parent 2 0 R /MediaBox \[0 0 612 792\] /Resources << /Font << /F1 3 0 R >> >> /Contents ([0-9]+) 0 R >>',objects[int(ref)])
        if not page:raise ValueError('stress page resource/geometry/contents structure')
        stream=re.fullmatch(rb'<< /Length ([0-9]+) >>\nstream\n(.*)endstream',objects[int(page[1])],re.S)
        if not stream or len(stream[2])!=int(stream[1]):raise ValueError('stress stream extent')
        out.append(stream[2])
    return tuple(out)

@lru_cache(maxsize=2)
def interior_mask(w,h,dpi,margin):
    scale=F(dpi,72)
    if (w,h)!=(math.ceil(612*scale),math.ceil(792*scale)) or dpi not in (18,72):raise ValueError('stress certificate canvas')
    # Every queried pixel is entirely inside the exact opaque rectangle.
    # False with1.5devicepixel margin certifies exclusion from the only later
    # stroke, including pixel extent and the existing production error allowance.
    model=StrokeModel(CURVE,1,cap='butt',join='miter',miter_limit=10,matrix=(scale,0,0,-scale,0,792*scale),budget=Budget(max_work=32_000_000,max_nodes=200_000,max_queries=100_000))
    x0=200*scale;x1=360*scale;y0=(792-196)*scale;y1=(792-160)*scale;mask=bytearray(w*h)
    for y in range(math.floor(y0),math.ceil(y1)):
        for x in range(math.floor(x0),math.ceil(x1)):
            if x0<x and x+1<x1 and y0<y and y+1<y1 and model.contains((F(2*x+1,2),F(2*y+1,2)),margin) is False:mask[y*w+x]=1
    return bytes(mask),dict(work=model.b.work,nodes=model.b.nodes,queries=model.b.queries,certified_interior_pixels=sum(mask))

def certificate(data,page,w,h,dpi):
    if not 1<=page['page']<=500:raise ValueError('stress certificate page')
    proof=parse_content(contents(data)[page['page']-1])
    if tuple(page['initial_ctm'])!=IDENTITY_PAGE or page['initial_clip']!=[0,0,612,792] or page['cleanup_pops']:raise ValueError('native source geometry/clip differs')
    paths=[v for v in page['events'] if v['kind']!='glyph']
    if [v['kind'] for v in paths]!=['stroke','fill','stroke'] or any(v['clip_ids'] for v in paths):raise ValueError('native path order/clip differs')
    fill=paths[1];curve=paths[2]
    if fill['path']!=[['m',200,160],['l',360,160],['l',360,196],['l',200,196],['h']] or fill['rule']!='nonzero' or tuple(fill['matrix'])!=IDENTITY_PAGE:raise ValueError('native rectangle differs')
    if fill['color']['space']!='rgb' or [f32(v) for v in fill['color']['values']]!=[f32(v) for v in COLORS]:raise ValueError('native rectangle color differs')
    if tuple(tuple(v) for v in curve['path'])!=CURVE or tuple(curve['matrix'])!=IDENTITY_PAGE or curve['style']!={'width':1,'start_cap':0,'dash_cap':0,'end_cap':0,'join':0,'miter':10,'phase':0,'dash':[]}:raise ValueError('native later stroke differs')
    policy=[native_width_policy(page,dpi,aa=aa,factor=factor)[-1] for aa,factor in ((0,4),(8,1))]
    margin=F(3,2)+F(dpi,72)*F(max(v['native_user_flatness']+abs(v['native_effective_user_width']-1)/2 for v in policy))
    mask,usage=interior_mask(w,h,dpi,margin);proof.update(usage,exclusion_margin=str(margin));return mask,proof

def compare(actual,native,w,h,band,mask=None):
    stats=differences(actual,native,w,h,band);pair=color_pair();certified=0;unexpected=0;verified=0;violations=0
    for i in range(w*h):
        a=actual[3*i:3*i+3];n=native[3*i:3*i+3]
        if mask is not None and mask[i]:
            if tuple(a)!=pair[0] or tuple(n)!=pair[1]:unexpected+=1;violations+=1
            else:
                verified+=1
                if not band[i]:certified+=1
        elif a!=n and not band[i]:unexpected+=1
    stats.update(source_color_policy_certified_pixels=certified,source_color_policy_verified_pixels=verified,source_color_proof_violations=violations,unexplained_outside_boundary=unexpected)
    return stats
