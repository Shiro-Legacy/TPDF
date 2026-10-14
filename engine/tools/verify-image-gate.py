#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Fail-closed final R23 gate: exact coverage, source identity and saved pixel evidence."""
import argparse
import collections
import copy
import gzip
import hashlib
import json
import re
import tempfile
from pathlib import Path

ENGINE = Path(__file__).resolve().parents[1]
REPORTS = dict(sampling='image-sampling-differential-r23.json',
               decode='image-decode-differential-r23b.json',
               decode_validation='image-decode-validation-r23b.json',
               inventory='image-corpus-inventory-r23b.json',
               pages='image-page-differential-r23c.json',
               page_validation='image-page-validation-r23c.json',
               corpus_model_validation='image-corpus-model-validation-r23c.json',
               corpus='image-corpus-differential-r23c.json',
               build='image-build-r23.json', checks='image-rust-checks-r23.json')
sha = lambda b: hashlib.sha256(b).hexdigest()


def require(ok, why):
    if not ok:
        raise ValueError(why)


def keyed(rows, key, label):
    result = {key(r): r for r in rows}
    require(len(result) == len(rows), 'duplicate ' + label)
    return result


def ppm(data):
    m = re.match(rb'P6\n([1-9][0-9]*) ([1-9][0-9]*)\n255\n', data)
    require(m is not None, 'PPM envelope')
    w, h = map(int, m.groups())
    require(w <= 16384 and h <= 16384 and len(data) == m.end() + w*h*3, 'PPM dimensions')
    return w, h, data[m.end():]


def blob(record):
    require(set(record) >= {'path', 'sha256'}, 'missing raw evidence descriptor')
    with gzip.open(record['path'], 'rb') as stream:
        data = stream.read((256 << 20) + 1)
    require(len(data) <= 256 << 20 and sha(data) == record['sha256'], 'changed raw evidence ' + record['path'])
    return data


def counts(rows):
    return dict(collections.Counter(r['status'] for r in rows))


def refusal(error, prefix):
    return isinstance(error, str) and error.startswith(prefix) and bool(error[len(prefix):].strip())


def difference(left, right):
    require(len(left) == len(right) and left, 'comparison dimensions')
    return dict(different_channels=sum(a != b for a,b in zip(left,right)),
                max_channel_delta=max(abs(a-b) for a,b in zip(left,right)))


def trace_page(data, page_number):
    value = json.loads(data)
    require(value.get('repaired') is False and isinstance(value.get('pages'),list) and
            len(value['pages']) == 1, 'native trace envelope')
    page = value['pages'][0]
    require(page.get('page') == page_number and page.get('status') in ('accepted','refused'), 'native trace identity')
    if page['status'] == 'refused':
        require(isinstance(page.get('error'),str) and page['error'].strip(), 'native trace refusal')
    else:
        require(isinstance(page.get('events'),list), 'native trace events')
    return page


def verify_render(row, source_pdf, certified):
    ev = row['evidence_files']; rust = blob(ev['rust']); native = blob(ev['native4x'])
    require(sha(source_pdf) == row['input_sha256'], 'render source PDF identity')
    require(sha(rust) == row['rust_ppm_sha256'] and sha(native) == row['native_4x_ppm_sha256'], 'saved corpus render identity')
    w,h,rgb = ppm(rust); nw,nh,native_rgb = ppm(native)
    require((w,h) == (row['width'],row['height']) and (nw,nh) == (w*4,h*4), 'corpus canvas')
    from image_page_model import resolve
    resolved = resolve(native_rgb,nw,nh)
    require(sha(resolved) == row['native_resolved_sha256'] and
            difference(rgb,resolved) == row['native_raw_comparison'], 'native raw comparison')
    if not certified:
        return
    trace = blob(ev['native_trace']); model = blob(ev['model_rgb']); bounds_bytes = blob(ev['model_bounds'])
    require(sha(trace) == row['native_trace_sha256'] and sha(model) == row['model_rgb_sha256'] and
            sha(bounds_bytes) == row['model_bounds_sha256'], 'corpus model provenance')
    page = trace_page(trace,row['page'])
    require(page['status'] == row['native_trace_status'] == 'accepted', 'certified native trace status')
    from image_corpus_model import certify
    expected, boundary, stats = certify(page,w,h,row['dpi'],source_pdf=source_pdf)
    bounds = stats.pop('_channel_bounds')
    encoded_bounds = (json.dumps(bounds,separators=(',',':'))+'\n').encode()
    require(model == expected and bounds_bytes == encoded_bounds, 'recomputed corpus model/bounds')
    require(stats == row['model_stats'] and len(boundary) == w*h and len(bounds) == len(rgb), 'recomputed model statistics')
    exact = violations = maximum = 0
    for actual,want,pair in zip(rgb,expected,bounds):
        require(len(pair) == 2 and all(type(v) is int for v in pair), 'channel bound type')
        lo,hi = pair
        require(0 <= lo <= want <= hi <= 255, 'certified channel bound')
        exact += lo == hi
        delta = max(lo-actual,actual-hi,0); violations += delta != 0; maximum = max(maximum,delta)
    certificate = dict(pixels=w*h,exact_channels=exact,bounded_channels=len(rgb)-exact,
                       violating_channels=violations,max_violation=maximum,
                       images=stats['images'],source_ctm_checks=stats['source_ctm_checks'])
    require(certificate == row['model_certificate'] and violations == 0 and
            certificate['images'] > 0 and certificate['source_ctm_checks'] == certificate['images'], 'recomputed model certificate')


