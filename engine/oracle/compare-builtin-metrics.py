#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Read-only, independently parsed AFM/CFF parity, without runtime tables."""
import argparse,hashlib,json,subprocess
from decimal import Decimal
from fractions import Fraction
from pathlib import Path
parser=argparse.ArgumentParser();parser.add_argument('oracle');parser.add_argument('source');parser.add_argument('report');parser.add_argument('--timeout',type=float,default=10);args=parser.parse_args()
if not 0<args.timeout<=60:raise SystemExit('timeout must be in(0,60]')
root=Path(__file__).resolve().parent.parent
inventory=json.loads((root/'reference/font-metrics-inventory.json').read_text())
encodings=json.loads((root/'reference/encoding-table-inventory.json').read_text())['tables']
source=Path(args.source);reports=[];failures=[];exclusions=[]
for expected in inventory['afm']:
 data=(source/expected['path']).read_bytes()
 if hashlib.sha256(data).hexdigest()!=expected['sha256']:raise SystemExit('AFM source hash mismatch')
 metrics={}
 for line in data.decode('ascii').splitlines():
  if not line.startswith('C '):continue
  fields={}
  for field in line.split(';'):
   parts=field.strip().split(maxsplit=1)
   if len(parts)==2:fields[parts[0]]=parts[1]
  name=fields['N'];width=Fraction(Decimal(fields['WX']))
  if name in metrics:raise SystemExit('duplicate AFM name')
  metrics[name]=width
 names=sorted(set(metrics)|{r['glyph'] for rows in encodings.values() for r in rows if r['glyph'] is not None}|{'DefinitelyUnknownMetricGlyph'})
 if len(names)>4096:raise SystemExit('too many queries')
 run=subprocess.run([args.oracle,expected['baseFont']],input=('\n'.join(names)+'\n').encode('ascii'),capture_output=True,timeout=args.timeout)
 if run.returncode or run.stderr:raise SystemExit(f'native oracle failed: {run.returncode} {run.stderr!r}')
 lines=run.stdout.decode('ascii').splitlines()
 if len(lines)!=len(names):raise SystemExit('oracle row mismatch')
 rows=[];native={}
 for requested,line in zip(names,lines):
  name,found,gid,units,raw,pdf_integer=line.split('\t')
  if name!=requested:raise SystemExit('oracle query mismatch')
  found,gid,units,raw,pdf_integer=map(int,(found,gid,units,raw,pdf_integer))
  actual=Fraction(raw*1000,units) if found else None
  expected_width=metrics.get(name)
  if expected_width is not None and not found:exclusions.append({'face':expected['baseFont'],'name':name,'afm':str(expected_width),'reason':'AFM glyph absent from bundled CFF exact-name subset'})
  elif expected_width is not None and actual!=expected_width:failures.append({'face':expected['baseFont'],'name':name,'afm':str(expected_width),'native':str(actual)})
  if expected_width is None and found:failures.append({'face':expected['baseFont'],'name':name,'reason':'CFF name outside AFM'})
  row={'name':name,'found':bool(found),'gid':gid,'unitsPerEm':units,'rawAdvance':raw,'pdfIntegerWidth':pdf_integer,'afmWidth':str(expected_width) if expected_width is not None else None};rows.append(row);native[name]=row
 tables={}
 for table,table_rows in encodings.items():
  tables[table]={'namedSlots':sum(r['glyph'] is not None for r in table_rows),'undefinedSlots':sum(r['glyph'] is None for r in table_rows),'missingMetrics':[r['byte'] for r in table_rows if r['glyph'] is not None and not native[r['glyph']]['found']]}
 reports.append({'baseFont':expected['baseFont'],'afmSha256':expected['sha256'],'queries':len(rows),'afmEntries':len(metrics),'nativeFound':sum(r['found'] for r in rows),'tables':tables,'rows':rows})
refusals=[]
for data in [b'\n',b'A'*128+b'\n',b'A B\n',b'A\x00B\n',b'A\n'*4097]:
 run=subprocess.run([args.oracle,'Helvetica'],input=data,capture_output=True,timeout=args.timeout)
 refusals.append({'inputBytes':len(data),'exit':run.returncode})
 if run.returncode==0:failures.append({'reason':'malformed query accepted','bytes':len(data)})
report={'reference':'Pinned CFF exact FT_Get_Name_Index/FT_Get_Advance vs independently parsed AFM WX rational values','oracleSha256':hashlib.sha256(Path(args.oracle).read_bytes()).hexdigest(),'faces':reports,'afmEntries':sum(r['afmEntries'] for r in reports),'queries':sum(r['queries'] for r in reports),'malformedQueries':refusals,'exactWidthComparisons':sum(r['nativeFound'] for r in reports),'subsetExclusions':exclusions,'failures':failures,'limitations':['No runtime metric tables or glyph renderer implemented.','Undefined encoding slots and missing AFM glyph names remain explicit; no .notdef/Unicode fallback.','Only exact Helvetica and Times-Roman bundled faces are inspected.']}
Path(args.report).write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items() if k not in ('faces','subsetExclusions')},indent=2));raise SystemExit(bool(failures))
