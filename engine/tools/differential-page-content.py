#!/usr/bin/env python3
"""Compare exact logical page-content bytes against pinned MuPDF's concat API.

Each native invocation opens the PDF and calls pdf_open_contents_stream; this is
independent of the Python fixture builder and Rust joining implementation.
"""
import argparse
import hashlib
import json
import subprocess
from pathlib import Path


def fixture(parts, array):
    data = bytearray(b'%PDF-1.7\n')
    references = b' '.join(f'{index+4} 0 R'.encode() for index in range(len(parts)))
    contents = b'[' + references + b']' if array else references
    values = [b'<< /Type /Catalog /Pages 2 0 R >>', b'<< /Type /Pages /Count 1 /Kids [3 0 R] >>', b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 20] /Contents ' + contents + b' >>']
    values += [f'<< /Length {len(part)} >>\nstream\n'.encode() + part + b'\nendstream' for part in parts]
    offsets = []
    for number, value in enumerate(values, 1):
        offsets.append(len(data))
        data += f'{number} 0 obj\n'.encode() + value + b'\nendobj\n'
    xref = len(data)
    data += f'xref\n0 {len(values)+1}\n0000000000 65535 f \n'.encode()
    for offset in offsets: data += f'{offset:010} 00000 n \n'.encode()
    data += f'trailer\n<< /Size {len(values)+1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode()
    return bytes(data)


def compare(rust, oracle, data, page, expected=None):
    native = subprocess.run([str(oracle), str(page)], input=data, capture_output=True, timeout=15)
    actual = subprocess.run([str(rust), f'--page-content={page}', '-'], input=data, capture_output=True, timeout=15)
    if native.returncode or actual.returncode:
        raise ValueError(f'native={native.returncode} rust={actual.returncode}: ' + native.stderr.decode(errors='replace')[-1000:] + actual.stderr.decode(errors='replace')[-1000:])
    if native.stdout != actual.stdout: raise ValueError(f'joined bytes differ: native={len(native.stdout)} rust={len(actual.stdout)}')
    if expected is not None and native.stdout != expected: raise ValueError('native output differs from independent expected boundary vector')
    return {'bytes': len(actual.stdout), 'sha256': hashlib.sha256(actual.stdout).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rust', type=Path, default=Path('target/debug/pdf-inspect'))
    parser.add_argument('--oracle', type=Path, default=Path('/private/tmp/tandem-mupdf-reference/mupdf-content-oracle'))
    parser.add_argument('--fixtures', type=Path, default=Path('../fixtures'))
    parser.add_argument('--inventory', type=Path, default=Path('reference/content-inventory.json'))
    parser.add_argument('--fixture-inventory', type=Path, default=Path('reference/fixture-inventory.json'))
    parser.add_argument('--report', type=Path, default=Path('reference/page-content-differential.json'))
    args = parser.parse_args()
    rust, oracle = args.rust.resolve(), args.oracle.resolve()
    binary_hashes = {'rust_binary_sha256': hashlib.sha256(rust.read_bytes()).hexdigest(), 'reference_binary_sha256': hashlib.sha256(oracle.read_bytes()).hexdigest()}
    boundary_cases = [([b'q'], False), ([b'q'], True), ([b'', b'q', b''], True), ([b'%comment', b'q\nQ'], True), ([b'(A', b'B) Tj'], True), ([b'1', b'2 cm'], True), ([], True)]
    compare(rust, oracle, fixture(*boundary_cases[0]), 1, b'q')  # CLI preflight.
    results = []
    for index, (parts, array) in enumerate(boundary_cases):
        row = {'case': f'boundary-{index}', 'page': 1}
        expected = b''.join(part + b' ' for part in parts) if array else parts[0]
        try: row.update(status='matched', **compare(rust, oracle, fixture(parts, array), 1, expected))
        except (ValueError, subprocess.TimeoutExpired) as error: row.update(status='failure', error=str(error))
        results.append(row)
    inventory = json.loads(args.inventory.read_text())
    fixture_hashes = {row['file']: row['sha256'] for row in json.loads(args.fixture_inventory.read_text())['documents']}
    current_file, data = None, None
    for page in inventory['pages']:
        if page['file'] != current_file:
            current_file = page['file']
            data = (args.fixtures / current_file).read_bytes()
            if hashlib.sha256(data).hexdigest() != fixture_hashes[current_file]: raise ValueError('fixture hash changed: ' + current_file)
        row = {'file': current_file, 'page': page['page'] + 1, 'references': len(page['contents'])}
        try: row.update(status='matched', **compare(rust, oracle, data, page['page'] + 1))
        except (ValueError, subprocess.TimeoutExpired) as error: row.update(status='failure', error=str(error))
        results.append(row)
    report = {'reference': 'MuPDF1.28.1 native pdf_open_contents_stream; repaired inputs refused', 'generated_boundary_cases': len(boundary_cases), 'fixture_pages': len(inventory['pages']), 'fixture_documents': len({page['file'] for page in inventory['pages']}), 'pages_with_multiple_references': sum(len(page['contents']) > 1 for page in inventory['pages']), 'matched': sum(row['status'] == 'matched' for row in results), 'matched_bytes': sum(row.get('bytes', 0) for row in results), 'failures': sum(row['status'] != 'matched' for row in results), 'limitations': ['Compares exact decoded logical bytes and native joining, not execution or rendering.', 'Inline-image data remains opaque bytes in this loader comparison; content grammar separately refuses BI.'], 'results': results}
    if binary_hashes['rust_binary_sha256'] != hashlib.sha256(rust.read_bytes()).hexdigest() or binary_hashes['reference_binary_sha256'] != hashlib.sha256(oracle.read_bytes()).hexdigest():
        raise RuntimeError('a comparison binary changed during the run; rerun after builds finish')
    report.update(binary_hashes)
    args.report.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({key: value for key, value in report.items() if key != 'results'}, indent=2))
    for row in [row for row in results if row['status'] != 'matched'][:10]: print(row)
    raise SystemExit(bool(report['failures']))


if __name__ == '__main__': main()