def expected_faults():
    decode = {'signal','unexpected-exit','silent-refusal','refusal-stdout','refusal-prefix',
              'prefix-only','prefix-whitespace','prefix-garbage','success-warning','empty-success',
              'json-success','wrong-magic','negative-width','zero-width','too-wide','wrong-depth',
              'truncated','extra','partial-header'}
    wrappers = {'not-pdf','repair','missing-object','non-image','zero-arg','negative-arg','huge-arg','input-bound'}
    decode |= {who+'-'+label for who in ('native','rust') for label in wrappers}
    from image_page_cases import generated
    pages = {case['label']+'-native-byte-'+str(offset) for case in generated()
             for offset in (0,case.get('width',16)*case.get('height',16)*48//2,
                            case.get('width',16)*case.get('height',16)*48-1)}
    pages |= {'empty','wrong-magic','truncated','extra','negative-dimension','dimension-overflow',
              'missing-page','zero-page','dpi-bound','bad-pdf','repaired','input-bound'}
    return dict(decode_validation=decode,page_validation=pages)


def verify(reports, policy, check_files=True):
    require(set(reports) == set(REPORTS), 'exact required report identities')
    expected_tools = {str(p) for p in (ENGINE/'tools').rglob('*') if p.suffix in ('.py','.c','.sh')}
    require(set(policy['tool_sha256']) == expected_tools, 'policy tool dependency closure')
    for name, report in reports.items():
        require(report.get('status') == 'passed' and not report.get('failures'), name + ' failed/missing')
    build = reports['build']
    expected_sources = sorted((ENGINE/'src').rglob('*.rs')) + sorted((ENGINE/'examples').glob('*.rs')) + [ENGINE/'Cargo.toml', ENGINE/'Cargo.lock']
    require(set(build['source_sha256']) == {str(p) for p in expected_sources}, 'build source closure')
    binaries = build['binary_sha256']
    by_name = {Path(p).name: (p, h) for p, h in binaries.items()}
    require(len(binaries) == len(by_name) == 3 and set(by_name) == {'pdf-inspect', 'image_decode_probe', 'image_sampling_probe'}, 'build binary closure')
    all_hashes = dict(policy['tool_sha256'])
    all_hashes.update(build['source_sha256'])
    all_hashes.update(binaries)
    for name, report in reports.items():
        if name == 'build':
            continue
        evidence = report.get('evidence_sha256', {})
        require(evidence, name + ' missing provenance')
        for path, digest in evidence.items():
            require(path not in all_hashes or all_hashes[path] == digest, 'conflicting evidence ' + path)
            all_hashes[path] = digest
        if name not in ('checks','page_validation'):
            binary = 'image_sampling_probe' if name == 'sampling' else 'image_decode_probe' if name in ('decode','decode_validation','inventory') else 'pdf-inspect'
            path, digest = by_name[binary]
            require(evidence.get(path) == digest, name + ' uses a different Rust build')
    dependencies = {
        'sampling': ['differential-image-sampling.py'],
        'corpus_model_validation': ['test-image-corpus-model.py','image_corpus_model.py','mupdf-image-trace-oracle.c','build-image-trace-reference.sh','page_raster_review_model.py','page_raster_reference.py','stroke_review_model.py','image_page_model.py','image_page_cases.py','image_decode_cases.py','differential-lzw.py'],
        'decode': ['differential-images.py','image_decode_cases.py','differential-lzw.py','mupdf-image-decode-oracle.c','build-image-reference.sh'],
        'decode_validation': ['test-image-reference.py','differential-images.py','image_decode_cases.py','differential-lzw.py'],
        'inventory': ['image-corpus-inventory.py','differential-images.py','mupdf-image-inventory-oracle.c','mupdf-image-decode-oracle.c'],
        'pages': ['differential-image-pages.py','image_page_cases.py','image_page_model.py','image_native_policy.py','image_decode_cases.py','differential-lzw.py','mupdf-image-page-oracle.c','build-image-reference.sh'],
        'page_validation': ['test-image-page-reference.py','differential-image-pages.py','image_page_cases.py','image_page_model.py','image_native_policy.py','image_decode_cases.py','differential-lzw.py'],
        'corpus': ['differential-image-corpus.py','differential-image-pages.py','image_page_model.py','image_corpus_model.py','page_raster_review_model.py','stroke_review_model.py','image_decode_cases.py','differential-lzw.py','mupdf-image-page-oracle.c','mupdf-image-trace-oracle.c','build-image-reference.sh','build-image-trace-reference.sh'],
    }
    native_dependencies = {
        'corpus_model_validation': {'mupdf-image-trace-oracle'},
        'decode': {'mupdf-image-decode-oracle'}, 'decode_validation': {'mupdf-image-decode-oracle'},
        'inventory': {'mupdf-image-decode-oracle','mupdf-image-inventory-oracle'},
        'pages': {'mupdf-image-page-oracle','mupdf-stroke-raster-oracle'},
        'page_validation': {'mupdf-image-page-oracle','mupdf-stroke-raster-oracle'},
        'corpus': {'mupdf-image-page-oracle','mupdf-image-trace-oracle'},
    }
    for name in ('decode','inventory','pages','page_validation','corpus'):
        require(str(ENGINE/'reference/image-native-pins.sha256') in reports[name]['evidence_sha256'], name+' missing native source pins')
    native_hashes = {}
    for name, filenames in dependencies.items():
        evidence = reports[name]['evidence_sha256']
        require(all(str(ENGINE/'tools'/filename) in evidence for filename in filenames), name+' missing tool dependency')
        for basename in native_dependencies.get(name,set()):
            matches = [(p,h) for p,h in evidence.items() if Path(p).name == basename]
            require(len(matches) == 1, name+' missing/duplicate native binary '+basename)
            path,digest = matches[0]
            require(basename not in native_hashes or native_hashes[basename] == (path,digest), 'inconsistent native build '+basename)
            native_hashes[basename] = (path,digest)
    checks = reports['checks']
    require({str(p) for p in (ENGINE/'tests').rglob('*.rs')} <= set(checks['evidence_sha256']), 'Rust test source closure')
    require(checks['tests_current'] == 796 and checks['tests_msrv'] == 796 and
            checks['fmt'] is True and checks['clippy'] is True and checks['wasm'] is True, 'Rust verification gates')
    if check_files:
        for path, digest in all_hashes.items():
            require(sha(Path(path).read_bytes()) == digest, 'changed evidence ' + path)
    sample = reports['sampling']
    require(sample['cases'] == 10084 and sample['outcomes'] == {'selected':1358,'outside':7118,'refused':1608}, 'numeric sampling coverage')
    require(sample['input_sha256'] == 'd0747811d418ed30d7da0e44faa5003379b815f7c02db9dedd3868fbb5d5ed7e' and
            sample['output_sha256'] == 'd6016502fc1a265ded04e96dcb02e04a55844aeff78d4ece8116008a0ff5346b' and
            len(sample['rejected_mutations']) == 5 and set(sample['rejected_mutations']) == {'omitted sample','extra sample','wrong row/column','clamped outside','silent refusal'}, 'numeric sampling outcomes')
    for name, expected in expected_faults().items():
        rows = keyed(reports[name]['rows'],lambda r:r['label'],name+' fault')
        require(set(rows) == expected and reports[name]['checks'] == len(expected) and
                all(r['status'] == 'rejected' for r in rows.values()), name + ' fault coverage')
    for name in ('decode','pages'):
        report = reports[name]
        rows = keyed(report['rows'], lambda r:r['label'], name)
        require(set(rows) == set(policy[name]), name + ' exact case identities')
        require(report['counts'] == counts(report['rows']), name + ' summary counts')
        for label, row in rows.items():
            require(row['status'] == policy[name][label], name + ' case outcome ' + label)
    require(reports['decode']['counts'] == {'exact_match':265,'both_refused':13,'strict_refused_native_accepted':11}, 'decode outcome coverage')
    from image_decode_cases import generated as decode_cases
    cases = keyed(list(decode_cases()), lambda r:r['label'], 'decode generator')
    require(set(cases) == set(policy['decode']), 'decode generator identity')
    for row in reports['decode']['rows']:
        case = cases[row['label']]
        require(row['input_sha256'] == sha(case['pdf']) and row['accepted'] == case['accepted'], 'decode input/expectation')
        if case['accepted']:
            require(row['expected_sha256'] == sha(case['expected']) and
                    all(row[k]['exit'] == 0 and not row[k]['stderr'] and row[k]['stdout_sha256'] == row['expected_sha256'] for k in ('native','rust')), 'decode exact bytes')
        else:
            require(row['rust']['exit'] == 2 and row['rust']['stdout_sha256'] == sha(b'') and
                    row['rust']['stderr'].startswith('image-decode: ') and row['rust']['stderr'][len('image-decode: '):].strip(), 'decode refusal envelope')
            native = row['native']
            if row['status'] == 'both_refused':
                require(native['exit'] == 2 and native['stdout_sha256'] == sha(b'') and
                        refusal(native['stderr'],'image decode oracle: '), 'native decode refusal envelope')
            else:
                require(native['exit'] == 0 and not native['stderr'] and native['stdout_sha256'] != sha(b''), 'native recovery envelope')
        if check_files:
            ev = row['evidence_files']
            require(blob(ev['pdf']) == case['pdf'], 'decode PDF bytes')
            for kind in ('native','rust'):
                data = blob(ev[kind]); require(sha(data) == row[kind]['stdout_sha256'], 'decode saved output')
                if case['accepted']:
                    require(data == case['expected'], 'decode saved bytes')
                elif row[kind]['exit'] == 0:
                    ppm(data)
                else:
                    require(data == b'', 'refusal saved stdout')
    del cases
    from image_page_cases import generated as page_cases
    cases = keyed(list(page_cases()),lambda r:r['label'],'page generator')
    require(len(cases) == 28 and set(cases) == set(policy['pages']), 'page geometry/composition coverage')
    for row in reports['pages']['rows']:
        case = cases[row['label']]
        require(row['input_sha256'] == sha(case['pdf']) and row['native_parity_required'] == case['native_parity'], 'page input/expectation')
        require(row['rust_model'] == {'different_channels':0,'max_channel_delta':0} and
                row['native_source_witness'] == {'different_channels':0,'max_channel_delta':0}, 'unexplained generated pixel difference')
        if case['native_parity']:
            require(row['native_samples_model']['different_channels'] == 0, 'mandatory native parity')
        require((row['status'] == 'exact_native_model_match') == (row['native_samples_model']['different_channels'] == 0), 'native difference misclassified')
        if check_files:
            ev = row['evidence_files']; data = {key:blob(rec) for key,rec in ev.items()}
            require(data['pdf'] == case['pdf'], 'saved page PDF')
            for key, payload in data.items():
                require(row[key+'_sha256'] == sha(payload), 'saved generated payload hash')
            w,h,rust = ppm(data['rust.ppm']); nw,nh,native = ppm(data['native.ppm'])
            require((w,h) == (case.get('width',16),case.get('height',16)) and (nw,nh) == (w*4,h*4), 'generated canvas')
            from image_page_model import render
            samples, expected = render(case['events'],w,h)
            require(rust == expected == data['model.rgb'] and samples == data['model.samples.rgb'], 'recomputed exact page model')
            from image_native_policy import render as witness
            native_expected, _ = witness(case['events'],w,h)
            require(native == native_expected == data['native-source-witness.rgb'], 'recomputed native source witness')
            from image_page_model import resolve
            require(row['native_samples_model'] == difference(native,samples) and
                    row['native_resolved_model'] == difference(resolve(native,nw,nh),expected), 'recomputed native page differences')
    require(reports['pages']['counts'] == {'exact_native_model_match':19,'exact_model_native_policy_difference':9}, 'native source policy coverage')
    validation = reports['corpus_model_validation']
    validation_rows = keyed(validation['rows'],lambda r:r['label'],'corpus model validation')
    from image_corpus_model import mixed_cases
    mixed_cases_by_label = keyed(list(mixed_cases()),lambda r:r['label'],'mixed model generator')
    rejected_labels = {'changed_native_image_placement','truncated_native_samples','wrong_native_sample_shape',
                       'wrong_image_source_origin','missing_image_callback','changed_image_clip_ancestry',
                       'truncated_original_do_span','changed_source_cm','changed_source_paint_color','changed_original_page_geometry'}
    atomic_labels = {'soft_mask','interpolation','late_unsupported_operator'}
    require(set(validation_rows) == set(cases) | set(mixed_cases_by_label) | rejected_labels | atomic_labels |
            {'changed_native_sample_colors'}, 'corpus model exact validation identities')
    require(validation['counts'] == counts(validation['rows']) == {'exact_independent_match':28,
            'mixed_pixels_certified':2,'rejected':10,'different_certified_pixels':1,'native_atomic_refusal':3}, 'corpus model validation outcomes')
    page_rows = keyed(reports['pages']['rows'],lambda r:r['label'],'generated source model row')
    for label,case in cases.items():
        row = validation_rows[label]
        require(row['status'] == 'exact_independent_match' and row['input_sha256'] == sha(case['pdf']) and
                row['images'] == sum(e['kind'] == 'image' for e in case['events']) and
                row['model_pixel_sha256'] == page_rows[label]['model.rgb_sha256'], 'independent model cross-check')
    for label,case in mixed_cases_by_label.items():
        row = validation_rows[label]
        require(row['status'] == 'mixed_pixels_certified' and row['input_sha256'] == sha(case['pdf']) and
                row['images'] == {'text-image-stroke':1,'image-text-stroke-image':2}[label] and
                row['channel_interval_violations'] == 0, 'mixed model validation')
    for label in rejected_labels | atomic_labels:
        row = validation_rows[label]
        require(row['status'] == ('rejected' if label in rejected_labels else 'native_atomic_refusal') and
                isinstance(row['reason'],str) and row['reason'].strip(), 'named model fault rejected')
    changed = validation_rows['changed_native_sample_colors']
    require(changed['status'] == 'different_certified_pixels' and changed['different_channels'] > 0 and
            changed['baseline_channel_violations'] > 0, 'corrupted decoded sample sensitivity')
    inventory = json.loads((ENGINE/'reference/fixture-inventory.json').read_text())
    docs = {d['file']:d for d in inventory['documents'] if not d.get('passwordRequired')}
    inv = reports['inventory']; observed = keyed(inv['documents'],lambda r:r['file'],'image inventory document')
    require(len(docs) == 49 and set(observed) == set(docs), 'complete image inventory document coverage')
    objects = keyed(inv['rows'],lambda r:(r['file'],r['object'],r['generation']),'image object')
    expected_objects = {tuple(v) for v in policy['image_objects']}
    require(len(policy['image_objects']) == len(expected_objects) == len(objects) == 20 and set(objects) == expected_objects and inv['counts'] == counts(inv['rows']) == {'exact_match':18,'both_refused':2}, 'complete image object census')
    for name,doc in observed.items():
        require(doc['sha256'] == docs[name]['sha256'] and doc['images'] == sum(r['file'] == name for r in inv['rows']), 'image census per-document counts')
        if check_files:
            source = (ENGINE.parent/'fixtures'/name).read_bytes()
            require(sha(source) == doc['sha256'], 'inventory fixture bytes')
            raw = blob(dict(path=str(Path(inv['evidence_dir'])/name/'inventory.txt.gz'),sha256=doc['inventory_sha256']))
            lines = raw.decode('ascii').splitlines()
            require(lines and lines[0] == 'image-object-inventory-v1' and len(lines)-1 == doc['images'], 'raw inventory envelope/count')
            fields = ('object','generation','width','height','bits','components','mask','smask','interpolate','imagemask','filters')
            parsed = []
            for line in lines[1:]:
                tokens = line.split()
                require(len(tokens) == len(fields) and all(re.fullmatch(r'-?[0-9]+',t) for t in tokens), 'raw inventory fields')
                parsed.append(dict(zip(fields,map(int,tokens))))
            parsed = keyed(parsed,lambda r:(name,r['object'],r['generation']),'raw inventory object')
            require(set(parsed) == {k for k in objects if k[0] == name}, 'raw inventory object identities')
            for key,record in parsed.items():
                require(all(objects[key][field] == value for field,value in record.items()), 'raw inventory metadata')
    for row in inv['rows']:
        if row['status'] == 'exact_match':
            require(row['native_accepted'] and row['rust_accepted'] and row['native_sha256'] == row['rust_sha256'], 'corpus decoded pixels')
        else:
            require(not row['rust_accepted'] and not row['native_accepted'], 'corpus image refusal')
        for kind,prefix in [('rust','image-decode: '),('native','image decode oracle: ')]:
            require((not row[kind+'_error']) if row[kind+'_accepted'] else refusal(row[kind+'_error'],prefix), 'corpus decode diagnostic')
            if check_files:
                data = blob(row['evidence_files'][kind])
                require(sha(data) == row[kind+'_sha256'], 'corpus decode evidence identity')
                if row[kind+'_accepted']:
                    w,h,_ = ppm(data)
                    require((w,h) == (row['width'],row['height']), 'corpus decoded image dimensions')
                else:
                    require(not data, 'corpus refused image stdout')
    prior = json.loads((ENGINE/'reference/form-differential-f0.json').read_text())
    key = lambda r:(r['file'],r['page'],r['dpi'])
    previous = keyed(prior['pixel_rows'],key,'R22 baseline')
    require(len(previous) == 620, 'R22 baseline coverage')
    old_ok = {k:r for k,r in previous.items() if r['status']=='native_diagnostic_matched'}
    old_refused = {k for k,r in previous.items() if k[2] == 18 and r['status']=='refused'}
    corpus = reports['corpus']; rows = keyed(corpus['pixel_rows'],key,'corpus page/DPI')
    expected_new = {tuple(v) for v in policy['new_image_pages']}
    require(len(policy['new_image_pages']) == len(expected_new) == 12 and len(rows) == 632, 'final corpus/new-image counts')
    new_keys = {(f,p,dpi) for f,p in expected_new for dpi in (18,72)}
    require(set(rows) == set(previous) | new_keys, 'complete corpus page/DPI identities')
    require({k for k,r in rows.items() if r['status']=='new_image_render'} == new_keys, 'new image acceptance coverage')
    require(len(old_ok) == corpus['preserved_r22_hashes'] == 593 and
            len(old_refused) == 27 and {key(r) for r in corpus['prior_refusal_census']} == old_refused and
            len(corpus['prior_refusal_census']) == 27, 'preservation/refusal census coverage')
    for row in corpus['prior_refusal_census']:
        require(row == rows[key(row)], 'census differs from corpus row')
    for dpi in (18,72):
        require(corpus[f'corpus_{dpi}dpi'] == counts([r for r in rows.values() if r['dpi']==dpi]), 'corpus summary counts')
    for k,row in rows.items():
        require(row['input_sha256'] == docs[k[0]]['sha256'], 'corpus input identity')
        if k in old_ok:
            require(row['status'] == 'preserved_r22' and row['rust_ppm_sha256'] == old_ok[k]['rust_ppm_sha256'], 'prior accepted pixels changed')
        elif k not in new_keys:
            require(row['status'] == 'refused' and row['refusal'].startswith('pdf-inspect: ') and row['refusal'][len('pdf-inspect: '):].strip(), 'remaining explicit refusal')
        else:
            cert = row['model_certificate']
            require(cert['pixels'] == row['width']*row['height'] and cert['exact_channels']+cert['bounded_channels'] == cert['pixels']*3 and
                    cert['violating_channels'] == 0 and cert['max_violation'] == 0 and cert['images'] > 0 and cert['source_ctm_checks'] == cert['images'], 'new corpus model certificate')
        if check_files:
            source_pdf = (ENGINE.parent/'fixtures'/row['file']).read_bytes()
            require(sha(source_pdf) == row['input_sha256'], 'corpus source fixture bytes')
            if row['status'] != 'refused':
                verify_render(row,source_pdf,k in new_keys)
            else:
                trace = blob(row['evidence_files']['native_trace'])
                require(sha(trace) == row['native_trace_sha256'], 'refusal native trace provenance')
                page = trace_page(trace,row['page'])
                require(page['status'] == row['native_trace_status'], 'refusal native trace status')
                if page['status'] == 'refused':
                    require(page['error'] == row['native_trace_refusal'], 'refusal native reason')
    from image_corpus_model import mixed_cases
    mixed = keyed(list(mixed_cases()),lambda r:r['label'],'mixed generator')
    generated = keyed(corpus['generated_model_rows'],lambda r:r['label'],'mixed model row')
    require(len(mixed) == 2 and set(generated) == set(mixed), 'mixed text/image/stroke coverage')
    for label,row in generated.items():
        case = mixed[label]
        require(row['status'] == 'new_image_render' and row['file'] == 'generated/'+label and
                row['page'] == 1 and row['dpi'] == 72 and row['input_sha256'] == sha(case['pdf']) and
                (row['width'],row['height']) == (case['width'],case['height']), 'mixed model identity')
        require(row['rust_ppm_sha256'] == validation_rows[label]['rust_ppm_sha256'], 'mixed rendered validation identity')
        cert = row['model_certificate']
        require(cert['pixels'] == case['width']*case['height'] and cert['images'] > 0 and
                cert['source_ctm_checks'] == cert['images'] and cert['violating_channels'] == cert['max_violation'] == 0 and
                cert['exact_channels']+cert['bounded_channels'] == cert['pixels']*3, 'mixed model certificate')
        if check_files:
            require(blob(row['evidence_files']['pdf']) == case['pdf'], 'mixed original PDF')
            verify_render(row,case['pdf'],True)
    return dict(status='passed',generated_decodes=289,generated_pages=28,image_objects=20,corpus_attempts=len(rows),
                newly_supported_pages=len(expected_new),preserved_r22_renders=len(old_ok),verified_hashes=len(all_hashes))


