#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Actual embedded PDF source-code/GID/outline comparison with honest refusals."""
import argparse
import collections
import gzip
import hashlib
import json
from pathlib import Path
import subprocess
from truetype_cmap_model import records,tables,u16
ENGINE=Path(__file__).resolve().parent.parent

def sha(data):return hashlib.sha256(data).hexdigest()
def invoke(binary,args,data):
    p=subprocess.run([str(binary),*args],input=data,capture_output=True,timeout=20)
    if p.returncode or p.stderr:raise ValueError(f'{binary}: exit{p.returncode}: '+p.stderr.decode(errors='replace')[:2000])
    if len(p.stdout)>64*1024*1024:raise ValueError('output bound')
    return json.loads(p.stdout),p.stdout

def compare_slots(input_slots,model,outlines,label='font'):
    provenance={r['code']:r for r in model['nonzero_codes']}
    routes={'source_code_cmap':'raw_byte','source_code_symbol_offset':'private_use','encoding_name_unicode':'encoding_unicode','encoding_name_macroman':'encoding_mac_roman'}
    fields=['gid','kind','range','points','contour_endpoints','commands']
    assert len(input_slots)==256
    counts=collections.Counter();slots=[];outline_count=0;commands=0
    for code,slot in enumerate(input_slots):
        assert slot['code']==code
        status=slot['status'];counts[status]+=1
        expected=model['code_to_gid'][code]
        result=dict(code=code,status=status,native_gid=expected,route=slot['route'])
        if status=='unsupported':
            assert slot['gid'] is None and slot['outline'] is None
            result['limitation']='Native name fallback not implemented; no mapping or outline claim.'
        elif status=='missing':
            assert expected==0 and slot['gid'] is None and slot['outline'] is None
            result['gid']=None
        else:
            assert status=='mapped' and slot['gid']==expected and expected>0
            modeled_slot=provenance[code]
            assert slot['route']==routes[modeled_slot['route']] and slot['lookup_code']==modeled_slot['cmap_codepoint'],(label,code,'mapping provenance differs')
            outline=slot['outline'];want=outlines['glyphs'][expected]
            assert all(outline[k]==want[k] for k in fields),(label,code,expected,'outline mismatch')
            result.update(gid=expected,outline_kind=outline['kind'],outline_commands=len(outline['commands']),lookup_code=slot['lookup_code'])
            outline_count+=1;commands+=len(outline['commands'])
        slots.append(result)
    return counts,slots,outline_count,commands


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--rust',type=Path,required=True)
    p.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-glyph-inventory-oracle'))
    p.add_argument('--outline-native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-outline-oracle'))
    p.add_argument('--fonts',type=Path,default=Path('/private/tmp/outline-fonts'))
    p.add_argument('--evidence-dir',type=Path)
    p.add_argument('--report',type=Path,default=ENGINE/'reference/glyph-selection-differential-g1.json')
    a=p.parse_args()
    inventory=ENGINE/'reference/glyph-selection-inventory.json'
    original_inventory=ENGINE/'reference/embedded-outline-inventory.json'
    sources=[a.rust,a.native,a.outline_native,inventory,original_inventory,Path(__file__),ENGINE/'tools/mupdf-glyph-inventory-oracle.c',ENGINE/'tools/truetype_cmap_model.py']
    hashes={str(f):sha(f.read_bytes()) for f in sources}
    fonts=json.loads(original_inventory.read_text())['fonts'];modeled=json.loads(inventory.read_text())['rows'];rows=[]
    for i,(font,model) in enumerate(zip(fonts,modeled)):
        pdf=(ENGINE.parent/'fixtures'/font['file']).read_bytes();assert sha(pdf)==font['fixtureSha256']
        program=(a.fonts/f'font{i}.ttf').read_bytes();assert sha(program)==font['programs'][0]['sha256']
        rust,rbytes=invoke(a.rust,[f"--font-glyphs={font['font']}",'-'],pdf)
        native,nbytes=invoke(a.native,[font['font'].split(':')[0]],pdf)
        outlines,obytes=invoke(a.outline_native,['all'],program)
        if a.evidence_dir:
            a.evidence_dir.mkdir(parents=True,exist_ok=True)
            for label,raw in [('rust',rbytes),('native',nbytes),('outlines',obytes)]:
                (a.evidence_dir/f'font{i}-{label}.json.gz').write_bytes(gzip.compress(raw,mtime=0))
        assert native['code_to_gid']==model['code_to_gid'],'independent native/model table changed'
        assert rust['font']==list(map(int,font['font'].split(':')))
        assert rust['program']==list(map(int,font['programs'][0]['object'].split()[:2]))
        assert rust['glyph_count']==native['glyph_count']==outlines['glyph_count']
        assert rust['units_per_em']==outlines['units_per_em'] and rust['flags']==font['flags']
        assert all(rust['selection'][k]==v for k,v in native['selected_cmap'].items())
        rec=records(program)[model['selected_cmap']['index']];start=rec['offset'];at,size=tables(program)[b'cmap']
        assert rust['selection']==dict(**native['selected_cmap'],language=u16(program,start+4),segments=u16(program,start+6)//2,cmap_range=[at,at+size],subtable_range=[start,start+u16(program,start+2)])
        counts,slots,outline_count,commands=compare_slots(rust['slots'],model,outlines,font['file'])
        rows.append(dict(file=font['file'],font=font['font'],fixture_sha256=sha(pdf),program_sha256=sha(program),
                         rust_output_sha256=sha(rbytes),native_output_sha256=sha(nbytes),native_outline_output_sha256=sha(obytes),
                         counts=dict(counts),native_nonzero=sum(g>0 for g in model['code_to_gid']),
                         native_nonzero_unverified=sum(s['status']=='unsupported' and s['native_gid']>0 for s in slots),
                         outlines_compared=outline_count,outline_commands_compared=commands,
                         selection=rust['selection'],usage=rust['usage'],slots=slots,status='matched_supported_slots'))
    assert hashes=={str(f):sha(f.read_bytes()) for f in sources},'evidence changed during gate'
    totals=collections.Counter()
    for r in rows:totals.update(r['counts'])
    assert not any(r['native_nonzero_unverified'] for r in rows),'targeted fixture nonzero mapping left unverified'
    report=dict(stage='G1 PDF glyph selection + O0 bridge',fonts=len(rows),codes=256*len(rows),counts=dict(totals),
                native_nonzero=sum(r['native_nonzero'] for r in rows),native_nonzero_unverified=sum(r['native_nonzero_unverified'] for r in rows),
                outlines_compared=sum(r['outlines_compared'] for r in rows),outline_commands_compared=sum(r['outline_commands_compared'] for r in rows),
                failures=0,evidence_sha256=hashes,comparison='Exact integer code/GID and O0 points/contours/doubled quadratic commands, all supported slots.',
                limitations=['Unsupported per-code name fallback is explicitly counted; native success for those slots is not Rust support.',
                             'Native GID0 is represented as an explicit Rust missing mapping with null GID; no .notdef outline rendering claim.',
                             'Includes every source byte, including unused codes; does not prove complete page support or image/pixel equality.',
                             'PDF width/Unicode and hinting remain independent of this glyph identity/unhinted outline comparison.'],rows=rows)
    a.report.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({k:v for k,v in report.items() if k not in ('evidence_sha256','rows','limitations','comparison')}))
if __name__=='__main__':main()
