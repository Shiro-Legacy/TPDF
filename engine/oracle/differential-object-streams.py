#!/usr/bin/env python3
"""Deterministic object-stream PDFs checked against pinned native MuPDF without repair."""
import argparse
import hashlib
import json
import random
import subprocess
import zlib
from pathlib import Path


def generate(seed):
    rng = random.Random(seed)
    page_count = 1 + seed % 5
    compressed = bool(seed & 1)
    groups_count = 1 + seed % 3
    members = [(1, b'<< /Type /Catalog /Pages 2 0 R >>')]
    kids = b' '.join(f'{3+i} 0 R'.encode() for i in range(page_count))
    members.append((2, b'<< /Type /Pages /Count ' + str(page_count).encode() + b' /Kids [' + kids + b'] >>'))
    for i in range(page_count):
        width, height = rng.randrange(30, 800), rng.randrange(30, 1000)
        rotation = [0, 90, 180, 270][(seed + i) % 4]
        members.append((3+i, f'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Rotate {rotation} /Contents 50 0 R >>'.encode()))
    members.append((80, b'(unreferenced auxiliary member)'))
    groups = [members[i::groups_count] for i in range(groups_count)]
    data = bytearray(b'%PDF-1.7\n%\x80\x81\x82\x83\n')
    entries = [[0, 0, 0] for _ in range(201)]
    entries[0] = [0, 0, 65535]
    entries[50] = [1, len(data), 0]
    data += b'50 0 obj\n<< /Length 3 >>\nstream\nq Q\nendstream\nendobj\n'
    for group_index, group in enumerate(groups):
        container = 100 + group_index
        body = bytearray(b' \n' if seed % 4 == 0 else b'')
        header = bytearray()
        for member_index, (number, value) in enumerate(group):
            header += f'{number} {len(body)} '.encode()
            body += value + b'\n'
            entries[number] = [2, container, member_index]
        header += b'% header boundary\n' if seed % 7 == 0 else b''
        decoded = bytes(header + body)
        payload = zlib.compress(decoded, [1, 6, 9][seed % 3]) if compressed else decoded
        entries[container] = [1, len(data), 0]
        filter_entry = b' /Filter /FlateDecode' if compressed else b''
        data += f'{container} 0 obj\n<< /Type /ObjStm /N {len(group)} /First {len(header)} /Length {len(payload)}'.encode() + filter_entry + b' >>\nstream\n'
        data += payload + b'\nendstream\nendobj\n'
    xref = len(data)
    entries[200] = [1, xref, 0]
    rows = b''.join(bytes([kind]) + first.to_bytes(4, 'big') + second.to_bytes(2, 'big') for kind, first, second in entries)
    payload = zlib.compress(rows) if seed % 3 else rows
    filter_entry = b' /Filter /FlateDecode' if seed % 3 else b''
    data += f'200 0 obj\n<< /Type /XRef /Size 201 /Root 1 0 R /W [1 4 2] /Length {len(payload)}'.encode() + filter_entry + b' >>\nstream\n'
    data += payload + f'\nendstream\nendobj\nstartxref\n{xref}\n%%EOF\n'.encode()
    if seed % 4 == 1:
        replacement = len(data)
        data += b'3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 123 456] /Contents 50 0 R >>\nendobj\n'
        latest = len(data)
        data += f'xref\n3 1\n{replacement:010} 00000 n \ntrailer\n<< /Size 201 /Root 1 0 R /Prev {xref} >>\nstartxref\n{latest}\n%%EOF\n'.encode()
    elif seed % 4 == 2:
        latest = len(data)
        data += f'xref\n80 1\n0000000000 00001 f \ntrailer\n<< /Size 201 /Root 1 0 R /Prev {xref} >>\nstartxref\n{latest}\n%%EOF\n'.encode()
    return bytes(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rust', type=Path, default=Path('target/debug/pdf-inspect'))
    parser.add_argument('--oracle', type=Path, default=Path('/private/tmp/tandem-mupdf-reference/mupdf-document-oracle'))
    parser.add_argument('--cases', type=int, default=96)
    parser.add_argument('--report', type=Path, default=Path('reference/object-stream-differential.json'))
    parser.add_argument('--save-dir', type=Path)
    args = parser.parse_args()
    results = []
    if args.save_dir:
        args.save_dir.mkdir(parents=True, exist_ok=True)
    for seed in range(args.cases):
        data = generate(seed)
        if args.save_dir:
            (args.save_dir / f'object-stream-{seed:03}.pdf').write_bytes(data)
        expected_run = subprocess.run([str(args.oracle.resolve())], input=data, capture_output=True, timeout=10)
        actual_run = subprocess.run([str(args.rust.resolve()), '--document', '-'], input=data, capture_output=True, timeout=10)
        row = {'seed': seed, 'sha256': hashlib.sha256(data).hexdigest()}
        try:
            if expected_run.returncode or actual_run.returncode:
                raise ValueError(f'oracle={expected_run.returncode} rust={actual_run.returncode}: ' + expected_run.stderr.decode(errors='replace')[-500:] + actual_run.stderr.decode(errors='replace')[-500:])
            expected = json.loads(expected_run.stdout)
            actual = json.loads(actual_run.stdout)
            bounds = [page['bounds'] for page in actual['pages']]
            if expected['repaired']:
                raise ValueError('MuPDF repaired the generated PDF')
            if expected['pages'] != bounds:
                raise ValueError(f'page bounds differ: oracle={expected["pages"]} rust={bounds}')
            row.update(status='matched', pages=len(bounds), repaired=False)
        except (ValueError, KeyError) as error:
            row.update(status='failure', error=str(error))
        results.append(row)
    summary = {'reference': 'MuPDF 1.28.1 native', 'cases': len(results), 'matched': sum(row['status'] == 'matched' for row in results), 'matched_pages': sum(row.get('pages', 0) for row in results), 'failures': sum(row['status'] != 'matched' for row in results), 'results': results}
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps({key: value for key, value in summary.items() if key != 'results'}, indent=2))
    for row in results:
        if row['status'] != 'matched':
            print(json.dumps(row))
    return 0 if summary['failures'] == 0 else 1


if __name__ == '__main__':
    raise SystemExit(main())
