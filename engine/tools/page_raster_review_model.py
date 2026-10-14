# SPDX-License-Identifier: AGPL-3.0-or-later
"""Independent exact per-sample winding, rational Bernstein curve certification.
No scanline sorting, spans, tiny-skia, production flattening, or float geometry.
Input commands: ('m', x,y), ('l',x,y), ('q',cx,cy,x,y),
('c',c1x,c1y,c2x,c2y,x,y), ('h',). Numbers accepted by Fraction.
Events: {kind:'fill'|'clip'|'pop', path:[commands], matrix:[a,b,c,d,e,f],
         color:[R,G,B] integer bytes, rule:'nonzero'|'evenodd'}.
Path coordinates transformed into output pixel space with matrix (default identity).
Initial bounds are unsnapped output-space [x0,y0,x1,y1]. All paths close implicitly.
render returns RGB bytes plus per-output-pixel uncertainty and counts. Curves use
original exact geometry; uncertainty includes a conservative1/32 pixel boundary
band for production flattening+quantization. Lines snap1/256 iff snap_lines=True.
"""
from fractions import Fraction as F
from math import lcm

class Unresolved(Exception): pass

def snap(v):
    v=F(v)*256
    sign=-1 if v<0 else 1
    a=abs(v); q,r=divmod(a.numerator,a.denominator)
    return F(sign*(q+(2*r>=a.denominator)),256)

def midpoint(a,b):return tuple((x+y)/2 for x,y in zip(a,b))
def split(p):
    rows=[p]
    while len(rows[-1])>1:rows.append(tuple(midpoint(a,b) for a,b in zip(rows[-1],rows[-1][1:])))
    return tuple(r[0] for r in rows),tuple(r[-1] for r in rows[::-1])

def line_winding(p,s):
    (x0,y0),(x1,y1)=p; x,y=s
    cross=(x1-x0)*(y-y0)-(x-x0)*(y1-y0)
    if y0<=y<y1 and cross>0:return 1
    if y1<=y<y0 and cross<0:return -1
    return 0

def curve_winding(p,s,depth=0):
    if len(p)==2:return line_winding(p,s)
    xs=[a[0] for a in p];ys=[a[1] for a in p];x,y=s
    if min(ys)>y or max(ys)<y or max(xs)<=x or min(ys)==max(ys):return 0
    inc=all(a<=b for a,b in zip(ys,ys[1:]));dec=all(a>=b for a,b in zip(ys,ys[1:]))
    if min(xs)>x and (inc or dec):
        if ys[0]<=y<ys[-1]:return 1
        if ys[-1]<=y<ys[0]:return -1
        return 0
    if depth>=32:raise Unresolved('curve meets exact sample or subdivision bound')
    a,b=split(p);return curve_winding(a,s,depth+1)+curve_winding(b,s,depth+1)

def near_line(p,s,epsilon=F(1,256)):
    # Exact segment/square intersection; conservative endpoint snapping band.
    lo,hi=F(0),F(1)
    for axis in range(2):
        a=p[0][axis];d=p[1][axis]-a;left=s[axis]-epsilon;right=s[axis]+epsilon
        if d==0:
            if not left<=a<=right:return False
        else:
            u,v=(left-a)/d,(right-a)/d
            lo=max(lo,min(u,v));hi=min(hi,max(u,v))
            if lo>hi:return False
    return True

def near_curve(p,s,epsilon=F(1,32),depth=0):
    # Conservative convex-hull interval enclosure, deliberately no chord flatness test.
    xs=[v[0] for v in p];ys=[v[1] for v in p];x,y=s
    if max(xs)<x-epsilon or min(xs)>x+epsilon or max(ys)<y-epsilon or min(ys)>y+epsilon:return False
    if depth>=24 or max(xs)-min(xs)<=epsilon/4 and max(ys)-min(ys)<=epsilon/4:return True
    a,b=split(p);return near_curve(a,s,epsilon,depth+1) or near_curve(b,s,epsilon,depth+1)

def segments(commands,matrix=None,snap_lines=False):
    m=tuple(F(x) for x in (matrix or [1,0,0,1,0,0]))
    def point(x,y):
        x,y=F(x),F(y);return m[0]*x+m[2]*y+m[4],m[1]*x+m[3]*y+m[5]
    result=[];start=current=None
    def close():
        if current is not None and start is not None and current!=start:result.append((current,start))
    for cmd in commands:
        op=cmd[0]
        if op=='m':close();start=current=point(*cmd[1:]);continue
        if op=='h':close();current=start;continue
        if current is None:raise ValueError('draw without move')
        values=cmd[1:];points=tuple(point(*values[i:i+2]) for i in range(0,len(values),2))
        if op not in {'l','q','c'} or len(points)!={'l':1,'q':2,'c':3}[op]:raise ValueError(cmd)
        result.append((current,)+points);current=points[-1]
    close()
    if snap_lines:
        if any(len(p)!=2 for p in result):raise ValueError('snap_lines applies only to straight polygons')
        result=[tuple(tuple(snap(v) for v in p) for p in segment) for segment in result]
    return result

