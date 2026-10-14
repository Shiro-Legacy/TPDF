#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Real-PDF raster cases with independently specified page-space geometry."""
import copy,random,runpy,struct
from fractions import Fraction as F
from pathlib import Path
from page_glyph_model import pdf,transform
from outline_model import font
from truetype_cmap_model import tables,with_cmap,cmap_table,encode
from text_position_model import number


def path_text(commands):
    out=[];current=start=None
    for command in commands:
        kind=command[0];args=list(command[1:])
        if kind=='q':
            control=args[:2];end=args[2:];args=[current[i]+(control[i]-current[i])*F(2,3) for i in range(2)]+[end[i]+(control[i]-end[i])*F(2,3) for i in range(2)]+end;kind='c'
        # PDF only has cubic curves; exact quadratic controls use3-divisible cases,
        # or sufficiently precise fixed decimal for geometry-only native diagnostics.
        def operand(v):
            try:return number(v)
            except ValueError:return format(float(v),'.15f').rstrip('0').rstrip('.')
        out.append(' '.join([*(operand(v) for v in args),kind]))
        if kind=='m':current=start=args[:]
        elif kind in ('l','c'):current=args[-2:]
        else:current=start
    return out


def event_content(events):
    out=[];depth=0
    for event in events:
        kind=event['kind']
        if kind=='pop':out.append('Q');depth-=1;continue
        if kind=='clip':out.append('q');depth+=1
        if 'matrix' in event:raise ValueError('generator pretransform expected')
        if kind=='fill':out.append(' '.join(format(c/255,'.15f') for c in event['color'])+' rg')
        out.extend(path_text(event['path']))
        out.append(('W* n' if event.get('rule')=='evenodd' else 'W n') if kind=='clip' else ('f*' if event.get('rule')=='evenodd' else 'f'))
    out.extend('Q' for _ in range(depth));return ('\n'.join(out)+'\n').encode()


