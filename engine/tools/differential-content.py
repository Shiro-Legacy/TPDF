#!/usr/bin/env python3
"""Independent content grammar grouping over pinned native MuPDF lexer tokens.

Checks operation ordering, operand counts, and recursively typed operand values.
The Rust CLI's canonical operands are re-lexed by MuPDF rather than by Rust.
This proves syntax/value agreement for the bounded subset, not operator semantics.
"""
import argparse
import hashlib
import json
import math
import random
import subprocess
from pathlib import Path

RESERVED = {b'R', b'obj', b'endobj', b'stream', b'endstream', b'xref', b'trailer', b'startxref', b'newobj', b'BI', b'ID', b'EI'}
SEPARATOR = b'__reference_operation_separator__'


def invoke(command, data):
    run = subprocess.run(command, input=data, capture_output=True, timeout=15)
    return run.returncode, run.stdout, run.stderr.decode(errors='replace')[:1000]


def native_tokens(oracle, data):
    status, output, error = invoke([str(oracle)], data)
    if status:
        raise ValueError('native lexer refusal: ' + error)
    result = []
    for line in output.splitlines():
        fields = line.split(b'\t')
        kind = fields[0].decode()
        if kind == 'integer': value = int(fields[1])
        elif kind == 'real': value = float(fields[1])
        elif kind in ('keyword', 'name', 'string'): value = bytes.fromhex(fields[1].decode())
        else: value = None
        result.append((kind, value))
    return result


def value(tokens, index):
    if index >= len(tokens): raise ValueError('missing operand')
    kind, payload = tokens[index]
    index += 1
    if kind in ('integer', 'real', 'name', 'string'):
        return (kind, payload), index
    if kind == 'keyword' and payload in (b'true', b'false', b'null'):
        return (payload.decode(), None), index
    if kind == 'array-start':
        items = []
        while index < len(tokens) and tokens[index][0] != 'array-end':
            item, index = value(tokens, index)
            items.append(item)
        if index == len(tokens): raise ValueError('unterminated array')
        return ('array', items), index + 1
    if kind == 'dict-start':
        items = {}
        while index < len(tokens) and tokens[index][0] != 'dict-end':
            if tokens[index][0] != 'name': raise ValueError('dictionary key is not a name')
            key = tokens[index][1]
            if key in items: raise ValueError('duplicate dictionary key')
            item, index = value(tokens, index + 1)
            items[key] = item
        if index == len(tokens): raise ValueError('unterminated dictionary')
        return ('dictionary', items), index + 1
    raise ValueError('illegal operand token ' + repr((kind, payload)))


def operations(tokens):
    index, pending, output = 0, [], []
    while index < len(tokens):
        kind, payload = tokens[index]
        if kind == 'keyword' and payload not in (b'true', b'false', b'null'):
            if payload in RESERVED: raise ValueError('structural/inline-image keyword ' + repr(payload))
            output.append((payload, pending))
            pending = []
            index += 1
        else:
            item, index = value(tokens, index)
            pending.append(item)
    if pending: raise ValueError('pending operands at EOF')
    return output


def equal(a, b):
    if type(a) is not type(b): return False
    if isinstance(a, float): return math.isclose(a, b, rel_tol=2e-6, abs_tol=1e-7)
    if isinstance(a, (list, tuple)): return len(a) == len(b) and all(equal(x, y) for x, y in zip(a, b))
    if isinstance(a, dict): return a.keys() == b.keys() and all(equal(a[k], b[k]) for k in a)
    return a == b


def compare(oracle, rust, data):
    expected = operations(native_tokens(oracle, data))
    status, output, error = invoke([str(rust), '--content', '-'], data)
    if status: raise ValueError('Rust content refusal: ' + error)
    records, canonical = [], []
    for line in output.splitlines():
        fields = line.split(b'\t')
        if len(fields) != 3: raise ValueError('invalid CLI record')
        records.append((bytes.fromhex(fields[0].decode()), int(fields[1])))
        canonical.append(bytes.fromhex(fields[2].decode()) + b' ' + SEPARATOR + b'\n')
    actual_values = operations(native_tokens(oracle, b''.join(canonical)))
    if len(expected) != len(records) or len(records) != len(actual_values):
        raise ValueError('operation count mismatch')
    for index, ((operator, operands), (actual_operator, count), (marker, actual)) in enumerate(zip(expected, records, actual_values)):
        if marker != SEPARATOR or actual_operator != operator or count != len(operands) or not equal(operands, actual):
            raise ValueError(f'operation {index} mismatch: expected {(operator, operands)!r}; actual {(actual_operator, count, actual)!r}')
    return len(expected)


def name(data):
    return b'/' + b''.join(bytes([byte]) if 33 <= byte <= 126 and byte not in b'()<>[]{}/%#' else f'#{byte:02x}'.encode() for byte in data)


