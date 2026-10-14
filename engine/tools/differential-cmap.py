#!/usr/bin/env python3
"""Compare strict Rust ToUnicode maps with independent Python mappings and MuPDF.

Generated expectations use standard-library UTF16 decoding and explicit source
bytes. Native widthless lookup and malformed-input recovery are reported apart
from valid parity, never used to invent expected Unicode values.
"""
import argparse
import hashlib
import importlib.util
import json
import random
import re
import subprocess
import tempfile
from pathlib import Path


def utf16(scalars): return ''.join(chr(value) for value in scalars).encode('utf-16-be')
def scalars(data): return [ord(character) for character in data.decode('utf-16-be')]
def hex_string(data): return b'<' + data.hex().encode() + b'>'
def source(value, width): return value.to_bytes(width, 'big')
def literal(data): return b'(' + b''.join(f'\\{byte:03o}'.encode() for byte in data) + b')'


def make_program(spaces, sections, wrapped=False):
    output = bytearray(b'/CIDInit /ProcSet findresource begin\n12 dict begin\n' if wrapped else b'')
    output += b'begincmap\n/CMapName /Independent-Test def\n/CMapType 2 def\n/WMode 0 def\n'
    output += f'{len(spaces)} begincodespacerange\n'.encode()
    output += b'\n'.join(hex_string(low) + b' ' + hex_string(high) for low, high in spaces)
    output += b'\nendcodespacerange\n'
    for kind, records in sections:
        output += f'{len(records)} begin{kind}\n'.encode() + b'\n'.join(records) + f'\nend{kind}\n'.encode()
    output += b'endcmap\n'
    if wrapped: output += b'CMapName currentdict /CMap defineresource pop\nend\nend\n'
    return bytes(output)


def generate(seed, count):
    rng = random.Random(seed)
    unicode_examples = [[0], [0xfeff], [0xffff], [0x10ffff], [0x1f600], [65, 0x301], [102, 102, 105], [0x1f600, 65], [65, 0x1f600], [0x1f600, 0x10ffff], list(range(64))]
    for case in range(count):
        width = case % 4 + 1
        maximum = (1 << (width * 8)) - 1
        spaces = [(source(0, width), source(maximum, width))]
        mapping, sections = {}, []
        if case % 3 == 0:
            values = {0, maximum, min(maximum, 0x80), min(maximum, 0x7fffffff), min(maximum, 0x80000000)}
            desired = 100 if case % 12 == 0 else rng.randrange(8, 30)
            while len(values) < desired: values.add(rng.randrange(maximum + 1))
            records = []
            for index, value in enumerate(sorted(values)):
                code = source(value, width)
                target = unicode_examples[(case + index) % len(unicode_examples)]
                mapping[code] = target
                encode = literal if index % 7 == 0 else hex_string
                records.append(encode(code) + b' ' + encode(utf16(target)))
            sections.append(('bfchar', records))
        else:
            start = rng.randrange(0, min(maximum, 65535) - 20)
            entries = rng.randrange(1, 16)
            lo, hi = source(start, width), source(start + entries - 1, width)
            if case % 3 == 1:
                base = rng.choice([[0x41], [0], [0xfeff], [0x1f600], [0x1f600, 0x41], [0x41, 0x1f600]])
                encoded = utf16(base)
                final = int.from_bytes(encoded[-2:], 'big')
                for delta in range(entries): mapping[source(start + delta, width)] = scalars(encoded[:-2] + (final + delta).to_bytes(2, 'big'))
                destination = hex_string(encoded)
            else:
                destinations = []
                for delta in range(entries):
                    target = unicode_examples[(case + delta) % len(unicode_examples)]
                    mapping[source(start + delta, width)] = target
                    destinations.append(hex_string(utf16(target)))
                destination = b'[' + b' '.join(destinations) + b']'
            sections.append(('bfrange', [hex_string(lo) + b' ' + hex_string(hi) + b' ' + destination]))
            sections.append(('bfchar', [hex_string(source(maximum, width)) + b' <0041>']))
            mapping[source(maximum, width)] = [65]
        sections += [('bfchar', []), ('bfrange', [])]
        yield f'generated-{case}', make_program(spaces, sections, bool(case % 2)), spaces, mapping


