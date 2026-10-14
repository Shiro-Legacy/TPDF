#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Validate local operator schemas in the hash-pinned page/Form content corpus."""
import argparse
import hashlib
import json
import subprocess
from pathlib import Path
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--rust', type=Path, default=Path('target/debug/pdf-inspect'))
p.add_argument('--inventory', type=Path, default=Path('reference/content-inventory.json'))
p.add_argument('--corpus-dir', type=Path, required=True)
p.add_argument('--report', type=Path, default=Path('reference/content-schema-r6a.json'))
a=p.parse_args()
rows=[]
for entry in json.loads(a.inventory.read_text())['results']:
    data=(a.corpus_dir / entry['savedFile']).read_bytes()
    if hashlib.sha256(data).hexdigest()!=entry['sha256']: raise ValueError('changed content stream')
    run=subprocess.run([str(a.rust.resolve()),'--check-content'],input=data,capture_output=True,timeout=15)
    row={key:entry[key] for key in ('file','id','sha256')}
    expected=sum(entry['operators'].values())
    if not entry['complete']:
        row.update(status='unsupported' if run.returncode==2 and b'unsupported' in run.stderr and not run.stdout else 'failure', reason=run.stderr.decode(errors='replace'))
    elif run.returncode:
        row.update(status='failure',reason=run.stderr.decode(errors='replace'))
    else:
        actual=json.loads(run.stdout)
        row.update(status='matched' if actual['operations']==expected else 'failure',operations=actual['operations'],expected=expected)
    rows.append(row)
result={'comparison':'Local operator schema/counts against hash-pinned content inventory; no graphics execution or resource validation.', 'rustSha256':hashlib.sha256(a.rust.read_bytes()).hexdigest(),'matched':sum(r['status']=='matched' for r in rows),'unsupported':sum(r['status']=='unsupported' for r in rows),'failures':sum(r['status']=='failure' for r in rows),'operations':sum(r.get('operations',0) for r in rows),'results':rows}
a.report.write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({k:v for k,v in result.items() if k!='results'},indent=2))
for row in rows:
    if row['status']=='failure': print(json.dumps(row))
raise SystemExit(0 if result['failures']==0 and result['matched']==590 else 1)
