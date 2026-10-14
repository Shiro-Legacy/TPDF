#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Fail-closed R24 coverage and raw-evidence replay; no partial corpus acceptance."""
import argparse,collections,copy,gzip,hashlib,importlib.util,json,re
from pathlib import Path
from soft_mask_corpus_model import certify,source_alpha,validate_mask_callbacks,source_glyph_paths
ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('differential_masks',ROOT/'tools/differential-soft-masks.py');D=importlib.util.module_from_spec(spec);spec.loader.exec_module(D)
sha=D.sha;require=D.require

def cases():
    result=list(D.generated());result.append(D.case('nonopaque-strong',bytes([30,70,120])*35,7,5,bytes([0])*35,'',bytes([0])*35,False));return {c['label']:c for c in result}
def key(row):return row['file'],row['page'],row['dpi']
def generated_evidence(case):
    common={'pdf','unmasked','alpha','expected_base'}
    if case['accepted']:
        return common | {'rust_mask','rust_base','rust_page','rust_unmasked',
                         'native_mask','native_page','native_unmasked',
                         'native_trace','model'}
    if case['label'].startswith('nonopaque'):
        return common | {'rust_stderr','native_mask','native_page','native_unmasked'}
    if case['label']=='nested':return common | {'rust_stderr','native_stderr'}
    return common | {'rust_stderr','native_mask'}

def verify_identities(report):
    required={str(p) for p in D.identity_files(ROOT,report['binaries'])}
    require(set(report['identities'])==required,'exact source/tool/binary identities')
    for path,digest in report['identities'].items():
        require(sha(Path(path).read_bytes())==digest,'changed source/tool/binary '+path)
    checks=json.loads((ROOT/'reference/soft-mask-rust-checks-r24.json').read_text())
    verify_checks(checks)
    require(report['binaries']=={k:v['path'] for k,v in checks['binaries'].items()},
            'executed binaries must match checked build')
    for role,path in report['binaries'].items():
        require(report['identities'][path]==checks['binaries'][role]['sha256'],
                'checked binary identity '+role)

def verify_checks(checks):
    expected={str(p.relative_to(ROOT)) for p in D.source_files(ROOT)}
    require(set(checks['production_sha256'])==expected,'complete checked source manifest')
    for rel,digest in checks['production_sha256'].items():
        require(sha((ROOT/rel).read_bytes())==digest,'Rust checks source freshness')
    require(set(checks['checks'])=={'tests','msrv_tests','fmt','clippy','wasm','build'},
            'Rust check coverage')
    for name,row in checks['checks'].items():
        data=Path(row['path']).read_bytes()
        require(row['exit_code']==0 and sha(data)==row['sha256'],'Rust check '+name)
        text=data.decode()
        require(not re.search(r'(?m)^(?:error(?:\[|:)|test result: FAILED)',text),
                'failed Rust check '+name)
        if name in ('tests','msrv_tests'):
            summaries=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;',text)
            count=sum(int(n) for n,_,_ in summaries)
            require(summaries and all(f=='0' and i=='0' for _,f,i in summaries) and
                    count==checks[name] and count>=809 and 'Doc-tests tandem_pdf_engine' in text,
                    'complete passing tests '+name)
        elif name=='fmt':require(not data,'format check diagnostics')
        else:require('Finished ' in text,'missing successful build/check completion '+name)
    require(checks['tests']==checks['msrv_tests'],'current/MSRV test count equality')
    require(set(checks['binaries'])==D.BINARY_ROLES,'checked binary role coverage')
    for role,row in checks['binaries'].items():
        path=Path(row['path'])
        require(path.is_absolute() and sha(path.read_bytes())==row['sha256'],'checked binary '+role)

