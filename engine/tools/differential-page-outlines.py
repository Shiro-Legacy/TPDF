#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""R20 pinned/embedded page identities and canonical unhinted drawing-origin comparison."""
import argparse,collections,copy,gzip,hashlib,json,math,runpy,subprocess
from fractions import Fraction as F
from pathlib import Path
from page_glyph_model import transform
from page_outlines_model import generated,check_commands
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

def compare_page(r,n,assets,model=None):
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
        fields=dict(code_index=i,event_index=event_index,context=event['context'],source=c['code'],font=w['font'],units_per_em=w['units_per_em'],source_kind=w['source_kind'])
        if w['source_kind']=='pinned_base14':
            pins={'Helvetica':'366720e054067fa49d030f58005c739fc787dcc9727a9327a28767777791d706','Times-Roman':'eb609a306b81357853ba91c2a3c47f3bba04f3a8cfdeedc733b1aa4af5d03d4c'}
            if w['asset_sha256']!=pins[w['face']]:raise ValueError('native page asset provenance')
            fields.update(native_gid=w['gid'],face=w['face'],glyph_name=w['glyph_name'],asset_sha256=w['asset_sha256'])
            if any(k in g for k in ('program','selection','gid','origin_shift','metric_lsb')):raise ValueError('builtin has synthetic TT provenance')
            canonical=assets[w['face']][w['glyph_name']]
            doubled=[[cmd[0],*[v*2 for v in cmd[1:]]] for cmd in canonical['commands']]
            if w['raw_commands']!=doubled or w['bearing_adjustment']!=0 or w['gid']!=canonical['native_gid']:raise ValueError('canonical CFF drawing origin differs')
        else:
            fields.update(program=w['program'],gid=w['gid'],origin_shift=-w['bearing_adjustment'],metric_lsb=w['metric_lsb'],hhea_range=w['hhea_range'],hmtx_range=w['hmtx_range'])
            if any(g['selection'][k]!=v for k,v in w['selection'].items()):raise ValueError('native selected cmap differs')
        for key,value in fields.items():
            if g[key]!=value:raise ValueError(f'glyph{i}/{key}: {g[key]} != {value}')
        identities={f['id']:f['identity'] for f in r['trace']['fonts']}
        if identities[c['font_id']]!={'indirect':g['font']}:raise ValueError('font provenance differs from code')
        low,high=g['command_range']
        if low!=cursor or high<low or high>len(r['commands']):raise ValueError('glyph command range/order')
        cursor=high;commands=r['commands'][low:high]
        close(commands,w['unhinted_commands'],stats,f'glyph{i}/unhinted')
        # Independently reapply the actual drawing-origin transform to exact raw native points.
        if w['source_kind']=='embedded_true_type':
            raw_expected=transform(w['raw_commands'],w['bearing_adjustment'],w['units_per_em'],list(map(F,c['transform'])))
            # Native f32 is the bounded primary parity gate; model exactness only claimed for dyadic generator.
            close(commands,json_numbers(raw_expected),stats,f'glyph{i}/raw-transform')
        unhinted=w['unhinted_commands'];hinted=w['native_outline']
        if [len(v) for v in unhinted]!=[len(v) for v in hinted] or [v[0] for v in unhinted]!=[v[0] for v in hinted]:stats['hinted_topology_differences']+=1
        else:
            for a,b in zip(unhinted,hinted):
                for x,y in zip(a[1:],b[1:]):
                    d=abs(x-y);stats['hinted_coordinate_differences']+=bool(d);stats['hinted_max_absolute_difference']=max(stats['hinted_max_absolute_difference'],d)
        if w['source_kind']=='pinned_base14':
            check_commands(commands,canonical['commands'],c['transform'],stats)
        if model:
            mc=model['expected']['text']['codes'][i];font=model['fonts'][mc['font']];name=font['names'][mc['byte']]
            if g['source']!=mc['byte'] or g['face']!=font['face'] or g['glyph_name']!=name or g['font']!=[font['object'],0]:raise ValueError('independent PDF source/name/font identity')
            close(c['transform'],list(mc['rendering']),stats,'rational-matrix',False)
            check_commands(commands,assets[font['face']][name]['commands'],mc['rendering'],stats)
    if cursor!=len(r['commands']):raise ValueError('unattached command tail')
    if model:
        modeled=PM['semantic_expected'](model['expected'])
        for v in modeled:v.pop('clip_depth')
        if expected!=modeled:raise ValueError('independent rational mixed-state trace differs')
    stats.update(codes=len(native),commands=len(r['commands']),semantic_events=len(expected),nonzero_origin_shifts=sum(g.get('origin_shift',0)!=0 for g in r['glyphs']),builtin_codes=sum(g['source_kind']=='pinned_base14' for g in r['glyphs']))
    return stats