def operand(rng, depth=0):
    kind = rng.randrange(6 if depth >= 3 else 8)
    if kind == 0: return str(rng.randrange(-100000, 100001)).encode()
    if kind == 1: return f'{rng.randrange(-100000, 100001) / 4:.2f}'.encode()
    if kind == 2: return rng.choice([b'true', b'false', b'null'])
    if kind == 3: return name(bytes(rng.randrange(1, 256) for _ in range(rng.randrange(12))))
    if kind == 4:
        data = bytes(rng.randrange(256) for _ in range(rng.randrange(24)))
        return b'<' + data.hex().encode() + b'>'
    if kind == 5:
        data = bytes(rng.choice(b'abcXYZ()\\ \t\n\r\0\xff') for _ in range(rng.randrange(24)))
        return b'(' + b''.join(f'\\{byte:03o}'.encode() if byte < 33 or byte > 126 or byte in b'()\\' else bytes([byte]) for byte in data) + b')'
    if kind == 6: return b'[' + b' '.join(operand(rng, depth + 1) for _ in range(rng.randrange(5))) + b']'
    keys = rng.sample([b'Z', b'A', b'key with spaces', b'#', b'\xff', b'/'], rng.randrange(5))
    return b'<< ' + b' '.join(name(key) + b' ' + operand(rng, depth + 1) for key in keys) + b' >>'


def generated(seed, count):
    rng = random.Random(seed)
    operators = [b'q', b'Q', b'cm', b'TJ', b'Tj', b'BDC', b'Ext.Custom', b"'", b'"', b'f*', b'XOperator']
    for case in range(count):
        parts = []
        for _ in range(rng.randrange(1, 25)):
            operands = [operand(rng) for _ in range(rng.randrange(7))]
            parts.append(b' '.join(operands + [rng.choice(operators)]))
        yield f'generated-{case}', b'\n%comment\r\n'.join(parts)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rust', type=Path, default=Path('target/debug/pdf-inspect'))
    parser.add_argument('--oracle', type=Path, default=Path('/private/tmp/tandem-mupdf-reference/mupdf-lex-oracle'))
    parser.add_argument('--seed', type=int, default=20261003)
    parser.add_argument('--cases', type=int, default=300)
    parser.add_argument('--inventory', type=Path)
    parser.add_argument('--corpus-dir', type=Path)
    parser.add_argument('--report', type=Path, default=Path('reference/content-differential-r6a.json'))
    args = parser.parse_args()
    if not 1 <= args.cases <= 10000: parser.error('--cases must be1..10000')
    if bool(args.inventory) != bool(args.corpus_dir): parser.error('inventory and corpus-dir must be supplied together')
    oracle, rust = args.oracle.resolve(), args.rust.resolve()
    binary_hashes = {'rust_binary_sha256': hashlib.sha256(rust.read_bytes()).hexdigest(), 'reference_binary_sha256': hashlib.sha256(oracle.read_bytes()).hexdigest()}
    compare(oracle, rust, b'q')  # Fail fast on missing/outdated CLI or oracle ABI.
    cases = list(generated(args.seed, args.cases))
    inline = []
    if args.inventory:
        for row in json.loads(args.inventory.read_text())['results']:
            path = args.corpus_dir / row['savedFile']
            data = path.read_bytes()
            if hashlib.sha256(data).hexdigest() != row['sha256']: raise ValueError('corpus hash changed')
            if row['complete']: cases.append((row['savedFile'], data))
            else: inline.append((row['savedFile'], data))
    results = []
    for label, data in cases:
        row = {'case': label, 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
        try: row.update(status='matched', operations=compare(oracle, rust, data))
        except (ValueError, subprocess.TimeoutExpired) as error: row.update(status='failure', error=str(error)[:2000])
        results.append(row)
    malformed = [b'1 0 R', b'[1 0 R] op', b'<< /A 1 /A 2 >> op', b'1', b'[q] op', b'<< /A q >> op', b'BI\x00\xffbinary', b'ID', b'EI', b'endstream', b'q [1', b'q (BI) Tj BI \xffbinary']
    refusals = []
    for label, data in [*[(f'malformed-{n}', data) for n, data in enumerate(malformed)], *inline]:
        status, output, error = invoke([str(rust), '--content', '-'], data)
        refusals.append({'case': label, 'passed': status != 0 and not output, 'diagnostic': error})
    report = {'reference': 'MuPDF1.28.1 lexer plus independent Python content grouping', 'seed': args.seed, 'generated_cases': args.cases, 'valid_cases': len(results), 'matched': sum(r['status'] == 'matched' for r in results), 'operations': sum(r.get('operations', 0) for r in results), 'failures': sum(r['status'] != 'matched' for r in results) + sum(not r['passed'] for r in refusals), 'strict_refusals': refusals, 'limitations': ['Syntax and operand values only; no operator arity, resources, rendering, or text semantics.', 'Real operands compare native32-bit values with relative tolerance2e-6 / absolute1e-7.', 'Inline images are refused; binary payloads are never scanned by the native lexer.'], 'results': results}
    if binary_hashes['rust_binary_sha256'] != hashlib.sha256(rust.read_bytes()).hexdigest() or binary_hashes['reference_binary_sha256'] != hashlib.sha256(oracle.read_bytes()).hexdigest():
        raise RuntimeError('a comparison binary changed during the run; rerun after builds finish')
    report.update(binary_hashes)
    args.report.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: v for k, v in report.items() if k not in ('results', 'strict_refusals')}, indent=2))
    for row in [row for row in results if row['status'] != 'matched'][:10]:
        print(row)
    raise SystemExit(bool(report['failures']))


if __name__ == '__main__': main()
