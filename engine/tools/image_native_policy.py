# SPDX-License-Identifier: AGPL-3.0-or-later
"""Source-derived pinned MuPDF f32/gridfit/fixed14 image policy witness.

This is a diagnostic model of native policy, never the Rust pixel oracle.
Coordinates use draw-affine.c PREC=14, truncating coefficient quantization,
float initial texture positions and integer row/span steps. Native clipping
comes from the frozen separately-tested AA0 path raster oracle, never from
full-page output or Rust output. Every output channel must match this witness.
"""
import math,struct,subprocess
from fractions import Fraction as F
from image_page_model import prepare,color_at
FROZEN_CLIP='/private/tmp/tandem-mupdf-reference/mupdf-stroke-raster-oracle'
def f32(x):return struct.unpack('f',struct.pack('f',float(x)))[0]
def add(a,b):return f32(f32(a)+f32(b))
def mul(a,b):return f32(f32(a)*f32(b))
def gridfit(m):
    a,b,c,d,e,f=m
    # draw-affine.c:3727, ordinary (non-tiled) gridfit with MY_EPSILON=.001f.
    def fit(size,offset):
        start=float(int(offset))
        if size>0:
            if f32(start-offset)>f32(.001):start=f32(start-1)
            size=add(size,f32(offset-start));offset=start;end=float(int(size))
            if f32(size-end)>f32(.001):end=f32(end+1)
        elif size<0:
            if f32(offset-start)>f32(.001):start=f32(start+1)
            size=add(size,f32(offset-start));offset=start;end=float(int(size))
            if f32(end-size)>f32(.001):end=f32(end-1)
        else:return size,offset
        return end,offset
    if abs(b)<2**-23 and abs(c)<2**-23:a,e=fit(a,e);d,f=fit(d,f)
    elif abs(a)<2**-23 and abs(d)<2**-23:b,f=fit(b,f);c,e=fit(c,e)
    return [a,b,c,d,e,f]
def native_image(event,width,height):
    a,b,c,d,e,f=map(f32,event['matrix'])
    # pdf-op-run.c:918 turns PDF bottom-up image unit coordinates into the
    # top-down native stored-sample coordinates before device fill_image.
    original=[mul(4,a),mul(4,b),mul(-4,c),mul(-4,d),mul(4,add(c,e)),mul(4,add(d,f))]
    m=gridfit(original);a,b,c,d,e,f=m
    corners=[(e,f),(add(a,e),add(b,f)),(add(add(a,c),e),add(add(b,d),f)),(add(c,e),add(d,f))]
    x0=max(0,math.floor(min(x for x,y in corners)));y0=max(0,math.floor(min(y for x,y in corners)));x1=min(width,math.ceil(max(x for x,y in corners)));y1=min(height,math.ceil(max(y for x,y in corners)))
    w,h,n=event['width'],event['height'],event['components']
    a=mul(a,f32(1/w));b=mul(b,f32(1/w));c=mul(c,f32(1/h));d=mul(d,f32(1/h))
    det=1/(float(a)*d-float(b)*c)
    A=d*det;B=-b*det;C=-c*det;D=a*det
    E=-e*A-f*C;G=-e*B-f*D
    A,B,C,D,E,G=[mul(f32(x),16384) for x in [A,B,C,D,E,G]]
    fa,fb,fc,fd=map(int,[A,B,C,D])
    u=int(add(add(add(mul(A,x0),mul(C,y0)),E),mul(add(A,C),.5)))
    v=int(add(add(add(mul(B,x0),mul(D,y0)),G),mul(add(B,D),.5)))
    result=[None]*(width*height)
    for y in range(y0,y1):
      for x in range(x0,x1):
        col=(u+(x-x0)*fa+(y-y0)*fc)>>14;row=(v+(x-x0)*fb+(y-y0)*fd)>>14
        if 0<=col<w and 0<=row<h:
          k=(row*w+col)*n;p=[255-val if channel in event.get('invert',()) else val for channel,val in enumerate(event['raw'][k:k+n])];result[y*width+x]=p*3 if n==1 else p
    return result,dict(gridfit_changed=original!=m,original_topdown_matrix=original,gridfit_topdown_matrix=m,fixed14_steps=[fa,fb,fc,fd],fixed14_origin=[u,v],bbox=[x0,y0,x1,y1])
def path_mask(polygons,width,height):
    if not polygons:return [True]*(width*height)
    def num(x):return format(float(x)*4,'.12f').rstrip('0').rstrip('.') or '0'
    content=['q']
    for points in polygons:
        for i,(x,y) in enumerate(points):content.append(num(x)+' '+num(y)+(' m' if i==0 else ' l'))
        content+=['h W n']
    content+=['0 g',f'0 0 {width} {height} re f','Q']
    proc=subprocess.run([FROZEN_CLIP,str(width),str(height),'0'],input=('\n'.join(content)+'\n').encode(),capture_output=True,timeout=20)
    expected=f'P6\n{width} {height}\n255\n'.encode()
    if proc.returncode or proc.stderr or not proc.stdout.startswith(expected) or len(proc.stdout)!=len(expected)+width*height*3:raise ValueError('native clip witness failure')
    raw=proc.stdout[len(expected):]
    if any(v not in (0,255) for v in raw) or any(raw[i]!=raw[i+1] or raw[i]!=raw[i+2] for i in range(0,len(raw),3)):raise ValueError('native clip witness is not binary')
    return [raw[i]==0 for i in range(0,len(raw),3)]
def render(events,w=16,h=16):
    width,height=w*4,h*4;out=[(255,255,255)]*(width*height);witness=[]
    for event in events:
        if event['kind']=='image':pixels,record=native_image(event,width,height);witness.append(record);clips=event.get('clips',())
        elif event['kind']=='fill':pixels=[event['color']]*(width*height);clips=[event['polygon'],*event.get('clips',())]
        else:raise ValueError('native source model event kind')
        mask=path_mask(clips,width,height)
        for i,new in enumerate(pixels):
            if new is not None and mask[i]:out[i]=new
    return bytes(c for p in out for c in p),witness
