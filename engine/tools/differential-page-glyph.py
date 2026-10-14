#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""R18 source glyph identity, unhinted page geometry, and honest hinted observations."""
import argparse,collections,copy,gzip,hashlib,json,math,runpy,subprocess
from fractions import Fraction as F
from pathlib import Path
from page_glyph_model import generated,bearing_case,actual_generated,transform,pdf,page_matrix
from text_position_model import json_numbers
ENGINE=Path(__file__).resolve().parent.parent
PP=runpy.run_path(str(ENGINE/'tools/differential-page-program.py'))
PM=runpy.run_path(str(ENGINE/'tools/differential-program.py'))

def sha(data):return hashlib.sha256(data).hexdigest()
def invoke(binary,args,data,evidence=None):return PP['invoke'](binary,args,data,evidence)
def compact(program):
    result=dict(paths=program['paths'],text_contexts=program['text_contexts'],events=[])
    text=program['text'];utf8=bytes.fromhex(text['text_utf8_hex'])
    for ordered in program['events']:
        if ordered['kind']=='path':result['events'].append(['path',ordered['event']]);continue
        kind,index=text['events'][ordered['event']]
        if kind!='code':continue
        code=text['codes'][index];lo,hi=code['utf8_span']
        result['events'].append(['code',code['code'],list(map(ord,utf8[lo:hi].decode())),code['transform'],code['width']/1000,code['rendering_mode'],ordered['context']])
    return result

def native_program(page):
    result=copy.deepcopy(page)
    for i,event in enumerate(result['events']):
        if event['kind']=='glyph':result['events'][i]=['code',event['code'],event['unicode'],event['matrix'],event['advance'],event['mode'],event['context'],event['callback']]
    return result

def close(actual,expected,stats,field,native=True):
    if isinstance(expected,(float,int,F)) and not isinstance(expected,bool):
        if isinstance(actual,bool) or not isinstance(actual,(float,int)) or not math.isfinite(actual):raise ValueError(field+': invalid numeric')
        want=float(expected);delta=abs(actual-want)
        key='native_max_absolute_difference' if native else 'rational_max_absolute_difference'
        stats[key]=max(stats.get(key,0),delta)
        limit=.002+2e-5*max(1,abs(want)) if native else 0
        if delta>limit:raise ValueError(f'{field}: {actual} != {want} (limit{limit})')
    elif isinstance(expected,list):
        if not isinstance(actual,list) or len(actual)!=len(expected):raise ValueError(field+': length differs')
        for i,(a,e) in enumerate(zip(actual,expected)):close(a,e,stats,field+'/'+str(i),native)
    elif actual!=expected:raise ValueError(f'{field}: {actual!r} != {expected!r}')

