# SPDX-License-Identifier: AGPL-3.0-or-later
"""Independent original byte images and PDF envelopes for R23 decode evidence."""
import base64, random, runpy, zlib
from pathlib import Path
LZW=runpy.run_path(str(Path(__file__).with_name('differential-lzw.py')))['encode']
def pdf(objects):
    data=bytearray(b'%PDF-1.7\n'); offsets=[]
    for i,obj in enumerate(objects,1):
        offsets.append(len(data));data.extend(f'{i} 0 obj\n'.encode()+obj+b'\nendobj\n')
    xref=len(data);data.extend(f'xref\n0 {len(objects)+1}\n0000000000 65535 f \n'.encode())
    for off in offsets:data.extend(f'{off:010d} 00000 n \n'.encode())
    data.extend(f'trailer\n<< /Size {len(objects)+1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode());return bytes(data)
def stream(data,meta=''):
    return f'<< /Length {len(data)} {meta} >>\nstream\n'.encode()+data+b'\nendstream'
def document(raw,w=7,h=5,c=3,meta='',extra=(),replace=None,content=None):
    fields={'Type':'/XObject','Subtype':'/Image','Width':str(w),'Height':str(h),'BitsPerComponent':'8','ColorSpace':'/DeviceRGB' if c==3 else '/DeviceGray'}
    fields.update(replace or {});fields=' '.join('/'+k+' '+v for k,v in fields.items() if v is not None)
    return pdf([b'<< /Type /Catalog /Pages 2 0 R >>',b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 32 32] /Resources << /XObject << /Im 5 0 R >> >> /Contents 4 0 R >>',stream(content or b'q 24 0 0 24 4 4 cm /Im Do Q'),stream(raw,fields+' '+meta),*extra])
def ppm(raw,w,h,c,invert=()):
    pixels=bytearray()
    for i in range(0,len(raw),c):
        p=[255-x if j in invert else x for j,x in enumerate(raw[i:i+c])];pixels.extend(p*3 if c==1 else p)
    return f'P6\n{w} {h}\n255\n'.encode()+pixels

def predictor(raw,w,h,c,p):
    out=bytearray();prior=bytes(w*c)
    for row in range(h):
        current=raw[row*w*c:(row+1)*w*c];mode=(row%5 if p==15 else p-10)
        if p!=2:out.append(mode)
        for i,value in enumerate(current):
            left=current[i-c] if i>=c else 0;above=prior[i];ul=prior[i-c] if i>=c else 0
            est=left+above-ul;paeth=min((left,above,ul),key=lambda v:abs(est-v))
            predicted=left if p==2 else (0,left,above,(left+above)//2,paeth)[mode]
            out.append((value-predicted)%256)
        prior=current
    return bytes(out)
def runlength(raw):
    return b''.join(bytes([len(raw[i:i+128])-1])+raw[i:i+128] for i in range(0,len(raw),128))+b'\x80'
def generated():
    rng=random.Random(20261007)
    for c in (1,3):
      for w,h in ((1,1),(7,5),(257,7)):
        raw=bytes(rng.randrange(256) for _ in range(w*h*c))
        variants=[('identity',raw,''),('hex',raw.hex().encode()+b'>','/Filter /ASCIIHexDecode'),('ascii85',base64.a85encode(raw)+b'~>','/Filter /ASCII85Decode'),('runlength',runlength(raw),'/Filter /RunLengthDecode'),('flate',zlib.compress(raw),'/Filter /FlateDecode'),('chain',base64.a85encode(zlib.compress(raw))+b'~>','/Filter [/ASCII85Decode /FlateDecode] /DecodeParms [null null]')]
        for early in (0,1):variants.append((f'lzw-{early}',LZW(raw,early)[0],f'/Filter /LZWDecode /DecodeParms << /EarlyChange {early} >>'))
        for p in (2,10,11,12,13,14,15):
          filtered=predictor(raw,w,h,c,p)
          for filter_name in ('FlateDecode','LZWDecode'):
            encoded=zlib.compress(filtered) if filter_name=='FlateDecode' else LZW(filtered,0)[0]
            variants.append((f'{filter_name}-predictor-{p}',encoded,f'/Filter /{filter_name} /DecodeParms << /EarlyChange 0 /Predictor {p} /Colors {c} /Columns {w} /BitsPerComponent 8 >>' if filter_name=='LZWDecode' else f'/Filter /{filter_name} /DecodeParms << /Predictor {p} /Colors {c} /Columns {w} /BitsPerComponent 8 >>'))
        for label,encoded,meta in variants:
          for inverted in (False,True):
            inv=tuple(range(0,c,2)) if inverted else ();decode='/Decode ['+' '.join('1 0' if i in inv else '0 1' for i in range(c))+']'
            yield dict(label=f'{c}-{w}x{h}-{label}-inv{int(inverted)}',pdf=document(encoded,w,h,c,meta+' '+decode),expected=ppm(raw,w,h,c,inv),accepted=True)
    raw=bytes(range(105));base=dict(raw=raw)
    refs=dict(Width='6 0 R',Height='7 0 R',ColorSpace='8 0 R',BitsPerComponent='9 0 R')
    yield dict(label='indirect-dimensions-colorspace-depth',pdf=document(raw,extra=[b'7',b'5',b'/DeviceRGB',b'8'],replace=refs),expected=ppm(raw,7,5,3),accepted=True)
    for field,value in [('Width','0'),('Width','7.0'),('Height','-1'),('BitsPerComponent','16'),('ColorSpace','/DeviceCMYK'),('Interpolate','true'),('ImageMask','true'),('Decode','[0 0.5 0 1 0 1]'),('Decode','[0 1]'),('Mask','[0 1 0 1 0 1]'),('SMask','8 0 R'),('Interpolate','3'),('OC','<<>>'),('SMaskInData','1'),('ColorSpace','6 0 R')]:
      yield dict(label='strict-'+field+'-'+value.replace('/','').replace(' ','_'),pdf=document(raw,replace={field:value}),accepted=False)
    for label,other in [('truncated',raw[:-1]),('extra',raw+b'x')]:yield dict(label=label,pdf=document(other),accepted=False)
    for label,field,extras in [('cyclic-mask','Mask',[b'6 0 R']),('cyclic-interpolate','Interpolate',[b'6 0 R']),('stream-width','Width',[stream(b'7')])]:yield dict(label=label,pdf=document(raw,replace={field:'6 0 R'},extra=extras),accepted=False)
    for label,meta in [('predictor-colors','/Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors 1 /Columns 7 >>'),('predictor-columns','/Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors 3 /Columns 8 >>'),('bad-filter','/Filter /DCTDecode'),('unknown-params','/Filter /FlateDecode /DecodeParms << /Unknown 1 >>')]:yield dict(label=label,pdf=document(zlib.compress(raw),meta=meta),accepted=False)