def read_process(run,kind,page):
    if len(run.stdout)>64*1024*1024:raise ValueError(kind+' oversized output')
    if run.returncode:
        if kind=='rust' and run.returncode==2 and not run.stdout and run.stderr.startswith(b'pdf-inspect: ') and len(run.stderr.strip())>13:
            return dict(pages=[dict(page=page,status='refused',error=run.stderr.decode().strip())])
        if kind=='native' and run.returncode==2 and not run.stdout and run.stderr.startswith(b'page-outline oracle: ') and len(run.stderr.strip())>21:
            return dict(pages=[dict(page=page,status='refused',error=run.stderr.decode().strip())])
        raise ValueError(f'{kind} unexpected process exit{run.returncode}: '+run.stderr.decode(errors='replace')[:2000])
    if run.stderr:raise ValueError(kind+' unexpected diagnostic: '+run.stderr.decode(errors='replace')[:2000])
    output=json.loads(run.stdout)
    if output.get('repaired') or len(output['pages'])!=1 or output['pages'][0]['page']!=page:raise ValueError(kind+' page envelope/order')
    if output['pages'][0]['status'] not in ('accepted','refused'):raise ValueError(kind+' unexpected page status')
    return output

def invoke_page(binary,args,data,kind,page,evidence=None):
    run=subprocess.run([str(binary),*args],input=data,capture_output=True,timeout=60)
    output=read_process(run,kind,page)
    if evidence:
        evidence.parent.mkdir(parents=True,exist_ok=True);evidence.write_bytes(gzip.compress(run.stdout,mtime=0))
    return output['pages'][0],sha(run.stdout)

def totals(rows):
    stats=dict(pages=len(rows),statuses=dict(collections.Counter(v['status'] for v in rows)),native_statuses=dict(collections.Counter(v.get('native_status','accepted') for v in rows)))
    for key in ['codes','builtin_codes','commands','semantic_events','nonzero_origin_shifts','hinted_topology_differences','hinted_coordinate_differences','rational_coordinates','rational_nonexact_coordinates']:stats[key]=sum(r.get(key,0) for r in rows)
    for key in ['native_max_absolute_difference','rational_max_absolute_difference','rational_max_enclosure_radius','hinted_max_absolute_difference']:stats[key]=max([r.get(key,0) for r in rows]+[0])
    return stats

def load_assets(directory):
    return {face:{g['name']:g for g in json.loads(gzip.decompress((directory/(face+'-native.json.gz')).read_bytes()))['glyphs']} for face in ('Helvetica','Times-Roman')}