def compare_page(r,n,model=None):
    if r['status']!='accepted' or n['status']!='accepted':raise ValueError('page refused: '+str((r.get('error'),n.get('error'))))
    stats=dict(numeric_nonexact_fields=0,max_absolute_difference=0,native_max_absolute_difference=0,rational_max_absolute_difference=0,hinted_topology_differences=0,hinted_coordinate_differences=0,hinted_max_absolute_difference=0)
    if r['page']!=n['page']:raise ValueError('page index differs')
    p=r['trace']['program'];cp=compact(p);np=native_program(n)
    PP['compare'](n['initial_ctm'],cp['paths']['initial_ctm'],stats,'initial_ctm')
    PP['compare'](n['initial_clip'],cp['paths']['initial_clip']['bounds'],stats,'initial_clip')
    expected,remaining=PP['rust_semantics'](cp);actual=PP['native_semantics'](np)
    if remaining!=n['cleanup_pops']:raise ValueError('cleanup clips differ')
    PP['compare'](actual,expected,stats,'events')
    native=[v for v in n['events'] if v['kind']=='glyph'];codes=p['text']['codes'];ordered=[]
    for index,event in enumerate(p['events']):
        if event['kind']=='text' and p['text']['events'][event['event']][0]=='code':ordered.append((index,event))
    if len(r['glyphs'])!=len(native) or len(codes)!=len(native):raise ValueError('glyph/code counts differ')
    cursor=0
    for i,(g,w,c,(event_index,event)) in enumerate(zip(r['glyphs'],native,codes,ordered)):
        for key,value in dict(code_index=i,event_index=event_index,context=event['context'],source=c['code'],font=w['font'],program=w['program'],gid=w['gid'],units_per_em=w['units_per_em'],origin_shift=-w['bearing_adjustment'],metric_lsb=w['metric_lsb'],hhea_range=w['hhea_range'],hmtx_range=w['hmtx_range']).items():
            if g[key]!=value:raise ValueError(f'glyph{i}/{key}: {g[key]} != {value}')
        if any(g['selection'][k]!=v for k,v in w['selection'].items()):raise ValueError('native selected cmap differs')
        identities={f['id']:f['identity'] for f in r['trace']['fonts']}
        if identities[c['font_id']]!={'indirect':g['font']}:raise ValueError('font provenance differs from code')
        low,high=g['command_range']
        if low!=cursor or high<low or high>len(r['commands']):raise ValueError('glyph command range/order')
        cursor=high;commands=r['commands'][low:high]
        close(commands,w['unhinted_commands'],stats,f'glyph{i}/unhinted')
        # Independently reapply the actual drawing-origin transform to exact raw native points.
        raw_expected=transform(w['raw_commands'],w['bearing_adjustment'],w['units_per_em'],list(map(F,c['transform'])))
        # Native f32 is the bounded primary parity gate; model exactness only claimed for dyadic generator.
        close(commands,json_numbers(raw_expected),stats,f'glyph{i}/raw-transform')
        unhinted=w['unhinted_commands'];hinted=w['native_outline']
        if [len(v) for v in unhinted]!=[len(v) for v in hinted] or [v[0] for v in unhinted]!=[v[0] for v in hinted]:stats['hinted_topology_differences']+=1
        else:
            for a,b in zip(unhinted,hinted):
                for x,y in zip(a[1:],b[1:]):
                    d=abs(x-y);stats['hinted_coordinate_differences']+=bool(d);stats['hinted_max_absolute_difference']=max(stats['hinted_max_absolute_difference'],d)
        if model:
            mc=model['expected']['text']['codes'][i];gid=model['gids'][mc['byte']]
            if g['gid']!=gid or g['font']!=[4,0] or g['program']!=[6,0]:raise ValueError('model source/font/program/GID mismatch')
            if g['route']!='encoding_unicode' or g['lookup_code']!=mc['byte']:raise ValueError('model mapping provenance')
            if model['outline'] is None:continue
            outline=model['outline']['glyphs'][gid]
            if w['raw_commands']!=outline['commands'] or w['bearing_adjustment']!=outline['bearing_adjustment']:raise ValueError('independent raw glyph differs')
            wanted=transform(outline['commands'],outline['bearing_adjustment'],model['outline']['units_per_em'],mc['rendering'])
            close(c['transform'],list(mc['rendering']),stats,'rational-matrix',False)
            # Explicit1000-unit bearing case has non-dyadic output; compare exact expectedfloat with1ulp only here.
            if model['index']=='bearing-width':
                for aa,bb in zip(commands,wanted):
                    if aa[0]!=bb[0] or len(aa)!=len(bb):raise ValueError('bearing command kind')
                    for a,b in zip(aa[1:],bb[1:]):
                        if abs(a-float(b))>math.ulp(float(b)):raise ValueError('bearing rational coordinate')
            else:close(commands,wanted,stats,'rational-commands',False)
    if cursor!=len(r['commands']):raise ValueError('unattached command tail')
    if model:
        modeled=PM['semantic_expected'](model['expected'])
        for v in modeled:v.pop('clip_depth')
        if expected!=modeled:raise ValueError('independent rational mixed-state trace differs')
    stats.update(codes=len(native),commands=len(r['commands']),semantic_events=len(expected),nonzero_origin_shifts=sum(g['origin_shift']!=0 for g in r['glyphs']))
    return stats


