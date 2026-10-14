#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Deterministic O0 mutation smoke checks; not coverage-guided fuzzing or a security audit."""
import argparse, collections, hashlib, json, random, subprocess, time
from pathlib import Path
from outline_model import cases
ROOT = Path(__file__).resolve().parents[1]
def sha(data): return hashlib.sha256(data).hexdigest()
def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--rust', type=Path, default=ROOT/'target/debug/pdf-inspect')
    p.add_argument('--cases', type=int, default=2400)
    p.add_argument('--report', type=Path, default=ROOT/'reference/outline-mutation-o0.json')
    a = p.parse_args()
    if not 1 <= a.cases <= 10000: p.error('cases must be in 1..10000')
    binary = a.rust.resolve(); before = sha(binary.read_bytes()); rng = random.Random(0x0F0715)
    seeds = [data for data, _ in cases()][:40]
    inv = json.loads((ROOT/'reference/embedded-outline-inventory.json').read_text())
    for i, row in enumerate(inv['fonts']):
        assert sha((ROOT.parent/'fixtures'/row['file']).read_bytes()) == row['fixtureSha256']
        data = Path(f'/private/tmp/outline-fonts/font{i}.ttf').read_bytes()
        assert sha(data) == row['programs'][0]['sha256']; seeds.append(data)
    counts = collections.Counter(); corpus = hashlib.sha256(); start = time.monotonic(); largest = 0
    for i in range(a.cases):
        original = seeds[rng.randrange(len(seeds))]; data = bytearray(original); mode = i % 6
        if mode == 0:
            for _ in range(rng.randrange(1, 9)): data[rng.randrange(len(data))] ^= 1 << rng.randrange(8)
        elif mode == 1: del data[rng.randrange(len(data)):]
        elif mode == 2:
            at = rng.randrange(len(data)); data[at:at+rng.randrange(1, 17)] = rng.randbytes(rng.randrange(1, 17))
        elif mode == 3:
            at = rng.randrange(len(data)); data[at:at] = rng.randbytes(rng.randrange(1, 25))
        elif mode == 4:
            at = rng.randrange(len(data)); data[at:at+rng.randrange(1, 25)] = bytes(rng.randrange(1, 25))
        else:
            for _ in range(rng.randrange(1, 5)):
                at = rng.randrange(len(data)); data[at:at+2] = rng.choice([b'\xff\xff', b'\x7f\xff', b'\x80\x00', b'\x00\x00'])
        data = bytes(data); corpus.update(len(data).to_bytes(8, 'big')); corpus.update(data)
        proc = subprocess.run([str(binary), '--outline=all'], input=data, capture_output=True, timeout=8)
        assert proc.returncode in (0, 2), (i, proc.returncode, proc.stderr[:200])
        assert b'panicked' not in proc.stderr, (i, proc.stderr)
        largest = max(largest, len(proc.stdout))
        if proc.returncode:
            assert not proc.stdout, (i, 'partial output'); assert proc.stderr
            counts['refused'] += 1
        else:
            assert not proc.stderr, (i, proc.stderr)
            result = json.loads(proc.stdout); assert len(result['glyphs']) == result['glyph_count']
            for gid, glyph in enumerate(result['glyphs']):
                assert glyph['gid'] == gid
                assert len(glyph['points']) <= 32768 and len(glyph['contour_endpoints']) <= 4096
                assert all(-32768 <= x <= 32767 and -32768 <= y <= 32767 and type(on) is bool for x, y, on in glyph['points'])
                assert all(c[0] in ('m', 'l', 'q', 'h') for c in glyph['commands'])
            assert result['outline_usage']['decode_attempts'] == result['glyph_count']
            counts['accepted'] += 1
    assert before == sha(binary.read_bytes()), 'binary changed during run'
    report = dict(purpose=__doc__, seed=0x0F0715, cases=a.cases, outcomes=dict(counts), seed_count=len(seeds), corpus_sha256=corpus.hexdigest(), binary_sha256=before, largest_stdout_bytes=largest, crashes=0, timeouts=0, partial_outputs=0, seconds=round(time.monotonic()-start, 3))
    a.report.write_text(json.dumps(report, indent=2)+'\n'); print(json.dumps(report, indent=2))
if __name__ == '__main__': main()
