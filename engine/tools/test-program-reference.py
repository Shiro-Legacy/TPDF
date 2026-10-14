#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Bounded mixed native adapter tests; independent model/differential is separate."""
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
def accept(content, **kwargs):
 global checks
 r=invoke(pdf(content, **kwargs));assert r.returncode==0,(r.returncode,r.stderr);assert not r.stderr,r.stderr
 checks+=1;t=json.loads(r.stdout);assert t['initial_ctm']==[1,0,0,1,0,0] and t['initial_clip'] is None
 return t
def refuse(data):
 global checks
 r=invoke(data);assert r.returncode!=0 and not r.stdout,(r.returncode,r.stdout[:256],r.stderr);checks+=1

t=accept(b'0 0 5 5 re f q 0 0 10 10 re W n BT /F0 10 Tf 1 0 0 rg (A) Tj 0 0 1 rg (B) Tj ET Q 2 2 5 5 re S')
e=t['events'];assert [x['kind'] for x in e]==['fill','clip','text','text','pop','stroke']
assert [x['clip_ids'] for x in e]==[[],[],[1],[1],[],[]]
assert e[2]['color']=={'space':'rgb','values':[1,0,0]} and e[3]['color']=={'space':'rgb','values':[0,0,1]}
assert [x['unicode'] for x in e[2]['items']]==[102,105]
assert [x['primary'] for x in e[2]['items']]==[True,False]
assert e[2]['items'][0]['callback']==0 and e[3]['items'][0]['callback']==1
assert e[3]['items'][0]['page_matrix']==[10,0,0,10,5,0]
assert t['cleanup_pops']==0

t=accept(b'0 0 50 50 re W n q 1 1 20 20 re W* n BT /F0 10 Tf (D) Tj ET Q BT /F0 10 Tf (B) Tj ET')
e=t['events'];assert [x['kind'] for x in e]==['clip','clip','text','pop','text']
assert [x['clip_ids'] for x in e]==[[],[0],[0,1],[0],[0]]
assert e[2]['items'][0]['unicode']==0x1f642 and t['cleanup_pops']==1

t=accept(b'0 0 1 rg q 0 1 0 rg 2 0 0 3 7 8 cm BT /F0 10 Tf (A) Tj ET Q BT /F0 10 Tf (A) Tj ET')
e=t['events'];assert e[0]['color']['values']==[0,1,0] and e[1]['color']['values']==[0,0,1]
assert e[0]['items'][0]['page_matrix']==[20,0,0,30,7,8] and e[1]['items'][0]['page_matrix']==[10,0,0,10,0,0]

t=accept(b'3 Tr 0.5 g BT /F0 10 Tf 2 Tc 3 Tw (A B) Tj ET')
e=t['events'][0];assert e['kind']=='text' and e['mode']==3 and not e['color_observable'] and e['color'] is None
assert [x['page_matrix'][4] for x in e['items'] if x['primary']]==[0,7,17]

for op,space,values in [(b'0.5 g','gray',[.5]),(b'1 0 1 rg','rgb',[1,0,1]),(b'0 1 0 1 k','cmyk',[0,1,0,1])]:
 t=accept(op+b' BT /F0 10 Tf (B) Tj ET 0 0 3 4 re f');assert len(t['events'])==2
 assert all(x['color']=={'space':space,'values':values} for x in t['events'])

t=accept(b'q 3 w 2 J 1 j 7 M [2 3] 1 d 1 0 0 RG 0 0 m 2 3 l S BT /F0 10 Tf (A) Tj ET Q')
s=t['events'][0]['style'];assert s=={'width':3,'start_cap':2,'dash_cap':2,'end_cap':2,'join':1,'miter':7,'phase':1,'dash':[2,3]}
t=accept(b'q 0 0 3 3 re W B Q');assert [x['kind'] for x in t['events']]==['fill','stroke','clip','pop']
t=accept(b'q W n BT /F0 10 Tf (B) Tj ET Q');assert [x['kind'] for x in t['events']]==['clip','text','pop'];assert t['events'][0]['path']==[]
t=accept(b'f S B');assert [x['kind'] for x in t['events']]==['fill','stroke','fill','stroke'];assert all(x['path']==[] for x in t['events'])
t=accept(b'BT /F0 10 Tf (A) Tj ET',cmap=False,base=b'Times-Roman');assert t['events'][0]['items'][0]['unicode']==65

for content in [
 b'BT /F0 10 Tf q (A) Tj Q ET',b'BT /F0 10 Tf 7 Tr (A) Tj ET',
 b'BT /F0 10 Tf (A) Tj',b'BT /F0 10 Tf (A) Tj ET /x Do',b'BT BT ET ET',b'ET',b'Q',
 b'BT /Absent 10 Tf (A) Tj ET',b'BT /F0 10 Tf [ [(A)] ] TJ ET',b'BT 1 0 0 1 0 0 cm ET',
 b'0 0 m BT /F0 10 Tf (A) Tj ET',b'W BT /F0 10 Tf (A) Tj ET',b'BT 0 0 m ET',b'BT W ET',b'BT f ET',
 b'0 0 m 1 0 0 1 2 3 cm n',b'0 0 m',b'W',b'q',b'1 0 0 rg BI ID xyz EI',b'/x sh',b'/G gs',b'/Span BMC EMC',
 b'BT /F0 10 Tf (B) Tj ET '+b'q '*129+b'Q '*129,
 b'0 0 1 1 re W n '*1025,
 b'0 '*1000001,
]:
 refuse(pdf(content))
refuse(pdf(b'BT /F0 10 Tf (A) Tj ET',font_extra=b'/FontFile 99 0 R'))
refuse(pdf(b'BT /F0 10 Tf (A) Tj ET',base=b'Courier'))
refuse(pdf(b'BT /F0 10 Tf (A) Tj ET').split(b'xref')[0])
refuse(b' '*(4*1024*1024+1))
print(json.dumps({'native_adapter_cases':checks,'status':'passed'}))