def shapes(report):
    require(report['status']=='passed','report status')
    base=json.loads((ROOT/'reference/image-corpus-differential-r23c.json').read_text())['pixel_rows'];expected={key(r):r for r in base}
    for file in ('p3/edit-objects.pdf','redact.pdf'):expected[(file,1,72)]={}
    rows={key(r):r for r in report['corpus']};require(len(rows)==len(report['corpus'])==634 and set(rows)==set(expected),'exact corpus identities')
    require(report['counts']=={'preserved_r23':617,'new_opaque_mask_render':2,'refused':15},'exact corpus counts')
    require(dict(collections.Counter(r['status'] for r in rows.values()))==report['counts'],'actual corpus counts')
    for k,r in rows.items():
        prior=expected[k]
        required={'rust'} if 'rust_ppm_sha256' in prior else ({'rust','native_trace','native4x','model','bounds'} if k[0]=='p3/edit-objects.pdf' else {'native_trace','rust_stderr'})
        require(set(r['evidence'])==required,'exact corpus raw evidence '+str(k))
        if 'rust_ppm_sha256' in prior:require(r['status']=='preserved_r23' and r['rust_ppm_sha256']==prior['rust_ppm_sha256'],'R23 hash preservation')
        elif k[0]=='p3/edit-objects.pdf':require(r['status']=='new_opaque_mask_render' and r['violations']==0,'new page acceptance')
        else:require(r['status']=='refused' and r['refusal'].startswith('pdf-inspect: ') and 'rust' not in r['evidence'],'atomic corpus refusal')
    wanted=cases();generated={r['label']:r for r in report['generated']};require(len(generated)==len(report['generated'])==56 and set(generated)==set(wanted),'generated coverage')
    for label,c in wanted.items():
        r=generated[label];require(set(r['evidence'])==generated_evidence(c),'exact generated raw evidence '+label);require(r['status']=='passed' and r['accepted']==c['accepted'],'generated admission')
        if label=='nested':require(isinstance(r['native_refusal'],str) and r['native_refusal'].startswith('image decode oracle: '),'nested native refusal')
        else:require(r['native_refusal'] is None,'unexpected native refusal')
        if c['accepted']:require(r['rust_refusal'] is None and 'rust_mask' in r['evidence'],'positive mask provenance required')
        else:require(r['rust_refusal'].startswith('soft-mask: ') and 'rust_mask' not in r['evidence'] and 'rust_page' not in r['evidence'],'unsupported mask cannot succeed')
    require([(r['file'],r['base'],r['mask'],r['samples']) for r in report['masks']]==[('p3/edit-objects.pdf',10,12,60000),('redact.pdf',10,11,3240)],'actual mask coverage')
    for row in report['masks']:require(set(row['evidence'])=={'rust','native','source_alpha'},'actual mask raw evidence')
    return wanted

def read(record):
    data=gzip.decompress(Path(record['path']).read_bytes());require(len(data)<=64<<20 and sha(data)==record['sha256'],'raw evidence identity');return data