def decode_expected(data, spaces, mapping):
    for width in range(1, min(len(data), 4) + 1):
        code = data[:width]
        if any(len(low) == width and low <= code <= high for low, high in spaces):
            return {'width': width, 'value': int.from_bytes(code, 'big'), 'consumed': width, 'scalars': mapping.get(code)}
    for low, high in spaces:
        if len(data) < len(low) and low[:len(data)] <= data <= high[:len(data)]: return {'error': 'Incomplete'}
    return {'error': 'Invalid'}


def queries_for(spaces, mapping):
    queries = [('lookup', code) for code in sorted(mapping)] + [('decode', code) for code in sorted(mapping)]
    for low, high in spaces:
        for code in (low, high, low[:-1], high[:-1]): queries.append(('decode', code))
        for code in (low, high): queries.append(('lookup', code))
        if len(low) < 4: queries.append(('decode', low + b'\xff' * (4-len(low))))
    queries += [('decode', b''), ('decode', b'\xff'), ('decode', b'\0'), ('decode', b'\0\0\0\0')]
    return list(dict.fromkeys(queries))


def invoke(binary, query_file, program, rust=False):
    command = [str(binary), f'--cmap={query_file}', '-'] if rust else [str(binary), str(query_file)]
    result = subprocess.run(command, input=program, capture_output=True, timeout=20)
    if result.returncode: return result.returncode, None, result.stderr.decode(errors='replace')[:2000], result.stdout
    try: output = json.loads(result.stdout)
    except ValueError as error: raise ValueError('invalid query JSON: ' + result.stdout[:1000].decode(errors='replace')) from error
    return 0, output, result.stderr.decode(errors='replace')[:2000], result.stdout


def run_case(rust, native, program, spaces, mapping, directory, compare_native=True):
    queries = queries_for(spaces, mapping)
    query_file = directory / 'queries.txt'
    query_file.write_text(''.join(kind + ' ' + (code.hex() or '-') + '\n' for kind, code in queries))
    status, actual, error, _ = invoke(rust, query_file, program, True)
    if status: raise ValueError('Rust refusal: ' + error)
    status, reference, error, _ = invoke(native, query_file, program)
    if status: raise ValueError('native refusal: ' + error)
    if len(actual['queries']) != len(queries) or len(reference['queries']) != len(queries): raise ValueError('query count mismatch')
    fallback, differences = [], []
    for (kind, code), result, native_result in zip(queries, actual['queries'], reference['queries']):
        expected = {'width': len(code), 'value': int.from_bytes(code, 'big'), 'scalars': mapping.get(code)} if kind == 'lookup' else decode_expected(code, spaces, mapping)
        if result.get('kind') != kind or result.get('input') != code.hex(): raise ValueError('Rust query identity mismatch')
        if native_result.get('kind') != kind or native_result.get('input') != code.hex(): raise ValueError('native query identity mismatch')
        if 'error' in expected:
            if result.get('error') != expected['error']: raise ValueError(f'decode classification {code.hex()}: {expected} != {result}')
            if native_result.get('matched_codespace') is not False: raise ValueError('native unexpected matching code space')
            fallback.append({'input': code.hex(), 'strict_error': expected['error'], 'native': native_result})
            continue
        for key, value in expected.items():
            if result.get(key) != value: raise ValueError(f'Rust {kind} {code.hex()} {key}: {result.get(key)!r} != {value!r}')
        if result.get('valid_scalars') is not True: raise ValueError('Rust invalid scalars on success')
        if kind == 'decode' and result.get('matched_codespace') is not True: raise ValueError('Rust successful decode must match codespace')
        for key, value in expected.items():
            if native_result.get(key) != value:
                if compare_native: raise ValueError(f'native {kind} {code.hex()} {key}: {native_result.get(key)!r} != {value!r}')
                differences.append({'kind': kind, 'input': code.hex(), 'expected': expected, 'native': native_result})
                break
    return {'queries': len(queries), 'mapped_entries': len(mapping), 'native_fallbacks': fallback, 'native_widthless_differences': differences}