def sample_mask(path,rule,width,height):
    mask=[];uncertain=[]
    curves=[p for p in path if len(p)>2]
    if not curves:
        # Independent integer per-sample ray cast, with an exact common scale.
        scale=lcm(8,*(value.denominator for edge in path for point in edge for value in point))
        integer=[tuple(tuple(int(v*scale) for v in p) for p in edge) for edge in path]
        for y in range(height*4):
            sy=(2*y+1)*(scale//8)
            for x in range(width*4):
                sx=(2*x+1)*(scale//8);w=0
                for (x0,y0),(x1,y1) in integer:
                    cross=(x1-x0)*(sy-y0)-(sx-x0)*(y1-y0)
                    if y0<=sy<y1 and cross>0:w+=1
                    elif y1<=sy<y0 and cross<0:w-=1
                mask.append(w!=0 if rule=='nonzero' else w%2!=0)
        return mask,[False]*len(mask)
    for y in range(height*4):
        for x in range(width*4):
            sample=F(2*x+1,8),F(2*y+1,8)
            near=any(near_curve(p,sample) if len(p)>2 else near_line(p,sample) for p in path)
            try:w=sum(curve_winding(p,sample) for p in path);inside=w!=0 if rule=='nonzero' else w%2!=0
            except Unresolved:inside=False;near=True
            mask.append(inside);uncertain.append(near)
    return mask,uncertain

def render(events,width,height,initial_bounds=None,snap_lines=False):
    bounds=tuple(F(v) for v in (initial_bounds or [0,0,width,height]));n=16*width*height
    base=[]
    for y in range(4*height):
        for x in range(4*width):
            px,py=F(2*x+1,8),F(2*y+1,8)
            base.append(bounds[0]<=px<bounds[2] and bounds[1]<=py<bounds[3])
    stack=[(base,[False]*n)];pixels=[(255,255,255)]*n;uncertain=[False]*n
    for event in events:
        if event['kind']=='pop':
            if len(stack)==1:raise ValueError('clip stack underflow')
            stack.pop();continue
        path=segments(event['path'],event.get('matrix'),snap_lines)
        mask,ambiguous=sample_mask(path,event.get('rule','nonzero'),width,height)
        parent,parent_ambiguous=stack[-1]
        if event['kind']=='clip':
            stack.append(([a and b for a,b in zip(parent,mask)],[a or (inside and b) for a,inside,b in zip(parent_ambiguous,parent,ambiguous)]));continue
        if event['kind']!='fill':raise ValueError('unknown event')
        color=tuple(event['color']);assert len(color)==3 and all(isinstance(v,int) and 0<=v<=255 for v in color)
        for i in range(n):
            if parent[i] and mask[i]:pixels[i]=color
            # A later opaque guaranteed fill can hide earlier uncertainty.
            if parent[i] and mask[i] and not(parent_ambiguous[i] or ambiguous[i]):uncertain[i]=False
            else:uncertain[i] |= parent_ambiguous[i] or (parent[i] and ambiguous[i])
    output=bytearray();boundary=[]
    for y in range(height):
        for x in range(width):
            slots=[(4*y+j)*(4*width)+(4*x+i) for j in range(4) for i in range(4)]
            output.extend((sum(pixels[k][c] for k in slots)+8)//16 for c in range(3));boundary.append(any(uncertain[k] for k in slots))
    return bytes(output),boundary,{'uncertain_pixels':sum(boundary),'pixels':width*height,'samples':n}

if __name__=='__main__':
    box=[('m',0,0),('l',1,0),('l',1,1),('l',0,1),('h',)]
    assert render([{'kind':'fill','path':box,'color':[0,0,0]}],2,1)[0]==bytes([0,0,0,255,255,255])
    half=[('m',0,0),('l',F(1,2),0),('l',F(1,2),1),('l',0,1),('h',)]
    e=[{'kind':'clip','path':half},{'kind':'clip','path':half},{'kind':'fill','path':box,'color':[0,0,0]}]
    assert render(e,1,1)[0]==bytes([128]*3)
    curve=[('m',0,0),('q',F(1,2),1,1,0),('h',)]
    p=segments(curve);assert curve_winding(p[0],(F(1,2),F(1,4)))==-1
    print('three exact model self-checks pass')
