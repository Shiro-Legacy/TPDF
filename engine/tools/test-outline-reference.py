#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Native adapter invariants; independent Rust/geometry model is separate."""
import argparse, json, struct, subprocess, tempfile
from pathlib import Path

def sfnt(glyph=b'', lsb=0, short=False, include_metrics=True):
    head=bytearray(54);struct.pack_into('>I',head,0,0x10000);struct.pack_into('>I',head,12,0x5f0f3cf5);struct.pack_into('>H',head,18,1000);struct.pack_into('>h',head,50,0 if short else 1)
    if short and len(glyph)%2:glyph+=b'\0'
    maxp=struct.pack('>I14H',0x10000,1,64,8,0,0,2,0,0,0,0,0,0,0,0)
    loca=struct.pack('>2H' if short else '>2I',0,len(glyph)//2 if short else len(glyph))
    tables={b'head':head,b'maxp':maxp,b'loca':loca,b'glyf':glyph}
    if include_metrics:
        hhea=bytearray(36);struct.pack_into('>I',hhea,0,0x10000);struct.pack_into('>H',hhea,34,1)
        tables[b'hhea']=hhea;tables[b'hmtx']=struct.pack('>Hh',500,lsb)
    # No cmap/name/post is needed for explicit-GID FreeType loading.
    out=bytearray(struct.pack('>I4H',0x10000,len(tables),0,0,0)+bytes(16*len(tables)))
    for i,(tag,data) in enumerate(sorted(tables.items())):
        while len(out)%4:out.append(0)
        padded=bytes(data)+bytes((-len(data))%4);checksum=sum(struct.unpack('>'+str(len(padded)//4)+'I',padded))&0xffffffff if padded else 0
        struct.pack_into('>4s3I',out,12+16*i,tag,checksum,len(out),len(data));out+=data
    return bytes(out)

def glyph():
    # Two off-curve points (1,1),(4,6): implied start=(2.5,3.5).
    return struct.pack('>5h2H',1,1,1,4,6,1,0)+bytes([0,0])+struct.pack('>4h',1,3,1,5)

p=argparse.ArgumentParser();p.add_argument('oracle');a=p.parse_args();checks=0
with tempfile.TemporaryDirectory(prefix='outline-adapter-') as directory:
    q=Path(directory)/'queries.txt'
    def invoke(data,query=b'0\n',all_glyphs=False):
        q.write_bytes(query)
        return subprocess.run([a.oracle,'all' if all_glyphs else str(q)],input=data,capture_output=True,timeout=20)
    def accept(data,query=b'0\n',all_glyphs=False):
        global checks
        r=invoke(data,query,all_glyphs);assert r.returncode==0,(r.returncode,r.stderr);assert not r.stderr;r=json.loads(r.stdout);checks+=1;return r
    def refuse(data,query=b'0\n'):
        global checks
        r=invoke(data,query);assert r.returncode!=0 and not r.stdout,(r.returncode,r.stdout[:300],r.stderr);checks+=1
    for short in (False,True):
        for lsb in (-17,0,1,31):
            t=accept(sfnt(glyph(),lsb,short))['glyphs'][0]
            assert t['points']==[[1,1,False],[4,6,False]]
            assert t['bearing_adjustment']==1-lsb
            assert t['commands']==[['m',5,7],['q',2,2,5,7],['q',8,12,5,7],['h']]
            assert t['contour_endpoints']==[1]
    t=accept(sfnt(),all_glyphs=True);assert t['glyphs'][0]['kind']=='empty_loca' and t['glyphs'][0]['commands']==[]
    t=accept(sfnt(bytes(10)));assert t['glyphs'][0]['kind']=='zero_contours'
    t=accept(sfnt(glyph()),b' 0\r\n\n0\t\n');assert len(t['glyphs'])==2
    t=accept(sfnt(glyph()),b'');assert t['glyphs']==[]
    for bad in [b'-1',b'+0',b'1',b'4294967296',b'0 0',b'0\x00',b'\xff',b'0\n'*65537,b' '*1048577]:refuse(sfnt(glyph()),bad)
    refuse(sfnt(glyph(),include_metrics=False))
    refuse(sfnt(struct.pack('>h',-1)+bytes(8)))
    refuse(b'OTTO'+sfnt()[4:])
    refuse(sfnt(glyph())[:-1])
    refuse(b' '*(32*1024*1024+1))
print(json.dumps({'native_adapter_cases':checks,'status':'passed','minimal_native_tables':['head','maxp','loca','glyf','hhea','hmtx']}))
