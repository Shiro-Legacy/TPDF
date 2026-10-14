#!/usr/bin/env python3
"""Compare all explicit PDF widths and 256 code outcomes to native dictionary reads."""
import argparse,hashlib,json,subprocess
from pathlib import Path

def digest(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--rust',type=Path,default=Path('target/debug/pdf-inspect'));p.add_argument('--fixtures',type=Path,default=Path('../fixtures'));p.add_argument('--report',type=Path,default=Path('reference/font-metrics-differential.json'));a=p.parse_args();rust=a.rust.resolve()
    paths=[Path('reference/font-metrics-inventory.json'),Path('reference/fixture-inventory.json'),Path('reference/simple-font-inventory.json')]
    metadata,fixtures,resources=[json.loads(p.read_text()) for p in paths];hashes={str(p):digest(p) for p in [rust,*paths]}
    expected={(f['file'],f['id']):f for f in metadata['fonts']};rows=[];seen=set();builtin=set();lookups=entries=zero=bindings=type0=0
    for record in fixtures['documents']:
        data=(a.fixtures/record['file']).read_bytes()
        if hashlib.sha256(data).hexdigest()!=record['sha256']:raise SystemExit('immutable fixture changed')
        if record.get('passwordRequired'):continue
        row={'file':record['file'],'sha256':record['sha256']}
        try:
            run=subprocess.run([str(rust),'--font-metrics','-'],input=data,capture_output=True,timeout=30)
            if run.returncode or run.stderr:raise ValueError(run.stderr.decode(errors='replace')[:2000])
            result=json.loads(run.stdout)
            if len(result['pages'])!=record['pages']:raise ValueError('page count mismatch')
            for page in result['pages']:
                wanted=next(p for p in resources['pageResources'] if p['file']==record['file'] and p['page']==page['page'])
                actual={bytes.fromhex(b['name_hex']).decode():b for b in page['bindings']}
                if set(actual)!={f['name'] for f in wanted['fonts']}:raise ValueError('binding set mismatch')
                for font in wanted['fonts']:
                    got=actual[font['name']];bindings+=1;key=(record['file'],font['id'])
                    if got['target']!=list(map(int,font['id'].split(':'))):raise ValueError('target mismatch')
                    if font['subtype']=='Type0':
                        if got['status']!='unsupported':raise ValueError('Type0 accepted')
                        type0+=1;continue
                    want=expected[key]
                    if want['widthsKind']=='absent':
                        if got['status']!='needs_builtin_metrics':raise ValueError('absent widths not distinct')
                        builtin.add(key)
                    elif got['status']!='loaded' or got['font']!='i:'+font['id']:raise ValueError('explicit widths failed')
            for font in result['fonts']:
                key=(record['file'],font['id'][2:]);want=expected[key];seen.add(key)
                if (font['first'],font['last'])!=(want['firstChar'],want['lastChar']):raise ValueError('range differs')
                expected_missing={'advance':want['missingWidth'] if want['missingWidthKind']!='absent' else 0,'origin':'declared_missing' if want['missingWidthKind']!='absent' else 'default_missing'}
                if font['missing']!=expected_missing:raise ValueError('missing-width provenance differs')
                values=[]
                for code in range(256):
                    if want['firstChar']<=code<=want['lastChar']:value={'advance':want['widths'][code-want['firstChar']],'origin':'widths'}
                    else:value=expected_missing
                    values.append({'code':code,**value})
                if font['lookups']!=values:raise ValueError('lookup values/provenance differ')
                lookups+=256;entries+=len(want['widths']);zero+=sum(v==0 for v in want['widths'])
            if result['usage']['metrics']!=len(result['fonts']):raise ValueError('metric cache count mismatch')
            row.update(status='matched',fonts=len(result['fonts']),usage=result['usage'])
        except (ValueError,KeyError,subprocess.TimeoutExpired) as e:row.update(status='failure',error=str(e))
        rows.append(row)
    missing=sorted({key for key,value in expected.items() if value['widthsKind']=='array'}-seen)
    if hashes!={str(p):digest(p) for p in [rust,*paths]}:raise SystemExit('binary/evidence changed')
    report={'reference':'Pinned MuPDF1.28.1 dictionary-read inventory; no font-substitution oracle','evidence_sha256':hashes,'documents':len(rows),'bindings':bindings,'explicit_fonts':len(seen),'builtin_required_fonts':len(builtin),'unsupported_type0_bindings':type0,'code_lookups':lookups,'explicit_entries':entries,'explicit_zero_entries':zero,'failures':sum(r['status']=='failure' for r in rows)+len(missing),'missing_explicit_fonts':missing,'rows':rows,'limitations':['Fractional and negative widths use independent unit tests; native font loading truncates declared reals.','No builtin metrics, glyph equivalence or text positioning claimed.']}
    a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items() if isinstance(v,int)}))
    if report['failures']:raise SystemExit(1)
if __name__=='__main__':main()
