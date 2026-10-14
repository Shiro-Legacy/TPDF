#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Independent exact polygon/Gaussian model for R23a numeric sampling only."""
import argparse
import hashlib
import json
import math
import random
import struct
import subprocess
from collections import Counter
from fractions import Fraction as F
from pathlib import Path

CAP = 1 << 21
EPSILON = F(1, 1 << 512)


def expected(matrix, width, height, sx, sy):
    if not all(math.isfinite(v) for v in matrix):
        return 'refused'
    m = tuple(F(v) for v in matrix)
    if any(abs(v) > CAP or (v * (1 << 80)).denominator != 1 for v in m):
        return 'refused'
    if not (1 <= width <= 65536 and 1 <= height <= 65536):
        return 'refused'
    a, b, c, d, e, f = m
    points = [(e, f), (a + e, b + f), (a + c + e, b + d + f), (c + e, d + f)]
    if any(abs(v) > CAP for point in points for v in point):
        return 'refused'
    # Gaussian row elimination, deliberately separate from production determinants.
    if a:
        pivot = d - b * c / a
    else:
        pivot = b if c else F(0)
    if not pivot or sx >= 1 << 23 or sy >= 1 << 23:
        return 'refused'
    x, y = F(2 * sx + 1, 8), F(2 * sy + 1, 8)
    # Lexicographic symbolic perturbation realized exactly in this bounded domain.
    # Nonzero original edge predicates are >=2^-160; perturbations are <2^-489.
    # At a boundary, -dy*epsilon dominates dx*epsilon^2 unless dy==0.
    px, py = x + EPSILON, y + EPSILON * EPSILON
    inside = False
    for (x0, y0), (x1, y1) in zip(points, points[1:] + points[:1]):
        if (y0 > py) != (y1 > py):
            intercept = x0 + (py - y0) * (x1 - x0) / (y1 - y0)
            if px < intercept:
                inside = not inside
    if not inside:
        return 'outside'
    dx, dy = x - e, y - f
    if a:
        v = (dy - b * dx / a) / pivot
        u = (dx - c * v) / a
    else:
        v = dx / c
        u = (dy - d * v) / b
    assert 0 <= u <= 1 and 0 <= v <= 1
    column, row = int(width * u // 1), int(height * (1 - v) // 1)
    if column == width:
        assert u == 1
        column -= 1
    if row == height:
        assert v == 0
        row -= 1
    assert 0 <= row < height and 0 <= column < width
    return f'{row} {column}'


def cases():
    matrices = [
        [1, 0, 0, 1, 0, 0], [-1, 0, 0, 1, 1.125, .125],
        [1, 0, 0, -1, .125, 1.125], [0, 1, -1, 0, 1.125, .125],
        [0, -1, 1, 0, .125, 1.125], [1, 0, 1, 1, .125, .125],
        [2, 1, -1, 3, .125, .125], [-2, 1, 1, 3, .125, .125],
        [1, 3, 15, 45, 0, 0], [0] * 6,
        [2**-80, 0, 0, 2**-80, .125, .125],
        [2**-81, 0, 0, 1, 0, 0], [2**-1074, 0, 0, 1, 0, 0],
        [1, 0, 0, 1, CAP - 1, 0], [1, 0, 0, 1, CAP, 0],
        [1, 0, 0, 1, -CAP, -CAP], [CAP, 0, 0, CAP, 0, 0],
        [CAP, CAP, -CAP, CAP, 0, -CAP],
        [float('nan'), 0, 0, 1, 0, 0], [float('inf'), 0, 0, 1, 0, 0],
    ]
    delta = 2**-52
    for e, f in [(0.125, 0.125), (-.875 + 2**-53, -.875 - 2**-53)]:
        matrices.append([1, 1 + delta, 1 - delta, 1, e, f])
    for i, m in enumerate(matrices):
        for w, h in [(1, 1), (2, 2), (3, 7), (65536, 65536)]:
            for x in [0, 1, 2, 3, 4, 7, (1 << 23) - 1, 1 << 23]:
                for y in [0, 2, 4]:
                    yield f'admission-{i}', list(map(float, m)), w, h, x, y
    for w, h in [(0, 1), (1, 0), (65537, 1), (1, 65537)]:
        yield 'invalid-dimensions', [1., 0., 0., 1., 0., 0.], w, h, 0, 0
    # Place exact transformed vertices/edges/texel boundaries at a sample center,
    # then move each translation one f64 neighbor in both directions.
    for m in matrices[:8]:
        a, b, c, d = map(F, m[:4])
        for u in [F(0), F(1, 4), F(1, 2), F(1)]:
            for v in [F(0), F(1, 4), F(1, 2), F(1)]:
                e, f = float(F(1, 8) - a * u - c * v), float(F(1, 8) - b * u - d * v)
                for axis in [4, 5]:
                    for direction in [None, -math.inf, math.inf]:
                        matrix = list(map(float, m[:4])) + [e, f]
                        if direction is not None:
                            matrix[axis] = math.nextafter(matrix[axis], direction)
                        yield 'ties-and-neighbors', matrix, 4, 4, 0, 0
    rng = random.Random(23006)
    for _ in range(450):
        power = rng.choice([0, 1, 3, 8, 20, 40, 79, 80, 81])
        m = [rng.randint(-32, 32) * 2.**(-power) for _ in range(4)]
        m += [rng.randint(-16, 16) / 8, rng.randint(-16, 16) / 8]
        w, h = rng.choice([(2, 2), (3, 7), (180, 18), (200, 300), (65536, 1)])
        for x in range(4):
            for y in range(4):
                yield 'random-dyadic', m, w, h, x, y


def compare(lines, expected_lines):
    if len(lines) != len(expected_lines):
        raise ValueError('sample count mismatch')
    for index, (actual, want) in enumerate(zip(lines, expected_lines)):
        if actual != want:
            raise ValueError(f'sample {index}: {actual!r} != {want!r}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--probe', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--engine', type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    sources = [Path(__file__), args.probe, args.engine / 'src/image_sampling.rs',
               args.engine / 'src/geometry.rs', args.engine / 'tests/image_sampling.rs',
               args.engine / 'examples/image_sampling_probe.rs', args.engine / 'R23-SAMPLING-DESIGN.md',
               args.engine / 'Cargo.lock']
    digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
    hashes = {str(p): digest(p) for p in sources}
    suite = list(cases())
    lines, answers = [], []
    for _, matrix, w, h, x, y in suite:
        encoded = [struct.pack('>d', v).hex() for v in matrix]
        lines.append(' '.join(encoded + list(map(str, (w, h, x, y)))))
        answers.append(expected(matrix, w, h, x, y))
    encoded = ('\n'.join(lines) + '\n').encode()
    process = subprocess.run([str(args.probe)], input=encoded, capture_output=True, timeout=90)
    if process.returncode or process.stderr:
        raise ValueError('probe failure: ' + process.stderr.decode())
    actual = process.stdout.decode().splitlines()
    compare(actual, answers)
    rejects = []
    numeric = next(i for i, v in enumerate(answers) if v not in ('outside', 'refused'))
    outside = answers.index('outside')
    for label, altered in [('omitted sample', actual[:-1]), ('extra sample', actual + ['outside'])]:
        try:
            compare(altered, answers)
        except ValueError:
            rejects.append(label)
        else:
            raise AssertionError('mutation escaped: ' + label)
    for label, index, value in [('wrong row/column', numeric, '99999 99999'),
                                ('clamped outside', outside, '0 0'),
                                ('silent refusal', numeric, 'refused')]:
        altered = actual.copy()
        altered[index] = value
        try:
            compare(altered, answers)
        except ValueError:
            rejects.append(label)
        else:
            raise AssertionError('mutation escaped: ' + label)
    assert hashes == {str(p): digest(p) for p in sources}
    result = dict(status='passed', scope='numeric image sampling only; no PDF decode/render claim',
                  cases=len(suite), groups=dict(Counter(c[0] for c in suite)),
                  outcomes=dict(Counter('selected' if ' ' in v else v for v in answers)),
                  input_sha256=hashlib.sha256(encoded).hexdigest(),
                  output_sha256=hashlib.sha256(process.stdout).hexdigest(),
                  rejected_mutations=rejects, evidence_sha256=hashes)
    args.report.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({k: v for k, v in result.items() if k not in ('groups', 'evidence_sha256')}))


if __name__ == '__main__':
    main()
