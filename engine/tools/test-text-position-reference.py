#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Bounded native text adapter tests; independent placement model is separate."""
import argparse, subprocess, json
def pdf(content, cmap=True, font_extra=b'', base=b'Helvetica'):
 mapping=b'1 begincodespacerange <00> <ff> endcodespacerange 4 beginbfchar <41> <00660069> <42> <0020> <20> <005a> <44> <d83dde42> endbfchar'
 font=b'<< /Type /Font /Subtype /Type1 /BaseFont /'+base+b' /Encoding /WinAnsiEncoding /FirstChar 32 /LastChar 90 /Widths ['+b'500 '*59+b'] '+font_extra+(b' /ToUnicode 6 0 R' if cmap else b'')+b' >>'
 objects=[b'<< /Type /Catalog /Pages 2 0 R >>',b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F0 4 0 R >> >> /Contents 5 0 R >>',font,b'<< /Length '+str(len(content)).encode()+b' >>\nstream\n'+content+b'\nendstream',b'<< /Length '+str(len(mapping)).encode()+b' >>\nstream\n'+mapping+b'\nendstream']
 data=b'%PDF-1.7\n';off=[0]
 for i,obj in enumerate(objects,1):off.append(len(data));data+=str(i).encode()+b' 0 obj\n'+obj+b'\nendobj\n'
 xref=len(data);data+=b'xref\n0 7\n0000000000 65535 f \n'+b''.join(f'{x:010d} 00000 n \n'.encode() for x in off[1:]);data+=b'trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n'+str(xref).encode()+b'\n%%EOF\n';return data

p=argparse.ArgumentParser();p.add_argument('oracle');args=p.parse_args();checks=0
def invoke(data):
 return subprocess.run([args.oracle],input=data,capture_output=True,timeout=20)
def accept(data):
 global checks
 r=invoke(data);assert r.returncode==0,(r.returncode,r.stderr);assert not r.stderr,r.stderr
 checks+=1;return json.loads(r.stdout)
def refuse(data):
 global checks
 r=invoke(data);assert r.returncode!=0 and not r.stdout,(r.returncode,r.stdout,r.stderr);checks+=1
trace=accept(pdf(b'BT /F0 10 Tf 1 0 0 1 10 20 Tm (A BD) Tj ET'))
items=trace['items'];assert [x['cid'] for x in items]==[65,-1,32,66,68]
assert [x['unicode'] for x in items]==[102,105,90,32,0x1f642]
assert [x['primary'] for x in items]==[True,False,True,True,True]
assert [x['page_matrix'][4] for x in items]==[10,10,15,20,25]
assert all(x['callback']==0 and x['mode']==0 for x in items)
trace=accept(pdf(b'3 Tr BT /F0 10 Tf 2 Tc 3 Tw (A B) Tj ET'))
assert [x['page_matrix'][4] for x in trace['items'] if x['primary']]==[0,7,17]
assert all(x['mode']==3 for x in trace['items'])
trace=accept(pdf(b'q 2 0 0 3 7 8 cm BT /F0 10 Tf (A) Tj ET Q'))
assert trace['items'][0]['page_matrix']==[20,0,0,30,7,8]
trace=accept(pdf(b'BT /F0 10 Tf (A) Tj ET',False));assert trace['items'][0]['unicode']==65
trace=accept(pdf(b'BT /F0 10 Tf (A) Tj ET',False,base=b'Times-Roman'));assert trace['items'][0]['unicode']==65
for content in [b'BT /F0 10 Tf q (A) Tj Q ET',b'BT /F0 10 Tf 7 Tr (A) Tj ET',b'BT /F0 10 Tf (A) Tj',b'BT /F0 10 Tf (A) Tj ET /x Do',b'BT BT ET ET',b'ET',b'Q',b'BT /Absent 10 Tf (A) Tj ET',b'BT /F0 10 Tf [ [(A)] ] TJ ET',b'BT 1 0 0 1 0 0 cm ET']:
 refuse(pdf(content))
refuse(pdf(b'BT /F0 10 Tf (A) Tj ET',font_extra=b'/FontFile 99 0 R'))
refuse(pdf(b'BT /F0 10 Tf (A) Tj ET',base=b'Courier'))
refuse(pdf(b'BT /F0 10 Tf (A) Tj ET').split(b'xref')[0])
refuse(b' '*(4*1024*1024+1))
print(json.dumps({'native_adapter_cases':checks,'status':'passed'}))
