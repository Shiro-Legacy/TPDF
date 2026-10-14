#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""PP1 complete page execution and native semantic comparison (never pixel parity)."""
import argparse
import collections
import copy
import hashlib
import gzip
import runpy
import json
import math
from pathlib import Path
import subprocess

ENGINE = Path(__file__).resolve().parent.parent
TEXT_ABSOLUTE = .002
TEXT_RELATIVE = 2e-5
PATH_ABSOLUTE = 1e-5
PATH_RELATIVE = 2e-6


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def clip_chain(clip, clips):
    result = []
    while clip is not None:
        if clip in result or not 0 <= clip < len(clips):
            raise ValueError('invalid Rust clip ancestry')
        result.append(clip)
        clip = clips[clip]['parent']
    return list(reversed(result))


def normalized_path(commands):
    """Exactly the two documented native P0 compressions; no degenerate deletion."""
    output = []
    point = None
    start = None
    for value in commands:
        command = list(value)
        if command[0] == 'm' and output and output[-1][0] == 'm':
            output.pop()
        if command[0] == 'c' and point == command[1:3] == command[3:5] == command[5:7]:
            command = ['l'] + command[5:7]
        output.append(command)
        if command[0] in ('m', 'l'):
            point = command[1:3]
            if command[0] == 'm':
                start = point
        elif command[0] == 'c':
            point = command[5:7]
        elif command[0] == 'h':
            point = start
    return output


def rust_semantics(program):
    paths = program['paths']
    current = None
    output = []
    for ordered in program['events']:
        if ordered[0] == 'code':
            _, code, unicode, matrix, advance, mode, context, *_ = ordered
            context = program['text_contexts'][context]
            if context['clip'] != current:
                raise ValueError('Rust text clip differs from ordered clip state')
            output.append(dict(kind='code', code=code, unicode=unicode, matrix=matrix,
                               advance=advance, mode=mode, clip_ids=clip_chain(current, paths['clips']),
                               color=context['color'] if mode == 0 else None))
            continue
        event = paths['events'][ordered[1]]
        kind = event['kind']
        if kind == 'restore':
            while current != event['clip']:
                if current is None:
                    raise ValueError('Rust clip restore is not an ancestor')
                current = paths['clips'][current]['parent']
                output.append(dict(kind='pop', clip_ids=clip_chain(current, paths['clips'])))
            continue
        if kind == 'clip':
            node = paths['clips'][event['clip']]
            if node['parent'] != current:
                raise ValueError('Rust ordered clip parent differs')
            value = dict(kind='clip', path=normalized_path(paths['paths'][node['path']]),
                         matrix=node['matrix'], rule=node['rule'], clip_ids=clip_chain(current, paths['clips']))
            current = event['clip']
        else:
            if event['clip'] != current:
                raise ValueError('Rust paint clip differs from ordered clip state')
            value = dict(kind=kind, path=normalized_path(paths['paths'][event['path']]),
                         matrix=event['matrix'], color=event['color'], clip_ids=clip_chain(current, paths['clips']))
            if kind == 'fill':
                value['rule'] = event['rule']
            else:
                style = event['style']
                value['style'] = dict(width=style['width'], start_cap=style['cap'], dash_cap=style['cap'],
                                      end_cap=style['cap'], join=style['join'], miter=style['miter'],
                                      phase=style['phase'], dash=[] if style['dash'] is None else paths['dashes'][style['dash']])
        output.append(value)
    return output, len(clip_chain(current, paths['clips']))


def native_semantics(page):
    output = []
    clips = {}
    for index, event in enumerate(page['events']):
        if isinstance(event, list):
            _, code, unicode, matrix, advance, mode, context, callback = event
            context = page['contexts'][context]
            if context['color_observable'] != (mode == 0):
                raise ValueError('native color observation differs')
            output.append(dict(kind='code', code=code, unicode=unicode, matrix=matrix,
                               advance=advance, mode=mode, clip_ids=[clips[i] for i in context['clip_ids']], color=context['color']))
        else:
            value = copy.deepcopy(event)
            ids = [clips[i] for i in value.pop('clip_ids')]
            if value.pop('clip_depth') != len(ids):
                raise ValueError('native clip depth differs')
            value['clip_ids'] = ids
            output.append(value)
            if value['kind'] == 'clip':
                clips[index] = len(clips)
    return output


