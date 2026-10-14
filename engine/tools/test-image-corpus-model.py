#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Fault-sensitive native trace/source-model checks, separate from Rust rendering."""
import argparse,copy,hashlib,json,subprocess
from pathlib import Path
from image_corpus_model import certify,mixed_cases
from image_page_cases import generated,doc
from image_page_model import render
from page_raster_reference import parse_ppm
ENGINE=Path(__file__).resolve().parent.parent

def sha(data):return hashlib.sha256(data).hexdigest()
def main():
    parser=argparse.ArgumentParser();parser.add_argument('--trace',type=Path,required=True);parser.add_argument('--rust',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);args=parser.parse_args();rows=[]
    files=[Path(__file__),args.trace,args.rust]+[ENGINE/'tools'/name for name in ['image_corpus_model.py','mupdf-image-trace-oracle.c','build-image-trace-reference.sh','page_raster_review_model.py','stroke_review_model.py','image_page_model.py','image_page_cases.py','image_decode_cases.py','page_raster_reference.py','differential-lzw.py']]
    before={str(path.resolve()):sha(path.read_bytes()) for path in files}
    def native(data):
        process=subprocess.run([str(args.trace),'1'],input=data,capture_output=True,timeout=50)
        if process.returncode or process.stderr:raise ValueError('native trace failed: '+process.stderr.decode(errors='replace'))
        return json.loads(process.stdout)['pages'][0]
    def rejected(label,page,data):
        try:certify(page,16,16,72,source_pdf=data)
        except (ValueError,KeyError,IndexError,ArithmeticError) as error:rows.append(dict(label=label,status='rejected',reason=str(error)));return
        raise ValueError('model mutation was accepted: '+label)
    # A separately authored source-event model predates this native-trace model.
    for case in generated():
        page=native(case['pdf']);w,h=case.get('width',16),case.get('height',16)
        actual,band,stats=certify(page,w,h,72,source_pdf=case['pdf']);expected=render(case['events'],w,h)[1]
        if actual!=expected or any(band):raise ValueError('independent image model disagreement: '+case['label'])
        bounds=stats.pop('_channel_bounds')
        if any(lo!=hi for lo,hi in bounds):raise ValueError('image-only certificate unexpectedly uncertain')
        rows.append(dict(label=case['label'],status='exact_independent_match',input_sha256=sha(case['pdf']),model_pixel_sha256=sha(actual),images=stats['images']))
    cases=list(mixed_cases());case=cases[0];page=native(case['pdf']);baseline,band,stats=certify(page,16,16,72,source_pdf=case['pdf'])
    for mixed in cases:
        trace=native(mixed['pdf']);expected,band,stats=certify(trace,16,16,72,source_pdf=mixed['pdf']);bounds=stats['_channel_bounds']
        result=subprocess.run([str(args.rust),'--page-render=1@72','-'],input=mixed['pdf'],capture_output=True,timeout=60)
        if result.returncode or result.stderr:raise ValueError('mixed Rust render failed')
        w,h,pixels=parse_ppm(result.stdout)
        violations=sum(not lo<=v<=hi for v,(lo,hi) in zip(pixels,bounds))
        if (w,h)!=(16,16) or violations:raise ValueError('mixed pixel certificate failed')
        rows.append(dict(label=mixed['label'],status='mixed_pixels_certified',input_sha256=sha(mixed['pdf']),rust_ppm_sha256=sha(result.stdout),images=stats['images'],channel_interval_violations=violations))
    images=[i for i,e in enumerate(page['events']) if e['kind']=='image'];index=images[0]
    changed=copy.deepcopy(page);changed['events'][index]['matrix'][4]+=1;rejected('changed_native_image_placement',changed,case['pdf'])
    changed=copy.deepcopy(page);changed['events'][index]['samples_hex']=changed['events'][index]['samples_hex'][:-2];rejected('truncated_native_samples',changed,case['pdf'])
    changed=copy.deepcopy(page);changed['events'][index]['width']+=1;rejected('wrong_native_sample_shape',changed,case['pdf'])
    changed=copy.deepcopy(page);changed['events'][index]['operation_index']=99999;rejected('wrong_image_source_origin',changed,case['pdf'])
    changed=copy.deepcopy(page);changed['events'].pop(index);rejected('missing_image_callback',changed,case['pdf'])
    changed=copy.deepcopy(page);changed['events'][index]['clip_ids']=[999];rejected('changed_image_clip_ancestry',changed,case['pdf'])
    changed=copy.deepcopy(page);origin=changed['events'][index]['operation_index'];changed['operation_origins'][origin]['span'][1]-=1;rejected('truncated_original_do_span',changed,case['pdf'])
    changed=copy.deepcopy(page);source=bytes.fromhex(changed['content_hex']);changed['content_hex']=source.replace(b'8 0 0 8 4 4 cm',b'9 0 0 8 4 4 cm').hex();rejected('changed_source_cm',changed,case['pdf'])
    changed=copy.deepcopy(page);source=bytes.fromhex(changed['content_hex']);changed['content_hex']=source.replace(b'0 0 0 RG',b'1 0 0 RG').hex();rejected('changed_source_paint_color',changed,case['pdf'])
    rejected('changed_original_page_geometry',page,case['pdf'].replace(b'/MediaBox [0 0 16 16]',b'/MediaBox [0 0 15 16]'))
    # Native samples are independent decoded evidence; corrupting them must change
    # exact output constraints rather than being absorbed into a generic image band.
    changed=copy.deepcopy(page);raw=bytearray.fromhex(changed['events'][index]['samples_hex']);raw[:3]=bytes([7,11,19]);changed['events'][index]['samples_hex']=raw.hex();altered,_,altered_stats=certify(changed,16,16,72,source_pdf=case['pdf'])
    difference=sum(a!=b for a,b in zip(altered,baseline));bounds=altered_stats['_channel_bounds'];violations=sum(not lo<=v<=hi for v,(lo,hi) in zip(baseline,bounds))
    if not difference or not violations:raise ValueError('image sample mutation failed sensitivity check')
    rows.append(dict(label='changed_native_sample_colors',status='different_certified_pixels',different_channels=difference,baseline_channel_violations=violations))
    for label,data in [('soft_mask',doc('q 8 0 0 8 4 4 cm /Im Do Q',imeta='/SMask 5 0 R')),('interpolation',doc('q 8 0 0 8 4 4 cm /Im Do Q',imeta='/Interpolate true')),('late_unsupported_operator',doc('q 8 0 0 8 4 4 cm /Im Do Q /Bad sh'))]:
        refused=native(data)
        if refused['status']!='refused' or any(k in refused for k in ('events','content_hex','form_invocations')):raise ValueError('native refused page exposed trace prefix')
        rows.append(dict(label=label,status='native_atomic_refusal',reason=refused['error']))
    after={str(path.resolve()):sha(path.read_bytes()) for path in files}
    if before!=after:raise ValueError('model validation evidence changed while checks ran')
    report=dict(status='passed',scope='source/native trace mutation sensitivity and independent generated image/mixed pixel models; not full corpus acceptance',rows=rows,counts={status:sum(r['status']==status for r in rows) for status in sorted({r['status'] for r in rows})},evidence_sha256=before)
    args.output.write_text(json.dumps(report,indent=2)+'\n');print(report['counts'])
if __name__=='__main__':main()
