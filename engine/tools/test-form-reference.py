#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Fail-closed and adversarial validation of R22 native/model/provenance comparison."""
import argparse,copy,hashlib,json,subprocess,time,runpy
from types import SimpleNamespace
from unittest.mock import patch
from pathlib import Path
from form_cases import generated
from form_reference import native_model,ENGINE

def read_native(process,page):
    if process.returncode!=0:raise ValueError('native process failure '+str(process.returncode))
    if process.stderr or len(process.stdout)>64*1024*1024:raise ValueError('native diagnostic/output bound')
    obj=json.loads(process.stdout)
    if not isinstance(obj,dict) or obj.get('repaired') is not False or not isinstance(obj.get('pages'),list) or len(obj['pages'])!=1:raise ValueError('native envelope')
    row=obj['pages'][0]
    if row.get('page')!=page or row.get('status') not in ('accepted','refused'):raise ValueError('native page identity/status')
    if row['status']=='refused' and (not isinstance(row.get('error'),str) or not row['error'].strip()):raise ValueError('native refusal diagnostic')
    return row

def main():
    ap=argparse.ArgumentParser();ap.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-form-page-oracle'));ap.add_argument('--report',type=Path,required=True);ap.add_argument('--rust',type=Path);args=ap.parse_args();start=time.monotonic();cases=list(generated());positives=[];refusals=[];mutations=[]
    evidence={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in [args.native,Path(__file__),ENGINE/'tools/form_reference.py',ENGINE/'tools/form_cases.py',ENGINE/'tools/form_review_model.py',ENGINE/'tools/mupdf-form-page-oracle.c',ENGINE/'tools/build-form-page-reference.sh',ENGINE/'reference/form-review-model-selfcheck.json',ENGINE/'reference/form-reference-policy-f0.json',ENGINE/'reference/form-reference-policy-notes-f0.md']}
    if args.rust:
        for p in [args.rust,ENGINE/'tools/differential-forms.py',ENGINE/'tools/form_raster_reference.py',ENGINE/'tools/form_pixel_model.py',ENGINE/'reference/stroke-raster-differential-s1.json']:
            evidence[str(p)]=hashlib.sha256(p.read_bytes()).hexdigest()
    chosen=None
    for c in cases:
        run=subprocess.run([str(args.native),'1'],input=c['pdf'],capture_output=True,timeout=50);n=read_native(run,1)
        if c['accepted']:
            stats=native_model(n,c);positives.append(dict(label=c['label'],**stats))
            if c['label']=='nested-0':chosen=(c,n)
        elif c['scope']=='strict-scope-native-fallback':
            if n['status']!='accepted':raise ValueError('native outer scope fallback changed')
            refusals.append(dict(label=c['label'],classification='native_accepts_strict_model_refuses'))
        else:
            if n['status']!='refused':raise ValueError('bad native fixture accepted')
            refusals.append(dict(label=c['label'],classification='explicit_native_refusal',error=n['error']))
    c,n=chosen
    def reject(label,change):
        altered=copy.deepcopy(n);change(altered)
        try:native_model(altered,c)
        except (ValueError,KeyError,IndexError):mutations.append(label)
        else:raise AssertionError('comparator accepted mutation '+label)
    reject('missing invocation',lambda x:x['form_invocations'].pop())
    reject('reordered invocation',lambda x:x['form_invocations'].reverse())
    for key,value in [('id',90),('parent',50),('object',[90,0]),('caller_provider',99),('effective_provider',99),('source_sha256','0'*64),('event_range',[1,2]),('matrix',[2,0,0,1,0,0]),('bbox',[0,0,40,40]),('ctm',[2,0,0,1,0,0])]:reject('invocation '+key,lambda x,k=key,v=value:x['form_invocations'][0].__setitem__(k,v))
    reject('missing operation origin',lambda x:x['operation_origins'].pop())
    reject('source local span',lambda x:x['operation_origins'][0].__setitem__('span',[99,100]))
    reject('wrong invocation source',lambda x:x['operation_origins'][0].__setitem__('invocation',9))
    reject('missing event',lambda x:x['events'].pop())
    reject('reordered events',lambda x:x['events'].reverse())
    def glyph(x,key,value):next(e for e in x['events'] if e['kind']=='glyph')[key]=value
    reject('source font object',lambda x:glyph(x,'font',[77,0]));reject('source code',lambda x:glyph(x,'code',90));reject('Unicode',lambda x:glyph(x,'unicode',[90]));reject('placement',lambda x:glyph(x,'matrix',[1,0,0,1,0,0]))
    clipindex=next(i for i,v in enumerate(n['events']) if v['kind']=='clip')
    reject('BBox clip command',lambda x:x['events'][clipindex]['path'][0].__setitem__(1,999))
    reject('clip parent IDs',lambda x:x['events'][clipindex].__setitem__('clip_ids',[999]))
    wrappers=[]
    base={'pages':[dict(page=1,status='accepted')],'repaired':False}
    def wrapper(label,code=0,stderr=b'',obj=None,stdout=None):
        p=subprocess.CompletedProcess([],code,stdout if stdout is not None else json.dumps(obj if obj is not None else base).encode(),stderr)
        try:read_native(p,1)
        except (ValueError,KeyError,IndexError,json.JSONDecodeError):wrappers.append(label)
        else:raise AssertionError('bad wrapper accepted '+label)
    for code in (-11,-9,1,2,3):wrapper('process exit '+str(code),code=code)
    wrapper('success diagnostic',stderr=b'warning')
    wrapper('repaired input',obj={'pages':[dict(page=1,status='accepted')],'repaired':True})
    wrapper('unexpected page status',obj={'pages':[dict(page=1,status='banana')],'repaired':False})
    wrapper('empty refusal',obj={'pages':[dict(page=1,status='refused',error='')],'repaired':False})
    wrapper('nonstring refusal',obj={'pages':[dict(page=1,status='refused',error=4)],'repaired':False})
    wrapper('wrong page',obj={'pages':[dict(page=2,status='accepted')],'repaired':False})
    wrapper('empty output',stdout=b'');wrapper('partial output',stdout=b'{')
    actual_wrappers=[]
    for label,data,argv,exitcode in [('invalid arg',c['pdf'],['0'],1),('input bound',b' '*((16<<20)+1),['1'],1),('broken document',b'%PDF-1.7 broken',['1'],2)]:
        run=subprocess.run([str(args.native),*argv],input=data,capture_output=True,timeout=50)
        if run.returncode!=exitcode or run.stdout or (exitcode==2 and len([line for line in run.stderr.splitlines() if line.startswith(b'form-page oracle: ') and line[len(b'form-page oracle: '):].strip()])!=1):raise ValueError('native negative wrapper '+label)
        actual_wrappers.append(label)
    rust_mutations=[];rust_wrapper_faults=[];pixel_mutations=[]
    if args.rust:
        from form_reference import OUT,rust_model,provenance
        from form_raster_reference import model_events,compare_model,dimensions
        from form_pixel_model import render
        driver=runpy.run_path(str(ENGINE/'tools/differential-forms.py'));config=SimpleNamespace(rust=args.rust,evidence_dir=None)
        r,_=driver['invoke_rust'](config,c['pdf'],1,'validation');assets=OUT['load_assets'](Path('/private/tmp/tandem-r20-builtin-evidence'))
        def check(a,b):OUT['compare_page'](a,b,assets);rust_model(a,c);provenance(a,b,c)
        check(r,n)
        def rust_reject(label,change,native=False):
            a,b=copy.deepcopy(r),copy.deepcopy(n);change(b if native else a)
            try:check(a,b)
            except (ValueError,KeyError,IndexError):rust_mutations.append(label)
            else:raise AssertionError('Rust comparator accepted '+label)
        for key,value in [('id',99),('parent',99),('stream_id',99),('object',[90,0]),('caller_scope',99),('effective_scope',0),('execution_range',[0,1]),('call_origin',dict(kind='page',span=[99,100])),('bbox',[0,0,20,20]),('matrix',[2,0,0,1,0,0])]:rust_reject('Rust invocation '+key,lambda x,k=key,v=value:x['trace']['form_invocations'][0].__setitem__(k,v))
        rust_reject('Rust retained source',lambda x:x['trace']['form_streams'][0].__setitem__('content_hex','00'))
        rust_reject('Rust source omission',lambda x:x['trace']['form_streams'].pop())
        rust_reject('Rust origin omission',lambda x:x['trace']['operation_origins'].pop())
        rust_reject('Rust wrong source span',lambda x:x['trace']['operation_origins'][0].__setitem__('span',[99,100]))
        rust_reject('Rust scope provider',lambda x:x['trace']['resource_scopes'][1].__setitem__('provider',[99,0]))
        rust_reject('Rust font scope',lambda x:x['trace']['fonts'][0].__setitem__('scope_id',99))
        rust_reject('Rust font operand',lambda x:x['trace']['fonts'][0].__setitem__('name_hex','00'))
        rust_reject('Rust font identity',lambda x:x['trace']['fonts'][0].__setitem__('identity',{'indirect':[99,0]}))
        rust_reject('Rust inherited font selection',lambda x:x['trace']['program']['text']['codes'][0].__setitem__('font_id',99))
        rust_reject('Rust operation index',lambda x:x['trace']['program']['text']['codes'][0].__setitem__('operation_index',99))
        rust_reject('Rust source code',lambda x:x['glyphs'][0].__setitem__('source',99))
        rust_reject('Rust outline coordinates',lambda x:x['commands'][0].__setitem__(1,999))
        rust_reject('Native glyph ID',lambda x:glyph(x,'gid',99),True)
        rust_reject('Native raw outline',lambda x:next(e for e in x['events'] if e['kind']=='glyph')['raw_commands'][0].__setitem__(1,999),True)
        rust_reject('Native unhinted outline',lambda x:next(e for e in x['events'] if e['kind']=='glyph')['unhinted_commands'][0].__setitem__(1,999),True)
        for code,stdout,stderr in [(-11,b'',b''),(3,b'',b'pdf-inspect: failure'),(2,b'',b''),(2,b'partial',b'pdf-inspect: error'),(0,b'{}',b''),(0,b'{}',b'warning')]:
            process=subprocess.CompletedProcess([],code,stdout,stderr)
            with patch('subprocess.run',return_value=process):
                try:driver['invoke_rust'](config,b'',1,'fault')
                except ValueError:rust_wrapper_faults.append(dict(code=code,stdout=bool(stdout),stderr=bool(stderr)))
                else:raise AssertionError('Rust wrapper fault accepted')
        pc=next(v for v in cases if v['label']=='clip-0');expected,uncertain,counts=render(model_events(pc,assets,72),32,32);bounds=counts['_channel_bounds'];compare_model(expected,expected,32,32,uncertain,bounds)
        changed=bytearray(expected);index=next(i for i,(lo,hi) in enumerate(bounds) if hi<255);changed[index]=255
        try:compare_model(changed,expected,32,32,uncertain,bounds)
        except ValueError:pixel_mutations.append('missing certified Form ink')
        else:raise AssertionError('missing Form ink accepted')
        try:dimensions(31,32,{'initial_clip':[0,0,32,32]},72)
        except ValueError:pixel_mutations.append('wrong native canvas')
        else:raise AssertionError('wrong canvas accepted')
        for tc in [v for v in cases if v['label'] in ('affine-0-0','affine-1-1')]:
            events=model_events(tc,assets,18);expected,uncertain,counts=render(events,8,8);bounds=counts['_channel_bounds']
            missing=[v for v in events if not(v['kind']=='fill' and any(p[0]=='c' for p in v['path']))]
            actual,_,_=render(missing,8,8)
            try:compare_model(actual,expected,8,8,uncertain,bounds)
            except ValueError:pixel_mutations.append('missing curved glyph '+tc['label'])
            else:raise AssertionError('text pixel model failed to constrain glyph ink')
        prior=json.loads((ENGINE/'reference/stroke-raster-differential-s1.json').read_text());rows=[copy.deepcopy(v) for v in prior['rows']];baseline={(v['file'],v['page'],v['dpi']):v for v in rows if v['status']=='native_diagnostic_matched'}
        for v in rows:
            if v['file']=='redact-shared-only.pdf':v.update(status='native_diagnostic_matched',rust_ppm_sha256='new')
        rows.extend(dict(file='redact-shared-only.pdf',page=p,dpi=72,status='native_diagnostic_matched',rust_ppm_sha256='new') for p in (1,2));driver['verify_pixel_baseline'](rows,baseline)
        for label,change in [('prior-page omission',lambda x:x.pop(1)),('prior-output hash',lambda x:next(r for r in x if r['file']=='large.pdf').__setitem__('rust_ppm_sha256','tampered')),('duplicate page',lambda x:x.append(x[0])),('actual-Form omission',lambda x:x.pop())]:
            bad=copy.deepcopy(rows);change(bad)
            try:driver['verify_pixel_baseline'](bad,baseline)
            except ValueError:pixel_mutations.append(label)
            else:raise AssertionError('baseline mutation accepted '+label)
    if evidence!={p:hashlib.sha256(Path(p).read_bytes()).hexdigest() for p in evidence}:raise ValueError('reference source changed')
    report=dict(status='passed',elapsed_seconds=time.monotonic()-start,evidence_sha256=evidence,positive_models=positives,negative_cases=refusals,rejected_mutations=mutations,rejected_process_faults=wrappers,native_negative_wrappers=actual_wrappers,rust_comparator_mutations=rust_mutations,rust_process_faults=rust_wrapper_faults,pixel_baseline_mutations=pixel_mutations)
    args.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:len(v) for k,v in report.items() if isinstance(v,list)}))
if __name__=='__main__':main()
