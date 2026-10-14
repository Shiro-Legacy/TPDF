#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""R22 independent Form compositor: analytic strokes and exact Bernstein glyph curves."""
from fractions import Fraction as F
from page_raster_review_model import segments,sample_mask
from stroke_review_model import StrokeModel,Budget

def stroke_model(event):
    style=event['style'];return StrokeModel(event['path'],style['width'],cap=['butt','round','square'][style['cap']],join=['miter','round','bevel'][style['join']],miter_limit=style.get('miter',10),matrix=event.get('matrix',(1,0,0,1,0,0)),budget=Budget(max_work=32_000_000,max_nodes=200_000,max_queries=100_000))

def render(events,width,height,initial_bounds=None,margin=F(1,32)):
    if width>64 or height>64:raise ValueError('independent generated model canvas bound')
    n=width*height*16;bounds=tuple(map(F,initial_bounds or [0,0,width,height]));base=[]
    for y in range(height*4):
        for x in range(width*4):
            px,py=F(2*x+1,8),F(2*y+1,8);base.append(bounds[0]<=px<bounds[2] and bounds[1]<=py<bounds[3])
    stack=[(base,[False]*n)];pixels=[(255,255,255)]*n;uncertain=[False]*n;stats=dict(stroke_queries=0,stroke_work=0,stroke_nodes=0,uncertain_stroke_samples=0,certified_inside_stroke_samples=0,certified_outside_stroke_samples=0)
    for event in events:
        if event['kind']=='pop':
            if len(stack)==1:raise ValueError('model clip underflow')
            stack.pop();continue
        if event['kind']=='stroke':
            model=stroke_model(event);mask=[];ambiguous=[]
            for y in range(height*4):
                for x in range(width*4):
                    value=model.contains((F(2*x+1,8),F(2*y+1,8)),margin);mask.append(value is True);ambiguous.append(value is None)
            stats['stroke_queries']+=model.b.queries;stats['stroke_work']+=model.b.work;stats['stroke_nodes']+=model.b.nodes;stats['uncertain_stroke_samples']+=sum(ambiguous);stats['certified_inside_stroke_samples']+=sum(mask);stats['certified_outside_stroke_samples']+=sum(not a and not m for a,m in zip(ambiguous,mask))
        else:mask,ambiguous=sample_mask(segments(event['path'],event.get('matrix'),snap_lines=all(command[0] in ('m','l','h') for command in event['path'])),event.get('rule','nonzero'),width,height)
        parent,parent_unknown=stack[-1]
        if event['kind']=='clip':stack.append(([a and b for a,b in zip(parent,mask)],[a or (inside and b) for a,inside,b in zip(parent_unknown,parent,ambiguous)]));continue
        if event['kind'] not in ('fill','stroke'):raise ValueError('model event kind')
        color=tuple(event['color'])
        if len(color)!=3 or any(not isinstance(v,int) or not 0<=v<=255 for v in color):raise ValueError('model color')
        for i in range(n):
            if parent[i] and mask[i]:pixels[i]=color
            if parent[i] and mask[i] and not(parent_unknown[i] or ambiguous[i]):uncertain[i]=False
            else:uncertain[i]|=parent_unknown[i] or (parent[i] and ambiguous[i])
    if len(stack)!=1:raise ValueError('model clip not restored')
    out=bytearray();band=[];channel_bounds=[]
    for y in range(height):
        for x in range(width):
            ids=[(4*y+j)*(4*width)+4*x+i for j in range(4) for i in range(4)];out.extend((sum(pixels[k][c] for k in ids)+8)//16 for c in range(3));band.append(any(uncertain[k] for k in ids))
            unknown=sum(uncertain[k] for k in ids)
            for c in range(3):
                known=sum(pixels[k][c] for k in ids if not uncertain[k]);channel_bounds.append(((known+8)//16,(known+255*unknown+8)//16))
    stats['_channel_bounds']=channel_bounds
    stats.update(pixels=width*height,samples=n,uncertain_pixels=sum(band),uncertain_samples=sum(uncertain));return bytes(out),band,stats