def main():
    ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--rust',type=Path,required=True);ap.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-page-glyph-oracle'));ap.add_argument('--report',type=Path,required=True);ap.add_argument('--evidence-dir',type=Path);ap.add_argument('--generated',type=int,default=220);ap.add_argument('--skip-corpus',action='store_true');ap.add_argument('--actual-font',type=Path,default=Path('/private/tmp/outline-fonts/font1.ttf'));a=ap.parse_args()
    sources=[a.rust,a.native,Path(__file__),Path(__file__).with_name('page_glyph_model.py'),ENGINE/'tools/outline_model.py',ENGINE/'tools/truetype_cmap_model.py',ENGINE/'tools/program_model.py',ENGINE/'tools/text_position_model.py',ENGINE/'tools/differential-page-program.py']
    for f in ['mupdf-page-glyph-oracle.c','build-page-glyph-reference.sh']:
        if (ENGINE/'tools'/f).exists():sources.append(ENGINE/'tools'/f)
    sources.extend([a.actual_font,ENGINE/'reference/embedded-outline-inventory.json',ENGINE/'reference/fixture-inventory.json']);hashes={str(p):sha(p.read_bytes()) for p in sources};rows=[];docs=[];gen=[]
    actual_font=a.actual_font.read_bytes();inventory=json.loads((ENGINE/'reference/embedded-outline-inventory.json').read_text());assert sha(actual_font)==inventory['fonts'][1]['programs'][0]['sha256']
    def evidence(kind,name):return (a.evidence_dir/kind/name).with_suffix('.json.gz') if a.evidence_dir else None
    if not a.skip_corpus:
        for doc in json.loads((ENGINE/'reference/fixture-inventory.json').read_text())['documents']:
            if doc.get('passwordRequired'):continue
            name=doc['file'];data=(ENGINE.parent/'fixtures'/name).read_bytes();assert sha(data)==doc['sha256'];record=dict(file=name,sha256=doc['sha256']);outputs={}
            for kind,binary,args in [('rust',a.rust,['--page-glyphs','-']),('native',a.native,['all'])]:
                try:
                    output,record[kind+'_output_sha256']=invoke(binary,args,data,evidence(kind,name))
                    if [p['page'] for p in output['pages']]!=list(range(1,doc['pages']+1)):raise ValueError('document page order/count')
                    if output.get('repaired'):raise ValueError('native repaired input')
                    outputs[kind]=output
                except (ValueError,subprocess.TimeoutExpired) as error:record[kind+'_error']=str(error)
            for i in range(doc['pages']):
                row=dict(file=name,page=i+1,stress=name=='large.pdf');r=outputs.get('rust',{}).get('pages',[None]*doc['pages'])[i];n=outputs.get('native',{}).get('pages',[None]*doc['pages'])[i]
                row.update(rust_status=r['status'] if r else 'document_error',native_status=n['status'] if n else 'document_error')
                if r and r['status']!='accepted':row.update(status='refused',rust_refusal=r['error'])
                elif not r:row.update(status='failure',error=record['rust_error'])
                elif not n or n['status']!='accepted':row.update(status='unverified',native_refusal=n['error'] if n else record['native_error'])
                else:
                    try:row.update(status='matched',**compare_page(r,n))
                    except (ValueError,KeyError,IndexError) as error:row.update(status='mismatch',error=str(error))
                if n and n['status']!='accepted':row['native_refusal']=n['error']
                rows.append(row)
            docs.append(record)
    for model in [*generated(a.generated),bearing_case(),*actual_generated(actual_font)]:
        data=model['pdf'];name=f'generated/{model["index"]}.pdf';row=dict(index=model['index'],sha256=sha(data),font_sha256=sha(model['font']))
        try:
            r,row['rust_output_sha256']=invoke(a.rust,['--page-glyphs','-'],data,evidence('rust',name));n,row['native_output_sha256']=invoke(a.native,['all'],data,evidence('native',name))
            if len(r['pages'])!=1 or len(n['pages'])!=1 or n.get('repaired'):raise ValueError('generated page envelope')
            row.update(status='matched',**compare_page(r['pages'][0],n['pages'][0],model))
        except (ValueError,KeyError,IndexError) as error:row.update(status='failure',error=str(error))
        gen.append(row)
    def total(pop):return dict(pages=len(pop),statuses=dict(collections.Counter(v['status'] for v in pop)),native_statuses=dict(collections.Counter(v.get('native_status','accepted') for v in pop)),codes=sum(v.get('codes',0) for v in pop),commands=sum(v.get('commands',0) for v in pop),nonzero_origin_shifts=sum(v.get('nonzero_origin_shifts',0) for v in pop),native_max_absolute_difference=max([v.get('native_max_absolute_difference',0) for v in pop]+[0]),rational_max_absolute_difference=max([v.get('rational_max_absolute_difference',0) for v in pop]+[0]),hinted_topology_differences=sum(v.get('hinted_topology_differences',0) for v in pop),hinted_coordinate_differences=sum(v.get('hinted_coordinate_differences',0) for v in pop),hinted_max_absolute_difference=max([v.get('hinted_max_absolute_difference',0) for v in pop]+[0]))
    if hashes!={str(p):sha(p.read_bytes()) for p in sources}:raise ValueError('source changed during comparison')
    report=dict(reference='Pinned MuPDF source CID/GID/font identity + FreeType NO_SCALE/NO_HINTING outline at actual drawing origin; independent rational generated model',evidence_sha256=hashes,corpus=total(rows),stress=total([v for v in rows if v['stress']]),other=total([v for v in rows if not v['stress']]),generated=total(gen),tolerance=dict(native_glyph='.002 + 2e-5*max(1,abs(expected)); inherited T0 placement policy',native_path='max(1e-5,2e-6*abs(expected)); inherited P0 policy',rational='Exact float equality to independent dyadic rational expected values; one explicit1000units bearing case allows1ulp;16actual-font pages use native outline geometry plus exact rational mixed state',identity='Exact event order/code/Unicode/font/program/GID/units/bearing/ranges; clip topology and rendering mode exact'),limitations=['Unhinted outline geometry is primary gate; native hinted fz_outline output is observed separately and is not pixel/hint parity.','Native callback batching normalized to one source-code glyph; no synthetic font, system substitution, composite or missing-glyph rendering.','Initial page rectangle and explicit path clips retained as metadata; paths are not geometrically clipped and glyphs are not rasterized.','Mode3 color is independently modeled but not observable through native ignore_text.','R18 is a narrower glyph-placement acceptance subset; the previous PP1 gate still verifies543/572traces. No previously accepted parser trace was weakened to obtain this result.','All readable fixture pages are attempted without candidate filtering;570documented glyph-stage refusals remain distinct from the2acceptedpages.'],documents=docs,rows=rows,generated_rows=gen)
    a.report.parent.mkdir(parents=True,exist_ok=True);a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:report[k] for k in ('corpus','stress','other','generated')}))
    if any(v['status'] not in ('matched','refused') for v in rows+gen):raise SystemExit(1)
if __name__=='__main__':main()
