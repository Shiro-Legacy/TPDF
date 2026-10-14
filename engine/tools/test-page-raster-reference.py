#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""R19 independent model identities, pixel comparator mutations, native fail-closed checks."""
import argparse,copy,hashlib,json,runpy,subprocess
from fractions import Fraction as F
from pathlib import Path
from unittest.mock import patch
from page_raster_cases import box,fill,clip,path_pdf,curves
from page_raster_reference import parse_ppm,box4
from page_raster_review_model import render
D=runpy.run_path(str(Path(__file__).with_name('differential-page-raster.py')))

def main():
    p=argparse.ArgumentParser();p.add_argument('--rust',required=True);p.add_argument('--native',default='/private/tmp/tandem-mupdf-reference/mupdf-page-raster-oracle');p.add_argument('--report',type=Path,required=True);a=p.parse_args();models=[];mutations=[];wrappers=[];strict=[]
    def model(label,events,w,h,wanted,bounds=None):
        pixels,uncertain,_=render(events,w,h,bounds,snap_lines=not any(c[0] in ('q','c') for e in events if e['kind']!='pop' for c in e['path']))
        assert pixels==wanted and not any(uncertain),(label,pixels,uncertain);models.append(label)
    square=box(0,0,1,1);triangle=[['m',0,0],['l',1,0],['l',0,1],['h']];reverse=[['m',0,0],['l',0,1],['l',1,0],['h']]
    model('full pixel',[fill(square)],1,1,bytes([0]*3))
    model('half-open sample boundary triangle',[fill(triangle)],1,1,bytes([159]*3))
    model('opposite triangle orientation',[fill(reverse)],1,1,bytes([159]*3))
    model('coincident nonzero doubles',[fill(triangle+triangle)],1,1,bytes([159]*3))
    model('coincident evenodd cancels',[fill(triangle+triangle,rule='evenodd')],1,1,bytes([255]*3))
    model('opposing winding cancels',[fill(triangle+reverse)],1,1,bytes([255]*3))
    half=box(0,0,F(1,2),1);model('repeat clip idempotence',[clip(half),clip(half),fill(square)],1,1,bytes([128]*3))
    model('original unsnapped page bound',[fill(square)],1,1,bytes([64]*3),[F(129,1024),0,1,1])
    model('initial right boundary exclusive',[fill(square)],1,1,bytes([255]*3),[0,0,F(1,8),1])
    model('opaque event order',[fill(square,[255,0,0]),fill(square,[0,0,255])],1,1,bytes([0,0,255]))
    model('resolve after overlap',[fill(square,[255,0,0]),fill(half,[0,0,255])],1,1,bytes([128,0,128]))
    quad=[['m',0,0],['q',F(1,2),1,1,0],['h']];model('analytical quadratic six samples',[fill(quad)],1,1,bytes([159]*3))
    uncertain_curve=[['m',1,6],['c',1,0,7,0,7,6],['h']]
    model('later opaque fill hides curve uncertainty',[fill(uncertain_curve),fill(box(0,0,8,8),[255,0,0])],8,8,bytes([255,0,0])*64)
    # Byte-level comparator never ignores a changed certified interior, exterior, or channel.
    wanted=bytes([0,0,0,255,0,0,0,255,0,255,255,255]);uncertain=[False]*4
    D['compare_pixels'](wanted,wanted,2,2,uncertain);positive=1
    for label,changed in [('missing pixels',wanted[:-3]),('extra pixels',wanted+b'\0\0\0'),('reordered pixels',wanted[3:6]+wanted[:3]+wanted[6:]),('missing shape',bytes([255]*12)),('wrong background',bytes([0]*12)),('changed color',bytes([1])+wanted[1:]),('wrong dimensions',wanted)]:
        try:D['compare_pixels'](changed,wanted,3 if label=='wrong dimensions' else 2,2,uncertain)
        except (ValueError,IndexError):mutations.append(label)
        else:raise AssertionError('comparator accepted '+label)
    curve_pixels,band,_=render([fill(uncertain_curve)],8,8)
    safe=next(i for i,v in enumerate(band) if not v);inside=next(i for i,v in enumerate(band) if v)
    changed=bytearray(curve_pixels);changed[3*safe]^=255
    try:D['compare_pixels'](bytes(changed),curve_pixels,8,8,band)
    except ValueError:mutations.append('curve certified pixel changed')
    else:raise AssertionError('certified curve pixel ignored')
    changed=bytearray(curve_pixels);changed[3*inside]^=255;D['compare_pixels'](bytes(changed),curve_pixels,8,8,band);positive+=1
    baseline=[fill(box(0,0,4,4),[255,0,0]),clip(box(1,1,3,3)),fill(box(0,0,4,4),[0,0,255]),dict(kind='pop')];wanted,_,_=render(baseline,4,4, snap_lines=True)
    altered=[('paint order',[fill(box(0,0,4,4),[0,0,255]),fill(box(0,0,4,4),[255,0,0])]),('clip deleted',[baseline[0],baseline[2]]),('fill deleted',[baseline[0]]),('wrong winding rule',[fill(square+square,[0,0,0],rule='evenodd')])]
    for label,events in altered:
        got,_,_=render(events,4,4,snap_lines=True)
        try:D['compare_pixels'](got,wanted,4,4)
        except ValueError:mutations.append(label)
        else:raise AssertionError('semantic mutation undetected '+label)
    for label,code,diagnostic in [('signal',-9,b'pdf-inspect: killed'),('unexpected exit1',1,b'pdf-inspect: bad'),('unexpected exit3',3,b'pdf-inspect: bad'),('empty diagnostic',2,b''),('wrong diagnostic prefix',2,b'crashed'),('empty prefixed diagnostic',2,b'pdf-inspect: \n')]:
        fake=subprocess.CompletedProcess([],code,b'',diagnostic)
        with patch('subprocess.run',return_value=fake):
            try:D['rust_render']('unused',b'')
            except ValueError:mutations.append(label)
            else:raise AssertionError('unexpected exit classified as refusal: '+label)
    with patch('subprocess.run',return_value=subprocess.CompletedProcess([],2,b'',b'pdf-inspect: unsupported')):
        assert D['rust_render']('unused',b'')[0] is None;positive+=1
    expected_baseline={('fixture.pdf',1)}
    D['verify_baseline']([dict(file='fixture.pdf',page=1,status='native_boundary_diagnostic')],expected_baseline);positive+=1
    for label,rows in [('missing eligible page',[]),('eligible page silently refused',[dict(file='fixture.pdf',page=1,status='refused')])]:
        try:D['verify_baseline'](rows,expected_baseline)
        except ValueError:mutations.append(label)
        else:raise AssertionError(label)
    # Exact resolver and P6 extent checks.
    assert box4(bytes([0,0,0]*8+[255,255,255]*8),1,1)==bytes([128]*3);models.append('integer box resolve tie')
    for label,data in [('truncated PPM',b'P6\n1 1\n255\n\0\0'),('PPM extra payload',b'P6\n1 1\n255\n\0\0\0x'),('PPM channel max',b'P6\n1 1\n256\n\0\0\0')]:
        try:parse_ppm(data)
        except ValueError:mutations.append(label)
        else:raise AssertionError(label)
    def native_case(label,data,args=['2','2','0'],fail=False):
        r=subprocess.run([a.native,*args],input=data,capture_output=True,timeout=50)
        if fail:assert r.returncode!=0 and not r.stdout,(label,r.returncode,r.stdout[:20])
        else:assert r.returncode==0 and not r.stderr,(label,r.stderr);parse_ppm(r.stdout)
        wrappers.append(label)
    native_case('valid filled geometry',b'0 0 2 2 re f')
    native_case('valid AA8 geometry',b'0 0 2 2 re f',['2','2','8'])
    native_case('empty white result',b'')
    for label,data in [('unsupported stroke suffix',b'0 0 2 2 re f 0 0 1 1 re S'),('CMYK suffix',b'0 0 1 1 re f 0 0 0 1 k'),('text forbidden',b'BT ET'),('resource forbidden',b'/X Do'),('operand count',b'0 0 1 re f'),('unfinished path',b'0 0 m'),('unfinished operand',b'1'),('unbalanced save',b'q'),('restore underflow',b'Q'),('invalid clip',b'W n'),('negative color',b'-1 g'),('large color',b'2 0 0 rg'),('array forbidden',b'[]'),('string forbidden',b'(x)'),('large coordinate',b'1000001 0 m 0 0 l f'),('stack depth',b'q '*129),('token bound',b'0 g '*250001),('input bound',b' '*(4*1024*1024+1))]:native_case(label,data,fail=True)
    for label,args in [('zero dimension',['0','2','0']),('dimension bound',['4097','2','0']),('invalid AA',['2','2','4']),('AA suffix',['2','2','8x'])]:native_case(label,b'',args,True)
    for label,raw in [('stroke suffix',b'0 0 1 1 re f 0 0 1 1 re S'),('CMYK paint suffix',b'0 0 1 1 re f 0 0 0 1 k 0 0 1 1 re f')]:
        from page_raster_cases import plain_pdf
        data=plain_pdf(raw,2,2);r=subprocess.run([a.rust,'--page-render=1@72','-'],input=data,capture_output=True,timeout=20);assert r.returncode!=0 and not r.stdout,(label,r.stdout);strict.append(label)
    report=dict(status='passed',independent_model_checks=len(models),model_checks=models,positive_comparator_checks=positive,rejected_comparator_mutations=len(mutations),mutations=mutations,native_wrapper_checks=len(wrappers),wrapper_checks=wrappers,rust_atomic_refusal_checks=len(strict),rust_refusals=strict,evidence_sha256={str(v):hashlib.sha256(Path(v).read_bytes()).hexdigest() for v in [a.rust,a.native,__file__,Path(__file__).with_name('differential-page-raster.py'),Path(__file__).with_name('page_raster_review_model.py'),Path(__file__).with_name('page_raster_reference.py')]})
    a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items() if not isinstance(v,(dict,list))}))
if __name__=='__main__':main()