def main():
    import concurrent.futures
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--rust',type=Path,required=True);p.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-page-outlines-oracle'));p.add_argument('--report',type=Path,required=True);p.add_argument('--evidence-dir',type=Path);p.add_argument('--asset-evidence',type=Path,default=Path('/private/tmp/tandem-r20-builtin-evidence'));p.add_argument('--generated',type=int,default=220);p.add_argument('--skip-corpus',action='store_true');p.add_argument('--workers',type=int,default=4);a=p.parse_args();assets=load_assets(a.asset_evidence)
    sources=[a.rust,a.native,Path(__file__),ENGINE/'reference/fixture-inventory.json',ENGINE/'reference/builtin-outlines-manifest.json',ENGINE/'reference/builtin-outlines-differential-b0.json']+[ENGINE/'tools'/v for v in ['page_outlines_model.py','mupdf-page-outlines-oracle.c','build-page-outlines-reference.sh','differential-page-program.py','differential-program.py','program_model.py','text_position_model.py','page_glyph_model.py']]+[a.asset_evidence/(face+'-native.json.gz') for face in assets]
    hashes={str(v):sha(v.read_bytes()) for v in sources}
    def run_case(name,index,data,model=None):
        row=dict(file=name,page=index,stress=name=='large.pdf',sha256=sha(data));outputs={}
        for kind,binary,args in [('rust',a.rust,[f'--page-outlines={index}','-']),('native',a.native,[str(index)])]:
            try:
                ev=a.evidence_dir/kind/name/(str(index)+'.json.gz') if a.evidence_dir else None
                outputs[kind],row[kind+'_output_sha256']=invoke_page(binary,args,data,kind,index,ev);row[kind+'_status']=outputs[kind]['status']
                if outputs[kind]['status']=='refused':row[kind+'_refusal']=outputs[kind]['error']
            except (ValueError,subprocess.TimeoutExpired) as error:row[kind+'_status']='document_error';row[kind+'_error']=str(error)
        r=outputs.get('rust');n=outputs.get('native')
        if row.get('native_status')=='document_error':row.update(status='failure',error=row['native_error'])
        elif not r:row.update(status='failure',error=row['rust_error'])
        elif r['status']=='refused':row['status']='refused'
        elif not n or n['status']!='accepted':row['status']='unverified'
        else:
            try:row.update(status='matched',**compare_page(r,n,assets,model))
            except (ValueError,KeyError,IndexError) as error:row.update(status='mismatch',error=str(error))
        return row
    tasks=[]
    if not a.skip_corpus:
        for doc in json.loads((ENGINE/'reference/fixture-inventory.json').read_text())['documents']:
            if doc.get('passwordRequired'):continue
            data=(ENGINE.parent/'fixtures'/doc['file']).read_bytes()
            if sha(data)!=doc['sha256']:raise ValueError('fixture changed')
            tasks.extend((doc['file'],i+1,data) for i in range(doc['pages']))
    with concurrent.futures.ThreadPoolExecutor(max_workers=a.workers) as pool:
        rows=list(pool.map(lambda v:run_case(*v),tasks))
        gen=list(pool.map(lambda m:run_case('generated/'+str(m['index']),1,m['pdf'],m),generated(a.generated)))
    if hashes!={str(v):sha(v.read_bytes()) for v in sources}:raise ValueError('reference source changed')
    if rows:
        previous=json.loads((ENGINE/'reference/page-glyph-differential-g1.json').read_text())
        known={(r['file'],r['page']) for r in previous['rows'] if r['status']=='matched'}
        actual={(r['file'],r['page']) for r in rows if r['status']=='matched'}
        if not known<=actual:raise ValueError('previous accepted page disappeared')
    report=dict(evidence_sha256=hashes,corpus=totals(rows),stress=totals([r for r in rows if r['stress']]),other=totals([r for r in rows if not r['stress']]),generated=totals(gen),tolerance=dict(native_glyph='.002 + 2e-5*max(1,abs(expected)); unchanged T0 placement policy',native_path='max(1e-5,2e-6*abs(expected)); unchanged P0 policy',rational='Exact rational state/matrices for generated PDFs. CFF coordinate enclosure uses one endpoint ULP per normalized division/product/sum; no fitted global epsilon.',identity='Exact source byte, Unicode, font object, name/GID, asset hash, command topology/ranges/order, clip/color/state'),limitations=['Only two explicit pinned substitute faces; no general CFF parser or arbitrary font substitution.','Canonical drawing-origin unhinted geometry is verified; native fz_outline hint/normalization output is diagnostic, not pixel parity.','Corpus page calls are independent sessions; cumulative whole-document policy is covered by Rust tests, not this single-page corpus gate.','The exact rational model uses independently generated state; canonical CFF commands are independently native-derived and separately exhaustively checked, not independently parsed Type2 charstrings.'],rows=rows,generated_rows=gen)
    a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:report[k] for k in ('corpus','stress','other','generated')}))
    if any(r['status'] not in ('matched','refused') for r in rows) or any(r['status']!='matched' for r in gen):raise SystemExit(1)
if __name__=='__main__':main()
