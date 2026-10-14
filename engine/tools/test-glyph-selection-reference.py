#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Native reference wrapper checks and explicit PDF routing conflicts."""
import argparse
import json
from pathlib import Path
import struct
import subprocess
from outline_model import font as outline_font
from truetype_cmap_model import encode,cmap_table,with_cmap,generated


def pdf(program, flags, encoding=b'', tounicode=False):
    mapping=b'1 begincodespacerange <00><ff> endcodespacerange 1 beginbfchar <41><03a9> endbfchar'
    objects=[b'<< /Type /Catalog /Pages 2 0 R >>', b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
             b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /Font << /F0 4 0 R >> >> >>',
             b'<< /Type /Font /Subtype /TrueType /BaseFont /Synthetic /FontDescriptor 5 0 R /FirstChar 0 /LastChar 255 /Widths ['+b'600 '*256+b'] '+encoding+(b' /ToUnicode 7 0 R' if tounicode else b'')+b' >>',
             b'<< /Type /FontDescriptor /FontName /Synthetic /Flags '+str(flags).encode()+b' /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 1000 /Descent 0 /CapHeight 700 /StemV 80 /FontFile2 6 0 R >>',
             b'<< /Length '+str(len(program)).encode()+b' >>\nstream\n'+program+b'\nendstream']
    if tounicode:objects.append(b'<< /Length '+str(len(mapping)).encode()+b' >>\nstream\n'+mapping+b'\nendstream')
    data=b'%PDF-1.7\n';offsets=[]
    for i,value in enumerate(objects,1):
        offsets.append(len(data));data+=str(i).encode()+b' 0 obj\n'+value+b'\nendobj\n'
    xref=len(data);count=len(objects)+1
    data+=b'xref\n0 '+str(count).encode()+b'\n0000000000 65535 f \n'+b''.join(f'{v:010} 00000 n \n'.encode() for v in offsets)
    return data+b'trailer\n<< /Root 1 0 R /Size '+str(count).encode()+b' >>\nstartxref\n'+str(xref).encode()+b'\n%%EOF\n'


def cases():
    base,_=outline_font([None]*257)
    unicode=encode([(65,65,11-65,None)])
    symbol=encode([(65,65,5-65,None),(0xf041,0xf041,9-0xf041,None),(0xf042,0xf042,10-0xf042,None)])
    pair=with_cmap(base,cmap_table([(3,1,unicode),(3,0,symbol)]))
    yield 'symbolic-priority-direct-before-f000',pdf(pair,4),dict(index=1,platform=3,encoding=0,format=4),{65:5,66:10,0:0}
    yield 'nonsymbolic-priority',pdf(pair,32,b'/Encoding /WinAnsiEncoding'),dict(index=0,platform=3,encoding=1,format=4),{65:11,66:0}
    yield 'nonsymbolic-bit-wins-both-flags',pdf(pair,36,b'/Encoding /WinAnsiEncoding'),dict(index=0,platform=3,encoding=1,format=4),{65:11}
    yield 'symbolic-ignores-encoding-names',pdf(pair,4,b'/Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /B] >>'),dict(index=1,platform=3,encoding=0,format=4),{65:5}
    yield 'tounicode-independent',pdf(pair,32,b'/Encoding /WinAnsiEncoding',True),dict(index=0,platform=3,encoding=1,format=4),{65:11}
    failed=with_cmap(base,cmap_table([(3,1,encode([(65,65,5-65,None)]))]))
    yield 'failed-name-override-retains-byte-gid',pdf(failed,32,b'/Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /B] >>'),dict(index=0,platform=3,encoding=1,format=4),{65:5}
    success=with_cmap(base,cmap_table([(3,1,encode([(65,66,5-65,None)]))]))
    yield 'successful-name-override',pdf(success,32,b'/Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /B] >>'),dict(index=0,platform=3,encoding=1,format=4),{65:6}
    mac=with_cmap(base,cmap_table([(1,0,encode([(32,32,106-32,None),(79,79,211-79,None)]))]))
    yield 'macroman-name-space-alias',pdf(mac,32,b'/Encoding /MacRomanEncoding'),dict(index=0,platform=1,encoding=0,format=4),{32:106,202:106,79:211}
    first=with_cmap(base,cmap_table([(3,1,encode([(65,65,0,None)])),(1,0,encode([(65,65,9-65,None)]))]))
    yield 'priority-record-no-cross-cmap-fallback',pdf(first,32,b'/Encoding /WinAnsiEncoding'),dict(index=0,platform=3,encoding=1,format=4),{65:65}


def main():
    p=argparse.ArgumentParser();p.add_argument('--cmap',default='/private/tmp/tandem-mupdf-reference/mupdf-truetype-cmap-oracle');p.add_argument('--glyph',default='/private/tmp/tandem-mupdf-reference/mupdf-glyph-inventory-oracle');a=p.parse_args()
    checks=0
    def run(binary,args,data):return subprocess.run([binary,*args],input=data,capture_output=True,timeout=20)
    data,expected=next(generated(1));r=run(a.cmap,[],data);assert r.returncode==0 and not r.stderr;assert json.loads(r.stdout)['records']==expected['records'];checks+=1
    for label,data,selected,mappings in cases():
        r=run(a.glyph,['4'],data);assert r.returncode==0 and not r.stderr,(label,r.stderr)
        j=json.loads(r.stdout);assert j['selected_cmap']==selected,(label,j['selected_cmap'])
        assert all(j['code_to_gid'][code]==gid for code,gid in mappings.items()),(label,j['code_to_gid'])
        checks+=1
    for binary,args,data in [(a.cmap,[],b' '*(32*1024*1024+1)),(a.cmap,[],b'OTTO'),(a.glyph,['4'],b' '*(16*1024*1024+1)),(a.glyph,['-1'],b''),(a.glyph,['4'],b'%PDF-1.7\n')]:
        r=run(binary,args,data);assert r.returncode!=0 and not r.stdout,(binary,r.returncode,r.stdout[:100]);checks+=1
    print(json.dumps({'native_reference_checks':checks,'status':'passed'}))
if __name__=='__main__':main()
