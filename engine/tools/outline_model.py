#!/usr/bin/env python3
"""Independent O0 fixture encoder and cyclic expanded-contour integer model.
No third-party modules. Commands use exact doubled font coordinates.
"""
import random, struct

def commands(contours):
    out=[]
    for contour in contours:
        # Insert implicit on-curve points into the complete cyclic contour first.
        expanded=[]
        for i,(x,y,on) in enumerate(contour):
            expanded.append((2*x,2*y,on,i))
            nx,ny,non=contour[(i+1)%len(contour)]
            if not on and not non:
                expanded.append((x+nx,y+ny,True,None))
        if contour[0][2]: start=next(i for i,p in enumerate(expanded) if p[3]==0)
        elif contour[-1][2]: start=next(i for i,p in enumerate(expanded) if p[3]==len(contour)-1)
        else: start=len(expanded)-1
        ring=expanded[start:]+expanded[:start]
        ring.append(ring[0]);out.append(['m',*ring[0][:2]])
        i=1
        while i<len(ring):
            p=ring[i]
            if p[2]: out.append(['l',*p[:2]]);i+=1
            else: out.append(['q',*p[:2],*ring[i+1][:2]]);i+=2
        out.append(['h'])
    return out

def glyph(contours, rng, instructions=b'', overlap=False, compact=True):
    if contours is None: return b'', {'kind':'empty_loca','points':[],'contour_endpoints':[],'commands':[],'instruction_bytes':0,'overlap':False,'bounds':None}
    pts=[p for c in contours for p in c]
    bounds=[min(p[0] for p in pts),min(p[1] for p in pts),max(p[0] for p in pts),max(p[1] for p in pts)] if pts else [0,0,0,0]
    ends=[];n=0
    for c in contours:n+=len(c);ends.append(n-1)
    data=bytearray(struct.pack('>hhhhh',len(contours),*bounds))
    data.extend(b''.join(struct.pack('>H',e) for e in ends));data.extend(struct.pack('>H',len(instructions)));data.extend(instructions)
    flags=[];axes=[bytearray(),bytearray()];last=[0,0]
    for p in pts:
        flag=int(p[2]) | (64 if overlap else 0)
        for axis,short,same in [(0,2,16),(1,4,32)]:
            delta=p[axis]-last[axis];last[axis]=p[axis]
            if compact and delta==0: flag|=same
            elif compact and 0<abs(delta)<=255 and rng.randrange(3):
                flag|=short | (same if delta>0 else 0);axes[axis].append(abs(delta))
            else: axes[axis].extend(struct.pack('>h',delta))
        flags.append(flag)
    i=0
    while i<len(flags):
        j=i+1
        while j<len(flags) and flags[j]==flags[i] and j-i<256:j+=1
        if j-i>1:data.extend([flags[i]|8,j-i-1])
        else:data.append(flags[i])
        i=j
    data.extend(axes[0]);data.extend(axes[1])
    return bytes(data),{'kind':'simple' if pts else 'zero_contours','points':[list(p) for p in pts],'contour_endpoints':ends,'commands':commands(contours),'instruction_bytes':len(instructions),'overlap':overlap if pts else False,'bounds':bounds}

def font(contours, seed=0, long=False, unaligned=False, units=1000):
    rng=random.Random(seed);glyf=bytearray();loca=[];expected=[];bearings=[]
    for gid,c in enumerate(contours):
        instructions=bytes(rng.randrange(256) for _ in range(rng.randrange(0,12))) if gid%3==0 else b''
        g,e=glyph(c,rng,instructions,overlap=(gid%4==3),compact=(gid%5!=0));loca.append(len(glyf));glyf.extend(g)
        if not long and len(glyf)%2:glyf.append(0)
        e.update(gid=gid,range=[loca[-1],len(glyf)]);expected.append(e);bearings.append(rng.randrange(-1100,1101))
    loca.append(len(glyf));n=len(contours)
    head=bytearray(54);struct.pack_into('>I',head,0,0x10000);struct.pack_into('>I',head,12,0x5f0f3cf5);struct.pack_into('>H',head,18,units);struct.pack_into('>hhhh',head,36,-2000,-2000,2000,2000);struct.pack_into('>h',head,50,int(long))
    maxp=bytearray(32);struct.pack_into('>IHHH',maxp,0,0x10000,n,max([len(e['points']) for e in expected]+[0]),max([len(e['contour_endpoints']) for e in expected]+[0]));struct.pack_into('>H',maxp,14,1)
    hhea=bytearray(36);struct.pack_into('>IhhhH',hhea,0,0x10000,2000,-2000,0,600);struct.pack_into('>h',hhea,18,1);metrics = 1 if seed%3==0 else n
    struct.pack_into('>H',hhea,34,metrics)
    tables={b'head':head,b'maxp':maxp,b'loca':b''.join(struct.pack('>I' if long else '>H',v if long else v//2) for v in loca),b'glyf':glyf,b'hhea':hhea,b'hmtx':b''.join(struct.pack('>Hh',600,b) if i<metrics else struct.pack('>h',b) for i,b in enumerate(bearings))}
    out=bytearray(12+16*len(tables));struct.pack_into('>IHHHH',out,0,0x10000,len(tables),64,2,len(tables)*16-64);records={}
    for i,(tag,data) in enumerate(sorted(tables.items())):
        if not unaligned:
            while len(out)%4:out.append(0)
        offset=len(out);padded=bytes(data)+bytes((-len(data))%4);checksum=sum(struct.unpack('>'+'I'*(len(padded)//4),padded))&0xffffffff
        struct.pack_into('>4sIII',out,12+16*i,tag,checksum,offset,len(data));records[tag.decode()]=[offset,len(data)];out.extend(data)
    for e,b in zip(expected,bearings):
        e['range']=[v+records['glyf'][0] for v in e['range']];e['bearing_adjustment']=(e['bounds'][0] if e['bounds'] else 0)-b
    return bytes(out),{'units_per_em':units,'glyph_count':n,'glyphs':expected,'tables':records}

def cases(count=220,seed=0x0150A11):
    rng=random.Random(seed)
    hand=[None,[],[[(1,3,True)]],[[(1,3,False)]],[[(0,0,False),(3,5,False)]],[[(0,0,False),(3,5,True)]],[[(0,0,True),(3,5,False)]],[[(1,1,False),(2,4,False),(7,2,False)]],[[(0,0,True),(0,0,True)],[(4,7,False),(9,2,True)]]]
    hand.extend([[[(5,7,True)]*300], [[(-32768,32767,False),(-1,0,False),(32766,-32767,True),(32767,-32768,False),(0,-1,True),(-32767,32766,True)]]])
    yield font(hand,seed,False,False,16)
    yield font(hand,seed+1,True,True,16384)
    for i in range(count-2):
        glyphs=[None,[]]
        for _ in range(rng.randrange(2,8)):
            cs=[]
            for _ in range(rng.randrange(1,5)):
                cs.append([(rng.randrange(-1000,1001),rng.randrange(-1000,1001),bool(rng.randrange(2))) for _ in range(rng.randrange(1,22))])
            glyphs.append(cs)
        yield font(glyphs,seed+i+2,bool(i%2),i%7==0,[16,1000,2048,16384][i%4])
