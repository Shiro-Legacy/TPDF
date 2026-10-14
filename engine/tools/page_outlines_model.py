#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Independent rational state and CFF page geometry, with derived IEEE roundoff enclosures."""
import copy,math
from functools import lru_cache
from fractions import Fraction as F
from page_glyph_model import page_matrix
from program_model import ProgramModel,generate_programs
from text_position_model import number

def pdf(raw,fonts,crop=(0,0,100,120),rotation=0,unit=1):
    def stream(b):return b'<< /Length '+str(len(b)).encode()+b' >>\nstream\n'+b+b'\nendstream'
    objects=[b'<< /Type /Catalog /Pages 2 0 R >>',b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',b'']
    resources=[]
    for label,f in fonts.items():
        obj=len(objects)+1;resources.append(f'/{label} {obj} 0 R')
        diffs=' '.join(str(c)+' /'+name for c,name in sorted(f['names'].items()))
        widths=' '.join(number(f['widths'].get(c,0)) for c in range(256))
        objects.append(f'<< /Type /Font /Subtype /Type1 /BaseFont /{f["face"]} /Encoding << /BaseEncoding /WinAnsiEncoding /Differences [{diffs}] >> /FirstChar 0 /LastChar 255 /Widths [{widths}] /ToUnicode {obj+1} 0 R >>'.encode())
        pairs=b' '.join(f'<{c:02x}><{v.encode("utf-16-be").hex()}>'.encode() for c,v in f['mapping'].items())
        objects.append(stream(b'begincmap 1 begincodespacerange <00><ff> endcodespacerange '+str(len(f['mapping'])).encode()+b' beginbfchar '+pairs+b' endbfchar endcmap'))
    objects[2]=('<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /CropBox ['+' '.join(map(number,crop))+f'] /Rotate {rotation} /UserUnit {number(unit)} /Resources << /Font << '+' '.join(resources)+f' >> >> /Contents {len(objects)+1} 0 R >>').encode()
    objects.append(stream(raw));data=b'%PDF-1.7\n';offsets=[]
    for i,obj in enumerate(objects,1):offsets.append(len(data));data+=f'{i} 0 obj\n'.encode()+obj+b'\nendobj\n'
    at=len(data);data+=f'xref\n0 {len(objects)+1}\n0000000000 65535 f \n'.encode()+b''.join(f'{v:010} 00000 n \n'.encode() for v in offsets)
    return data+f'trailer\n<< /Root 1 0 R /Size {len(objects)+1} >>\nstartxref\n{at}\n%%EOF\n'.encode()

def generated(count=220):
    for spec in generate_programs(count):
        index=spec['case'];fonts=copy.deepcopy(spec['fonts'])
        for j,(label,f) in enumerate(fonts.items()):
            f['face']=['Helvetica','Times-Roman'][(index+j)%2]
            f['names']={32:'space',65:'B',66:'A',67:'C',68:'D'}
            f['object']=4+2*j
        crop=(F(index%11,2),F(index%7,2),100+F(index%11,2),120+F(index%7,2));rotation=index%4*90;unit=[F(1),F(1,2),F(2),F(3,2)][index%4];matrix=page_matrix(crop,rotation,unit)
        model=ProgramModel(fonts);model.state['ctm']=matrix;model.path_state['matrix']=matrix
        raw,expected=model.run(spec['operations'])
        yield dict(index=index,pdf=pdf(raw,fonts,crop,rotation,unit),fonts=fonts,expected=expected,geometry=matrix)

def rounded_interval(low,high):
    # One full binary64 ULP at both exact interval endpoints safely encloses
    # correctly rounded arithmetic, including binade transitions and subnormals.
    radius=max(F(math.ulp(float(low))),F(math.ulp(float(high))))
    return low-radius,high+radius

def scalar_product_interval(coef,pair):
    a,b=pair;a,b=sorted((F(coef)*a,F(coef)*b));return rounded_interval(a,b)

@lru_cache(maxsize=65536)
def coordinate_enclosure(x,y,a,c,e):
    x=rounded_interval(F(x,1000),F(x,1000));y=rounded_interval(F(y,1000),F(y,1000))
    ax=scalar_product_interval(a,x);cy=scalar_product_interval(c,y)
    partial=rounded_interval(ax[0]+cy[0],ax[1]+cy[1])
    return rounded_interval(partial[0]+e,partial[1]+e)

def check_commands(actual,canonical,matrix,stats):
    if len(actual)!=len(canonical):raise ValueError('rational command count')
    a,b,c,d,e,f=map(F,matrix)
    for row,want in zip(actual,canonical):
        if row[0]!=want[0] or len(row)!=len(want):raise ValueError('rational command shape')
        for i in range(1,len(want),2):
            x,y=want[i:i+2]
            for value,aa,cc,ee in ((row[i],a,c,e),(row[i+1],b,d,f)):
                exact=F(x,1000)*aa+F(y,1000)*cc+ee;lo,hi=coordinate_enclosure(x,y,aa,cc,ee)
                if not isinstance(value,(int,float)) or not math.isfinite(value) or not lo<=F(value)<=hi:raise ValueError('coordinate outside independent IEEE rational enclosure')
                stats['rational_coordinates']=stats.get('rational_coordinates',0)+1
                stats['rational_nonexact_coordinates']=stats.get('rational_nonexact_coordinates',0)+(F(value)!=exact)
                stats['rational_max_absolute_difference']=max(stats.get('rational_max_absolute_difference',0),float(abs(F(value)-exact)))
                stats['rational_max_enclosure_radius']=max(stats.get('rational_max_enclosure_radius',0),float(max(exact-lo,hi-exact)))
