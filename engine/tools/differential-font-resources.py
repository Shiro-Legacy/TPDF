#!/usr/bin/env python3
"""Compare lazy simple-font declarations with pinned native resource inventory."""
import argparse
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path


def digest(path): return hashlib.sha256(path.read_bytes()).hexdigest()

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--rust',type=Path,default=Path('target/debug/pdf-inspect'))
    p.add_argument('--oracle',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-cmap-oracle'))
    p.add_argument('--fixtures',type=Path,default=Path('../fixtures'))
    p.add_argument('--cmaps',type=Path,default=Path('/private/tmp/tandem-cmap-inventory'))
    p.add_argument('--report',type=Path,default=Path('reference/font-resource-differential.json'))
    args=p.parse_args(); rust=args.rust.resolve(); oracle=args.oracle.resolve()
    evidence=[Path('reference/simple-font-inventory.json'),Path('reference/cmap-inventory.json'),Path('reference/encoding-table-inventory.json'),Path('reference/fixture-inventory.json')]
    hashes={str(f):digest(f) for f in [rust,oracle,*evidence]}
    inventory,cmap_inventory,encodings,fixtures=[json.loads(f.read_text()) for f in evidence]
    expected_fonts={(r['file'],r['id']):r for r in inventory['fonts'] if not r['isDescendant']}
    expected_pages={(r['file'],r['page']):r for r in inventory['pageResources']}
    expected_maps={}
    with tempfile.TemporaryDirectory() as tmp:
        queries=Path(tmp)/'queries.txt';queries.write_text(''.join(f'lookup {i:02x}\n' for i in range(256)))
        for record in cmap_inventory['cmaps']:
            eligible=[font_id for font_id in record['fontIds'] if expected_fonts.get((record['file'],font_id),{}).get('subtype') in ['Type1','TrueType']]
            if not eligible:continue
            data=(args.cmaps/record['savedFile']).read_bytes()
            if hashlib.sha256(data).hexdigest()!=record['sha256']:raise SystemExit('saved CMap changed')
            native=subprocess.run([str(oracle),str(queries)],input=data,capture_output=True,timeout=15)
            if native.returncode or native.stderr:raise SystemExit('native simple CMap failed: '+native.stderr.decode(errors='replace'))
            result=json.loads(native.stdout)
            entries=[{'code':int(q['input'],16),'scalars':q['scalars']} for q in result['queries'] if q['scalars'] is not None]
            for font_id in eligible:expected_maps[(record['file'],font_id)]=entries
    rows=[]; seen_fonts=set(); pages=bindings=loaded=unsupported=encoding_slots=map_entries=0
    for record in fixtures['documents']:
        data=(args.fixtures/record['file']).read_bytes()
        if hashlib.sha256(data).hexdigest()!=record['sha256']:raise SystemExit('immutable fixture changed')
        if record.get('passwordRequired'):continue
        row={'file':record['file'],'sha256':record['sha256']}
        try:
            run=subprocess.run([str(rust),'--fonts','-'],input=data,capture_output=True,timeout=30)
            if run.returncode or run.stderr:raise ValueError(run.stderr.decode(errors='replace')[:2000])
            result=json.loads(run.stdout)
            if len(result['pages'])!=record['pages']:raise ValueError('page count differs')
            actual_fonts={r['id']:r for r in result['fonts']}
            for actual in result['pages']:
                expected=expected_pages[(record['file'],actual['page'])]
                actual_bindings={bytes.fromhex(r['name_hex']).decode('utf-8'):r for r in actual['bindings']}
                if set(actual_bindings)!={r['name'] for r in expected['fonts']}:raise ValueError('resource names differ')
                pages+=1
                for want in expected['fonts']:
                    got=actual_bindings[want['name']];bindings+=1
                    target=[int(v) for v in want['id'].split(':')]
                    if got['target']!=target:raise ValueError('font object identity differs')
                    if want['subtype']=='Type0':
                        if got['status']!='unsupported':raise ValueError('Type0 must explicitly refuse')
                        unsupported+=1
                    else:
                        if got['status']!='loaded' or got['font']!='i:'+want['id']:raise ValueError('simple binding did not load: '+str(got))
                        if got['font'] not in actual_fonts:raise ValueError('binding refers to missing cache record')
                        loaded+=1
            for key,font in actual_fonts.items():
                if not key.startswith('i:'):raise ValueError('unexpected direct fixture font')
                ident=key[2:]; expected=expected_fonts[(record['file'],ident)];seen_fonts.add((record['file'],ident))
                if font['subtype']!=expected['subtype'] or bytes.fromhex(font['base_font_hex']).decode('utf-8')!=expected['baseFont']:raise ValueError('font metadata differs')
                mapping=font['mapping']
                if expected['toUnicode'] is not None:
                    entries=expected_maps[(record['file'],ident)]
                    if mapping['kind']!='to_unicode' or mapping['entries']!=entries:raise ValueError('ToUnicode entries differ')
                    map_entries+=len(entries)
                else:
                    name=expected['encodingName']
                    if mapping['kind']!='explicit_encoding' or mapping['base']!=name:raise ValueError('encoding selection differs')
                    want=[]
                    for slot in encodings['tables'][name]:
                        glyph=slot['glyph'];scalar=slot['nativeStrict']
                        want.append({'code':slot['byte'],'glyph_name_hex':glyph.encode('ascii').hex() if glyph is not None else None,'kind':'unicode' if glyph is not None else 'undefined','scalars':[scalar] if glyph is not None else None})
                    if mapping['slots']!=want:raise ValueError('encoding slots differ')
                    encoding_slots+=256
            if result['usage']['fonts']!=len(actual_fonts):raise ValueError('cache font count differs')
            row.update(status='matched',pages=len(result['pages']),fonts=len(actual_fonts),usage=result['usage'])
        except (ValueError,KeyError,subprocess.TimeoutExpired) as e:row.update(status='failure',error=str(e))
        rows.append(row)
    registered={(row['file'],font['id']) for row in inventory['pageResources'] for font in row['fonts']}
    all_simple={key for key,value in expected_fonts.items() if value['subtype'] in ['Type1','TrueType']}
    wanted_simple=all_simple & registered
    outside=[{'file':key[0],'id':key[1],'contexts':expected_fonts[key]['contexts'],'reason':'No inherited page Font registration; Form/appearance resources are outside F0.'} for key in sorted(all_simple-registered)]
    missing=sorted(wanted_simple-seen_fonts)
    if hashes!={str(f):digest(f) for f in [rust,oracle,*evidence]}:raise SystemExit('binary or evidence changed during comparison')
    report={'reference':'Pinned native dictionary resource inventory, encoding tables and CMap lookup oracle; no font loading/substitution','evidence_sha256':hashes,'documents':len(rows),'pages':pages,'bindings':bindings,'loaded_bindings':loaded,'unsupported_type0_bindings':unsupported,'simple_font_identities':len(seen_fonts),'encoding_slots':encoding_slots,'to_unicode_entries':map_entries,'failures':sum(r['status']=='failure' for r in rows)+len(missing),'missing_simple_fonts':missing,'nonpage_simple_fonts_outside_scope':outside,'rows':rows,'limitations':['Declared Unicode is not embedded glyph equivalence, metrics or text extraction.','Actual fixture encodings have no Differences; generated/library tests cover that separate path.','Type0 refusal does not exercise malformed-map policy; synthetic simple-font tests do.']}
    args.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items() if isinstance(v,int)}))
    if report['failures']:raise SystemExit(1)
if __name__=='__main__':main()
