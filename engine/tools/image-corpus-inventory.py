#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""All readable fixture original image objects; native/Rust decode parity, not draw reachability."""
import argparse,collections,gzip,hashlib,json,runpy,subprocess
from pathlib import Path
ENGINE=Path(__file__).resolve().parent.parent
D=runpy.run_path(str(Path(__file__).with_name('differential-images.py')))
sha=lambda b:hashlib.sha256(b).hexdigest()
def main():
    ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--rust',type=Path,required=True);ap.add_argument('--native-dir',type=Path,default=Path('/private/tmp/tandem-mupdf-reference'));ap.add_argument('--report',type=Path,required=True);ap.add_argument('--evidence-dir',type=Path,required=True);a=ap.parse_args();rows=[];documents=[]
    paths=[Path(__file__),ENGINE/'reference/fixture-inventory.json',a.rust,*[a.native_dir/('mupdf-image-'+s+'-oracle') for s in ['decode','inventory']],*[ENGINE/'tools'/s for s in ['mupdf-image-inventory-oracle.c','mupdf-image-decode-oracle.c','differential-images.py']],ENGINE/'reference/image-native-pins.sha256',ENGINE/'reference/image-build-r23.json',ENGINE/'reference/image-reference-policy-r23.json'];hashes={str(p):sha(p.read_bytes()) for p in paths}
    for doc in json.loads((ENGINE/'reference/fixture-inventory.json').read_text())['documents']:
      if doc.get('passwordRequired'):continue
      data=(ENGINE.parent/'fixtures'/doc['file']).read_bytes()
      if sha(data)!=doc['sha256']:raise ValueError('corpus fixture hash changed')
      inv=subprocess.run([str(a.native_dir/'mupdf-image-inventory-oracle')],input=data,capture_output=True,timeout=20)
      if inv.returncode or inv.stderr or not inv.stdout.startswith(b'image-object-inventory-v1\n'):raise ValueError('inventory process '+doc['file']+': '+inv.stderr.decode())
      lines=inv.stdout.decode().splitlines()[1:];documents.append(dict(file=doc['file'],sha256=sha(data),images=len(lines),inventory_sha256=sha(inv.stdout)))
      dst=a.evidence_dir/doc['file'];dst.mkdir(parents=True,exist_ok=True);(dst/'inventory.txt.gz').write_bytes(gzip.compress(inv.stdout,mtime=0))
      for line in lines:
        values=list(map(int,line.split()))
        if len(values)!=11:raise ValueError('inventory field count')
        row=dict(zip(('object','generation','width','height','bits','components','mask','smask','interpolate','imagemask','filters'),values));row['file']=doc['file'];row['evidence_files']={}
        if row['generation']!=0:raise ValueError('generation probe unsupported')
        for kind,path,prefix in [('native',a.native_dir/'mupdf-image-decode-oracle',b'image decode oracle: '),('rust',a.rust,b'image-decode: ')]:
          r=subprocess.run([str(path),str(row['object'])],input=data,capture_output=True,timeout=20);out=D['read_result'](r,prefix);row[kind+'_accepted']=out is not None;row[kind+'_sha256']=sha(r.stdout);row[kind+'_error']=r.stderr.decode(errors='replace')[:4096];path=dst/(str(row['object'])+'.'+kind+'.gz');path.write_bytes(gzip.compress(r.stdout,mtime=0));row['evidence_files'][kind]=dict(path=str(path.resolve()),sha256=sha(r.stdout))
        if row['rust_accepted']:
          if not row['native_accepted'] or row['native_sha256']!=row['rust_sha256']:raise ValueError('corpus decoder mismatch '+str(row))
          row['status']='exact_match'
        else:row['status']='strict_refused_native_accepted' if row['native_accepted'] else 'both_refused'
        rows.append(row)
    if hashes!={str(p):sha(p.read_bytes()) for p in paths}:raise ValueError('gate inputs changed')
    report=dict(status='passed',evidence_dir=str(a.evidence_dir.resolve()),scope='all original image objects in readable corpus, including unused objects; not invocation inventory',documents=documents,rows=rows,counts=dict(collections.Counter(r['status'] for r in rows)),evidence_sha256=hashes);a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(dict(status='passed',documents=len(documents),images=len(rows),counts=report['counts'])))
if __name__=='__main__':main()
