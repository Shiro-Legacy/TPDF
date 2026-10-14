# SPDX-License-Identifier: AGPL-3.0-or-later
"""Independent bounded R21 reference prototype; not production stroker output.

Exact line/cap decisions use rational dot/cross/distance tests. Joins use outward
rational intervals. Cubics enclose the TRUE normal-offset boundary via Bernstein
hulls; a point is certified by boundary homotopy only when every enclosure avoids
it. This never constructs production true-tangent quadrilateral strips.
"""
from fractions import Fraction as Q
from math import isqrt
from dataclasses import dataclass
import json

class RefLimit(Exception): pass
@dataclass
class Budget:
    max_work: int = 8_000_000
    max_nodes: int = 200_000
    max_queries: int = 100_000
    max_depth: int = 22
    max_bits: int = 4096
    work: int = 0
    nodes: int = 0
    queries: int = 0
    def tick(self, n=1):
        if self.work+n > self.max_work: raise RefLimit('work')
        self.work += n
    def check(self, x):
        self.tick()
        if max(abs(x.numerator).bit_length(), x.denominator.bit_length()) > self.max_bits:
            raise RefLimit('fraction bits')
        return x
    def node(self):
        if self.nodes >= self.max_nodes: raise RefLimit('nodes')
        self.nodes += 1
    def query(self):
        if self.queries >= self.max_queries: raise RefLimit('queries')
        self.queries += 1

