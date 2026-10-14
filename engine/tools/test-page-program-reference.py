#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Bounded original-page oracle self checks, independent of Rust acceptance."""
import argparse
import copy
import importlib.util
from pathlib import Path
import json
import subprocess


def pdf(contents, page_extra=b'', unused=b''):
    if isinstance(contents, bytes):
        contents = [contents]
    font = b'<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>'
    objects = [b'<< /Type /Catalog /Pages 2 0 R >>', b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
               b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] '+page_extra+b' /Resources << /Font << /F0 4 0 R '+unused+b' >> >> /Contents ['+
               b' '.join(str(i+5).encode()+b' 0 R' for i in range(len(contents)))+b'] >>', font]
    objects += [b'<< /Length '+str(len(c)).encode()+b' >>\nstream\n'+c+b'\nendstream' for c in contents]
    data = b'%PDF-1.7\n'; offsets = []
    for i, obj in enumerate(objects, 1):
        offsets.append(len(data)); data += str(i).encode()+b' 0 obj\n'+obj+b'\nendobj\n'
    xref = len(data); count = len(objects)+1
    data += b'xref\n0 '+str(count).encode()+b'\n0000000000 65535 f \n'+b''.join(f'{o:010d} 00000 n \n'.encode() for o in offsets)
    return data+b'trailer\n<< /Size '+str(count).encode()+b' /Root 1 0 R >>\nstartxref\n'+str(xref).encode()+b'\n%%EOF\n'


def main():
    parser = argparse.ArgumentParser(); parser.add_argument('oracle'); args = parser.parse_args()
    checks = 0
    def invoke(data):
        return subprocess.run([args.oracle, 'all'], input=data, capture_output=True, timeout=50)
    def accepted(content, **kwargs):
        nonlocal checks
        run = invoke(pdf(content, **kwargs))
        assert run.returncode == 0 and not run.stderr, (run.returncode, run.stderr)
        result = json.loads(run.stdout)['pages'][0]
        assert result['status'] == 'accepted', result
        checks += 1
        return result
    def refused(content, **kwargs):
        nonlocal checks
        run = invoke(pdf(content, **kwargs))
        assert run.returncode == 0 and not run.stderr, (run.returncode, run.stderr)
        result = json.loads(run.stdout)['pages'][0]
        assert result['status'] == 'refused' and 'events' not in result, result
        checks += 1
    t = accepted(b'q 0 0 30 40 re W B BT /F0 10 Tf (AB) Tj 1 0 0 rg (C) Tj ET Q 0 0 2 3 re S')
    assert t['initial_ctm'] == [1,0,0,-1,0,300] and t['initial_clip'] == [0,0,200,300]
    assert [e[0] if isinstance(e,list) else e['kind'] for e in t['events']] == ['fill','stroke','clip','code','code','code','pop','stroke']
    assert t['contexts'][0]['clip_ids'] == [2] and t['contexts'][1]['color']['values'] == [1,0,0]
    t = accepted([b'q BT /F0 10 Tf (A', b'B) Tj ET Q'])
    assert [e[1] for e in t['events']] == [65,32,66]
    t = accepted([b'BT /F0 10 Tf % comment', b'(A) Tj\n(B) Tj ET'])
    assert [e[1] for e in t['events']] == [66]
    t = accepted(b'0 0 30 40 re W n BT /F0 10 Tf 3 Tr (A) Tj ET')
    assert t['cleanup_pops'] == 1 and t['contexts'][0]['color'] is None
    t = accepted(b'BT /F0 10 Tf (A) Tj ET', page_extra=b'/CropBox [10 20 190 280] /Rotate 90 /UserUnit 2')
    assert t['initial_ctm'] == [0,2,2,0,-40,-20] and t['initial_clip'] == [0,0,520,360], t
    accepted(b'BT /F0 10 Tf (A) Tj ET', unused=b'/Unused << /Subtype /Type0 >>')
    accepted(b'')
    for content in [b'BT /Missing 10 Tf ET', b'BT /F0 10 Tf (A) Tj ET /X Do', b'q', b'BT /F0 10 Tf q Q ET', b'0 0 m BT ET', b'BT 0 0 m ET']:
        refused(content)
    refused(b'', page_extra=b'/Group << /S /Transparency >>')
    for data in [b' '*(16*1024*1024+1), pdf(b'').split(b'xref')[0]]:
        run = invoke(data); assert run.returncode and not run.stdout; checks += 1
    # Fault injection proves the corpus comparator cannot pass partial/count-only traces.
    location = Path(__file__).with_name('differential-page-program.py')
    spec = importlib.util.spec_from_file_location('page_comparison', location)
    comparison = importlib.util.module_from_spec(spec); spec.loader.exec_module(comparison)
    expected = [dict(kind='code', code=65, unicode=[65], matrix=[10,0,0,-10,0,300],
                     advance=.667, mode=0, clip_ids=[0], color={'space':'gray','values':[0]}),
                dict(kind='pop', clip_ids=[])]
    def compare(value):
        comparison.compare(value, expected, dict(numeric_nonexact_fields=0,max_absolute_difference=0), 'events')
    compare(copy.deepcopy(expected))
    mutations = [expected[:-1], list(reversed(expected))]
    for key, value in [('code',66),('unicode',[66]),('clip_ids',[]),('mode',3),
                       ('matrix',[10,0,0,-10,0,302]),('advance',.9),
                       ('color',{'space':'gray','values':[1]})]:
        changed = copy.deepcopy(expected); changed[0][key] = value; mutations.append(changed)
    for changed in mutations:
        try:
            compare(changed)
        except ValueError:
            checks += 1
        else:
            raise AssertionError('comparator accepted semantic mutation')
    try:
        comparison.compare([0,0,201,300],[0,0,200,300],dict(numeric_nonexact_fields=0,max_absolute_difference=0),'initial_clip')
    except ValueError:
        checks += 1
    else:
        raise AssertionError('comparator accepted initial geometry mutation')
    print(json.dumps({'native_page_oracle_and_comparator_checks':checks,'status':'passed'}))


if __name__ == '__main__':
    main()
