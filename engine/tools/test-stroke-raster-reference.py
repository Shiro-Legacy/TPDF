#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""R21 true-stroke interval, native wrapper, canvas and failure-accounting fault gates."""
import argparse,copy,gzip,hashlib,json,runpy,subprocess
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
from fractions import Fraction as F
from stroke_raster_cases import generated,stroke,case
from stroke_raster_model import render
from stroke_raster_reference import parse_ppm,geometry,render_native
import stroke_source_color as source_color
ENGINE=Path(__file__).resolve().parent.parent
D=runpy.run_path(str(ENGINE/'tools/differential-stroke-raster.py'))

def check_native_result(run,expected_exit):
    if run.returncode!=expected_exit:raise ValueError(f'unexpected native wrapper exit{run.returncode}, wanted{expected_exit}')
    if expected_exit==0:
        if run.stderr:raise ValueError('native successful diagnostic')
        parse_ppm(run.stdout)
    elif run.stdout:raise ValueError('native failure published partial output')
    elif expected_exit==2:
        prefix=b'stroke raster oracle: '
        if prefix not in run.stderr or not run.stderr.split(prefix,1)[1].strip():raise ValueError('native expected failure missing diagnostic')

def verify_evidence_hashes(report):
    for path,want in report['evidence_sha256'].items():
        if hashlib.sha256(Path(path).read_bytes()).hexdigest()!=want:raise ValueError('reference evidence digest differs')

