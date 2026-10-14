#!/usr/bin/env python3
"""Public viewing fixture for pinned styles and indexed color-key compositing."""
import pathlib,sys
output=pathlib.Path(sys.argv[1]);output.parent.mkdir(parents=True,exist_ok=True)
def stream(data):return f'<< /Length {len(data)} >>\nstream\n'.encode()+data+b'\nendstream'
fonts=[('ArialMT',[667,667,722,722]),('Arial-BoldMT',[722]*4),('Arial-BoldItalicMT',[722]*4)]
objects=[b'<< /Type /Catalog /Pages 2 0 R >>',b'<< /Type /Pages /Kids [9 0 R 10 0 R 11 0 R 12 0 R] /Count 4 >>']
for i,(name,widths) in enumerate(fonts):
 objects.append(f'<< /Type /Font /Subtype /TrueType /BaseFont /{name} /FontDescriptor {6+i} 0 R /FirstChar 65 /LastChar 68 /Widths [{" ".join(map(str,widths))}] /Encoding /WinAnsiEncoding >>'.encode())
for name,_ in fonts:
 objects.append(f'<< /Type /FontDescriptor /FontName /{name} /Flags 32 /FontBBox [-200 -300 1100 1100] /ItalicAngle 0 /Ascent 1000 /Descent -300 /CapHeight 700 /StemV 80 >>'.encode())
for i in range(4):
 resource=f'/Font << /F1 {3+i} 0 R >>' if i<3 else '/XObject << /Im 17 0 R >>'
 objects.append(f'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Resources << {resource} >> /Contents {13+i} 0 R >>'.encode())
for i in range(3):objects.append(stream(b'/RelativeColorimetric ri 0 i BT /F1 36 Tf 30 200 Td (ABCD) Tj ET'))
objects.append(stream(b'0 0 1 rg 0 0 300 300 re f q 100 0 0 100 0 0 cm /Im Do Q'))
objects.append(b'<< /Type /XObject /Subtype /Image /Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace [/Indexed /DeviceRGB 1 <ff000000ff00>] /Decode [0 255] /Mask [1 1] /Length 2 >>\nstream\n\x00\x01\nendstream')
data=bytearray(b'%PDF-1.7\n');offsets=[]
for i,obj in enumerate(objects):offsets.append(len(data));data+=f'{i+1} 0 obj\n'.encode()+obj+b'\nendobj\n'
xref=len(data);data+=f'xref\n0 {len(objects)+1}\n0000000000 65535 f \n'.encode()
for offset in offsets:data+=f'{offset:010} 00000 n \n'.encode()
data+=f'trailer\n<< /Size {len(objects)+1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode();output.write_bytes(data)
