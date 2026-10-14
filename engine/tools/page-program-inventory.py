#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Read-only mixed-page scope ceiling, not an execution acceptance test."""
import argparse,collections,hashlib,json,subprocess
from pathlib import Path
engine=Path(__file__).resolve().parent.parent
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--corpus',type=Path,default=Path('/private/tmp/tandem-content-inventory'))
p.add_argument('--rust',type=Path,default=engine/'target/debug/pdf-inspect')
p.add_argument('--report',type=Path,default=engine/'reference/page-program-inventory.json')
a=p.parse_args()
j=json.loads((engine/'reference/content-inventory.json').read_text())
f=json.loads((engine/'reference/simple-font-inventory.json').read_text())
streams={(r['file'],r['id']):r for r in j['results']}
resources={(r['file'],r['page']):{font['name'].encode():font for font in r['fonts']} for r in f['pageResources']}
path=set('q Q cm w J j M d m l c v y h re S s f F f* B B* b b* n W W* G g RG rg K k'.split())
text=set('BT ET Tc Tw Tz TL Tf Tr Ts Td TD Tm T* Tj TJ'.split())|{"'",'"'}
allowed=path|text
rows=[];counts=collections.Counter();unsupported_occurrences=collections.Counter()
for p in j['pages']:
    source=[streams[(p['file'],ident)] for ident in p['contents']]
    ops=collections.Counter()
    for r in source:ops.update(r['operators'])
    unsupported=sorted(set(ops)-allowed)
    unsupported_occurrences.update({k:ops[k] for k in unsupported})
    row={'file':p['file'],'page':p['page']+1,'content_references':len(source),'operations':sum(ops.values()),'unsupported_operators':unsupported,'incomplete_scan':any(not r['complete'] for r in source)}
    reasons=[];selected=[]
    if unsupported: reasons.append('unsupported_operator')
    if row['incomplete_scan']:reasons.append('inline_image_or_incomplete')
    if not reasons:
        if len(source)>1:
            content=subprocess.run([str(a.rust.resolve()),f"--page-content={p['page']+1}",str(engine.parent/'fixtures'/p['file'])],capture_output=True,timeout=20,check=True).stdout
        else:
            content=(a.corpus/source[0]['savedFile']).read_bytes() if source else b''
            if source and hashlib.sha256(content).hexdigest()!=source[0]['sha256']:raise ValueError('source changed')
        proc=subprocess.run([str(a.rust.resolve()),'--content'],input=content,capture_output=True,timeout=15)
        if proc.returncode:raise ValueError(proc.stderr)
        bt=False;active=False;depth=0;in_bt=[];path_in_bt=[];active_at_bt=[];tr=[]
        for index,line in enumerate(proc.stdout.splitlines()):
            op_hex,n,values_hex=line.split(b'\t');op=bytes.fromhex(op_hex.decode()).decode();values=bytes.fromhex(values_hex.decode())
            if op=='Tf':
                name=values.split(b' ',1)[0][1:]
                out=bytearray();i=0
                while i<len(name):
                    if name[i:i+1]==b'#':out.append(int(name[i+1:i+3],16));i+=3
                    else:out.append(name[i]);i+=1
                selected.append(bytes(out))
            if op=='BT':
                if active:active_at_bt.append(index)
                if bt:reasons.append('nested_BT')
                bt=True
            elif op=='ET':
                if not bt:reasons.append('ET_outside_BT')
                bt=False
            elif op in ('q','Q','cm'):
                if bt:in_bt.append({'index':index,'operator':op})
                if op=='q':depth+=1
                if op=='Q':
                    depth-=1
                    if depth<0:reasons.append('stack_underflow')
            elif op in ('m','l','c','v','y','h','re','W','W*'):
                if bt:path_in_bt.append(index)
                active=True
            elif op in ('S','s','f','F','f*','B','B*','b','b*','n'):
                if bt:path_in_bt.append(index)
                active=False
            elif op=='Tr' and values not in (b'0',b'3'):tr.append(values.decode())
        if bt:reasons.append('unclosed_BT')
        if depth:reasons.append('unbalanced_qQ')
        if active:reasons.append('unfinished_path')
        if in_bt:reasons.append('graphics_state_inside_BT')
        if path_in_bt:reasons.append('path_inside_BT')
        if active_at_bt:reasons.append('active_path_at_BT')
        if tr:reasons.append('unsupported_Tr')
        selected=sorted(set(selected))
        binding=resources[(p['file'],p['page']+1)]
        unavailable=[name.hex() for name in selected if name not in binding or binding[name]['subtype'] not in ('Type1','TrueType')]
        if unavailable:reasons.append('selected_unsupported_font')
        row.update(selected_font_names_hex=[n.hex() for n in selected],unavailable_font_names_hex=unavailable,graphics_inside_bt=in_bt,path_inside_bt=path_in_bt,active_path_at_bt=active_at_bt,unsupported_Tr=tr)
    row['reasons']=sorted(set(reasons));row['candidate']=not reasons
    counts.update(row['reasons']);rows.append(row)
report={'purpose':'Read-only P0+T0 operator/state/selected-font coverage ceiling; not execution acceptance.', 'pages':len(rows),'candidate_pages':sum(r['candidate'] for r in rows),'candidate_documents':len(set(r['file'] for r in rows if r['candidate'])),'refused_pages':sum(not r['candidate'] for r in rows),'overlapping_reasons':dict(counts),'unsupported_operator_occurrences':dict(unsupported_occurrences),'limitations':['Only operator-clean pages receive detailed state/used-Tf scan; unsupported pages may have additional font/numerical/state failures.','Candidates still require actual mapping, width, transform, clip and cumulative budget execution.','Single stream bytes use hash-pinned source inventory; multi-reference pages use accepted Rust logical page loader.','BT with active path and path construction/painting inside BT are proposed first-stage refusals.'], 'evidence_sha256':{name:hashlib.sha256((engine/'reference'/name).read_bytes()).hexdigest() for name in ['content-inventory.json','simple-font-inventory.json']},'rows':rows}
print(json.dumps({k:v for k,v in report.items() if k!='rows'},indent=2))
a.report.write_text(json.dumps(report,indent=2)+'\n')
