# SPDX-License-Identifier: AGPL-3.0-or-later
"""Exact independent Gaussian inversion, perturbed ray crossing and sample composition."""
from fractions import Fraction as F
EPS=F(1,1<<512)
def inside(points,x,y):
    # Symbolic right/down perturbation determines half-open ownership. All fixture
    # inputs are bounded dyadics with denominator<=1024, so EPS cannot cross a
    # nonincident boundary. This is not the production edge determinant test.
    x+=EPS;y+=EPS*EPS;found=False
    for (x0,y0),(x1,y1) in zip(points,points[1:]+points[:1]):
        if (y0>y)!=(y1>y) and x<x0+(y-y0)*(x1-x0)/(y1-y0):found=not found
    return found
def prepare(event):
    result=dict(event)
    if event['kind']=='image':
        a,b,c,d,e,f=map(F,event['matrix']);result['coeffs']=(a,b,c,d,e,f)
        result['polygon']=[(e,f),(a+e,b+f),(a+c+e,b+d+f),(c+e,d+f)]
        if a:result['pivot']=d-b*c/a
        else:result['pivot']=b if c else 0
        if not result['pivot']:raise ValueError('singular image model')
    return result
def color_at(event,x,y):
    if not inside(event['polygon'],x,y) or not all(inside(p,x,y) for p in event.get('clips',())):return None
    if event['kind']=='fill':return event['color']
    a,b,c,d,e,f=event['coeffs'];dx=x-e;dy=y-f
    if a:v=(dy-b*dx/a)/event['pivot'];u=(dx-c*v)/a
    else:v=dx/c;u=(dy-d*v)/b
    if not(0<=u<=1 and 0<=v<=1):raise ValueError('Gaussian/coverage disagreement')
    w,h,c=event['width'],event['height'],event['components'];col=min(w-1,int(w*u//1));row=min(h-1,int(h*(1-v)//1));start=(row*w+col)*c
    color=[255-v if i in event.get('invert',()) else v for i,v in enumerate(event['raw'][start:start+c])]
    return color*3 if c==1 else color
def render(events,w=16,h=16):
    if not (0<w<=64 and 0<h<=64):raise ValueError('generated image canvas admission')
    for event in events:
        coordinates=list(event.get('matrix',()))+[v for point in event.get('polygon',()) for v in point]+[v for polygon in event.get('clips',()) for point in polygon for v in point]
        for coordinate in coordinates:
            value=F(coordinate)
            if abs(value)>2**21 or value.denominator>1024 or value.denominator&(value.denominator-1):raise ValueError('generated dyadic model admission')
    events=[prepare(e) for e in events];samples=[]
    for sy in range(h*4):
      for sx in range(w*4):
        x,y=F(2*sx+1,8),F(2*sy+1,8);color=(255,255,255)
        for event in events:
          new=color_at(event,x,y)
          if new is not None:color=new
        samples.extend(color)
    return bytes(samples),resolve(bytes(samples),w*4,h*4)
def resolve(raw,w,h):
    if w%4 or h%4 or len(raw)!=w*h*3:raise ValueError('4x resolve dimensions')
    out=bytearray()
    for y in range(h//4):
      for x in range(w//4):
        for c in range(3):out.append((sum(raw[((4*y+dy)*w+4*x+dx)*3+c] for dy in range(4) for dx in range(4))+8)//16)
    return bytes(out)
