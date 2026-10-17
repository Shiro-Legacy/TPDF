#!/usr/bin/env python3
"""Read-only comparison of Rust page geometry with pinned MuPDF inventory."""
import argparse
import hashlib
import json
import math
import subprocess
import sys
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rust', type=Path, default=Path('target/debug/pdf-inspect'))
    parser.add_argument('--fixtures', type=Path, default=Path('../fixtures'))
    parser.add_argument('--inventory', type=Path, default=Path('reference/fixture-inventory.json'))
    parser.add_argument('--report', type=Path, default=Path('reference/fixture-comparison-r3.json'))
    parser.add_argument('--minimum-supported', type=int, default=37)
    args = parser.parse_args()
    golden = json.loads(args.inventory.read_text())
    if golden['reference'] != 'MuPDF 1.28.1 WASM':
        raise ValueError('unexpected reference version')
    results = []
    for expected in golden['documents']:
        path = args.fixtures / expected['file']
        if hashlib.sha256(path.read_bytes()).hexdigest() != expected['sha256']:
            raise ValueError(f'fixture changed: {expected["file"]}')
        run = subprocess.run([str(args.rust.resolve()), '--document', str(path)], capture_output=True, timeout=10)
        row = {'file': expected['file']}
        if run.returncode:
            error = run.stderr.decode('utf-8', 'replace').strip()
            row.update(status='unsupported' if run.returncode == 2 and 'unsupported' in error else 'failure', error=error)
        else:
            actual = json.loads(run.stdout)
            failures = []
            if expected.get('passwordRequired'):
                failures.append('encrypted fixture unexpectedly accepted without authentication')
            elif len(actual['pages']) != expected['pages']:
                failures.append(f'page count {len(actual["pages"])} != {expected["pages"]}')
            else:
                for i, (page, want) in enumerate(zip(actual['pages'], expected['pageDetails'])):
                    if len(page['bounds']) != 4 or not all(math.isfinite(a) and math.isclose(a, b, rel_tol=1e-6, abs_tol=1e-4) for a, b in zip(page['bounds'], want['bounds'])):
                        failures.append(f'page {i} bounds {page["bounds"]} != {want["bounds"]}')
            row.update(status='failure' if failures else 'matched', pages=len(actual['pages']))
            if failures: row['errors'] = failures
        results.append(row)
    matched = [r for r in results if r['status'] == 'matched']
    summary = {'reference': golden['reference'], 'fixtures': len(results), 'matched': len(matched),
        'matched_pages': sum(r['pages'] for r in matched),
        'unsupported': sum(r['status'] == 'unsupported' for r in results),
        'failures': sum(r['status'] == 'failure' for r in results), 'minimum_supported': args.minimum_supported,
        'passed': not any(r['status'] == 'failure' for r in results) and len(matched) >= args.minimum_supported,
        'results': results}
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps({k: v for k, v in summary.items() if k != 'results'}, indent=2))
    for row in results:
        if row['status'] != 'matched': print(json.dumps(row), file=sys.stderr)
    return 0 if summary['passed'] else 1


if __name__ == '__main__':
    sys.exit(main())