def raw_mutations(reports):
    """Mutate private temporary evidence; never overwrite accepted raw artifacts."""
    from image_corpus_model import mixed_cases
    source = {case['label']:case['pdf'] for case in mixed_cases()}
    baseline = reports['corpus']['generated_model_rows'][0]
    rejected = []
    with tempfile.TemporaryDirectory(prefix='r23-gate-mutations-') as directory:
        def check(label, key, change):
            row = copy.deepcopy(baseline)
            old = blob(row['evidence_files'][key]); altered = change(old)
            path = Path(directory)/(label+'.gz'); path.write_bytes(gzip.compress(altered,mtime=0))
            row['evidence_files'][key] = dict(path=str(path),sha256=sha(altered))
            digest_field = {'rust':'rust_ppm_sha256','native_trace':'native_trace_sha256',
                            'model_rgb':'model_rgb_sha256','model_bounds':'model_bounds_sha256'}[key]
            row[digest_field] = sha(altered)
            if key == 'rust':
                from image_page_model import resolve
                _,_,rgb = ppm(altered)
                nw,nh,native = ppm(blob(row['evidence_files']['native4x']))
                row['native_raw_comparison'] = difference(rgb,resolve(native,nw,nh))
            if key == 'model_bounds':
                # Keep the forged summary self-consistent: only recomputation
                # from source and native trace can distinguish these intervals.
                row['model_certificate']['exact_channels'] = 0
                row['model_certificate']['bounded_channels'] = row['width']*row['height']*3
            try:
                verify_render(row,source[row['label']],True)
            except (ValueError,KeyError,TypeError,IndexError,OSError):
                rejected.append(label)
            else:
                raise ValueError('accepted raw mutation '+label)
        check('forged full-range channel bounds','model_bounds',
              lambda old:(json.dumps([[0,255] for _ in json.loads(old)],separators=(',',':'))+'\n').encode())
        check('corrupted model RGB','model_rgb',lambda old:bytes([old[0]^1])+old[1:])
        def wrong_trace(old):
            value=json.loads(old);value['pages'][0]['page']+=1
            return json.dumps(value).encode()
        check('wrong native trace page','native_trace',wrong_trace)
        def wrong_pixel(old):
            w,h,rgb=ppm(old)
            bounds=json.loads(blob(baseline['evidence_files']['model_bounds']))
            index=next(i for i,pair in enumerate(bounds) if pair[0]==pair[1])
            changed=bytearray(rgb);changed[index]^=1
            return f'P6\n{w} {h}\n255\n'.encode()+changed
        check('corrupted certified exact pixel','rust',wrong_pixel)
    return rejected