def fixture_mapping(lexer, data):
    spec = importlib.util.spec_from_file_location('content_reference', Path(__file__).with_name('differential-content.py'))
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    tokens = module.native_tokens(lexer, data)
    spaces, mapping, index = [], {}, 0
    def string(at):
        if tokens[at][0] != 'string': raise ValueError('expected fixture string')
        return tokens[at][1]
    while index < len(tokens):
        kind, word = tokens[index]
        if kind != 'keyword' or word not in (b'begincodespacerange', b'beginbfchar', b'beginbfrange'):
            index += 1; continue
        count = tokens[index-1][1]
        index += 1
        for _ in range(count):
            low = string(index); index += 1
            if word == b'begincodespacerange': spaces.append((low, string(index))); index += 1
            elif word == b'beginbfchar': mapping[low] = scalars(string(index)); index += 1
            else:
                high = string(index); index += 1
                size = int.from_bytes(high, 'big') - int.from_bytes(low, 'big') + 1
                if tokens[index][0] == 'array-start':
                    index += 1
                    destinations = [string(index+offset) for offset in range(size)]; index += size
                    if tokens[index][0] != 'array-end': raise ValueError('fixture range array size')
                    index += 1
                else:
                    base = string(index); index += 1
                    final = int.from_bytes(base[-2:], 'big')
                    destinations = [base[:-2] + (final+offset).to_bytes(2, 'big') for offset in range(size)]
                for offset, destination in enumerate(destinations): mapping[source(int.from_bytes(low, 'big')+offset, len(low))] = scalars(destination)
        if tokens[index] != ('keyword', word.replace(b'begin', b'end', 1)): raise ValueError('fixture terminator mismatch')
        index += 1
    return spaces, mapping