def verify(report):
    wanted=shapes(report)
    verify_identities(report)
    for row in report['generated']:
        c=wanted[row['label']];ev={k:read(v) for k,v in row['evidence'].items()}
        for k in ('pdf','unmasked','alpha','expected_base'):require(ev[k]==c[k],'authored generated evidence')
        expected=dict(base_object=5,mask_object=6,width=c['w'],height=c['h'],alpha_hex=c['alpha'].hex())
        for field in ('rust','native'):
            if field+'_stderr' in ev:require(ev[field+'_stderr'].decode()==row[field+'_refusal'],'saved refusal diagnostic')
        if 'native_mask' in ev:require(json.loads(ev['native_mask'])==expected,'native complete attached mask')
        if c['accepted']:
            require(json.loads(ev['rust_mask'])==expected and ev['rust_base']==c['expected_base'],'Rust complete provenance/bytes')
            require(ev['native_page']==ev['native_unmasked'] and ev['rust_page']==ev['rust_unmasked'],'opaque/unmasked outcomes')
            trace=json.loads(ev['native_trace']);require(trace['repaired'] is False and len(trace['pages'])==1,'native generated envelope');p=trace['pages'][0];require(p['status']=='accepted','native generated acceptance');events=p['events'];require([e['kind'] for e in events]==['opaque_mask_clip','image','pop'],'actual generated mask callbacks')
            clip,image,pop=events;require(clip['base_object']==[5,0] and clip['mask_object']==[6,0] and bytes.fromhex(clip['alpha_hex'])==c['alpha'] and clip['matrix']==image['matrix'] and image['clip_ids']==[0] and pop['clip_ids']==[],'mask callback provenance')
            from fractions import Fraction as F
            from soft_mask_corpus_model import image_samples
            from image_page_model import resolve
            samples=bytearray([255])*(32*32*16*3);event=dict(width=c['w'],height=c['h'],components=3,samples_hex=c['expected_base'].split(b'\n',3)[3].hex())
            for index,color in image_samples(event,tuple(map(F,[24,0,0,-24,4,28])),32,32):samples[index*3:index*3+3]=color
            model=resolve(samples,128,128);w,h,pixels=D.pixels(ev['rust_page']);require((w,h)==(32,32) and pixels==model==ev['model'],'rederived generated pixel model')
        elif row['label']=='nonopaque-strong':require(row['native_differs_from_unmasked'] and ev['native_page']!=ev['native_unmasked'],'strong alpha control')
    model_checks=0
    for row in report['corpus']:
        data=(ROOT.parent/'fixtures'/row['file']).read_bytes();require(sha(data)==row['input_sha256'],'original fixture identity');ev={k:read(v) for k,v in row['evidence'].items()}
        if row['status']=='refused':
            require(ev['rust_stderr'].decode()==row['refusal'],'saved corpus refusal')
            trace=json.loads(ev['native_trace']);require(trace['repaired'] is False and len(trace['pages'])==1,'refusal trace envelope');page=trace['pages'][0];require(page['page']==row['page'] and page['status']==row['native_trace_status'] and page.get('error')==row['native_trace_refusal'],'refusal trace identity/status')
        if 'rust' in ev:require(sha(ev['rust'])==row['rust_ppm_sha256'],'corpus rendered bytes')
        if row['status']=='new_opaque_mask_render':
            trace=json.loads(ev['native_trace']);require(trace['repaired'] is False and len(trace['pages'])==1,'native corpus envelope');page=trace['pages'][0];require(page['page']==row['page'],'native corpus page');w,h,rgb=D.pixels(ev['rust']);nw,nh,_=D.pixels(ev['native4x']);require((w*4,h*4)==(nw,nh),'native corpus dimensions')
            model,_,stats=certify(page,w,h,row['dpi'],source_pdf=data);bounds=stats.pop('_channel_bounds');require(model==ev['model'] and [list(pair) for pair in bounds]==json.loads(ev['bounds']) and stats==row['model_stats'],'independently rederived corpus certificate');require(all(lo<=v<=hi for v,(lo,hi) in zip(rgb,bounds)),'corpus channel bounds');require(stats['opaque_masks']==1 and stats['source_glyph_checks']==21,'source witness coverage');model_checks+=1
    for row in report['masks']:
        data=(ROOT.parent/'fixtures'/row['file']).read_bytes();w,h,alpha=source_alpha(data,[row['mask'],0]);ev={k:read(v) for k,v in row['evidence'].items()};expected=dict(base_object=row['base'],mask_object=row['mask'],width=w,height=h,alpha_hex=alpha.hex());require(len(alpha)==row['samples'] and all(v==255 for v in alpha) and alpha==ev['source_alpha'] and json.loads(ev['rust'])==json.loads(ev['native'])==expected,'actual source/attached/owned mask proof')
    return dict(corpus_attempts=634,preserved_r23=617,new_renders=model_checks,generated_positive=44,generated_refused=12,actual_alpha_samples=63240)

