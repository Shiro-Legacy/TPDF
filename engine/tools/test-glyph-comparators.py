#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Fault injections ensure exhaustive comparators reject semantic corruption."""
import argparse
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
from truetype_cmap_model import records,dense
ENGINE=Path(__file__).resolve().parent.parent

def module(name):
    path=Path(__file__).with_name(name+'.py');spec=importlib.util.spec_from_file_location(name,path)
    result=importlib.util.module_from_spec(spec);spec.loader.exec_module(result);return result

def main():
    p=argparse.ArgumentParser();p.add_argument('--rust',type=Path,required=True);p.add_argument('--report',type=Path);a=p.parse_args()
    raw=module('differential-truetype-cmap');glyph=module('differential-glyph-selection')
    data=Path('/private/tmp/outline-fonts/font0.ttf').read_bytes()
    expected=[dict(index=r['index'],platform=r['platform'],encoding=r['encoding'],format=4,mappings=dense(data,r['index'])) for r in records(data)]
    native,_=raw.invoke(Path('/private/tmp/tandem-mupdf-reference/mupdf-truetype-cmap-oracle'),[],data)
    rust,_=raw.invoke(a.rust,['--tt-cmap=all','-'],data)
    raw.validate_rust(rust,native,expected,data)
    injections=[]
    def refuse(label,operation):
        try:operation()
        except (AssertionError,ValueError):injections.append(label)
        else:raise AssertionError('comparator accepted injected corruption: '+label)
    mutations=[]
    changed=copy.deepcopy(rust);changed['records'].reverse();mutations.append(('record order',changed))
    changed=copy.deepcopy(rust);changed['records'].pop();mutations.append(('missing record',changed))
    changed=copy.deepcopy(rust);changed['records'][0]['selection']['platform']=9;mutations.append(('record identity',changed))
    changed=copy.deepcopy(rust);changed['records'][0]['gids'][1]+=1;mutations.append(('raw cmap GID',changed))
    changed=copy.deepcopy(rust);changed['records'][0]['gids'].pop();mutations.append(('missing raw query',changed))
    for label,changed in mutations:refuse(label,lambda changed=changed:raw.validate_rust(changed,native,expected,data))
    pdf=(ENGINE.parent/'fixtures/p0b/edit-coded.pdf').read_bytes()
    rust,_=glyph.invoke(a.rust,['--font-glyphs=5:0','-'],pdf)
    outlines,_=glyph.invoke(Path('/private/tmp/tandem-mupdf-reference/mupdf-outline-oracle'),['all'],data)
    model=json.loads((ENGINE/'reference/glyph-selection-inventory.json').read_text())['rows'][0]
    glyph.compare_slots(rust['slots'],model,outlines)
    slots=rust['slots'];mutations=[]
    changed=copy.deepcopy(slots);changed[1],changed[2]=changed[2],changed[1];mutations.append(('source code order',changed))
    changed=copy.deepcopy(slots);changed[1]['code']=99;mutations.append(('source code identity',changed))
    changed=copy.deepcopy(slots);changed[1]['gid']+=1;mutations.append(('PDF GID',changed))
    changed=copy.deepcopy(slots);changed.pop();mutations.append(('missing PDF slot',changed))
    changed=copy.deepcopy(slots);changed[1]['outline']['points'][0][0]+=1;mutations.append(('outline point',changed))
    changed=copy.deepcopy(slots);changed[1]['outline']['commands'][0][1]+=1;mutations.append(('outline command',changed))
    changed=copy.deepcopy(slots);changed[1]['route']='raw_byte';mutations.append(('selection route',changed))
    changed=copy.deepcopy(slots);changed[1]['lookup_code']=1;mutations.append(('lookup codepoint',changed))
    for label,changed in mutations:refuse(label,lambda changed=changed:glyph.compare_slots(changed,model,outlines))
    files=[a.rust,Path(__file__),Path(raw.__file__),Path(glyph.__file__)]
    report=dict(baseline_checks=2,rejected_semantic_mutations=len(injections),mutations=injections,status='passed',evidence_sha256={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in files})
    if a.report:a.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))
if __name__=='__main__':main()