@dataclass(frozen=True)
class I:
    lo: Q
    hi: Q
    b: Budget
    @classmethod
    def point(cls, x, b):
        x=b.check(Q(x)); return cls(x,x,b)
    def _new(self, a, z): return I(self.b.check(a),self.b.check(z),self.b)
    def co(self, x): return x if isinstance(x,I) else I.point(x,self.b)
    def __add__(self, x):
        x=self.co(x); return self._new(self.lo+x.lo,self.hi+x.hi)
    __radd__=__add__
    def __neg__(self): return self._new(-self.hi,-self.lo)
    def __sub__(self,x): return self+-self.co(x)
    def __rsub__(self,x): return self.co(x)+-self
    def __mul__(self,x):
        x=self.co(x); z=[self.lo*x.lo,self.lo*x.hi,self.hi*x.lo,self.hi*x.hi]
        return self._new(min(z),max(z))
    __rmul__=__mul__
    def __truediv__(self,x):
        x=self.co(x)
        if x.lo<=0<=x.hi: raise RefLimit('unresolved denominator')
        return self*I(1/x.hi,1/x.lo,self.b)
    def sq(self):
        a=0 if self.lo<=0<=self.hi else min(self.lo*self.lo,self.hi*self.hi)
        return self._new(Q(a),max(self.lo*self.lo,self.hi*self.hi))
    def sqrt(self):
        if self.lo<0: raise RefLimit('negative square root')
        # Exact integer arithmetic gives outward bounds, with <=2^-80 width
        # for a point argument; no floating-point sqrt is trusted.
        s=1<<80
        a=isqrt((self.lo.numerator*s*s)//self.lo.denominator)
        z=isqrt((self.hi.numerator*s*s)//self.hi.denominator)
        hi=Q(z,s)
        if hi*hi<self.hi: hi=Q(z+1,s)
        return self._new(Q(a,s),hi)
    def midpoint(self): return self.b.check((self.lo+self.hi)/2)

def p(x,y): return (Q(x),Q(y))
def sub(a,b): return (a[0]-b[0],a[1]-b[1])
def dot(a,b): return a[0]*b[0]+a[1]*b[1]
def cross(a,b): return a[0]*b[1]-a[1]*b[0]
def midpoint(a,b): return ((a[0]+b[0])/2,(a[1]+b[1])/2)
def split(c):
    a,b,d=midpoint(c[0],c[1]),midpoint(c[1],c[2]),midpoint(c[2],c[3])
    e,f=midpoint(a,b),midpoint(b,d); g=midpoint(e,f)
    return (c[0],a,e,g),(g,f,d,c[3])
def vector_interval(v,b): return tuple(I.point(x,b) for x in v)
def hull(vs,b): return tuple(I(b.check(min(x[k] for x in vs)),b.check(max(x[k] for x in vs)),b) for k in (0,1))
def norm(v): return (v[0].sq()+v[1].sq()).sqrt()
def mapped(v,m):
    a,b,c,d,e,f=m
    return (a*v[0]+c*v[1]+e,b*v[0]+d*v[1]+f)
def inverse_point(v,m):
    a,b,c,d,e,f=map(Q,m); det=a*d-b*c
    if not det: raise RefLimit('singular transform')
    x,y=v[0]-e,v[1]-f
    return ((d*x-c*y)/det,(-b*x+a*y)/det)

def line_member(point,start,end,radius,cap='butt',matrix=(1,0,0,1,0,0)):
    """Exact geometric positive-width line + symmetric endpoint caps."""
    v=sub(inverse_point(point,matrix),start); d=sub(end,start)
    ll=dot(d,d); r=Q(radius)
    if ll<=0 or r<=0: raise RefLimit('degenerate line or radius')
    s=dot(v,d)
    if cap=='round':
        if s<0: return dot(v,v)<=r*r
        if s>ll:
            w=sub(v,d); return dot(w,w)<=r*r
    elif cap=='square':
        if s<0 and s*s>r*r*ll: return False
        if s>ll and (s-ll)**2>r*r*ll: return False
    elif cap=='butt':
        if s<0 or s>ll: return False
    else: raise ValueError(cap)
    return cross(d,v)**2<=r*r*ll

def convex_member(point,vertices):
    """True/False/None for CCW convex vertices enclosed by rational intervals."""
    signs=[]
    for a,z in zip(vertices,vertices[1:]+vertices[:1]):
        signs.append(cross(sub(z,a),sub(vector_interval(point,a[0].b),a)))
    if any(v.hi<0 for v in signs): return False
    if all(v.lo>0 for v in signs): return True
    return None

def join_member(point,previous,vertex,following,radius,join='bevel',miter_limit=10,budget=None,matrix=(1,0,0,1,0,0)):
    """Outer join component only; union with incident BUTT line bodies."""
    b=budget or Budget(); b.query(); v=sub(inverse_point(point,matrix),vertex)
    d0,d1=sub(vertex,previous),sub(following,vertex); turn=cross(d0,d1)
    if not turn or radius<=0: raise RefLimit('parallel/reversed/degenerate join')
    side=1 if turn>0 else -1
    # Exact angular sector predicates, independent of normal normalization.
    if dot(d0,v)<0 or dot(d1,v)>0: return False
    if join=='round': return dot(v,v)<=Q(radius)**2
    r=Q(radius)
    def outward(d):
        q=vector_interval(d,b); n=norm(q)
        return (side*r*q[1]/n,-side*r*q[0]/n)
    u0,u1=outward(d0),outward(d1); zero=vector_interval((0,0),b)
    verts=[zero,u0,u1]
    if join=='miter':
        t=cross(sub(u1,u0),vector_interval(d1,b))/turn
        meet=tuple(u0[k]+t*d0[k] for k in (0,1))
        ratio2=(meet[0].sq()+meet[1].sq())/(r*r)
        if ratio2.lo<=Q(miter_limit)**2<ratio2.hi: return None
        if ratio2.hi<=Q(miter_limit)**2: verts=[zero,u0,meet,u1]
    elif join!='bevel': raise ValueError(join)
    if side<0: verts.reverse()
    return convex_member(v,verts)

class Node:
    def __init__(self,c,r,sign,m,b,cache=None,t0=Q(0),t1=Q(1)):
        b.node(); self.c=c; self.r=r; self.sign=sign; self.m=m; self.b=b; self.children=None
        self.cache={} if cache is None else cache; self.t0=t0; self.t1=t1
        d=tuple(tuple(3*(c[i+1][k]-c[i][k]) for k in (0,1)) for i in range(3))
        cv,dv=hull(c,b),hull(d,b)
        length=norm(dv)
        self.box=None
        if length.lo>0:
            normal=(-dv[1]/length,dv[0]/length)
            self.box=mapped(tuple(cv[k]+sign*r*normal[k] for k in (0,1)),m)
        self.ends=[]; self.end_intervals=[]
        for t,pt,der in [(t0,c[0],d[0]),(t1,c[3],d[2])]:
            if t not in self.cache:
                di=vector_interval(der,b); ln=norm(di)
                if ln.lo<=0: raise RefLimit('zero endpoint tangent')
                no=(-di[1]/ln,di[0]/ln)
                val=mapped(tuple(I.point(pt[k],b)+sign*r*no[k] for k in (0,1)),m)
                self.cache[t]=(val,tuple(x.midpoint() for x in val))
            val,approx=self.cache[t]
            self.ends.append(approx); self.end_intervals.append(val)
        if self.box is not None:
            # Include shared approximated endpoints in each convex enclosure.
            self.box=tuple(I(min(x.lo,self.ends[0][k],self.ends[1][k]),max(x.hi,self.ends[0][k],self.ends[1][k]),b) for k,x in enumerate(self.box))
    def avoids(self,pt,margin):
        return self.box is not None and any(pt[k]+margin<self.box[k].lo or pt[k]-margin>self.box[k].hi for k in (0,1))
    def certify(self,pt,margin=Q(0),depth=0):
        self.b.tick()
        if self.avoids(pt,margin): return [self.ends[0],self.ends[1]]
        if depth>=self.b.max_depth: return None
        if self.children is None:
            a,z=split(self.c)
            t=(self.t0+self.t1)/2
            self.children=(Node(a,self.r,self.sign,self.m,self.b,self.cache,self.t0,t),Node(z,self.r,self.sign,self.m,self.b,self.cache,t,self.t1))
        a=self.children[0].certify(pt,margin,depth+1)
        if a is None: return None
        z=self.children[1].certify(pt,margin,depth+1)
        if a is None or z is None: return None
        assert a[-1]==z[0], "shared exact-parameter endpoint identity"
        self.b.tick(len(a)+len(z))
        return a[:-1]+z

def winding(point,poly,b):
    w=0
    for a,z in zip(poly,poly[1:]+poly[:1]):
        b.tick(); cr=cross(sub(z,a),sub(point,a))
        if cr==0 and min(a[0],z[0])<=point[0]<=max(a[0],z[0]) and min(a[1],z[1])<=point[1]<=max(a[1],z[1]): return None
        if a[1]<=point[1]<z[1] and cr>0: w+=1
        if z[1]<=point[1]<a[1] and cr<0: w-=1
    return w!=0

class CubicStrip:
    def __init__(self,controls,radius,matrix=(1,0,0,1,0,0),budget=None):
        self.b=budget or Budget()
        if len(controls)!=4: raise RefLimit('four controls required')
        self.c=tuple(tuple(self.b.check(Q(x)) for x in v) for v in controls)
        if any(len(v)!=2 for v in self.c): raise RefLimit('two coordinates required')
        self.r=self.b.check(Q(radius))
        if self.r<=0: raise RefLimit('nonpositive radius')
        if len(matrix)!=6: raise RefLimit('six affine coefficients required')
        self.m=tuple(self.b.check(Q(x)) for x in matrix)
        if self.m[0]*self.m[3]==self.m[1]*self.m[2]: raise RefLimit('singular transform')
        self.regularity=self.admit(self.c)
        self.left=Node(self.c,self.r,1,self.m,self.b); self.right=Node(self.c,self.r,-1,self.m,self.b)
    def admit(self,c):
        """Independent global sufficient forward/no-fold certificate."""
        d=tuple(tuple(3*(c[i+1][k]-c[i][k]) for k in (0,1)) for i in range(3))
        e=sub(c[3],c[0]); ee=dot(e,e)
        projection=min(dot(x,e) for x in d)
        if ee<=0 or projection<=0: raise RefLimit('forward cone not admitted')
        # m^2 = min(dot(D,chord))^2 / |chord|^2: exact rational, no unit vector.
        m2=projection**2/ee
        aa=[tuple(2*(d[i+1][k]-d[i][k]) for k in (0,1)) for i in range(2)]
        a2=max(dot(x,x) for x in aa)
        # (r*Amax/m^2)^2 <= 1/4 is an exact sufficient no-fold margin.
        bound2=self.r*self.r*a2/(m2*m2)
        if bound2>Q(1,4): raise RefLimit('global no-fold margin not admitted')
        return {'curvature_radius_upper_squared':str(bound2),'min_projection_squared':str(m2)}
    def contains(self,pt,boundary_margin=0):
        self.b.query(); pt=tuple(self.b.check(Q(x)) for x in pt)
        margin=self.b.check(Q(boundary_margin))
        if margin<0: raise RefLimit('negative boundary margin')
        a=self.left.certify(pt,margin); z=self.right.certify(pt,margin)
        if a is None or z is None: return None
        # Both true endpoint sections and their approximate polygon sections
        # lie in these interval-segment enclosures. Subdivision certifies their
        # entire connecting homotopy, including 2^-100 rounding-adjacent points.
        for i in (0,1):
            if not section_avoids(pt,self.left.end_intervals[i],self.right.end_intervals[i],margin,self.b):
                return None
        return winding(pt,a+list(reversed(z)),self.b)

def section_avoids(pt,a,z,margin,b,depth=0):
    b.tick()
    if any(pt[k]+margin<min(a[k].lo,z[k].lo) or pt[k]-margin>max(a[k].hi,z[k].hi) for k in (0,1)):
        return True
    if depth>=b.max_depth: return False
    mid=tuple((a[k]+z[k])/2 for k in (0,1))
    return section_avoids(pt,a,mid,margin,b,depth+1) and section_avoids(pt,mid,z,margin,b,depth+1)

IDENTITY=(1,0,0,1,0,0)

def inverse_margin(matrix,margin,b):
    a,bb,c,d,_,_=map(Q,matrix); det=a*d-bb*c
    if not det: raise RefLimit('singular transform')
    return (I.point((a*a+bb*bb+c*c+d*d)/(det*det),b).sqrt()*Q(margin)).hi

def rectangle_certificate(t,v,length,r,margin):
    if t.hi < -margin or t.lo > length.hi+margin or v.hi < -r-margin or v.lo > r+margin:
        return False
    if t.lo > margin and t.hi < length.lo-margin and v.lo > -r+margin and v.hi < r-margin:
        return True
    return None

def line_certificate(point,start,end,radius,matrix,b,margin):
    q=inverse_point(point,matrix); d=sub(end,start); v=sub(q,start)
    length=I.point(dot(d,d),b).sqrt()
    if length.lo<=0: raise RefLimit('zero line tangent')
    t=I.point(dot(v,d),b)/length; normal=I.point(cross(d,v),b)/length
    return rectangle_certificate(t,normal,length,Q(radius),inverse_margin(matrix,margin,b))

def cap_certificate(point,center,outward,radius,cap,matrix,b,margin):
    if cap=='butt': return False
    q=sub(inverse_point(point,matrix),center); d=outward
    length=I.point(dot(d,d),b).sqrt()
    if length.lo<=0: raise RefLimit('zero cap tangent')
    t=I.point(dot(q,d),b)/length; normal=I.point(cross(d,q),b)/length
    m=inverse_margin(matrix,margin,b); r=Q(radius)
    if cap=='square': return rectangle_certificate(t,normal,I.point(r,b),r,m)
    if cap!='round': raise RefLimit('cap')
    distance=I.point(dot(q,q),b).sqrt()
    if t.hi < -m or distance.lo > r+m: return False
    if t.lo > m and distance.hi < r-m: return True
    return None

def join_certificate(point,previous,vertex,following,radius,join,miter_limit,matrix,b,margin):
    q=sub(inverse_point(point,matrix),vertex); d0=sub(vertex,previous); d1=sub(following,vertex)
    turn=cross(d0,d1)
    if turn==0:
        if dot(d0,d1)>0: return False # straight continuation adds no component
        raise RefLimit('reversal or zero join tangent')
    l0=I.point(dot(d0,d0),b).sqrt(); l1=I.point(dot(d1,d1),b).sqrt()
    if l0.lo<=0 or l1.lo<=0: raise RefLimit('zero join tangent')
    m=inverse_margin(matrix,margin,b); r=Q(radius)
    t0=I.point(dot(d0,q),b)/l0; t1=I.point(dot(d1,q),b)/l1
    if t0.hi < -m or t1.lo > m: return False
    if join=='round':
        distance=I.point(dot(q,q),b).sqrt()
        if distance.lo>r+m: return False
        if t0.lo>m and t1.hi < -m and distance.hi<r-m: return True
        return None
    side=1 if turn>0 else -1
    u0=(I.point(side*r*d0[1],b)/l0,I.point(-side*r*d0[0],b)/l0)
    u1=(I.point(side*r*d1[1],b)/l1,I.point(-side*r*d1[0],b)/l1)
    verts=[vector_interval((0,0),b),u0,u1]
    if join=='miter':
        t=cross(sub(u1,u0),vector_interval(d1,b))/turn
        meet=tuple(u0[k]+t*d0[k] for k in (0,1))
        ratio2=(meet[0].sq()+meet[1].sq())/(r*r)
        if ratio2.lo<=Q(miter_limit)**2<ratio2.hi: raise RefLimit('uncertain miter policy threshold')
        if ratio2.hi<=Q(miter_limit)**2: verts=[verts[0],u0,meet,u1]
    elif join!='bevel': raise RefLimit('join')
    if side<0: verts.reverse()
    signs=[]
    for a,z in zip(verts,verts[1:]+verts[:1]):
        edge=sub(z,a); value=cross(edge,sub(vector_interval(q,b),a))
        bound=(norm(edge)*m).hi
        if value.hi < -bound: return False
        signs.append(value.lo>bound)
    return True if all(signs) else None

class StrokeModel:
    """Bounded full solid-stroke union reference.

    Commands are ('m',x,y), ('l',x,y), ('c',x1,y1,x2,y2,x,y), ('h',).
    Coordinates/matrix/width are exact rationals. cap and join are words.
    contains returns True/False/None; None means uncertified within the requested
    DEVICE boundary margin or finite interval depth. RefLimit is a failed model
    run and must never be silently interpreted as False or an accepted refusal.
    Cubics currently require the independently sufficient global forward/no-fold
    test, a narrower model domain than production's locally subdivided admission.
    """
    def __init__(self,commands,width,cap='butt',join='miter',miter_limit=10,matrix=IDENTITY,budget=None):
        self.b=budget or Budget(); self.r=self.b.check(Q(width)/2)
        if self.r<=0 or cap not in ('butt','round','square') or join not in ('miter','round','bevel') or Q(miter_limit)<1:
            raise RefLimit('stroke style')
        self.m=tuple(self.b.check(Q(x)) for x in matrix)
        if len(self.m)!=6: raise RefLimit('matrix')
        self.cap=cap; self.join=join; self.miter=Q(miter_limit); self.components=[]
        if len(commands)>4096: raise RefLimit('source commands')
        segments=[]; current=start=None; closed=False
        def finish():
            nonlocal segments,current,start,closed
            if not segments: raise RefLimit('empty/degenerate subpath')
            for kind,data,_,_ in segments:
                if kind=='line': self.components.append(('line',data))
                else: self.components.append(('curve',CubicStrip(data,self.r,self.m,self.b)))
            for old,new in zip(segments,segments[1:]):
                v=old[1][-1]; self.components.append(('join',(sub(v,old[3]),v,tuple(v[k]+new[2][k] for k in (0,1)))))
            if closed:
                v=segments[-1][1][-1]; self.components.append(('join',(sub(v,segments[-1][3]),v,tuple(v[k]+segments[0][2][k] for k in (0,1)))))
            else:
                self.components.append(('cap',(segments[0][1][0],tuple(-x for x in segments[0][2]))))
                self.components.append(('cap',(segments[-1][1][-1],segments[-1][3])))
            self.b.tick(len(self.components)); segments=[];current=start=None;closed=False
        for command in commands:
            self.b.tick(); kind=command[0]
            q=tuple(self.b.check(Q(x)) for x in command[1:])
            if kind=='m':
                if len(q)!=2: raise RefLimit('move arity')
                if start is not None: finish()
                current=start=q
            elif kind in ('l','c'):
                if current is None or closed: raise RefLimit('missing current point')
                if kind=='l':
                    if len(q)!=2 or q==current: raise RefLimit('line arity/zero length')
                    d=sub(q,current);segments.append(('line',(current,q),d,d));current=q
                else:
                    if len(q)!=6: raise RefLimit('cubic arity')
                    c=(current,q[:2],q[2:4],q[4:]);segments.append(('curve',c,sub(c[1],c[0]),sub(c[3],c[2])));current=c[-1]
            elif kind=='h':
                if q or current is None or closed: raise RefLimit('close')
                if current!=start:
                    d=sub(start,current);segments.append(('line',(current,start),d,d));current=start
                closed=True
            else: raise RefLimit('command')
        if start is not None: finish()
        if not self.components: raise RefLimit('empty stroke')
    def contains(self,point,boundary_margin=0):
        self.b.query(); point=tuple(self.b.check(Q(x)) for x in point)
        margin=self.b.check(Q(boundary_margin))
        if margin<0: raise RefLimit('negative boundary margin')
        uncertain=False
        for kind,value in self.components:
            self.b.tick()
            if kind=='line': result=line_certificate(point,*value,self.r,self.m,self.b,margin)
            elif kind=='cap': result=cap_certificate(point,*value,self.r,self.cap,self.m,self.b,margin)
            elif kind=='join': result=join_certificate(point,*value,self.r,self.join,self.miter,self.m,self.b,margin)
            else: result=value.contains(point,margin)
            if result is True: return True
            uncertain |= result is None
        return None if uncertain else False

def self_checks():
    b=Budget(); a,z=p(0,0),p(4,0)
    assert line_member(p(2,Q(1,2)),a,z,1)
    assert not line_member(p(-Q(1,2),0),a,z,1)
    assert line_member(p(-Q(1,2),0),a,z,1,'square')
    assert not line_member(p(-Q(9,10),Q(9,10)),a,z,1,'round')
    assert line_member(p(-Q(9,10),Q(9,10)),a,z,1,'square')
    prev,j,nxt=p(-4,0),p(0,0),p(0,4)
    assert join_member(p(Q(3,4),-Q(3,4)),prev,j,nxt,1,'bevel',budget=b) is False
    assert join_member(p(Q(3,4),-Q(3,4)),prev,j,nxt,1,'miter',budget=b) is True
    assert join_member(p(Q(3,4),-Q(3,4)),prev,j,nxt,1,'round',budget=b) is False
    assert join_member(p(Q(1,2),-Q(1,2)),prev,j,nxt,1,'round',budget=b) is True
    # Exact straight cubic strip agrees with analytic butt segment except boundary.
    c=CubicStrip([p(0,0),p(1,0),p(3,0),p(4,0)],Q(1,4))
    for x in range(-4,21):
        for y in range(-5,6):
            pt=p(Q(x,4)+Q(1,100),Q(y,8)+Q(1,100)); got=c.contains(pt)
            assert got is not None and got==line_member(pt,p(0,0),p(4,0),Q(1,4))
    # Reflection/shear are checked by inverse mapping, not isotropic widths.
    mat=(Q(2),Q(1,2),Q(1),Q(3),Q(7),Q(-4))
    for cap in ('butt','square','round'):
        for point in [p(2,Q(1,2)),p(-Q(1,2),0),p(-Q(9,10),Q(9,10))]:
            assert line_member(mapped(point,mat),a,z,1,cap,mat)==line_member(point,a,z,1,cap)
    assert join_member(p(Q(3,4),-Q(3,4)),prev,j,nxt,1,'miter',miter_limit=1,budget=b) is False
    assert join_member(p(Q(3,4),Q(3,4)),prev,j,p(0,-4),1,'miter',budget=b) is True
    # Zero endpoint derivative and excessive radius are not guessed through.
    for controls,r in [([p(0,0),p(0,0),p(1,1),p(2,0)],1),([p(0,0),p(1,1),p(2,1),p(3,0)],100)]:
        try: CubicStrip(controls,r)
        except RefLimit: pass
        else: raise AssertionError('nonadmitted strip accepted')
    try:
        CubicStrip([p(0,0),p(1,0),p(2,0),p(3,0)],1,budget=Budget(max_nodes=1))
    except RefLimit: pass
    else: raise AssertionError('node budget missed')
    limited=CubicStrip([p(0,0),p(1,0),p(2,0),p(3,0)],1,budget=Budget(max_queries=1))
    limited.contains(p(2,3))
    try: limited.contains(p(2,3))
    except RefLimit: pass
    else: raise AssertionError('query budget missed')
    # Non-axis endpoint rounding ambiguity: true cap boundary is x+y=0.
    diagonal=CubicStrip([p(0,0),p(1,1),p(2,2),p(3,3)],Q(1,4))
    tiny=Q(1,1<<100)
    assert diagonal.contains(p(Q(1,10)+tiny,-Q(1,10))) is None
    assert diagonal.contains(p(Q(1,10)-tiny,-Q(1,10))) is None
    assert diagonal.contains(p(Q(1,10)+Q(1,1000),-Q(1,10))) is True
    assert diagonal.contains(p(Q(1,10)-Q(1,1000),-Q(1,10))) is False
    assert diagonal.contains(p(1,1),boundary_margin=Q(1,64)) is True
    return {'analytic_identities':20,'straight_cubic_sample_equivalences':275,'refusal_and_budget_checks':4,'endpoint_and_margin_checks':5}

def complete_path_checks():
    identities=0
    for cap in ('butt','round','square'):
        model=StrokeModel([('m',0,0),('l',4,0)],2,cap)
        for x in range(-8,25):
            for y in range(-8,9):
                point=p(Q(x,4)+Q(1,100),Q(y,4)+Q(1,100))
                got=model.contains(point)
                if got is not None:
                    assert got==line_member(point,p(0,0),p(4,0),1,cap)
                    identities+=1
    square=StrokeModel([('m',0,0),('l',4,0),('l',4,4),('l',0,4),('h',)],1)
    assert square.contains(p(2,2)) is False
    assert square.contains(p(2,Q(1,10))) is True
    assert square.contains(p(-Q(1,4),-Q(1,4))) is True
    assert square.contains(p(-Q(3,4),-Q(3,4))) is False
    # Entire open path unions a cap, original join, line and true-cubic strip.
    curved=StrokeModel([('m',0,0),('l',2,0),('c',3,0,4,1,5,1)],Q(1,2),'round','round')
    assert curved.contains(p(-Q(1,10),0)) is True
    assert curved.contains(p(1,Q(1,10))) is True
    assert curved.contains(p(4,4)) is False
    return {'certified_line_cap_equivalences':identities,'closed_seam_identities':4,'mixed_line_cubic_cap_identities':3}

def demo():
    report={'checks':self_checks(),'cases':[], 'complete_path_checks':complete_path_checks()}
    curve=[p(72,60),p(200,200),p(400,200),p(540,60)]
    matrices=[(Q(1,32),0,0,Q(1,32),-2,-1),(Q(1,32),Q(1,128),Q(1,64),Q(1,32),-3,-2),(-Q(1,32),0,Q(1,128),Q(1,32),17,-1)]
    for i,m in enumerate(matrices):
        b=Budget(); c=CubicStrip(curve,Q(1,2),m,b)
        inside=outside=uncertain=0
        # Fixed 4x sample convention on a bounded 20x8 target.
        for y in range(32):
            for x in range(80):
                result=c.contains(p(Q(2*x+1,8),Q(2*y+1,8)))
                if result is None: uncertain+=1
                elif result: inside+=1
                else: outside+=1
        report['cases'].append({'name':['stress','shear','reflection'][i],'inside':inside,'outside':outside,'uncertain':uncertain,'regularity':c.regularity,'work':b.work,'nodes':b.nodes,'queries':b.queries})
    print(json.dumps(report,indent=2))
if __name__=='__main__': demo()