def mutations(report):
    result=[]
    def rejects(label,fn):
        try:fn()
        except (ValueError,KeyError,IndexError,TypeError) as e:result.append(dict(label=label,status='rejected',reason=str(e)));return
        raise ValueError('mutation passed: '+label)
    for label,mutate in [
        ('missing_corpus',lambda r:r['corpus'].pop()),
        ('duplicate_corpus',lambda r:r['corpus'].append(r['corpus'][0])),
        ('missing_generated',lambda r:r['generated'].pop()),
        ('unsupported_success',lambda r:r['generated'][-1].update(accepted=True,rust_refusal=None)),
        ('missing_actual_mask',lambda r:r['masks'].pop()),
        ('fake_counts',lambda r:r['counts'].update(preserved_r23=618)),
        ('changed_baseline_hash',lambda r:next(x for x in r['corpus'] if x['status']=='preserved_r23').update(rust_ppm_sha256='0'*64)),
        ('missing_native_mask',lambda r:r['generated'][0]['evidence'].pop('native_mask')),
        ('fabricated_native_refusal',lambda r:r['generated'][0].update(native_refusal='fabricated')),
        ('missing_preserved_pixels',lambda r:next(x for x in r['corpus'] if x['status']=='preserved_r23')['evidence'].pop('rust')),
        ('missing_refusal_trace',lambda r:next(x for x in r['corpus'] if x['status']=='refused')['evidence'].pop('native_trace')),
        ('removed_owned_provenance',lambda r:r['generated'][0]['evidence'].pop('rust_mask'))]:
        copy_report=copy.deepcopy(report);mutate(copy_report);rejects(label,lambda:shapes(copy_report))
    r=next(r for r in report['corpus'] if r['status']=='new_opaque_mask_render');p=json.loads(read(r['evidence']['native_trace']))['pages'][0];data=(ROOT.parent/'fixtures'/r['file']).read_bytes();index=next(i for i,e in enumerate(p['events']) if e['kind']=='opaque_mask_clip')
    for position in [0,len(p['events'][index]['alpha_hex'])//4,len(p['events'][index]['alpha_hex'])//2-1]:
        q=copy.deepcopy(p);raw=bytearray.fromhex(q['events'][index]['alpha_hex']);raw[position]=254;q['events'][index]['alpha_hex']=raw.hex();rejects('changed_alpha_'+str(position),lambda:validate_mask_callbacks(q,data))
    for label,mutate in [('skipped_alpha_sample',lambda q:q['events'][index].update(alpha_hex=q['events'][index]['alpha_hex'][:-2])),('removed_source_edge',lambda q:q['events'][index].update(mask_object=[999,0])),('missing_clip_callback',lambda q:q['events'].pop(index)),('missing_pop_callback',lambda q:q['events'].pop(index+2)),('changed_mask_transform',lambda q:q['events'][index].update(matrix=[1,0,0,1,0,0]))]:
        q=copy.deepcopy(p);mutate(q);rejects(label,lambda:validate_mask_callbacks(q,data))
    glyph_index=next(i for i,e in enumerate(p['events']) if e['kind']=='glyph');q=copy.deepcopy(p);q['events'][glyph_index]['matrix'][4]+=1;rejects('changed_glyph_placement',lambda:source_glyph_paths(q,data))
    q=copy.deepcopy(p);q['events'][glyph_index]['advance']+=.01;rejects('changed_glyph_advance',lambda:source_glyph_paths(q,data))
    q=copy.deepcopy(p);q['events'][glyph_index]['raw_commands'].pop();rejects('truncated_glyph_outline',lambda:source_glyph_paths(q,data))
    return result

def main():
    p=argparse.ArgumentParser();p.add_argument('--report',type=Path,default=ROOT/'reference/soft-mask-differential-r24.json');p.add_argument('--output',type=Path,required=True);a=p.parse_args();report=json.loads(a.report.read_text());counts=verify(report);faults=mutations(report)
    checks=json.loads((ROOT/'reference/soft-mask-rust-checks-r24.json').read_text())
    verify_checks(checks)
    result=dict(status='passed',scope='R24 automated acceptance evidence; separate independent source review recorded in R24 review document',counts=counts,mutation_rejections=faults,report_sha256=sha(a.report.read_bytes()),checks_sha256=sha((ROOT/'reference/soft-mask-rust-checks-r24.json').read_bytes()),verifier_sha256=sha(Path(__file__).read_bytes()))
    a.output.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(dict(status=result['status'],counts=counts,mutations=len(faults))))
if __name__=='__main__':main()
