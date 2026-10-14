#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Exhaustive native/model/Rust format4 mappings; strict integer equality."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
from truetype_cmap_model import dense, generated, records, tables, u16
ENGINE=Path(__file__).resolve().parent.parent

def sha(data):return hashlib.sha256(data).hexdigest()
def invoke(binary,args,data):
    p=subprocess.run([str(binary),*args],input=data,capture_output=True,timeout=20)
    if p.returncode or p.stderr:raise ValueError(f'exit {p.returncode}: {p.stderr.decode(errors="replace")[:1500]}')
    if len(p.stdout)>64*1024*1024:raise ValueError('unexpected output limit')
    return json.loads(p.stdout),sha(p.stdout)

def validate_rust(rust,native,expected,data,label='case'):
    if rust['glyph_count']!=native['glyph_count'] or len(rust['records'])!=len(expected):
        raise ValueError(label+': Rust metadata/count differs')
    raw_records=records(data);cmap_at,cmap_size=tables(data)[b'cmap']
    for got,want,raw in zip(rust['records'],expected,raw_records):
        selection=got['selection'];start=raw['offset']
        metadata=dict(index=want['index'],platform=want['platform'],encoding=want['encoding'],format=4,language=u16(data,start+4),segments=u16(data,start+6)//2,cmap_range=[cmap_at,cmap_at+cmap_size],subtable_range=[start,start+u16(data,start+2)])
        if selection!=metadata or got['gids']!=want['mappings']:
            raise ValueError(label+': Rust/model exhaustive mapping or provenance differs')


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-truetype-cmap-oracle'))
    p.add_argument('--rust',type=Path)
    p.add_argument('--fonts',type=Path,default=Path('/private/tmp/outline-fonts'))
    p.add_argument('--report',type=Path,default=ENGINE/'reference/truetype-cmap-differential-g0.json')
    a=p.parse_args()
    inputs=[a.native,Path(__file__),ENGINE/'tools/truetype_cmap_model.py',ENGINE/'tools/outline_model.py',ENGINE/'reference/embedded-outline-inventory.json',ENGINE/'tools/mupdf-truetype-cmap-oracle.c',ENGINE/'tools/build-glyph-selection-reference.sh']
    if a.rust:inputs.append(a.rust)
    hashes={str(f):sha(f.read_bytes()) for f in inputs}
    rows=[]
    def run(label,data,expected,source):
        row=dict(case=label,source=source,sha256=sha(data),records=len(expected),queries=65536*len(expected))
        native,row['native_output_sha256']=invoke(a.native,[],data)
        if native['records']!=expected:
            raise ValueError(label+': native/model exhaustive mapping differs')
        if a.rust:
            rust,row['rust_output_sha256']=invoke(a.rust,['--tt-cmap=all','-'],data)
            validate_rust(rust,native,expected,data,label)
        row['status']='matched';rows.append(row)
    inventory=json.loads((ENGINE/'reference/embedded-outline-inventory.json').read_text())
    for i,font in enumerate(inventory['fonts']):
        data=(a.fonts/f'font{i}.ttf').read_bytes();assert sha(data)==font['programs'][0]['sha256']
        assert sha((ENGINE.parent/'fixtures'/font['file']).read_bytes())==font['fixtureSha256']
        expected=[dict(index=r['index'],platform=r['platform'],encoding=r['encoding'],format=r['format'],mappings=dense(data,r['index'])) for r in records(data)]
        run(font['file']+' font '+font['font'],data,expected,'fixture')
    for data,expected in generated():
        # The independently expanded parser also cross-checks the generation model.
        assert all(dense(data,r['index'])==r['mappings'] for r in expected['records'])
        run('generated-'+str(expected['index']),data,expected['records'],'generated')
    assert hashes=={str(f):sha(f.read_bytes()) for f in inputs},'gate inputs changed'
    result=dict(stage='G0 cmap format4',native_only=a.rust is None,evidence_sha256=hashes,
                fixtures=sum(r['source']=='fixture' for r in rows),fixture_records=sum(r['records'] for r in rows if r['source']=='fixture'),
                generated=sum(r['source']=='generated' for r in rows),records=sum(r['records'] for r in rows),
                queries=sum(r['queries'] for r in rows),failures=0,comparison='Exact all65536 16-bit character codes including missing and sentinel per selected original record; no tolerance.',
                limitations=['Raw cmap selection by record only; PDF flags/encoding/glyph-name routing is a separate gate.',
                             'Native raw charmap query applies no U+F000 fallback; PDF helper behavior is deliberately separate.',
                             'Strict malformed and budget refusal behavior is tested independently; native recovery is not accepted as parity.'],rows=rows)
    a.report.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({k:result[k] for k in ['native_only','fixtures','fixture_records','generated','records','queries','failures']}))
if __name__=='__main__':main()
