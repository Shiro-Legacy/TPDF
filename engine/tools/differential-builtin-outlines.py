#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Every compiled pinned name/GID/advance/canonical command versus fresh native query."""
import argparse,gzip,hashlib,json,subprocess
from pathlib import Path
ENGINE=Path(__file__).resolve().parent.parent

def sha(data):return hashlib.sha256(data).hexdigest()
def compare(rust,native,face):
    if rust['face']!=face['face'] or native['face']!=face['face']:raise ValueError('face identity')
    if any(native[key]!=value for key,value in dict(asset_sha256=face['cff_sha256'],asset_bytes=face['cff_bytes'],units_per_em=1000,glyph_count=654).items()):raise ValueError('native asset identity or units')
    rows=face['records'];native_rows=native['glyphs']
    if len(native_rows)!=len(rows) or any(not g['found'] or g['name']!=record['name'] for g,record in zip(native_rows,rows)):raise ValueError('native exact query membership/order')
    if len(rust['glyphs'])!=654 or len(native_rows)!=654:raise ValueError('glyph membership count')
    for index,(actual,want,record) in enumerate(zip(rust['glyphs'],native_rows,rows)):
        expected=dict(name=record['name'],native_gid=record['native_gid'],units_per_em=1000,advance=record['advance'],asset_sha256=face['cff_sha256'],command_range=[record['command_start'],record['command_start']+record['command_len']],commands=want['commands'])
        if want['name']!=record['name'] or want['native_gid']!=record['native_gid'] or want['advance']!=record['advance']:raise ValueError('native name/GID/advance membership')
        if actual!=expected:raise ValueError(f'compiled canonical record{index} {record["name"]} differs')
        if sha(json.dumps(want['commands'],separators=(',',':')).encode())!=record['commands_sha256']:raise ValueError('native canonical commands changed since generation')
    if sum(len(g['commands']) for g in rust['glyphs'])!=face['commands']:raise ValueError('command total')
    return dict(glyphs=654,commands=face['commands'],empty_glyphs=sum(not g['commands'] for g in rust['glyphs']),static_bytes=rust['static_bytes'])

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--rust',type=Path,required=True);p.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-builtin-outlines-oracle'));p.add_argument('--report',type=Path,required=True);p.add_argument('--evidence-dir',type=Path);a=p.parse_args();manifest=ENGINE/'reference/builtin-outlines-manifest.json';m=json.loads(manifest.read_text());data=ENGINE/'src/builtin_outlines_data.rs';assert sha(data.read_bytes())==m['output_sha256']
    sources=[a.rust,a.native,Path(__file__),manifest,data,ENGINE/'tools/generate-builtin-outlines.py',ENGINE/'tools/mupdf-builtin-outlines-oracle.c',ENGINE/'tools/build-builtin-outlines-reference.sh'];hashes={str(v):sha(v.read_bytes()) for v in sources};rows=[]
    for face in m['faces']:
        query=('\n'.join(g['name'] for g in face['records'])+'\n').encode();r=subprocess.run([str(a.rust),'--builtin-outlines='+face['face']],capture_output=True,timeout=30);n=subprocess.run([str(a.native),face['face']],input=query,capture_output=True,timeout=50)
        if r.returncode or r.stderr or n.returncode or n.stderr:raise ValueError('asset oracle process failure')
        row=compare(json.loads(r.stdout),json.loads(n.stdout),face);row.update(face=face['face'],status='matched',rust_output_sha256=sha(r.stdout),native_output_sha256=sha(n.stdout));rows.append(row)
        if a.evidence_dir:
            a.evidence_dir.mkdir(parents=True,exist_ok=True)
            for kind,raw in [('rust',r.stdout),('native',n.stdout)]:(a.evidence_dir/(face['face']+'-'+kind+'.json.gz')).write_bytes(gzip.compress(raw,mtime=0))
    assert hashes=={str(v):sha(v.read_bytes()) for v in sources}
    result=dict(status='passed',glyphs=sum(r['glyphs'] for r in rows),commands=sum(r['commands'] for r in rows),empty_glyphs=sum(r['empty_glyphs'] for r in rows),rows=rows,evidence_sha256=hashes,limitations=['Canonical drawing-origin unhinted assets; no hinting or arbitrary CFF parsing.','Static byte count is the compiled target layout, not a portable struct-size promise.','.notdef is included only for table integrity/native query; PDF selection remains unsupported.'])
    a.report.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({k:v for k,v in result.items() if k not in ('rows','evidence_sha256','limitations')}))
if __name__=='__main__':main()