def malformed_programs():
    spaces = [(b'\0', b'\xff')]
    for destination in [b'<>', b'<00>', b'<004100>', b'<d800>', b'<dc00>', b'<d8000041>', b'<0041dc00>', b'<d800d800>', b'<dc00dc00>']:
        yield 'invalid-destination-' + destination.decode(), make_program(spaces, [('bfchar', [b'<01> ' + destination])])
    yield 'duplicate-bfchar', make_program(spaces, [('bfchar', [b'<01> <0041>', b'<01> <0042>'])])
    yield 'range-overlap', make_program(spaces, [('bfrange', [b'<01> <03> <0041>', b'<03> <05> <0044>'])])
    yield 'range-reverse', make_program(spaces, [('bfrange', [b'<03> <01> <0041>'])])
    yield 'surrogate-crossing', make_program(spaces, [('bfrange', [b'<01> <02> <d7ff>'])])
    yield 'low-surrogate-overflow', make_program(spaces, [('bfrange', [b'<01> <02> <d800dfff>'])])
    yield 'codeunit-overflow', make_program(spaces, [('bfrange', [b'<01> <02> <ffff>'])])
    yield 'short-array', make_program(spaces, [('bfrange', [b'<01> <03> [<0041> <0042>]'])])
    yield 'long-array', make_program(spaces, [('bfrange', [b'<01> <02> [<0041> <0042> <0043>]'])])
    yield 'mixed-source-widths', make_program(spaces, [('bfrange', [b'<01> <0002> <0041>'])])
    yield 'undeclared-source', make_program([(b'\x10', b'\x20')], [('bfchar', [b'<01> <0041>'])])
    yield 'prefix-ambiguous', make_program([(b'\0', b'\x10'), (b'\x10\0', b'\x10\xff')], [])
    yield 'codespace-overlap', make_program([(b'\0', b'\x10'), (b'\x10', b'\x20')], [])
    yield 'source-range-above-u0', make_program([(b'\0\0\0', b'\xff\xff\xff')], [('bfrange', [b'<010000> <010001> <0041>'])])
    base = make_program(spaces, [('bfchar', [b'<01> <0041>'])])
    for label, before, after in [('count-short', b'1 beginbfchar', b'0 beginbfchar'), ('count-long', b'1 beginbfchar', b'2 beginbfchar'), ('keyword-prefix', b'beginbfchar', b'beginbfcharGarbage'), ('section-101', b'1 beginbfchar', b'101 beginbfchar'), ('unknown-execution', b'endcmap', b'evil endcmap'), ('usecmap', b'endcmap', b'/Base usecmap endcmap')]:
        yield label, base.replace(before, after)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rust', type=Path, default=Path('target/debug/pdf-inspect'))
    parser.add_argument('--native', type=Path, default=Path('/private/tmp/tandem-mupdf-reference/mupdf-cmap-oracle'))
    parser.add_argument('--lexer', type=Path, default=Path('/private/tmp/tandem-mupdf-reference/mupdf-lex-oracle'))
    parser.add_argument('--cases', type=int, default=160)
    parser.add_argument('--seed', type=int, default=20261003)
    parser.add_argument('--inventory', type=Path, default=Path('reference/cmap-inventory.json'))
    parser.add_argument('--corpus-dir', type=Path, default=Path('/private/tmp/tandem-cmap-inventory'))
    parser.add_argument('--report', type=Path, default=Path('reference/cmap-differential-u0.json'))
    args = parser.parse_args()
    if not 1 <= args.cases <= 10000: parser.error('cases must be1..10000')
    rust, native, lexer = args.rust.resolve(), args.native.resolve(), args.lexer.resolve()
    hashes = {str(path): hashlib.sha256(path.read_bytes()).hexdigest() for path in (rust, native, lexer)}
    results, malformed, recovery = [], [], []
    with tempfile.TemporaryDirectory(prefix='tandem-cmap-differential-') as temporary:
        directory = Path(temporary)
        cases = list(generate(args.seed, args.cases))
        first = cases[0]
        run_case(rust, native, *first[1:], directory)  # Fail fast on ABI drift.
        for label, program, spaces, mapping in cases:
            row = {'case': label, 'sha256': hashlib.sha256(program).hexdigest()}
            try: row.update(status='matched', **run_case(rust, native, program, spaces, mapping, directory))
            except (ValueError, subprocess.TimeoutExpired) as error: row.update(status='failure', error=str(error)[:2000])
            results.append(row)
        spaces = [(b'\x80', b'\xff'), (b'\0\x80', b'\0\xff')]
        mapping = {b'\x80': [65], b'\0\x80': [66]}
        program = make_program(spaces, [('bfchar', [b'<80> <0041>', b'<0080> <0042>'])])
        width_case = run_case(rust, native, program, spaces, mapping, directory, False)
        for row in json.loads(args.inventory.read_text())['cmaps']:
            program = (args.corpus_dir / row['savedFile']).read_bytes()
            if hashlib.sha256(program).hexdigest() != row['sha256']: raise ValueError('fixture CMap hash changed')
            if not row['invalidUtf16Destinations']:
                spaces, mapping = fixture_mapping(lexer, program)
                result = {'case': row['savedFile'], 'file': row['file'], 'id': row['id'], 'sha256': row['sha256']}
                try: result.update(status='matched', **run_case(rust, native, program, spaces, mapping, directory))
                except (ValueError, subprocess.TimeoutExpired) as error: result.update(status='failure', error=str(error)[:2000])
                results.append(result)
            else:
                queries, expected_recovery = [], []
                invalid_bytes = {bytes.fromhex(item['destinationHex']) for item in row['invalidDestinationExamples']}
                invalid_offsets = []
                for match in re.finditer(rb'<([0-9a-fA-F\s]+)>', program):
                    digits = re.sub(rb'\s+', b'', match.group(1))
                    if len(digits) % 2: digits += b'0'
                    if bytes.fromhex(digits.decode()) in invalid_bytes: invalid_offsets.append(match.start())
                for invalid in row['invalidDestinationExamples']:
                    low, high, width = (invalid['source'][key] for key in ('lo','hi','width'))
                    queries += [('lookup', source(value, width)) for value in range(low, high+1)]
                    raw = bytes.fromhex(invalid['destinationHex'])
                    truncated = raw[:len(raw)//2*2]
                    final = int.from_bytes(truncated[-2:], 'big')
                    expected_recovery += [scalars(truncated[:-2] + (final + delta).to_bytes(2, 'big')) for delta in range(high-low+1)]
                query_file = directory/'queries.txt';query_file.write_text(''.join(kind+' '+code.hex()+'\n' for kind,code in queries))
                rust_status, _, error, output = invoke(rust, query_file, program, True)
                native_status, native_result, native_error, _ = invoke(native, query_file, program)
                error_offset = re.search(r'byte (\d+)', error)
                offset_matches = bool(error_offset) and int(error_offset.group(1)) in invalid_offsets
                recovery_matches = native_status == 0 and [item['scalars'] for item in native_result['queries']] == expected_recovery
                recovery.append({'destination_offsets': invalid_offsets, 'strict_offset_matches': offset_matches, 'expected_native_recovery': expected_recovery, 'native_recovery_matches': recovery_matches, 'file': row['file'], 'id': row['id'], 'strict_refused': rust_status != 0 and not output, 'strict_diagnostic': error, 'offending_destinations': row['invalidDestinationExamples'], 'native_status': native_status, 'native_recovery': native_result, 'native_diagnostic': native_error})
        query_file = directory/'queries.txt';query_file.write_text('lookup 01\n')
        for label, program in malformed_programs():
            status, _, error, output = invoke(rust, query_file, program, True)
            malformed.append({'case': label, 'passed': status != 0 and not output, 'diagnostic': error})
    if any(hashlib.sha256(Path(path).read_bytes()).hexdigest() != digest for path,digest in hashes.items()): raise RuntimeError('comparison binary changed during run')
    failures = sum(row['status'] != 'matched' for row in results) + sum(not row['passed'] for row in malformed) + sum(not row['strict_refused'] or not row['strict_offset_matches'] or not row['native_recovery_matches'] for row in recovery)
    report = {'reference': 'MuPDF 1.28.1 plus independent Python Unicode/source-byte expectations', 'seed': args.seed, 'binary_sha256': hashes, 'valid_cases': len(results), 'matched': sum(row['status']=='matched' for row in results), 'mapped_entries': sum(row.get('mapped_entries',0) for row in results), 'queries': sum(row.get('queries',0) for row in results), 'malformed_generated': malformed, 'actual_malformed_recovery': recovery, 'native_widthless_lookup_limitation': width_case, 'failures': failures, 'results': results, 'limitations': ['Standalone strict ToUnicode syntax and lookup only; no font loading, glyph decoding, fallback, text extraction, or rendering.', 'Native numeric lookup collapses source widths; typed collision case is checked against independent expectations and recorded separately.', 'Native invalid code segmentation fallback is recorded separately from strict Invalid/Incomplete errors.']}
    args.report.write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps({key:value for key,value in report.items() if key not in ('results','malformed_generated','actual_malformed_recovery','native_widthless_lookup_limitation')}, indent=2))
    for row in [row for row in results if row['status'] != 'matched'][:10]: print(row)
    raise SystemExit(bool(failures))


if __name__ == '__main__': main()
