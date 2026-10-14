#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Native unhinted trace -> font-free native draw geometry; no Rust raster input."""
import math,subprocess


def parse_ppm(data):
    fields=data.split(b'\n',3)
    if len(fields)!=4 or fields[0]!=b'P6' or fields[2]!=b'255':raise ValueError('unexpected PPM header')
    dims=fields[1].split()
    if len(dims)!=2 or any(not v.isdigit() for v in dims):raise ValueError('unexpected PPM dimensions')
    w,h=map(int,dims)
    if w<1 or h<1 or w*h>16777216 or len(fields[3])!=3*w*h:raise ValueError('unexpected PPM pixel extent')
    return w,h,fields[3]


def box4(data,w,h):
    if len(data)!=w*h*48:raise ValueError('4x reference dimensions')
    result=bytearray(w*h*3)
    for y in range(h):
        for x in range(w):
            for channel in range(3):
                total=sum(data[((4*y+j)*(4*w)+4*x+i)*3+channel] for j in range(4) for i in range(4))
                result[(y*w+x)*3+channel]=(total+8)//16
    return bytes(result)


def num(v):
    value=float(v)
    if not math.isfinite(value) or abs(value)>1e6:raise ValueError('native geometry coordinate bound')
    # Fixed-point decimal; avoid unsupported PDF exponent syntax.
    return format(value,'.12f').rstrip('0').rstrip('.') or '0'


def point(x,y,matrix):
    a,b,c,d,e,f=matrix;return [a*x+c*y+e,b*x+d*y+f]


def commands_pdf(commands,matrix=(1,0,0,1,0,0)):
    out=[];current=start=None
    for command in commands:
        kind=command[0];args=[]
        for i in range(1,len(command),2):args.extend(point(command[i],command[i+1],matrix))
        if kind=='q':
            if current is None:raise ValueError('quadratic without current point')
            control=args[:2];end=args[2:];args=[current[i]+(control[i]-current[i])*2/3 for i in range(2)]+[end[i]+(control[i]-end[i])*2/3 for i in range(2)]+end;kind='c'
        if kind not in ('m','l','c','h'):raise ValueError('unexpected geometry command')
        out.append(' '.join([*(num(v) for v in args),kind]))
        if kind=='m':start=args[:];current=args[:]
        elif kind in ('l','c'):current=args[-2:]
        else:current=start
    return out


def events_from_native(page):
    if page['status']!='accepted':raise ValueError(page['error'])
    events=[]
    for event in page['events']:
        kind=event['kind']
        if kind=='glyph':
            if event['mode']==3:continue
            if event['mode']!=0:raise ValueError('unsupported text rendering mode')
            ctx=page['contexts'][event['context']]
            events.append(dict(kind='fill',path=event['unhinted_commands'],matrix=[1,0,0,1,0,0],color=ctx['color'],rule='nonzero'))
        elif kind=='stroke':raise ValueError('stroke outside filled-page raster subset')
        else:events.append(event)
    for event in events:
        if event['kind']=='fill' and event['color']['space'] not in ('gray','rgb'):raise ValueError('painted CMYK outside filled-page raster subset')
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
        matrix=event['matrix'];a,b,c,d,e,f=matrix;matrix=[scale*a,scale*b,scale*c,scale*d,scale*e,scale*f]
        if kind=='fill':
            color=event['color'];out.append(' '.join([*(num(v) for v in color['values']),'g' if color['space']=='gray' else 'rg']))
        out.extend(commands_pdf(event['path'],matrix))
        if kind=='clip':out.append('W* n' if event['rule']=='evenodd' else 'W n')
        elif kind=='fill':out.append('f*' if event['rule']=='evenodd' else 'f')
        else:raise ValueError('unexpected native event')
    if depth-1!=page['cleanup_pops']:raise ValueError('native cleanup depth differs')
    out.extend('Q' for _ in range(depth));return ('\n'.join(out)+'\n').encode()


def render_native(binary,page,w,h,dpi=72,aa=0,factor=4):
    source=geometry(page,dpi/72*factor)
    p=subprocess.run([str(binary),str(w*factor),str(h*factor),str(aa)],input=source,capture_output=True,timeout=50)
    if p.returncode or p.stderr:raise ValueError(f'native geometry exit{p.returncode}: '+p.stderr.decode(errors='replace')[:1500])
    nw,nh,rgb=parse_ppm(p.stdout)
    if (nw,nh)!=(w*factor,h*factor):raise ValueError('native dimensions differ')
    return box4(rgb,w,h) if factor==4 else rgb,source


def differences(actual,expected,w,h,boundary=None):
    if len(actual)!=len(expected) or len(actual)!=w*h*3:raise ValueError('pixel comparison dimensions')
    channels=[abs(a-b) for a,b in zip(actual,expected)];changed=[i for i in range(w*h) if actual[3*i:3*i+3]!=expected[3*i:3*i+3]]
    outside=sum(not boundary[i] for i in changed) if boundary is not None else None
    # Darkness area in pixel equivalents, summed across channels and divided by3.
    area=sum(b-a for a,b in zip(actual,expected))/765
    return dict(different_pixels=len(changed),max_channel_error=max(channels,default=0),mean_absolute_channel_error=sum(channels)/len(channels),rms_channel_error=math.sqrt(sum(v*v for v in channels)/len(channels)),coverage_area_error=area,difference_bounds=None if not changed else [min(i%w for i in changed),min(i//w for i in changed),max(i%w for i in changed)+1,max(i//w for i in changed)+1],boundary_pixels=sum(boundary) if boundary is not None else None,different_pixels_outside_boundary=outside,investigation_mae_over2=sum(channels)/len(channels)>2,investigation_rms_over12=math.sqrt(sum(v*v for v in channels)/len(channels))>12)


def boundary_mask(events,w,h,initial_bounds=None):
    """Conservative1px L-infinity shape band: pixel box expanded1, rational enclosure.
    A curve leaf's residual interval extent is <=1/4096px; ties stay in the band.
    """
    from fractions import Fraction as F
    from page_raster_review_model import segments,near_line,split
    paths=[]
    for event in events:
        if event['kind']!='pop':paths.extend(segments(event['path'],event.get('matrix')))
    if initial_bounds is not None:
        x0,y0,x1,y1=map(F,initial_bounds);paths.extend([((x0,y0),(x1,y0)),((x1,y0),(x1,y1)),((x1,y1),(x0,y1)),((x0,y1),(x0,y0))])
    result=[False]*(w*h)
    def touches(p,box,depth=0):
        xs=[v[0] for v in p];ys=[v[1] for v in p]
        if max(xs)<box[0] or min(xs)>box[2] or max(ys)<box[1] or min(ys)>box[3]:return False
        if len(p)==2:return near_line(p,((box[0]+box[2])/2,(box[1]+box[3])/2),F(3,2))
        if depth>=32 or max(xs)-min(xs)<=F(1,4096) and max(ys)-min(ys)<=F(1,4096):return True
        a,b=split(p);return touches(a,box,depth+1) or touches(b,box,depth+1)
    for p in paths:
        xs=[v[0] for v in p];ys=[v[1] for v in p]
        x0=max(0,math.floor(min(xs))-2);x1=min(w,math.ceil(max(xs))+2);y0=max(0,math.floor(min(ys))-2);y1=min(h,math.ceil(max(ys))+2)
        for y in range(y0,y1):
            for x in range(x0,x1):
                if not result[y*w+x] and touches(p,(F(x-1),F(y-1),F(x+2),F(y+2))):result[y*w+x]=True
    return result
