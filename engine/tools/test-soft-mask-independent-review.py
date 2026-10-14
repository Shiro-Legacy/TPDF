#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Independent destructive omissions: an acceptance verifier must reject each."""
import argparse,copy,importlib.util,json,sys
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('engine',type=Path);p.add_argument('--output',type=Path,default=Path('/private/tmp/r24-independent-gate-results.json'));a=p.parse_args();root=a.engine.resolve();sys.path.insert(0,str(root/'tools'))
spec=importlib.util.spec_from_file_location('r24_gate_review',root/'tools/verify-soft-mask-gate.py');g=importlib.util.module_from_spec(spec);spec.loader.exec_module(g)
report=json.loads((root/'reference/soft-mask-differential-r24.json').read_text());checks=json.loads((root/'reference/soft-mask-rust-checks-r24.json').read_text());results=[]
# Positive control prevents stale hashes or broken verification from trivializing every rejection.
g.verify(report);g.verify_checks(checks)
print('Unmodified report and checks pass',flush=True)
def rejected(label,fn):
    try:fn()
    except (ValueError,KeyError,IndexError,TypeError,AssertionError) as e:
        results.append({'label':label,'status':'rejected','reason':str(e)});return
    results.append({'label':label,'status':'UNEXPECTED_ACCEPTANCE'})
def report_mutation(label,fn):
    candidate=copy.deepcopy(report);fn(candidate);rejected(label,lambda:g.verify(candidate))
def check_mutation(label,fn):
    candidate=copy.deepcopy(checks);fn(candidate);rejected(label,lambda:g.verify_checks(candidate))
# Test each raw artifact on representative accepted and refusal categories.
representatives=[('generated',next(i for i,r in enumerate(report['generated']) if r['accepted'])),('generated',next(i for i,r in enumerate(report['generated']) if r['label']=='nonopaque-strong'))]
for status in ('preserved_r23','new_opaque_mask_render','refused'):
    representatives.append(('corpus',next(i for i,r in enumerate(report['corpus']) if r['status']==status)))
representatives.append(('masks',0))
for group,index in representatives:
    for key in report[group][index]['evidence']:
        report_mutation(f'omitted/{group}/{index}/{key}',lambda r,group=group,index=index,key=key:r[group][index]['evidence'].pop(key))
for role in report['binaries']:
    report_mutation('omitted-binary-role/'+role,lambda r,role=role:r['binaries'].pop(role))
    report_mutation('wrong-binary-role/'+role,lambda r,role=role:r['binaries'].update({role:r['binaries'][next(k for k in r['binaries'] if k!=role)]}))
for role in checks['binaries']:
    check_mutation('omitted-checked-binary/'+role,lambda c,role=role:c['binaries'].pop(role))
    check_mutation('changed-checked-binary/'+role,lambda c,role=role:c['binaries'][role].update(sha256='0'*64))
for identity in report['identities']:
    report_mutation('omitted-identity/'+identity,lambda r,identity=identity:r['identities'].pop(identity))
index=next(i for i,r in enumerate(report['generated']) if r['accepted'])
report_mutation('positive-native-refusal',lambda r:r['generated'][index].update(native_refusal='image decode oracle: deliberate mismatch'))
check_mutation('empty-source-manifest',lambda c:c.update(production_sha256={}))
for key in checks['production_sha256']:
    check_mutation('omitted-check-source/'+key,lambda c,key=key:c['production_sha256'].pop(key))
for key in checks['checks']:
    check_mutation('omitted-check/'+key,lambda c,key=key:c['checks'].pop(key))
    check_mutation('failed-check/'+key,lambda c,key=key:c['checks'][key].update(exit_code=1))
a.output.write_text(json.dumps(results,indent=2)+'\n');failures=[r for r in results if r['status']!='rejected'];print(json.dumps({'mutations':len(results),'rejected':len(results)-len(failures),'failures':failures}));sys.exit(bool(failures))
