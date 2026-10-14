#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Certify complete R22 evidence; a successful shortened run is not acceptance."""
import argparse
import collections
import copy
import hashlib
import json
from pathlib import Path

ENGINE = Path(__file__).resolve().parents[1]


def require(value, message):
    if not value:
        raise ValueError(message)


def verify(report, validation, policy, prior, inventory, check_hashes=True):
    require(report.get('status') == 'passed' and report.get('native_only') is False,
            'full Rust-inclusive differential required')
    require(report.get('failures') == [], 'differential failures')
    require(validation.get('status') == 'passed', 'reference validation failed')
    generated = report['generated_rows']
    labels = [r['label'] for r in generated]
    require(len(labels) == len(set(labels)), 'duplicate generated case')
    expected = {'matched': policy['generated_positive'],
                'strict_scope_refusal': policy['generated_native_fallback_strict_refusals'],
                'structural_refusal': policy['generated_structural_refusals']}
    require(dict(collections.Counter(r['status'] for r in generated)) == expected,
            'generated case coverage')
    renders = [p for r in generated for p in r.get('pixels', [])]
    require(len(renders) == policy['generated_pixel_runs'], 'generated raster coverage')
    models = [p['model_comparison'] for p in renders if 'model_comparison' in p]
    require(len(models) == policy['independent_pixel_models'], 'independent model coverage')
    require(all(m.get('channel_interval_violations') == 0 for m in models),
            'independent sample interval violation')
    old = {(r['file'], r['page']) for r in prior['rows']
           if r['dpi'] == 18 and r['status'] == 'refused'}
    census = report['census_rows']
    require(len(census) == policy['old_refusal_census'] and
            {(r['file'], r['page']) for r in census} == old,
            'old refusal census coverage')
    require(all(r['status'] in ('matched', 'refused') for r in census), 'census failure')
    baseline = {(r['file'], r['page'], r['dpi']): r for r in prior['rows']
                if r['status'] == 'native_diagnostic_matched'}
    require(len(baseline) == policy['prior_pixel_hashes_required'], 'prior baseline count')
    expected18 = {(d['file'], p, 18) for d in inventory['documents']
                  if not d.get('passwordRequired') for p in range(1, d['pages'] + 1)}
    expected72 = {k for k in baseline if k[2] == 72} | {
        ('redact-shared-only.pdf', p, 72) for p in (1, 2)}
    require(len(expected18) == policy['corpus_pages'] and
            len(expected72) == policy['supplementary_72dpi_pages'], 'inventory coverage')
    pixels = report['pixel_rows']
    observed = {(r['file'], r['page'], r['dpi']): r for r in pixels}
    require(len(observed) == len(pixels) and set(observed) == expected18 | expected72,
            'exact corpus page/DPI coverage')
    require(all(r['status'] in ('native_diagnostic_matched', 'refused') for r in pixels),
            'corpus failure')
    require(sum(observed[k]['status'] == 'native_diagnostic_matched' for k in expected18)
            == policy['expected_18dpi_acceptance'], '18 DPI acceptance count')
    require(all(observed[k]['status'] == 'native_diagnostic_matched' for k in expected72),
            '72 DPI acceptance')
    for key, before in baseline.items():
        after = observed[key]
        require(after['status'] == 'native_diagnostic_matched' and
                after.get('rust_ppm_sha256') == before['rust_ppm_sha256'],
                'prior rendered bytes changed: ' + str(key))
    require(all(observed[('redact-shared-only.pdf', p, dpi)]['status'] ==
                'native_diagnostic_matched' for p in (1, 2) for dpi in (18, 72)),
            'actual Form-only page coverage')
    for row in renders + [r for r in pixels if r['status'] == 'native_diagnostic_matched']:
        for kind in ('native_4x', 'native_aa8'):
            require(row[kind].get('unexplained_outside_boundary') == 0 and
                    row[kind].get('source_color_proof_violations', 0) == 0,
                    'unexplained native diagnostic difference')
    checks = {'positive_models': 58, 'negative_cases': 14, 'rejected_mutations': 23,
              'rejected_process_faults': 13, 'native_negative_wrappers': 3,
              'rust_comparator_mutations': 25, 'rust_process_faults': 6,
              'pixel_baseline_mutations': 8}
    require(all(len(validation[k]) == n for k, n in checks.items()),
            'reference fault-check coverage')
    hashes = {}
    for evidence in (report, validation):
        require(bool(evidence.get('evidence_sha256')), 'missing evidence hashes')
        for name, digest in evidence['evidence_sha256'].items():
            require(name not in hashes or hashes[name] == digest, 'conflicting evidence hash')
            hashes[name] = digest
    if check_hashes:
        for name, digest in hashes.items():
            require(hashlib.sha256(Path(name).read_bytes()).hexdigest() == digest,
                    'evidence changed: ' + name)
        require(hashlib.sha256(Path(policy['notes_path']).read_bytes()).hexdigest() ==
                policy['notes_sha256'], 'fixed policy notes changed')
    return {'status': 'passed', 'generated_cases': len(generated),
            'generated_renders': len(renders), 'independent_models': len(models),
            'corpus_attempts': len(pixels), 'preserved_prior_renders': len(baseline),
            'verified_evidence_hashes': len(hashes)}


def mutation_checks(report, validation, policy, prior, inventory):
    checks = [
        ('native-only', lambda r: r.__setitem__('native_only', True)),
        ('missing corpus', lambda r: r.__setitem__('pixel_rows', [])),
        ('missing page', lambda r: r['pixel_rows'].pop()),
        ('duplicate page', lambda r: r['pixel_rows'].append(r['pixel_rows'][0])),
        ('missing generated case', lambda r: r['generated_rows'].pop()),
        ('missing generated raster', lambda r: next(x for x in r['generated_rows'] if x.get('pixels'))['pixels'].pop()),
        ('missing census', lambda r: r['census_rows'].pop()),
        ('changed prior pixels', lambda r: next(x for x in r['pixel_rows'] if x['file'] == 'large.pdf').__setitem__('rust_ppm_sha256', 'tampered')),
        ('missing pixel model', lambda r: next(p for x in r['generated_rows'] for p in x.get('pixels', []) if 'model_comparison' in p).pop('model_comparison')),
        ('sample violation', lambda r: next(p for x in r['generated_rows'] for p in x.get('pixels', []) if 'model_comparison' in p)['model_comparison'].__setitem__('channel_interval_violations', 1)),
    ]
    rejected = []
    for label, change in checks:
        altered = copy.deepcopy(report)
        change(altered)
        try:
            verify(altered, validation, policy, prior, inventory, False)
        except ValueError:
            rejected.append(label)
        else:
            raise AssertionError('accepted incomplete/tampered report: ' + label)
    return rejected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--validation', type=Path, required=True)
    parser.add_argument('--engine', type=Path, default=ENGINE)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    inputs = [args.report, args.validation, args.engine / 'reference/form-reference-policy-f0.json',
              args.engine / 'reference/stroke-raster-differential-s1.json',
              args.engine / 'reference/fixture-inventory.json']
    values = [json.loads(p.read_text()) for p in inputs]
    result = verify(*values)
    result['rejected_report_mutations'] = mutation_checks(*values)
    result['input_sha256'] = {str(p): hashlib.sha256(p.read_bytes()).hexdigest()
                              for p in inputs + [Path(__file__)]}
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