def compare(actual, expected, stats, field='', text_numeric=False):
    if isinstance(expected, (float, int)) and not isinstance(expected, bool):
        if isinstance(actual, bool) or not isinstance(actual, (float, int)) or not math.isfinite(actual) or not math.isfinite(expected):
            raise ValueError(f'{field}: nonfinite or invalid number')
        # Only geometry, paint components, and text placement/advance tolerate f32.
        exact = any(x in field for x in ('/clip_ids/', '/unicode/', '/code', '/mode', '_cap', '/join'))
        delta = abs(actual - expected)
        if delta:
            stats['numeric_nonexact_fields'] += 1
        stats['max_absolute_difference'] = max(stats['max_absolute_difference'], delta)
        category = 'text' if text_numeric else 'geometry' if field.startswith('initial_') else 'path_color_state'
        key = category + '_max_absolute_difference'
        stats[key] = max(stats.get(key, 0), delta)
        key = category + '_nonexact_fields'
        stats[key] = stats.get(key, 0) + bool(delta)
        if text_numeric:
            limit = TEXT_ABSOLUTE + TEXT_RELATIVE * max(1, abs(expected))
        else:
            limit = max(PATH_ABSOLUTE, PATH_RELATIVE * abs(expected))
        if delta > (0 if exact else limit):
            raise ValueError(f'{field}: {actual!r} != {expected!r} (limit {0 if exact else limit})')
    elif isinstance(expected, dict):
        if not isinstance(actual, dict) or set(actual) != set(expected):
            raise ValueError(f'{field}: object keys differ')
        for key, value in expected.items():
            compare(actual[key], value, stats, field+'/'+key,
                    text_numeric=expected.get('kind') == 'code' and key in ('matrix', 'advance'))
    elif isinstance(expected, list):
        if not isinstance(actual, list) or len(actual) != len(expected):
            raise ValueError(f'{field}: lengths {len(actual) if isinstance(actual, list) else "type"} != {len(expected)}')
        for index, (a, e) in enumerate(zip(actual, expected)):
            compare(a, e, stats, field+'/'+str(index), text_numeric)
    elif actual != expected:
        raise ValueError(f'{field}: {actual!r} != {expected!r}')


def invoke(binary, args, data, evidence=None):
    run = subprocess.run([str(binary), *args], input=data, capture_output=True, timeout=60)
    if run.returncode or run.stderr:
        raise ValueError(f'process exit {run.returncode}: '+run.stderr.decode(errors='replace')[:2000])
    if len(run.stdout) > 64*1024*1024:
        raise ValueError('unexpected output over 64MiB')
    if evidence is not None:
        evidence.parent.mkdir(parents=True, exist_ok=True)
        evidence.write_bytes(gzip.compress(run.stdout, mtime=0))
    return json.loads(run.stdout), hashlib.sha256(run.stdout).hexdigest()


