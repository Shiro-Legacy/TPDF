#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""R21 deterministic true-stroke cases encoded as ordinary resource-free PDFs."""
from fractions import Fraction as F
from page_raster_cases import plain_pdf,path_text,box,fill,clip
from text_position_model import number

def stroke(path,width=1,cap=0,join=0,miter=10,matrix=(1,0,0,1,0,0),color=(0,0,0)):
    return dict(kind='stroke',path=path,style=dict(width=width,cap=cap,join=join,miter=miter),matrix=matrix,color=list(color))

def content(events,height):
    out=[f'1 0 0 -1 0 {height} cm'];depth=0
    for event in events:
        kind=event['kind']
        if kind=='pop':out.append('Q');depth-=1;continue
        if kind=='clip':out.append('q');depth+=1
        else:out.append('q')
        if 'matrix' in event:out.append(' '.join(map(number,event['matrix']))+' cm')
        if kind in ('fill','stroke'):out.append(' '.join(number(F(v,255)) for v in event['color'])+(' RG' if kind=='stroke' else ' rg'))
        if kind=='stroke':
            s=event['style'];out.extend([number(s['width'])+' w',str(s['cap'])+' J',str(s['join'])+' j',number(s['miter'])+' M'])
        out.extend(path_text(event['path']))
        if kind=='clip':out.append('W* n' if event.get('rule')=='evenodd' else 'W n')
        else:out.extend(['S' if kind=='stroke' else 'f*' if event.get('rule')=='evenodd' else 'f','Q'])
    if depth:raise ValueError('generated unbalanced clip')
    return ('\n'.join(out)+'\n').encode()

def case(label,scope,events,size=8):return dict(label=label,scope=scope,events=events,width=size,height=size,pdf=plain_pdf(content(events,size),size,size))

def generated():
    index=0
    for cap in (0,2):
        for width in (F(1,2),1,2):
            for axis in (0,1):
                for reverse in (False,True):
                    points=[(1,3),(7,3)]
                    if axis:points=[(y,x) for x,y in points]
                    if reverse:points.reverse()
                    path=[['m',*points[0]],['l',*points[1]]]
                    yield case(f'exact-{index}','controlled-exact-native',[stroke(path,width,cap)]);index+=1
    matrices=[(1,0,0,1,1,1),(1,F(1,4),F(1,2),1,1,0),(-1,0,F(1,4),1,7,1),(F(3,2),0,0,F(1,2),0,3)]
    index=0
    for cap in range(3):
        for matrix in matrices:
            for path in [[['m',1,1],['l',4,3]],[['m',1,1],['l',4,1],['l',4,4]]]:
                yield case(f'line-{index}','analytic-lines',[stroke(path,F(3,4),cap,2,matrix=matrix,color=(0,0,255))]);index+=1
    index=0
    for join in range(3):
        for matrix in matrices:
            for closed in (False,True):
                path=[['m',1,1],['l',4,1],['l',4,4]]
                if closed:path.extend([['l',1,4],['h']])
                yield case(f'join-{index}','analytic-joins',[stroke(path,1,1,join,miter=1 if index%2 else 10,matrix=matrix,color=(255,0,0))]);index+=1
    index=0
    for cap in range(3):
        for matrix in matrices:
            for width in (F(1,2),1):
                path=[['m',1,2],['c',2,3,4,3,5,2]]
                yield case(f'curve-{index}','certified-true-curves',[stroke(path,width,cap,1,matrix=matrix,color=(0,0,0))]);index+=1
    for i,width in enumerate((F(1,8),F(3,16))):
        yield case(f'minimum-width-{i}','native-minimum-width',[stroke([['m',1,F(25,8)],['l',7,F(25,8)]],width,0,0)])
    for i in range(12):
        events=[fill(box(0,0,8,8),[255,0,0]),clip(box(1,1,7,7)),stroke([['m',0,3],['l',8,3]],1,i%3,i%3,color=(0,0,255)),stroke([['m',2,0],['l',2,8]],F(1,2),i%3,i%3,color=(0,255,0)),dict(kind='pop'),fill(box(4,4,6,6),[255,255,0])]
        if i%2:events.insert(-1,stroke([['m',1,5],['c',2,6,5,6,6,5]],F(1,2),i%3,1,color=(0,0,0)))
        yield case(f'ordered-{i}','clip-and-order',events)