def main():
    ap=argparse.ArgumentParser();ap.add_argument('--rust',type=Path,required=True);ap.add_argument('--native',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-stroke-raster-oracle'));ap.add_argument('--trace',type=Path,default=Path('/private/tmp/tandem-mupdf-reference/mupdf-page-outlines-oracle'));ap.add_argument('--report',type=Path,required=True);a=ap.parse_args();mutations=[];positives=[];wrappers=[]
    def reject(label,call):
        try:call()
        except (ValueError,KeyError,IndexError,TypeError):mutations.append(label)
        else:raise AssertionError(label+' unexpectedly passed')
    base=next(generated());expected,band,counts=render(base['events'],8,8);bounds=counts.pop('_channel_bounds');D['compare_model'](expected,expected,8,8,band,bounds);positives.append('exact orthogonal pixel/model intervals')
    for label,pixels,w,h,bb in [('missing stroke',bytes([255])*len(expected),8,8,bounds),('truncated pixels',expected[:-1],8,8,bounds),('extra pixels',expected+b'x',8,8,bounds),('wrong canvas',expected,4,16,bounds[:-1]),('wrong color',bytes([v^255 for v in expected]),8,8,bounds),('missing interval',expected,8,8,bounds[:-1])]:reject(label,lambda:D['compare_model'](pixels,expected,w,h,band,bb))
    curve=next(v for v in generated() if v['scope']=='certified-true-curves');expected,band,counts=render(curve['events'],8,8);bounds=counts.pop('_channel_bounds')
    # The midpoint of every independently derived channel range is admissible.
    mid=bytes((lo+hi)//2 for lo,hi in bounds);D['compare_model'](mid,expected,8,8,band,bounds);positives.append('uncertain curve sample intervals preserve admissible colors')
    reject('thin curve deletion despite whole-pixel uncertainty',lambda:D['compare_model'](bytes([255])*len(expected),expected,8,8,band,bounds))
    selected=next(i for i,(lo,hi) in enumerate(bounds) if hi<255);bad=bytearray(mid);bad[selected]=bounds[selected][1]+1;reject('upper channel certificate exceeded',lambda:D['compare_model'](bad,expected,8,8,band,bounds))
    selected=next(i for i,(lo,hi) in enumerate(bounds) if lo>0);bad=bytearray(mid);bad[selected]=bounds[selected][0]-1;reject('lower channel certificate exceeded',lambda:D['compare_model'](bad,expected,8,8,band,bounds))
    # Real generated suffixes must remain atomic after a valid first stroke.
    from page_raster_cases import plain_pdf
    strict=[]
    for label,raw in [('dash suffix',b'1 w 1 1 m 7 1 l S [2 1] 0 d 1 3 m 7 3 l S'),('hairline suffix',b'1 w 1 1 m 7 1 l S 0 w 1 3 m 7 3 l S'),('CMYK suffix',b'1 w 1 1 m 7 1 l S 0 0 0 1 K 1 3 m 7 3 l S')]:
        data=plain_pdf(raw,8,8);r=subprocess.run([str(a.rust),'--page-render=1@72','-'],input=data,capture_output=True,timeout=30)
        assert r.returncode==2 and not r.stdout and r.stderr.startswith(b'pdf-inspect: '),(label,r.returncode,r.stdout[:10],r.stderr);strict.append(label)
    def native(label,data,args=['8','8','0'],valid=False,expected_exit=2):
        r=subprocess.run([str(a.native),*args],input=data,capture_output=True,timeout=50)
        check_native_result(r,0 if valid else expected_exit)
        wrappers.append(label);return r.stdout
    native('solid line',b'1 w 1 2 m 7 2 l S',valid=True)
    native('solid cubic caps joins full affine',b'q 1 .25 .5 1 1 0 cm .5 w 1 J 2 j 1 1 m 3 2 4 2 5 1 c S Q',valid=True)
    native('clip plus stroke',b'q 1 1 6 6 re W n 2 w 0 4 m 8 4 l S Q',valid=True)
    native('AA8 stroke',b'1 w 1 2 m 7 2 l S',['8','8','8'],True)
    for label,data in [('hairline',b'0 w'),('negative width',b'-1 w'),('invalid cap',b'3 J'),('fractional cap',b'.5 J'),('invalid join',b'3 j'),('low miter',b'.5 M'),('dash forbidden',b'[1 2] 0 d'),('CMYK forbidden',b'0 0 0 1 K'),('text forbidden',b'BT ET'),('resource forbidden',b'/X Do'),('singular matrix',b'0 0 0 1 0 0 cm'),('accumulated matrix bound',b'1000000 0 0 1000000 0 0 cm 2 0 0 2 0 0 cm'),('invalid stroke operands',b'1 S'),('unfinished geometry',b'1 1 m'),('unfinished operand',b'1'),('unbalanced save',b'q'),('restore underflow',b'Q'),('invalid clip',b'W n'),('depth bound',b'q '*129),('input bound',b' '*(4*1024*1024+1)),('token bound',b'1 w '*250001),('partial valid stroke suffix',b'1 w 1 2 m 7 2 l S /X Do')]:native(label,data,expected_exit=1 if label=='input bound' else 2)
    for label,args in [('zero extent',['0','8','0']),('oversize extent',['4097','8','0']),('AA suffix',['8','8','0x']),('invalid AA',['8','8','4'])]:native(label,b'',args,expected_exit=1)
    for label,code in [('native negative-case SIGSEGV',-11),('native negative-case unexpected exit',3)]:
        reject(label,lambda:check_native_result(subprocess.CompletedProcess([],code,b'',b'stroke raster oracle: invalid'),2))
    reject('native negative-case missing diagnostic',lambda:check_native_result(subprocess.CompletedProcess([],2,b'',b''),2))
    page=dict(initial_clip=[0,0,8,8]);D['dimensions'](8,8,page,72);D['dimensions'](2,2,page,18);positives.append('independent DPI canvas extents')
    reject('cropped canvas',lambda:D['dimensions'](7,8,page,72));reject('expanded canvas',lambda:D['dimensions'](9,8,page,72));reject('wrong DPI canvas',lambda:D['dimensions'](8,8,page,18))
    baseline={('baseline.pdf',1)};rows=[dict(file='large.pdf',page=i,dpi=18,status='native_diagnostic_matched') for i in range(1,501)]+[dict(file='large.pdf',page=i,dpi=72,status='native_diagnostic_matched') for i in (1,250,500)]+[dict(file='baseline.pdf',page=1,dpi=72,status='native_diagnostic_matched')];D['verify_baseline'](rows,baseline);positives.append('complete required stress and previous coverage')
    for label,index in [('missing stress page',0),('missing stress72',500),('missing old72',503)]:
        bad=copy.deepcopy(rows);bad.pop(index);reject(label,lambda:D['verify_baseline'](bad,baseline))
    args=SimpleNamespace(rust=a.rust,trace=a.trace,native=a.native,evidence_dir=None)
    for label,code,out,err in [('native signal',-9,b'',b''),('native unexpectedexit',3,b'',b'error'),('native empty diagnostic',2,b'',b''),('native partial output',2,b'partial',b'page-outline oracle: failure'),('native successful diagnostic',0,b'{}',b'warning')]:
        with patch.dict(D['D'],{'rust_render':lambda *args:(None,'pdf-inspect: unsupported')}),patch('subprocess.run',return_value=subprocess.CompletedProcess([],code,out,err)):
            row=D['corpus_page']((args,'fault.pdf',1,18,b'',False));assert row['status']=='failure',(label,row);mutations.append('Rust refusal cannot hide '+label)
    for label,page in [('native unexpected page status',dict(page=1,status='mystery')),('native empty refusal',dict(page=1,status='refused',error='')),('native nonstring refusal',dict(page=1,status='refused',error=42))]:
        raw=json.dumps(dict(pages=[page],repaired=False)).encode()
        with patch.dict(D['D'],{'rust_render':lambda *args:(None,'pdf-inspect: unsupported')}),patch('subprocess.run',return_value=subprocess.CompletedProcess([],0,raw,b'')):
            row=D['corpus_page']((args,'fault.pdf',1,18,b'',False));assert row['status']=='failure',(label,row);mutations.append('Rust refusal cannot hide '+label)
    # Mutate real PDF semantics, render them, and compare against the original
    # independent certificate; this exercises dimensions beyond byte corruption.
    selected=[v for v in generated() if v['label'] in ('exact-0','join-8','join-9','ordered-0')]
    original=selected[0]
    semantic=[('actual changed width',original,lambda e:e[0]['style'].update(width=2)),('actual changed cap',original,lambda e:e[0]['style'].update(cap=2)),('actual incorrect affine width',original,lambda e:e[0].update(matrix=(1,0,0,2,0,-3))),('actual changed paint color',original,lambda e:e[0].update(color=[255,0,0]))]
    joincase=next(v for v in selected if v['label']=='join-8')
    semantic.append(('actual changed join',joincase,lambda e:e[0]['style'].update(join=2)))
    closed=next(v for v in selected if v['label']=='join-9')
    semantic.append(('actual removed closed seam',closed,lambda e:e[0]['path'].pop()))
    ordered=next(v for v in selected if v['label']=='ordered-0')
    semantic.append(('actual deleted clip',ordered,lambda e:e.__setitem__(slice(None),[v for v in e if v['kind'] not in ('clip','pop')])))
    for label,original,mutate in semantic:
        wanted,ambiguous,c=render(original['events'],8,8);certificate=c.pop('_channel_bounds');events=copy.deepcopy(original['events']);mutate(events);altered=case('mutation','mutation',events)
        raw,error=D['D']['rust_render'](a.rust,altered['pdf'])
        if error:raise AssertionError('semantic mutation unexpectedly refused: '+label+' '+error)
        width,height,pixels=parse_ppm(raw);reject(label,lambda:D['compare_model'](pixels,wanted,width,height,ambiguous,certificate))
    # Native user-space width differs from an incorrectly isotropic width after
    # pretransforming the centerline under a nonuniform matrix.
    correct=native('native full affine width',b'q 2 0 0 .5 0 0 cm 2 w 1 8 m 3 8 l S Q',valid=True)
    wrong=native('native isotropic comparison',b'2 w 2 4 m 6 4 l S',valid=True)
    assert correct!=wrong;positives.append('native full affine stroke cannot collapse to isotropic width')
    source=(ENGINE.parent/'fixtures/large.pdf').read_bytes();raw_content=source_color.contents(source)[0];source_color.parse_content(raw_content)
    source_page,_=D['D']['native_trace'](a.trace,source);mask,proof=source_color.certificate(source,source_page,153,198,18)
    at=next(i for i,value in enumerate(mask) if value);outside=next(i for i,value in enumerate(mask) if not value);target,native_color=source_color.color_pair()
    actual=bytearray(bytes(target)*len(mask));reference=bytearray(bytes(native_color)*len(mask));band=[True]*len(mask);band[at]=False
    original=source_color.compare(actual,reference,153,198,band,mask);assert original['unexplained_outside_boundary']==0 and original['source_color_policy_verified_pixels']==sum(mask);positives.append('structural exact source-color pair and visible interior')
    def checked_color(aa,nn,bb=band,mm=mask):
        value=source_color.compare(aa,nn,153,198,bb,mm)
        if value['unexplained_outside_boundary']:raise ValueError('unexplained or invalid source-color proof')
    for label,which,channel,value in [('wrong Rust color pair','rust',0,229),('wrong native color pair','native',0,230),('wrong green channel','rust',1,103),('wrong blue channel','native',2,52)]:
        aa=bytearray(actual);nn=bytearray(reference);(aa if which=='rust' else nn)[3*at+channel]=value;reject(label,lambda:checked_color(aa,nn))
    aa=bytearray(actual);nn=bytearray(reference);aa[3*at:3*at+3]=bytes(native_color);nn[3*at:3*at+3]=bytes(native_color);reject('colluding equality cannot bypass source color proof',lambda:checked_color(aa,nn))
    bb=band[:];bb[outside]=False;reject('same pair outside certified source interior',lambda:checked_color(actual,reference,bb))
    for label,before,after in [('source color changed',b'0.9 0.4 0.2 rg',b'0.8 0.4 0.2 rg'),('source rectangle changed',b'200 160 160 36 re',b'201 160 160 36 re'),('source hidden clip',b'BT /F1 14',b'q 0 0 1 1 re W n BT /F1 14'),('source hidden matrix',b'BT /F1 14',b'2 0 0 2 0 0 cm BT /F1 14'),('source hidden graphics state',b'BT /F1 14',b'/GS gs BT /F1 14'),('source text clip rendering mode',b'BT /F1 14',b'BT 7 Tr /F1 14'),('source later paint',b'540 60 c S',b'540 60 c S 0 g 200 160 160 36 re f')]:
        changed=raw_content.replace(before,after);assert changed!=raw_content;reject(label,lambda:source_color.parse_content(changed))
    lines=raw_content.splitlines();lines[-2],lines[-1]=lines[-1],lines[-2];reject('source paint order changed',lambda:source_color.parse_content(b'\n'.join(lines)+b'\n'))
    reject('source fixture digest tamper',lambda:source_color.contents(source+b' '))
    for label,mutate in [('native source clip changed',lambda p:next(e for e in p['events'] if e['kind']=='fill').update(clip_ids=[0])),('native rectangle changed',lambda p:next(e for e in p['events'] if e['kind']=='fill')['path'][0].__setitem__(1,201)),('native later curve changed',lambda p:p['events'][-1]['path'][1].__setitem__(1,201)),('native geometry changed',lambda p:p['initial_ctm'].__setitem__(0,2))]:
        bad=copy.deepcopy(source_page);mutate(bad);reject(label,lambda:source_color.certificate(source,bad,153,198,18))
    baseline_path=ENGINE/'reference/page-outlines-raster-b0.json';baseline_hash=hashlib.sha256(baseline_path.read_bytes()).hexdigest();evidence=dict(evidence_sha256={str(baseline_path):baseline_hash});verify_evidence_hashes(evidence);evidence['evidence_sha256'][str(baseline_path)]='0'*64;reject('baseline/report digest tamper',lambda:verify_evidence_hashes(evidence))
    # Exact, independent target-vs-native conversion fixture proves the source
    # rule directly, rather than obtaining the pair from stress mismatches.
    from page_raster_cases import plain_pdf
    color_pdf=plain_pdf(b'.9 .4 .2 rg 0 0 8 8 re f',8,8);rr,err=D['D']['rust_render'](a.rust,color_pdf);assert err is None;_,_,rgb=parse_ppm(rr);assert rgb==bytes(target)*64
    nr=native('source .9 truncation direct rectangle',b'.9 .4 .2 rg 0 0 8 8 re f',valid=True);_,_,rgb=parse_ppm(nr);assert rgb==bytes(native_color)*64;positives.append('direct source decimal round-versus-truncate rule')
    files=[Path(__file__),a.rust,a.native,a.trace]+[ENGINE/'tools'/name for name in ['stroke_source_color.py','differential-stroke-raster.py','stroke_raster_reference.py','stroke_raster_model.py','stroke_review_model.py','stroke_raster_cases.py','mupdf-stroke-raster-oracle.c','build-stroke-raster-reference.sh']]+[ENGINE/'reference'/name for name in ['stroke-color-policy-s1.json','stroke-review-model-selfcheck.json','stroke-raster-policy-s0.json','stroke-raster-policy-notes-s0.md','page-outlines-raster-b0.json']]
    report=dict(status='passed',positive_checks=len(positives),positive=positives,rejected_mutations=len(mutations),mutations=mutations,native_wrapper_checks=len(wrappers),wrappers=wrappers,rust_atomic_suffix_checks=len(strict),rust_suffixes=strict,evidence_sha256={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in files});a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items() if not isinstance(v,(dict,list))}))
if __name__=='__main__':main()
