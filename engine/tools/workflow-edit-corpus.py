#!/usr/bin/env python3
"""Attempt one actual edit/save/fresh-process reopen per admitted document."""
import hashlib,json,pathlib,subprocess,sys
cli,fixtures,directory=map(pathlib.Path,sys.argv[1:]);directory.mkdir(parents=True,exist_ok=True)
rows=[]
for path in sorted(fixtures.rglob('*.pdf')):
    inspection=subprocess.run([str(cli),'inspect',str(path)],text=True,capture_output=True,timeout=30)
    if inspection.returncode:continue
    pages=[json.loads(line) for line in inspection.stdout.splitlines()]
    candidate=next(((p['page'],t) for p in pages for t in p.get('targets',[]) if t['text'] and t['text'][::-1]!=t['text']),None)
    if not candidate:continue
    page,target=candidate;replacement=target['text'][::-1];destination=directory/(str(path.relative_to(fixtures)).replace('/','_')+'.edited.pdf')
    before=hashlib.sha256(path.read_bytes()).hexdigest()
    result=subprocess.run([str(cli),'edit',str(path),str(page),str(target['target']),replacement,str(destination)],capture_output=True,text=True,timeout=30)
    row={'input':str(path),'output':str(destination),'page':page,'oldText':target['text'],'newText':replacement,'oldBounds':target['bounds'],'accepted':result.returncode==0,'sourceSha256':before}
    assert hashlib.sha256(path.read_bytes()).hexdigest()==before
    if result.returncode:row['reason']=result.stderr.strip()
    else:
        reopened=subprocess.run([str(cli),'inspect',str(destination)],capture_output=True,text=True,timeout=30,check=True)
        targets=[t for line in reopened.stdout.splitlines() for t in json.loads(line).get('targets',[]) if t['text']==replacement]
        assert targets, (path,replacement)
        row.update(newBounds=targets[0]['bounds'],outputSha256=hashlib.sha256(destination.read_bytes()).hexdigest(),freshProcessReopen=True)
    rows.append(row)
report={'attempts':len(rows),'successful':sum(row['accepted'] for row in rows),'rows':rows}
(directory/'edits.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items() if k!='rows'}))
