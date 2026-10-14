#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Independent R18 PDF encoder + rational page/text/outline expectations."""
import copy, random, struct
from fractions import Fraction as F
from outline_model import font as outline_font
from truetype_cmap_model import tables, encode, cmap_table, with_cmap
from program_model import ProgramModel,generate_programs
from text_position_model import number,content,compose


def pdf(program,raw,widths,mapping,crop=(0,0,100,100),rotation=0,unit=1):
    pairs=b' '.join(f'<{code:02x}><{text.encode("utf-16-be").hex()}>'.encode() for code,text in mapping.items())
    tu=b'begincmap 1 begincodespacerange <00><ff> endcodespacerange '+str(len(mapping)).encode()+b' beginbfchar '+pairs+b' endbfchar endcmap'
    def stream(data):return b'<< /Length '+str(len(data)).encode()+b' >>\nstream\n'+data+b'\nendstream'
    geometry=(' /CropBox ['+' '.join(map(number,crop))+'] /Rotate '+str(rotation)+' /UserUnit '+number(unit)).encode()
    objects=[b'<< /Type /Catalog /Pages 2 0 R >>',b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
        b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300]'+geometry+b' /Resources << /Font << /F0 4 0 R >> >> /Contents 8 0 R >>',
        b'<< /Type /Font /Subtype /TrueType /BaseFont /Synthetic /Encoding /WinAnsiEncoding /FontDescriptor 5 0 R /FirstChar 0 /LastChar 255 /Widths ['+' '.join(number(widths.get(c,0)) for c in range(256)).encode()+b'] /ToUnicode 7 0 R >>',
        b'<< /Type /FontDescriptor /FontName /Synthetic /Flags 32 /FontBBox [-2000 -2000 2000 2000] /ItalicAngle 0 /Ascent 1000 /Descent -200 /CapHeight 700 /StemV 80 /FontFile2 6 0 R >>',stream(program),stream(tu),stream(raw)]
    data=b'%PDF-1.7\n';offsets=[]
    for i,obj in enumerate(objects,1):offsets.append(len(data));data+=f'{i} 0 obj\n'.encode()+obj+b'\nendobj\n'
    at=len(data);data+=f'xref\n0 {len(objects)+1}\n0000000000 65535 f \n'.encode()+b''.join(f'{v:010} 00000 n \n'.encode() for v in offsets)
    return data+f'trailer\n<< /Root 1 0 R /Size {len(objects)+1} >>\nstartxref\n{at}\n%%EOF\n'.encode()


def page_matrix(crop,rotation,unit):
    x0,y0,x1,y1=map(F,crop);u=F(unit)
    # Explicit four corner equations; no native/Rust geometry output feeds model.
    return {0:(u,0,0,-u,-u*x0,u*y1),90:(0,u,u,0,-u*y0,-u*x0),
        180:(-u,0,0,u,u*x1,-u*y0),270:(0,-u,-u,0,u*y1,u*x1)}[rotation]


def transform(commands,adjustment,units,matrix):
    a,b,c,d,e,f=matrix;result=[]
    for command in commands:
        out=[command[0]]
        for i in range(1,len(command),2):
            x=F(command[i]-2*adjustment,2*units);y=F(command[i+1],2*units)
            out.extend([a*x+c*y+e,b*x+d*y+f])
        result.append(out)
    return result


def generated(count=220):
    rng=random.Random(2026100318)
    for spec in generate_programs(count):
        index=spec['case'];contours=[None]
        for gid in (1,2):
            cs=[]
            for _ in range(1+index%3):
                cs.append([(rng.randrange(-400,801),rng.randrange(-400,801),bool(rng.randrange(2))) for _ in range(3+index%8)])
            contours.append(cs)
        font,outline=outline_font(contours,seed=index+1800,units=1024,long=bool(index%2))
        gids={32:2,65:1,66:2,67:1,68:2}
        font=with_cmap(font,cmap_table([(3,1,encode([(c,c,g-c,None) for c,g in sorted(gids.items())]))]))
        ops=copy.deepcopy(spec['operations'])
        for op,args in ops:
            if op=='Tf':args[0]='F0'
        crop=(F(index%11,2),F(index%7,2),100+F(index%11,2),120+F(index%7,2));rotation=(index%4)*90;unit=[F(1),F(1,2),F(2),F(3,2)][index%4]
        fonts={'F0':spec['fonts']['F0']};model=ProgramModel(fonts);matrix=page_matrix(crop,rotation,unit);model.state['ctm']=matrix;model.path_state['matrix']=matrix
        raw,expected=model.run(ops)
        yield dict(index=index,pdf=pdf(font,raw,fonts['F0']['widths'],fonts['F0']['mapping'],crop,rotation,unit),font=font,outline=outline,gids=gids,expected=expected,geometry=list(matrix),crop=crop,rotation=rotation,unit=unit)


def bearing_case():
    font,outline=outline_font([None,[[(100,200,True),(200,200,True),(200,300,True)]]],seed=1,units=1000)
    font=bytearray(font);at,size=tables(bytes(font))[b'hmtx'];struct.pack_into('>Hh',font,at+4,900,-40)
    font=with_cmap(bytes(font),cmap_table([(3,1,encode([(65,65,1-65,None)]))]));outline['glyphs'][1]['bearing_adjustment']=140
    widths={65:250};mapping={65:'\u03a9'};model=ProgramModel({'F0':dict(widths=widths,mapping=mapping)});matrix=page_matrix((0,0,100,100),0,1);model.state['ctm']=matrix;model.path_state['matrix']=matrix
    raw,expected=model.run([('BT',[]),('Tf',['F0',10]),('Tj',[b'AA']),('ET',[])])
    return dict(index='bearing-width',pdf=pdf(font,raw,widths,mapping),font=font,outline=outline,gids={65:1},expected=expected,geometry=list(matrix),crop=(0,0,100,100),rotation=0,unit=1)


def actual_generated(program,count=16):
    """Real fixture font; text/path geometry modeled, raw outline checked via native O0."""
    remap={32:72,65:65,66:76,67:80,68:72};mapping={65:'fi',72:'Z',76:' ',80:'\U0001f642'};widths={c:250 for c in mapping}
    for spec in generate_programs(count):
        index=spec['case'];ops=copy.deepcopy(spec['operations'])
        def replace(value):
            if isinstance(value,bytes):return bytes(remap[c] for c in value)
            if isinstance(value,list):return [replace(v) for v in value]
            return value
        ops=[(op,replace(args)) for op,args in ops]
        for op,args in ops:
            if op=='Tf':args[0]='F0'
        crop=(2,3,102,123);rotation=index%4*90;unit=[F(1),F(1,2),F(2),F(3,2)][index%4];matrix=page_matrix(crop,rotation,unit)
        model=ProgramModel({'F0':dict(widths=widths,mapping=mapping)});model.state['ctm']=matrix;model.path_state['matrix']=matrix;raw,expected=model.run(ops)
        yield dict(index='embedded-'+str(index),pdf=pdf(program,raw,widths,mapping,crop,rotation,unit),font=program,outline=None,gids={65:1,72:2,76:3,80:4},expected=expected)