def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--policy',type=Path,default=ENGINE/'reference/image-acceptance-policy-r23.json')
    ap.add_argument('--output',type=Path,required=True)
    args=ap.parse_args()
    reports={key:json.loads((ENGINE/'reference'/name).read_text()) for key,name in REPORTS.items()}
    policy=json.loads(args.policy.read_text())
    result=verify(reports,policy)
    # Mutation checks intentionally skip unchanged raw files after the complete
    # byte/source verification above; each mutation must fail a structural gate.
    mutations=[]
    def check(label,action):
        changed=copy.deepcopy(reports);action(changed)
        try:verify(changed,policy,False)
        except (ValueError,KeyError,TypeError,IndexError):mutations.append(label)
        else:raise ValueError('accepted mutation '+label)
    check('missing report',lambda r:r.pop('sampling'))
    check('empty decode',lambda r:r['decode'].__setitem__('rows',[]))
    check('empty pages',lambda r:r['pages'].__setitem__('rows',[]))
    check('missing tool dependency',lambda r:r['decode']['evidence_sha256'].pop(str(ENGINE/'tools/differential-images.py')))
    check('renamed fault',lambda r:r['decode_validation']['rows'][0].__setitem__('label','invented'))
    check('native refusal malformed',lambda r:next(x for x in r['decode']['rows'] if x['status']=='both_refused')['native'].__setitem__('stderr','image decode oracle: '))
    check('missing corpus model validation',lambda r:r['corpus_model_validation']['rows'].pop())
    check('corrupt sample sensitivity missing',lambda r:next(x for x in r['corpus_model_validation']['rows'] if x['label']=='changed_native_sample_colors').__setitem__('baseline_channel_violations',0))
    check('missing mixed model',lambda r:r['corpus']['generated_model_rows'].pop())
    check('duplicate mixed model',lambda r:r['corpus']['generated_model_rows'].__setitem__(1,copy.deepcopy(r['corpus']['generated_model_rows'][0])))
    check('mixed pixel violation',lambda r:r['corpus']['generated_model_rows'][0]['model_certificate'].__setitem__('violating_channels',1))
    check('omitted decode',lambda r:r['decode']['rows'].pop())
    check('duplicate decode',lambda r:r['decode']['rows'].append(copy.deepcopy(r['decode']['rows'][0])))
    check('omitted page',lambda r:r['pages']['rows'].pop())
    check('duplicate page',lambda r:r['pages']['rows'].__setitem__(1,copy.deepcopy(r['pages']['rows'][0])))
    check('wrong model pixel',lambda r:r['pages']['rows'][0]['rust_model'].__setitem__('different_channels',1))
    check('unexplained native pixel',lambda r:r['pages']['rows'][0]['native_source_witness'].__setitem__('different_channels',1))
    check('omitted corpus',lambda r:r['corpus']['pixel_rows'].pop())
    check('duplicate corpus',lambda r:r['corpus']['pixel_rows'].__setitem__(1,copy.deepcopy(r['corpus']['pixel_rows'][0])))
    check('omitted refusal census',lambda r:r['corpus']['prior_refusal_census'].pop())
    check('wrong prior pixel hash',lambda r:next(x for x in r['corpus']['pixel_rows'] if x['status']=='preserved_r22').__setitem__('rust_ppm_sha256','0'*64))
    check('missing new-page model',lambda r:next(x for x in r['corpus']['pixel_rows'] if x['status']=='new_image_render').pop('model_certificate'))
    check('new-page pixel violation',lambda r:next(x for x in r['corpus']['pixel_rows'] if x['status']=='new_image_render')['model_certificate'].__setitem__('violating_channels',1))
    check('refusal becomes success',lambda r:next(x for x in r['corpus']['pixel_rows'] if x['status']=='refused').__setitem__('status','new_image_render'))
    check('missing object census',lambda r:r['inventory']['rows'].pop())
    check('missing build source',lambda r:r['build']['source_sha256'].pop(next(iter(r['build']['source_sha256']))))
    check('missing fault test',lambda r:r['page_validation']['rows'].pop())
    check('short numeric run',lambda r:r['sampling'].__setitem__('cases',10083))
    check('different binary',lambda r:r['build']['binary_sha256'].__setitem__(next(iter(r['build']['binary_sha256'])),'0'*64))
    result['rejected_mutations']=mutations
    result['rejected_raw_mutations']=raw_mutations(reports)
    paths=[Path(__file__),args.policy,*[ENGINE/'reference'/p for p in REPORTS.values()]]
    result['evidence_sha256']={str(p):sha(p.read_bytes()) for p in paths}
    args.output.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result))

if __name__=='__main__':
    main()
