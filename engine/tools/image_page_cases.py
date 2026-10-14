# SPDX-License-Identifier: AGPL-3.0-or-later
"""Original image/Form page fixtures and independently authored semantic events."""
from fractions import Fraction as F
from image_decode_cases import pdf,stream
RAW=bytes([255,0,0, 0,255,0, 0,0,255, 255,255,0, 255,0,255, 0,255,255])
def matrix(m):return ' '.join(str(float(v)) if F(v).denominator!=1 else str(int(v)) for v in m)+' cm'
def concat(first,second):
    a,b,c,d,e,f=map(F,first);A,B,C,D,E,G=map(F,second)
    return [A*a+C*b,B*a+D*b,A*c+C*d,B*c+D*d,A*e+C*f+E,B*e+D*f+G]
def device(m):return concat(m,[1,0,0,-1,0,16])
def polygon(m,box):
    a,b,c,d,e,f=map(F,m);x0,y0,x1,y1=map(F,box)
    return [(a*x+c*y+e,b*x+d*y+f) for x,y in [(x0,y0),(x1,y0),(x1,y1),(x0,y1)]]
def image(m,clips=(),raw=RAW,w=3,h=2,c=3,inv=()):return dict(kind='image',matrix=device(m),width=w,height=h,components=c,raw=raw,invert=inv,clips=clips)
def fill(box,color,clips=()):return dict(kind='fill',polygon=polygon(device([1,0,0,1,0,0]),box),color=color,clips=clips)
def doc(content,image_data=RAW,imeta='',forms=(),page_extra='',resources='',gray=False):
    return pdf([b'<< /Type /Catalog /Pages 2 0 R >>',b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',f'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 16 16] /Resources << /XObject << /Im 5 0 R {resources} >> >> /Contents 4 0 R {page_extra} >>'.encode(),stream(content.encode()),stream(image_data,'/Type /XObject /Subtype /Image /Width 3 /Height 2 /BitsPerComponent 8 /ColorSpace '+('/DeviceGray' if gray else '/DeviceRGB')+' '+imeta),*forms])
def generated():
    matrices=[('axis',[12,0,0,8,2,4],True),('reflect-x',[-12,0,0,8,14,4],True),('reflect-y',[12,0,0,-8,2,12],True),('rotate-90',[0,12,-8,0,12,2],True),('rotate-270',[0,-12,8,0,4,14],True),('shear',[9,0,3,8,2,4],False),('full-affine',[9,3,-3,7,5,2],False),('reflect-shear',[-9,3,3,7,10,2],False),('fractional-axis',[9,0,0,7,F(17,8),F(25,8)],False),('outer-ties',[9,3,-3,7,F(33,8),F(17,8)],False),('small',[F(1,2),0,0,F(1,2),F(65,8),F(65,8)],False)]
    for name,m,parity in matrices:
        yield dict(label=name,pdf=doc('q '+matrix(m)+' /Im Do Q'),events=[image(m)],native_parity=parity)
    # The same image object under two transforms with intervening opaque path paint.
    m1=[9,0,0,6,1,1];m2=[0,9,-6,0,14,5]
    yield dict(label='ordered-image-fill-image-fill',pdf=doc('q '+matrix(m1)+' /Im Do Q 0 0 0 rg 2 3 10 4 re f q '+matrix(m2)+' /Im Do Q 1 1 1 rg 5 7 4 3 re f'),events=[image(m1),fill([2,3,12,7],[0,0,0]),image(m2),fill([5,7,9,10],[255,255,255])],native_parity=True)
    clip=polygon(device([1,0,0,1,0,0]),[5,3,10,11]);m=[12,0,0,8,2,4]
    yield dict(label='caller-clip-image',pdf=doc('q 5 3 5 8 re W n '+matrix(m)+' /Im Do Q'),events=[image(m,[clip])],native_parity=True)
    for inversion in ('[1 0 0 1 1 0]','[1 0 1 0 1 0]'):
        inv=(0,2) if inversion=='[1 0 0 1 1 0]' else (0,1,2)
        yield dict(label='decode-inverted-'+str(len(inv)),pdf=doc('q '+matrix(m)+' /Im Do Q',imeta='/Decode '+inversion),events=[image(m,inv=inv)],native_parity=True)
    # Nested affine Forms: noncommuting parent, Form and image transforms.
    caller=[1,0,0,1,2,1];fm=[1,F(1,4),F(1,4),1,1,1];nested=[1,0,F(1,4),1,1,0];im=[6,0,0,4,1,1]
    outer=concat(fm,caller);inner=concat(nested,outer);placed=concat(im,inner)
    clips=[polygon(device(outer),[0,0,10,10]),polygon(device(inner),[1,1,6,5])]
    forms=[stream('/Inner Do'.encode(),'/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Matrix [1 .25 .25 1 1 1] /Resources << /XObject << /Inner 7 0 R >> >>'),stream(('q '+matrix(im)+' /Im Do Q').encode(),'/Type /XObject /Subtype /Form /BBox [1 1 6 5] /Matrix [1 0 .25 1 1 0] /Resources << /XObject << /Im 5 0 R >> >>')]
    yield dict(label='nested-form-affine-bbox',pdf=doc('q '+matrix(caller)+' /Outer Do Q',forms=forms,resources='/Outer 6 0 R'),events=[image(placed,clips)],native_parity=False)
    # Exact interior texel boundaries and one representable PDF dyadic neighbor.
    for step in (-F(1,1024),F(0),F(1,1024)):
        mat=[9,0,0,8,F(17,8)+step,F(25,8)]
        yield dict(label='texel-tie-'+('minus' if step<0 else 'plus' if step>0 else 'exact'),pdf=doc('q '+matrix(mat)+' /Im Do Q'),events=[image(mat)],native_parity=False)

    # Page geometry is authored in original PDF metadata, not in content cm.
    def page_matrix(crop,rotation,unit):
        x0,y0,x1,y1=map(F,crop);u=F(unit)
        return {0:(u,0,0,-u,-u*x0,u*y1),90:(0,u,u,0,-u*y0,-u*x0),180:(-u,0,0,u,u*x1,-u*y0),270:(0,-u,-u,0,u*y1,u*x1)}[rotation]
    for rotation,crop,unit in [(90,[0,0,16,16],1),(180,[0,0,16,16],1),(270,[0,0,16,16],1),(0,[2,2,14,14],1),(90,[2,3,14,11],1),(0,[0,0,16,16],F(1,2)),(270,[2,2,14,14],F(1,2))]:
        m=[9,0,0,6,3,4];event=image(m);event['matrix']=concat(m,page_matrix(crop,rotation,unit));size=[(crop[2]-crop[0])*unit,(crop[3]-crop[1])*unit]
        if rotation in (90,270):size.reverse()
        extra='/Rotate '+str(rotation)+' /CropBox ['+' '.join(map(str,crop))+'] /UserUnit '+str(float(unit))
        yield dict(label='page-geometry-'+str(rotation)+'-'+str(crop[0])+'-'+str(unit).replace('/','_'),pdf=doc('q '+matrix(m)+' /Im Do Q',page_extra=extra),events=[event],native_parity=False,width=int(size[0]),height=int(size[1]))

    for inverted in (False,True):
        raw=bytes([0,51,102,153,204,255]);m=[12,0,0,8,2,4];event=image(m,raw=raw,c=1,inv=(0,) if inverted else ())
        yield dict(label='gray-inv'+str(int(inverted)),pdf=doc('q '+matrix(m)+' /Im Do Q',image_data=raw,gray=True,imeta='/Decode [1 0]' if inverted else ''),events=[event],native_parity=True)
