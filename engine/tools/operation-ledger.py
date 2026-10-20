#!/usr/bin/env python3
"""Check the real SessionRequest contract and map literal legacy test evidence.

Source references are evidence candidates, not claims that an operation passed.
Run legacy tests separately; the complete desktop drive suite remains required.
"""
import argparse,csv,hashlib,json,pathlib,re
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--refresh',action='store_true')
args=parser.parse_args()
root=pathlib.Path(__file__).resolve().parents[2]
source=root/'web/protocol.ts';text=source.read_text()
union=re.search(r'export type SessionRequest\s*=([^;]+);',text).group(1)
names=re.findall(r'\b\w+Request(?:Message)?\b',union)
ledger=root/'docs/rust-engine-operation-ledger.csv'
with ledger.open(newline='') as f: reader=csv.DictReader(f);fields=reader.fieldnames;rows=list(reader)
assert len(names)==len(set(names))
assert {r['request_type'] for r in rows}==set(names), 'SessionRequest/ledger type drift'
assert len({r['operation'] for r in rows})==len(rows), 'duplicate operations'
operations={}
for row in rows:
    name=row['request_type']
    declaration=re.search(r'export type '+re.escape(name)+r'\s*=',text)
    assert declaration, name
    operation=re.search(r'\bop:\s*"([^"]+)"',text[declaration.end():]).group(1)
    assert operation==row['operation'], (name,operation,row['operation'])
    for path in row['candidate_legacy_tests'].split('; '): assert (root/path).is_file(),path
    operations[operation]={'requestType':name,'declarationLine':text[:declaration.start()].count('\n')+1,'legacyTests':[],'fixtures':set(),'driveCandidates':row['candidate_legacy_tests'].split('; ')}
for file in sorted((root/'tests').glob('*.test.mjs')):
    body=file.read_text();fixtures=set(re.findall(r'fixtures/[\w./-]+\.pdf',body))
    tests=list(re.finditer(r'\btest\(\s*([\'"`])(.+?)\1',body))
    for i,test in enumerate(tests):
        end=tests[i+1].start() if i+1<len(tests) else len(body)
        block=body[test.end():end]
        for op in set(re.findall(r'\bop:\s*[\'"`]([^\'"`]+)[\'"`]',block)) & operations.keys():
            operations[op]['legacyTests'].append({'file':str(file.relative_to(root)),'line':body[:test.start()].count('\n')+1,'name':test.group(2)})
            operations[op]['fixtures'].update(fixtures)
for op,value in operations.items():
    value['helperLocations']=[]
    for file in sorted((root/'tests').glob('*.test.mjs')):
        body=file.read_text()
        for hit in re.finditer(r'\bop:\s*[\'\"`]'+re.escape(op)+r'[\'\"`]',body):
            line=body[:hit.start()].count('\n')+1
            value['helperLocations'].append({'file':str(file.relative_to(root)),'line':line})
    value['fixtures']=sorted(value['fixtures'])
report={'schema':1,'contractSha256':hashlib.sha256(source.read_bytes()).hexdigest(),'operations':operations,'limitation':'Literal-op source mapping may omit helper-generated requests. Fixture links are file-level candidates. Test run outcomes do not prove production Rust integration.'}
out=root/'engine/reference/m1-20261012/operation-evidence.json';out.write_text(json.dumps(report,indent=2)+'\n')
if args.refresh:
    new=['legacy_unit_tests','fixture_candidates','rust_evidence','worker_evidence','baseline_evidence','expected_refusal_policy']
    fields+= [f for f in new if f not in fields]
    for row in rows:
        op=row['operation'];evidence=operations[op]
        row['legacy_unit_tests']='; '.join(sorted({r['file'] for r in evidence['legacyTests']+evidence['helperLocations']})) or 'helper/drive mapping still required'
        row['fixture_candidates']='; '.join(evidence['fixtures']) or 'synthetic/drive fixtures; manual family mapping required'
        row['rust_evidence']='engine/tests/session.rs' if row['status']=='verified subset' else 'not yet implemented in session contract'
        row['worker_evidence']='engine/reference/m1-20261012/desktop-report.json' if op in ['open','render','edit-targets','edit-text','save','validate','undo','redo','saved','close'] else 'production adapter/drive gate pending'
        row['baseline_evidence']='engine/reference/m1-20261012/legacy-tests.log (545 passed; 100 skipped; full drive baseline pending)'
        row['expected_refusal_policy']='Preserve legacy permissions, signatures and unsupported-feature refusals; see linked named test assertions in operation-evidence.json'
    with ledger.open('w',newline='') as f:writer=csv.DictWriter(f,fields);writer.writeheader();writer.writerows(rows)
print(json.dumps({'contractOperations':len(names),'ledgerRows':len(rows),'operationsWithLiteralTestEvidence':sum(bool(o['legacyTests']) for o in operations.values()),'stillNeedHelperOrDriveMapping':[op for op,o in operations.items() if not o['legacyTests']]}))
