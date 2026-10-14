#!/usr/bin/env python3
"""Inventory actual workflow admission independently of rendering page totals."""
import concurrent.futures, hashlib, json, pathlib, subprocess, sys, time
cli, fixture_dir, destination = map(pathlib.Path, sys.argv[1:])
def inspect(path):
    started=time.monotonic()
    try:
        result=subprocess.run([str(cli),'inspect',str(path)],capture_output=True,text=True,timeout=30)
        row={'file':str(path.relative_to(fixture_dir)),'sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'elapsedSeconds':round(time.monotonic()-started,4),'accepted':result.returncode==0}
        if result.returncode:
            row['reason']=result.stderr.strip()
        else:
            pages=[json.loads(line) for line in result.stdout.splitlines()]
            row.update(pages=len(pages),editablePages=sum(bool(p.get('targets')) for p in pages),targets=sum(len(p.get('targets',[])) for p in pages),pageRefusals=[{'page':p['page'],'reason':p['reason']} for p in pages if 'reason' in p])
        return row
    except subprocess.TimeoutExpired:
        return {'file':str(path.relative_to(fixture_dir)),'accepted':False,'reason':'timeout after 30 seconds'}
paths=sorted(fixture_dir.rglob('*.pdf'))
with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
    rows=list(executor.map(inspect,paths))
report={'documents':len(rows),'opened':sum(r['accepted'] for r in rows),'documentsWithTargets':sum(r.get('targets',0)>0 for r in rows),'rows':rows}
destination.write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps({k:v for k,v in report.items() if k!='rows'}))