def generated_cases(args):
    # Reuse the accepted rational C0 generator through a real PDF page/session.
    # The initial page transform is fixed and applied independently to the model.
    from program_model import ProgramModel, generate_programs
    from text_position_model import compose, json_numbers
    from fractions import Fraction
    t0 = runpy.run_path(str(ENGINE/'tools/differential-text-position.py'))
    c0 = runpy.run_path(str(ENGINE/'tools/differential-program.py'))
    placement = tuple(map(Fraction, [1,0,0,-1,10000,10000]))
    rows = []
    for spec in generate_programs():
        raw, modeled = ProgramModel(spec['fonts']).run(spec['operations'])
        data = t0['native_pdf'](spec, raw)
        row = dict(case=spec['case'], pdf_sha256=hashlib.sha256(data).hexdigest())
        try:
            r, row['rust_output_sha256'] = invoke(args.rust.resolve(), ['--page-programs','-'], data)
            n, row['native_output_sha256'] = invoke(args.native.resolve(), ['all'], data)
            r, n = r['pages'][0], n['pages'][0]
            if r['status'] != 'accepted' or n['status'] != 'accepted':
                raise ValueError('generated complete page refused')
            expected = c0['semantic_expected'](modeled)
            for event in expected:
                event.pop('clip_depth')
                if 'matrix' in event:
                    event['matrix'] = json_numbers(compose(tuple(map(Fraction,event['matrix'])), placement))
            rust, depth = rust_semantics(r['program'])
            if rust != expected:
                raise ValueError('Rust trace differs from independently page-transformed rational model')
            if n['cleanup_pops'] != depth:
                raise ValueError('generated cleanup ancestry differs')
            stats = dict(numeric_nonexact_fields=0,max_absolute_difference=0)
            compare(native_semantics(n), expected, stats, 'events')
            row.update(status='matched', semantic_events=len(expected),
                       source_codes=sum(e['kind']=='code' for e in expected), **stats)
        except (ValueError, KeyError, IndexError, subprocess.TimeoutExpired) as error:
            row.update(status='failure', error=str(error))
        rows.append(row)
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rust', type=Path, default=ENGINE/'target/debug/pdf-inspect')
    parser.add_argument('--native', type=Path, default=Path('/private/tmp/tandem-mupdf-reference/mupdf-page-program-oracle'))
    parser.add_argument('--evidence-dir', type=Path)
    parser.add_argument('--fixtures', type=Path, default=ENGINE.parent/'fixtures')
    parser.add_argument('--report', type=Path, default=ENGINE/'reference/page-program-differential-pp1.json')
    args = parser.parse_args()
    fixture_inventory = ENGINE/'reference/fixture-inventory.json'
    candidate_inventory = ENGINE/'reference/page-program-inventory.json'
    candidates = {(r['file'], r['page']): r for r in json.loads(candidate_inventory.read_text())['rows']}
    evidence = [args.rust, args.native, Path(__file__), fixture_inventory, candidate_inventory,
                ENGINE/'tools/mupdf-page-program-oracle.c', ENGINE/'tools/build-page-program-reference.sh',
                ENGINE/'tools/program_model.py', ENGINE/'tools/text_position_model.py',
                ENGINE/'tools/differential-program.py', ENGINE/'tools/differential-text-position.py']
    hashes = {str(p): digest(p) for p in evidence}
    rows = []
    documents = []
    for document in json.loads(fixture_inventory.read_text())['documents']:
        if document.get('passwordRequired'):
            continue
        name = document['file']
        data = (args.fixtures/name).read_bytes()
        if hashlib.sha256(data).hexdigest() != document['sha256']:
            raise ValueError('immutable fixture changed: '+name)
        doc = dict(file=name, sha256=document['sha256'], pages=document['pages'])
        rust = native = None
        try:
            rust, doc['rust_output_sha256'] = invoke(args.rust.resolve(), ['--page-programs', '-'], data, (args.evidence_dir/'rust'/name).with_suffix('.json.gz') if args.evidence_dir else None)
            if len(rust['pages']) != document['pages'] or [r.get('page') for r in rust['pages']] != list(range(1, document['pages']+1)):
                raise ValueError('Rust document page count differs')
            doc['rust_usage'] = rust.get('usage', rust.get('session_usage'))
        except (ValueError, subprocess.TimeoutExpired) as error:
            doc['rust_error'] = str(error)
            rust = None
        try:
            native, doc['native_output_sha256'] = invoke(args.native.resolve(), ['all'], data, (args.evidence_dir/'native'/name).with_suffix('.json.gz') if args.evidence_dir else None)
            if native.get('repaired') or len(native['pages']) != document['pages'] or [r.get('page') for r in native['pages']] != list(range(1, document['pages']+1)):
                raise ValueError('native repaired document or page count differs')
            doc['native_work'] = native['work']
        except (ValueError, subprocess.TimeoutExpired) as error:
            doc['native_error'] = str(error)
            native = None
        for index in range(document['pages']):
            candidate = candidates[name, index+1]
            row = dict(file=name, page=index+1, candidate=candidate['candidate'],
                       inventory_reasons=candidate['reasons'], stress=name == 'large.pdf')
            r = rust['pages'][index] if rust else None
            n = native['pages'][index] if native else None
            row['rust_status'] = r['status'] if r else 'document_error'
            row['native_status'] = n['status'] if n else 'document_error'
            if not r or r['status'] != 'accepted':
                row.update(status='refused' if r else 'failure', rust_refusal=r if r else doc['rust_error'])
                if n and n['status'] != 'accepted':
                    row['native_refusal'] = n['error']
                rows.append(row)
                continue
            program = r['program']
            row['counts'] = program['counts']
            if not n or n['status'] != 'accepted':
                row.update(status='unverified', native_refusal=n['error'] if n else doc['native_error'])
                rows.append(row)
                continue
            stats = dict(numeric_nonexact_fields=0, max_absolute_difference=0)
            try:
                if r['page'] != index+1 or n['page'] != index+1:
                    raise ValueError('page order differs')
                paths = program['paths']
                compare(n['initial_ctm'], paths['initial_ctm'], stats, 'initial_ctm')
                if paths['initial_clip']['kind'] != 'rectangle':
                    raise ValueError('initial Rust clip not rectangle')
                compare(n['initial_clip'], paths['initial_clip']['bounds'], stats, 'initial_clip')
                expected, remaining_clips = rust_semantics(program)
                actual = native_semantics(n)
                if n['cleanup_pops'] != remaining_clips:
                    raise ValueError('native cleanup clip ancestry differs')
                compare(actual, expected, stats, 'events')
                row.update(status='matched', semantic_events=len(expected), **stats)
            except (ValueError, KeyError, IndexError) as error:
                row.update(status='mismatch', error=str(error), **stats)
            rows.append(row)
        documents.append(doc)
    generated = generated_cases(args)
    if hashes != {str(p): digest(p) for p in evidence}:
        raise ValueError('comparison evidence changed during run')
    def totals(population):
        return dict(pages=len(population), statuses=dict(collections.Counter(r['status'] for r in population)),
                    candidates=sum(r['candidate'] for r in population),
                    accepted=sum(r['rust_status']=='accepted' for r in population),
                    candidate_reductions=sum(r['candidate'] and r['rust_status']!='accepted' for r in population),
                    source_codes=sum(r.get('counts',{}).get('codes',0) for r in population),
                    semantic_events=sum(r.get('semantic_events',0) for r in population))
    report = dict(reference='Pinned MuPDF1.28.1 page CTM + bounded run processor, page content only',
                  evidence_sha256=hashes, totals=totals(rows), stress=totals([r for r in rows if r['stress']]),
                  other=totals([r for r in rows if not r['stress']]),
                  tolerance=dict(text_absolute=TEXT_ABSOLUTE,text_relative=TEXT_RELATIVE,
                                 text_formula='abs(native-rust) <= .002 + 2e-5*max(1,abs(rust))',
                                 path_absolute=PATH_ABSOLUTE,path_relative=PATH_RELATIVE,
                                 path_formula='abs(native-rust) <= max(1e-5,2e-6*abs(rust))',
                                 exact='event kind/order, clip topology, code, Unicode, mode, cap/join enums'),
                  limitations=['No glyph identity, glyph outlines, annotations, image execution or pixel equality claim.',
                               'Initial page bounds are explicit clip metadata; explicit path clip ancestry is compared relative to those bounds.',
                               'Native batching becomes one record per primary source code; native callback IDs remain in hashed original output.',
                               'Tr3 fill color is not observable through native ignore_text and is excluded from native color comparison.',
                               'Native code callbacks verify source-code semantics but do not recover resource alias/font binding identity; Rust font identity provenance is inspected separately.',
                               'Only documented P0 repeated-move/all-equal-cubic compression is normalized; empty paint differences are not hidden.',
                               'P0 existing native float32 tolerance applies to numeric path/style/color fields; dyadic C0 exactness is not claimed for decimal corpus.',
                               'T0 numerical threshold is retained, but corpus run lengths may exceed the earlier generated 100-code test envelope.'],
                  generated=dict(cases=len(generated), matched=sum(r['status']=='matched' for r in generated),
                                 source_codes=sum(r.get('source_codes',0) for r in generated),
                                 independent_model='Exact rational C0 model with page transform applied once', rows=generated),
                  documents=documents, rows=rows)
    args.report.write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps({k:report[k] for k in ('totals','stress','other')},indent=2))
    print(json.dumps({'generated_cases':len(generated),'generated_matched':sum(r['status']=='matched' for r in generated)}))
    if any(r['status'] in ('failure','mismatch','unverified') for r in [*rows,*generated]):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
