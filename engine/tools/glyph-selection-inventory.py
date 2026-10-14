#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Five actual embedded-font code/GID inventories, independent of ToUnicode."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess
from truetype_cmap_model import dense, records, tables

ENGINE = Path(__file__).resolve().parent.parent

def sha(data): return hashlib.sha256(data).hexdigest()
def array(text, name):
    return re.search(r'\b'+name+r'\[[^]]*\]\s*=\s*\{(.*?)\};', text, re.S).group(1)

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--source',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-1.28.1-source'))
    p.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-glyph-inventory-oracle'))
    p.add_argument('--fonts',type=Path,default=Path('/private/tmp/outline-fonts'))
    p.add_argument('--report',type=Path,default=ENGINE/'reference/glyph-selection-inventory.json')
    a=p.parse_args()
    pinned={'source/pdf/pdf-font.c':'c0c29c2826883e47191133f66b7d9fb769116f9923419606ac4f7136f54feca5',
            'source/fitz/font.c':'7fe9aa150fd9ac515e769f2213570b9e696a5650bdb7cd2a62a2bef35a48b6e6',
            'source/fitz/encodings.h':'224122c5581a91254a22d3a009c50322d39fbbac5f1b2c1c5464cba8528839de',
            'source/fitz/glyphlist.h':'16fe0427189e16ff852c02984a49dba184c0db51756b8ecef01524ad6efb6002',
            'source/fitz/encodings.c':'db046231a23e4470c89e31850cff8670a1ca95def64c71d0d4f4c416622fe4a6',
            'COPYING':'57c8ff33c9c0cfc3ef00e650a1cc910d7ee479a8bc509f6c9209a7c2a11399d6',
            'thirdparty/freetype/docs/FTL.TXT':'5a5ee54c5001bbad1cdc1a57cc3dd4c42199b2da09d39c7ee41fab002d02967f'}
    for file,want in pinned.items():
        assert sha((a.source/file).read_bytes())==want,file
    enc_text=(a.source/'source/fitz/encodings.h').read_text()
    encodings={}
    for label,key in [('WinAnsiEncoding','win_ansi'),('MacRomanEncoding','mac_roman')]:
        encodings[label]=[name or None for name,none in re.findall(r'"([^"]+)"|(_notdef)',array(enc_text,'fz_glyph_name_from_'+key))]
        assert len(encodings[label])==256
    glyph_text=(a.source/'source/fitz/glyphlist.h').read_text()
    names=re.findall(r'"([^"]+)"',array(glyph_text,'single_name_list'))
    codes=[int(v,16) for v in re.findall(r'0x([0-9a-fA-F]+)',array(glyph_text,'single_code_list'))]
    unicode=dict(zip(names,codes)); assert len(names)==len(codes)==len(unicode)
    # Exact native MacRoman exceptions, ordered before the first table-name match.
    mre=dict(notequal=173,infinity=176,lessequal=178,greaterequal=179,partialdiff=182,
             summation=183,product=184,pi=185,integral=186,Omega=189,radical=195,
             approxequal=197,Delta=198,lozenge=215,Euro=219,apple=240)
    for i,name in enumerate(encodings['MacRomanEncoding']):
        if name is not None: mre.setdefault(name,i)
    inventory=ENGINE/'reference/embedded-outline-inventory.json'
    evidence=[a.native,inventory,Path(__file__),ENGINE/'tools/truetype_cmap_model.py']
    hashes={str(f):sha(f.read_bytes()) for f in evidence}
    rows=[]
    for index,font in enumerate(json.loads(inventory.read_text())['fonts']):
        original=(ENGINE.parent/'fixtures'/font['file']).read_bytes();assert sha(original)==font['fixtureSha256']
        data=(a.fonts/f'font{index}.ttf').read_bytes();assert sha(data)==font['programs'][0]['sha256']
        recs=records(data);maps=[dense(data,i) for i in range(len(recs))]
        symbolic=bool(font['flags']&4) and not bool(font['flags']&32)
        selected=None
        for pair in ([(3,0)] if symbolic else [])+[(3,1),(1,0)]:
            selected=next((i for i,r in enumerate(recs) if (r['platform'],r['encoding'])==pair),None)
            if selected is not None:break
        if selected is None:selected=0
        rec=recs[selected];mapping=maps[selected]
        tab=tables(data);post_names={};post=None
        if b'post' in tab:
            off,length=tab[b'post'];body=data[off:off+length];version=struct.unpack_from('>I',body)[0]
            assert version==0x20000
            count=struct.unpack_from('>H',body,32)[0];indices=struct.unpack_from('>'+str(count)+'H',body,34)
            # Only these five standard post indices occur in the inventory. No generalized post parser claim.
            standard={0:'.notdef',36:'A',43:'H',47:'L',51:'P'}
            assert all(i in standard for i in indices) and 34+2*count==len(body)
            post_names={standard[name]:gid for gid,name in enumerate(indices)}
            post=dict(version='00020000',indices=list(indices),names=[standard[n] for n in indices])
        encoding=font['encoding'].lstrip('/') if font['encoding'] else None
        glyph_names=encodings[encoding] if encoding else [None]*256
        def char(code):
            for candidate in [code,0xf000+code]+([0x2026] if code==0x22ef else []):
                if 0<=candidate<65536 and mapping[candidate]:return mapping[candidate],candidate
            return 0,None
        expected=[];provenance=[]
        for code,name in enumerate(glyph_names):
            gid,cp=char(code);reason='source_code_cmap' if cp==code else 'source_code_symbol_offset' if cp is not None else 'notdef'
            if name and not symbolic and rec['platform']==3:
                candidate,namecp=char(unicode[name]);route='encoding_name_unicode'
                if not candidate:
                    candidate=post_names.get(name,0);route='post_exact_name'
                    # For the actual post repertoire, no alternate same-Unicode name changes the answer.
                    assert not any(g and n!=name and unicode.get(n)==unicode[name] for n,g in post_names.items())
                    assert not post_names.get(f'uni{unicode[name]:04X}',0)
                if candidate:gid,cp,reason=candidate,namecp,route
            elif name and not symbolic and rec['platform']==1:
                mrcode=mre.get(name,-1);candidate,namecp=char(mrcode) if mrcode>0 else (0,None)
                if candidate:gid,cp,reason=candidate,namecp,'encoding_name_macroman'
                elif post_names.get(name,0):gid,cp,reason=post_names[name],None,'post_exact_name'
            elif name and symbolic:
                assert (rec['platform'],rec['encoding'])==(3,0),'inventory needs wider symbolic rules'
            expected.append(gid)
            if gid:provenance.append(dict(code=code,gid=gid,encoding_name=name,cmap_codepoint=cp,route=reason))
        native=subprocess.run([str(a.native),font['font'].split(':')[0]],input=original,capture_output=True,timeout=20)
        assert native.returncode==0 and not native.stderr,(font['file'],native.stderr)
        got=json.loads(native.stdout)
        assert got['code_to_gid']==expected,(font['file'],[(i,a,b) for i,(a,b) in enumerate(zip(got['code_to_gid'],expected)) if a!=b])
        assert got['selected_cmap']=={k:rec[k] for k in ['index','platform','encoding','format']}
        for r,values,n in zip(recs,maps,got['cmaps']):
            assert n['mappings']==[[i,g] for i,g in enumerate(values) if g]
        rows.append(dict(file=font['file'],font=font['font'],fixture_sha256=font['fixtureSha256'],font_sha256=sha(data),
                         flags=font['flags'],symbolic=symbolic,encoding=encoding,selected_cmap=got['selected_cmap'],post=post,
                         cmap_records=[dict(**r,nonzero_pairs=[[i,g] for i,g in enumerate(values) if g]) for r,values in zip(recs,maps)],
                         code_to_gid=expected,nonzero_codes=provenance,notdef_codes=sum(g==0 for g in expected),
                         native_output_sha256=sha(native.stdout),status='matched'))
    assert hashes=={str(f):sha(f.read_bytes()) for f in evidence}
    result=dict(stage='R17 read-only mapping inventory',font_count=len(rows),cmap_records=sum(len(r['cmap_records']) for r in rows),
                compared_pdf_codes=256*len(rows),nonzero_pdf_codes=sum(len(r['nonzero_codes']) for r in rows),
                source_sha256=pinned,evidence_sha256=hashes,rows=rows,
                limitations=['No production implementation or used-page glyph rendering acceptance.',
                             'Independent inventory scopes post parsing to the five observed standard indices; generalized post/name/alias fallback is not implemented.',
                             'ToUnicode is never read by the independent mapping model; native font loading may read it but cid_to_gid is captured independently.',
                             'Code/GID inventory includes unused source codes; a mapping is not evidence of use or successful outlining.',
                             'MuPDF source policies/data are AGPL-3.0-or-later; bundled FreeType reference is test-only and carries its own license.'])
    a.report.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({k:result[k] for k in ['font_count','cmap_records','compared_pdf_codes','nonzero_pdf_codes']}))
if __name__=='__main__':main()