def plain_pdf(raw,width,height):
    objects=[b'<< /Type /Catalog /Pages 2 0 R >>',b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',f'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Resources << >> /Contents 4 0 R >>'.encode(),b'<< /Length '+str(len(raw)).encode()+b' >>\nstream\n'+raw+b'\nendstream']
    data=b'%PDF-1.7\n';offsets=[]
    for i,obj in enumerate(objects,1):offsets.append(len(data));data+=f'{i} 0 obj\n'.encode()+obj+b'\nendobj\n'
    at=len(data);data+=b'xref\n0 5\n0000000000 65535 f \n'+b''.join(f'{v:010} 00000 n \n'.encode() for v in offsets)
    return data+f'trailer\n<< /Root 1 0 R /Size 5 >>\nstartxref\n{at}\n%%EOF\n'.encode()


def path_pdf(events,width,height):
    raw=f'1 0 0 -1 0 {height} cm\n'.encode()+event_content(events)
    return plain_pdf(raw,width,height)


def box(x0,y0,x1,y1):return [['m',x0,y0],['l',x1,y0],['l',x1,y1],['l',x0,y1],['h']]
def fill(path,color=[0,0,0],rule='nonzero'):return dict(kind='fill',path=path,color=color,rule=rule)
def clip(path,rule='nonzero'):return dict(kind='clip',path=path,rule=rule)


def controlled(count=300):
    parent=Path(__file__).parent
    r0=runpy.run_path(str(parent/'differential-raster.py'));mask=r0['reference_module']().rect_mask;selected=[]
    for label,raw,w,h,expected in r0['generated'](count,2026100312,mask):
        data=plain_pdf(f'1 0 0 -1 0 {h} cm\n'.encode()+raw,w,h)
        case=dict(label='orthogonal-'+str(label),width=w,height=h,pdf=data,expected=expected,events=None,scope='controlled-exact-native');yield case
        if len(selected)<8:selected.append(case)


    for case in selected:
        w,h=case['width'],case['height'];source=case['expected']
        for dpi in (36,144):
            nw,nh=(w//2,h//2) if dpi==36 else (w*2,h*2);pixels=bytearray()
            for y in range(nh):
                for x in range(nw):
                    if dpi==144:pixels.extend(source[((y//2)*w+x//2)*3:((y//2)*w+x//2)*3+3])
                    else:pixels.extend((sum(source[((2*y+j)*w+2*x+i)*3+c] for j in range(2) for i in range(2))+2)//4 for c in range(3))
            yield dict(case,label=case['label']+f'-dpi{dpi}',width=nw,height=nh,expected=bytes(pixels),dpi=dpi)


def lines(count=100):
    rng=random.Random(2026100319);w=h=12
    special=[
        ('offcanvas-enclosure',[fill(box(-20,-20,30,30))]),
        ('coincident-opposed',[fill(box(1,1,11,11)+[['m',1,1],['l',1,11],['l',11,11],['l',11,1],['h']])]),
        ('self-crossing',[fill([['m',1,1],['l',11,11],['l',1,11],['l',11,1],['h']])]),
        ('evenodd-hole',[fill(box(0,0,12,12)+box(3,3,9,9),rule='evenodd')]),
        ('clip-idempotent',[clip(box(F(1,4),F(1,2),F(37,4),F(19,2))),clip(box(F(1,4),F(1,2),F(37,4),F(19,2))),fill(box(-3,-3,20,20))]),
        ('clip-restore',[clip(box(0,0,10,10)),clip(box(2,2,5,5)),fill(box(0,0,12,12),[255,0,0]),dict(kind='pop'),fill(box(6,0,12,12),[0,0,255])]),
        ('fractional-grid',[fill(box(F(1,4),F(1,2),F(37,4),F(19,2)))]),
        ('snap-ties',[fill(box(F(255,512),F(257,512),F(4097,512),F(4095,512)))]),
        ('negative-snap-ties',[fill(box(F(-257,512),F(-255,512),F(4097,512),F(4095,512)))]),
    ]
    for label,events in special:yield dict(label=label,width=w,height=h,events=events,pdf=path_pdf(events,w,h),scope='controlled-exact-native' if label in ('fractional-grid','clip-idempotent') else 'rational-lines')
    for index in range(count):
        events=[]
        if index%3==0:events.append(clip(box(F(1,4),F(1,4),F(43,4),F(45,4))))
        for paint in range(1+index%3):
            points=[(F(rng.randrange(-32,129),8),F(rng.randrange(-32,129),8)) for _ in range(3+index%7)]
            path=[['m',*points[0]],*[['l',*p] for p in points[1:]],['h']]
            events.append(fill(path,rng.choice([[0,0,0],[255,0,0],[0,255,0],[0,0,255]]),'evenodd' if index%2 else 'nonzero'))
        yield dict(label=f'polygon-{index}',width=w,height=h,events=events,pdf=path_pdf(events,w,h),scope='rational-lines')


def curves():
    w=h=8
    paths=[
        ('cubic-arch',[['m',1,6],['c',1,0,7,0,7,6],['h']]),
        ('cubic-overshoot',[['m',2,2],['c',14,2,-6,2,6,2],['l',6,6],['l',2,6],['h']]),
        ('cubic-loop',[['m',4,4],['c',-2,-2,10,-2,4,4],['c',-2,10,10,10,4,4],['h']]),
        ('cubic-reversal',[['m',1,1],['c',7,7,7,1,1,7],['c',7,1,7,7,1,1],['h']]),
        ('tiny-loop',[['m',F(7,2),F(7,2)],['c',F(9,2),F(7,2),F(9,2),F(9,2),F(7,2),F(9,2)],['c',F(5,2),F(9,2),F(5,2),F(7,2),F(7,2),F(7,2)],['h']]),
    ]
    for label,path in paths:
        events=[fill(path)]
        yield dict(label=label,width=w,height=h,events=events,pdf=path_pdf(events,w,h),scope='certified-curves')
        events=[clip(path),clip(path),fill(box(0,0,8,8),[255,0,0])]
        yield dict(label=label+'-repeat-clip',width=w,height=h,events=events,pdf=path_pdf(events,w,h),scope='certified-curves')
    # Simple-glyf quadratics are represented exactly rather than converted into PDF cubics.
    contours=[[(0,0,True),(768,0,True),(768,768,True),(0,768,True)],[(192,192,True),(192,576,True),(576,576,True),(576,192,True)]]
    round_contours=[[(0,0,True),(384,768,False),(768,0,True)]]
    for label,shape in [('glyph-hole',contours),('glyph-quadratic',round_contours)]:
        program,outline=font([None,shape],seed=1,units=1024);program=bytearray(program);at,_=tables(bytes(program))[b'hmtx'];xmin=outline['glyphs'][1]['bounds'][0];struct.pack_into('>h',program,at+6,xmin);outline['glyphs'][1]['bearing_adjustment']=0
        program=with_cmap(bytes(program),cmap_table([(3,1,encode([(65,65,1-65,None)]))]));matrix=list(map(F,[8,0,0,8,1,1]));commands=transform(outline['glyphs'][1]['commands'],0,1024,matrix)
        events=[fill(box(0,0,8,8),[255,0,0]),fill(commands),fill(box(3,3,5,5),[0,0,255])]
        prefix=b'1 0 0 -1 0 8 cm\n1 0 0 rg 0 0 8 8 re f 0 g BT /F0 8 Tf 1 0 0 1 1 1 Tm <41> Tj ET 0 0 1 rg 3 3 2 2 re f\n'
        data=pdf(program,prefix,{65:250},{65:'\u03a9'},crop=(0,0,8,8))
        yield dict(label=label,width=8,height=8,events=events,pdf=data,scope='certified-curves' if label=='glyph-quadratic' else 'rational-lines')
    # Fully certified analytical quadratic: x=t, y=2t(1-t). Exactly6of16samples.
    shape=[[(0,0,True),(512,1024,False),(1024,0,True)]]
    program,outline=font([None,shape],seed=1,units=1024);program=bytearray(program);at,_=tables(bytes(program))[b'hmtx'];struct.pack_into('>h',program,at+6,0)
    program=with_cmap(bytes(program),cmap_table([(3,1,encode([(65,65,1-65,None)]))]));commands=transform(outline['glyphs'][1]['commands'],0,1024,list(map(F,[1,0,0,1,0,0])))
    events=[fill(commands)];raw=b'1 0 0 -1 0 1 cm BT /F0 1 Tf <41> Tj ET\n';data=pdf(program,raw,{65:250},{65:'\u03a9'},crop=(0,0,1,1))
    yield dict(label='analytical-quadratic-six-samples',width=1,height=1,events=events,pdf=data,scope='certified-curves',analytical=bytes([159]*3))
