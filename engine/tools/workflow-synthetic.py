#!/usr/bin/env python3
"""Public synthetic stream-array and positioned-TJ workflow regressions."""
import pathlib,sys,zlib
output=pathlib.Path(sys.argv[1]);output.mkdir(parents=True,exist_ok=True)
def pdf(objects):
    data=bytearray(b'%PDF-1.7\n');offsets=[]
    for i,obj in enumerate(objects):
        offsets.append(len(data));data+=f'{i+1} 0 obj\n'.encode()+obj+b'\nendobj\n'
    xref=len(data);data+=f'xref\n0 {len(objects)+1}\n0000000000 65535 f \n'.encode()
    for offset in offsets:data+=f'{offset:010} 00000 n \n'.encode()
    data+=f'trailer\n<< /Size {len(objects)+1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode()
    return data
def stream(content):
    data=zlib.compress(content);return f'<< /Length {len(data)} /Filter /FlateDecode >>\nstream\n'.encode()+data+b'\nendstream'
font=b'<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>'
for name,parts in [('array',[b'BT /F1 12 Tf 20 250 Td',b'(ALPHA) Tj ET',b'BT /F1 12 Tf 20 200 Td (NEIGHBOR) Tj ET']),('tj',[b'BT /F1 12 Tf 20 250 Td [(AL) 30 (PHA)] TJ ET BT /F1 12 Tf 20 200 Td (NEIGHBOR) Tj ET'])]:
    refs=' '.join(f'{i+5} 0 R' for i in range(len(parts)))
    page=f'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Resources << /Font << /F1 4 0 R >> >> /Contents [{refs}] >>'.encode()
    data=pdf([b'<< /Type /Catalog /Pages 2 0 R >>',b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',page,font,*map(stream,parts)])
    (output/(name+'.pdf')).write_bytes(data)
