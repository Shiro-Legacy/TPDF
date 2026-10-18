#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Verify M1 exact glyph metrics, encoding selection and immutable fixture bindings."""
import argparse
import hashlib
import json
import random
import subprocess
from pathlib import Path


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def name(value):
    return b'/' + b''.join(bytes([v]) if 33 <= v <= 126 and v not in b'()<>[]{}/%#' else f'#{v:02x}'.encode() for v in value)


def program(base, differences=None):
    if differences is None:
        return name(base.encode())
    return b'<< /Type /Encoding /BaseEncoding ' + name(base.encode()) + b' /Differences [' + b' '.join(str(v).encode() if isinstance(v, int) else name(v) for v in differences) + b'] >>'


def slots(table, widths, differences=()):
    glyphs = [None if row['glyph'] is None else row['glyph'].encode('ascii') for row in table]
    cursor = None
    for item in differences:
        if isinstance(item, int):
            cursor = item
        else:
            assert cursor is not None and cursor < 256
            glyphs[cursor] = item
            cursor += 1
    result = []
    for code, glyph in enumerate(glyphs):
        undefined = glyph in (None, b'.notdef')
        value = None if undefined else widths.get(glyph)
        result.append({'code': code, 'glyph_name_hex': None if glyph is None else glyph.hex(), 'kind': 'undefined' if undefined else 'missing' if value is None else 'advance', 'advance': value})
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rust', type=Path, default=Path('target/debug/pdf-inspect'))
    parser.add_argument('--oracle', type=Path, default=Path('/private/tmp/tandem-mupdf-reference/mupdf-builtin-metrics-oracle'))
    parser.add_argument('--fixtures', type=Path, default=Path('../fixtures'))
    parser.add_argument('--report', type=Path, default=Path('reference/builtin-metrics-differential-m1.json'))
    args = parser.parse_args()
    rust = args.rust.resolve()
    evidence = [Path('reference') / f for f in ['builtin-metrics-parity.json', 'encoding-table-inventory.json', 'fixture-inventory.json', 'simple-font-inventory.json', 'font-metrics-inventory.json']]
    parity, encodings, fixtures, resources, metrics = [json.loads(p.read_text()) for p in evidence]
    paths = [rust, args.oracle, *evidence]
    hashes = {str(p): digest(p) for p in paths}
    if digest(args.oracle) != parity['oracleSha256']:
        raise ValueError('native oracle hash differs from pinned inventory')
    widths = {}
    exclusions = {}
    fresh_queries = 0
    for face in parity['faces']:
        rows = face['rows']
        native = subprocess.run([str(args.oracle), face['baseFont']], input=('\n'.join(r['name'] for r in rows) + '\n').encode('ascii'), capture_output=True, timeout=15)
        if native.returncode or native.stderr:
            raise ValueError('fresh native reference query failed')
        lines = native.stdout.decode('ascii').splitlines()
        if len(lines) != len(rows):
            raise ValueError('native reference row count differs')
        for row, line in zip(rows, lines):
            glyph, found, gid, units, raw, integer = line.split('\t')
            if (glyph, int(found), int(gid), int(units), int(raw), int(integer)) != (row['name'], int(row['found']), row['gid'], row['unitsPerEm'], row['rawAdvance'], row['pdfIntegerWidth']):
                raise ValueError('native reference changed')
        fresh_queries += len(rows)
        widths[face['baseFont']] = {r['name'].encode('ascii'): r['pdfIntegerWidth'] for r in rows if r['found'] and r['afmWidth'] is not None}
        exclusions[face['baseFont']] = [r['name'].encode('ascii') for r in rows if not r['found'] and r['afmWidth'] is not None]
        if len(widths[face['baseFont']]) != 654:
            raise ValueError('wrong shared AFM/CFF subset size')

    def invoke(mode, data):
        run = subprocess.run([str(rust), mode, '-'], input=data, capture_output=True, timeout=30)
        if run.returncode or run.stderr:
            raise ValueError('unexpected CLI refusal: ' + run.stderr.decode(errors='replace')[:1200])
        return json.loads(run.stdout)

    seed = 2026100312
    rng = random.Random(seed)
    cases = []
    named_combinations = 0
    checked_tables = set()
    table_slots = 0
    for face, values in widths.items():
        full = [{'name_hex': glyph.hex(), 'advance': value} for glyph, value in sorted(values.items())]
        pool = sorted(set(values) | {b'.notdef', b'zerosuperior', b'H18533', b'DefinitelyMissing', b'A.alt', b'A/', b'\xff'})
        for base, table in encodings['tables'].items():
            scenarios = [None, [0, b'.notdef', b'zerosuperior', b'H18533', b'DefinitelyMissing', b'fi', b'\xff', 255, b'A'], [65, b'i', 65, b'A', 65, b'fi']]
            scenarios.append([0, *exclusions[face]])
            for _ in range(20):
                diff = []
                for _ in range(rng.randrange(1, 8)):
                    start = rng.randrange(256)
                    diff.append(start)
                    diff.extend(rng.choices(pool, k=rng.randrange(1, min(9, 257-start))))
                scenarios.append(diff)
            for differences in scenarios:
                result = invoke('--builtin=' + face, program(base, differences))
                wanted = {'face': face, 'glyphs': full, 'slots': slots(table, values, differences or ())}
                if result != wanted:
                    raise ValueError(f'builtin table/slot mismatch {face} {base} {differences!r}')
                named_combinations += differences is None
                checked_tables.add(face)
                table_slots += 256
                cases.append({'face': face, 'encoding': base, 'differences_items': len(differences or ())})
    refusals = [
        ('--builtin=Arial', b'/WinAnsiEncoding'),
        ('--builtin=ABCDEF+Helvetica', b'/WinAnsiEncoding'),
        ('--builtin=helvetica', b'/WinAnsiEncoding'),
    ]
    bad = [b'', b'/Unknown', b'null', b'[]', b'<<>>', b'<< /BaseEncoding null >>', b'<< /BaseEncoding 3 0 R >>', b'<< /BaseEncoding /WinAnsiEncoding /Type /Wrong >>', b'<< /BaseEncoding /WinAnsiEncoding /Extra true >>', b'<< /BaseEncoding /WinAnsiEncoding /Differences (x) >>', b'<< /BaseEncoding /WinAnsiEncoding /Differences [/A] >>', b'<< /BaseEncoding /WinAnsiEncoding /Differences [-1 /A] >>', b'<< /BaseEncoding /WinAnsiEncoding /Differences [256 /A] >>', b'<< /BaseEncoding /WinAnsiEncoding /Differences [255 /A /B] >>', b'<< /BaseEncoding /WinAnsiEncoding /Differences [65 1.5] >>', b'<< /BaseEncoding /WinAnsiEncoding /Differences [65 /A#00] >>', b'/WinAnsiEncoding trailing']
    refusals += [('--builtin=' + face, data) for face in widths for data in bad]
    for mode, data in refusals:
        run = subprocess.run([str(rust), mode, '-'], input=data, capture_output=True, timeout=10)
        if run.returncode <= 0 or run.stdout or not run.stderr or b'panicked' in run.stderr:
            raise ValueError('non-atomic or missing strict refusal')

    font_meta = {(r['file'], r['id']): r for r in resources['fonts']}
    metric_meta = {(r['file'], r['id']): r for r in metrics['fonts']}
    pages = {(r['file'], r['page']): r for r in resources['pageResources']}
    registered = {(r['file'], f['id']) for r in resources['pageResources'] for f in r['fonts'] if f['subtype'] != 'Type0'}
    expected = {key for key in registered if metric_meta[key]['widthsKind'] == 'absent'}
    seen = set()
    explicit = set()
    fixture_rows = []
    bindings = page_count = type0 = explicit_bindings = fixture_slots = loaded = 0
    for record in fixtures['documents']:
        data = (args.fixtures / record['file']).read_bytes()
        if hashlib.sha256(data).hexdigest() != record['sha256']:
            raise ValueError('immutable fixture hash mismatch: ' + record['file'])
        if record.get('passwordRequired'):
            continue
        result = invoke('--builtin-metrics', data)
        if len(result['pages']) != record['pages']:
            raise ValueError('fixture page count differs')
        actual_fonts = {f['id']: f for f in result['fonts']}
        if len(actual_fonts) != len(result['fonts']):
            raise ValueError('duplicate font cache identity')
        for page in result['pages']:
            want = pages[(record['file'], page['page'])]
            got = {bytes.fromhex(b['name_hex']).decode('utf-8'): b for b in page['bindings']}
            if set(got) != {f['name'] for f in want['fonts']}:
                raise ValueError('fixture binding names differ')
            page_count += 1
            for font in want['fonts']:
                binding = got[font['name']]
                key = (record['file'], font['id'])
                bindings += 1
                if binding['target'] != [int(v) for v in font['id'].split(':')]:
                    raise ValueError('fixture binding target differs')
                if font['subtype'] == 'Type0':
                    type0 += 1
                    if binding['status'] != 'unsupported':
                        raise ValueError('Type0 refusal missing')
                elif metric_meta[key]['widthsKind'] != 'absent':
                    explicit.add(key)
                    explicit_bindings += 1
                    if binding['status'] != 'unsupported':
                        raise ValueError('explicit widths incorrectly fell back')
                else:
                    if binding['status'] != 'loaded' or binding['font'] != 'i:' + font['id'] or binding['font'] not in actual_fonts:
                        raise ValueError('eligible builtin binding failed')
                    loaded += 1
        for ident, font in actual_fonts.items():
            key = (record['file'], ident[2:])
            if not ident.startswith('i:') or key not in expected:
                raise ValueError('unexpected builtin identity')
            metadata = font_meta[key]
            face = metadata['baseFont']
            if metadata['encodingKind'] != 'name' or metadata['encodingName'] not in encodings['tables']:
                raise ValueError('fixture expectation needs explicit extension')
            wanted = slots(encodings['tables'][metadata['encodingName']], widths[face])
            if font['face'] != face or font['slots'] != wanted:
                raise ValueError('fixture exact advance/name/outcome differs')
            fixture_slots += 256
            seen.add(key)
        if result['usage']['builtin_metrics'] != len(actual_fonts) or result['usage']['retained_builtin_slots'] != 256 * len(actual_fonts):
            raise ValueError('retained builtin usage differs')
        fixture_rows.append({'file': record['file'], 'sha256': record['sha256'], 'pages': len(result['pages']), 'fonts': len(actual_fonts), 'status': 'matched'})
    if seen != expected or len(seen) != 48 or len(explicit) != 5 or type0 != 7:
        raise ValueError('fixture coverage differs from accepted scope')
    if hashes != {str(p): digest(p) for p in paths}:
        raise ValueError('binary or reference evidence changed during run')
    report = {'reference': 'Pinned native exact CFF glyph-name queries and independently inventoried PDF encodings/resources/explicit widths', 'seed': seed, 'evidence_sha256': hashes, 'fresh_native_queries': fresh_queries, 'excluded_afm_names_tested': sum(map(len, exclusions.values())), 'full_table_entries': sum(len(widths[f]) for f in checked_tables), 'named_face_encoding_combinations': named_combinations, 'standalone_cases': len(cases), 'standalone_slot_outcomes': table_slots, 'strict_cli_refusals': len(refusals), 'fixture_documents': len(fixture_rows), 'fixture_pages': page_count, 'fixture_bindings': bindings, 'loaded_bindings': loaded, 'builtin_font_identities': len(seen), 'fixture_slot_outcomes': fixture_slots, 'explicit_width_fonts_refused': len(explicit), 'explicit_width_bindings_refused': explicit_bindings, 'type0_bindings_refused': type0, 'failures': 0, 'cases': cases, 'fixture_rows': fixture_rows, 'explicit_width_fonts': [{'file': f, 'id': i} for f, i in sorted(explicit)], 'limitations': ['Exact AFM/CFF intersection only; excluded AFM names remain missing even when AFM alone supplies a width.', 'Undefined byte slots do not use native glyph-zero advances.', 'No font program parsing, implicit encoding, arbitrary Base14 aliases or text positioning claimed.']}
    args.report.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: v for k, v in report.items() if isinstance(v, int)}))


if __name__ == '__main__':
    main()
