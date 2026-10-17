#!/usr/bin/env python3
"""Compare the safe Rust lexer with the pinned MuPDF lexer on bounded inputs.

Uses only the Python standard library. Each process has a timeout. Valid PDF
syntax must match exactly except for explicitly reported f32/f64 real rounding.
Strict malformed-input refusals are checked separately, never counted as parity.
"""
import argparse
import json
import math
import random
import subprocess
import sys
from pathlib import Path


def invoke(binary, data):
    command = [str(binary)]
    if binary.name.startswith("pdf-inspect"):
        command.append("--tokens")
    result = subprocess.run(command, input=data, capture_output=True, timeout=5)
    if result.returncode < 0:
        raise AssertionError(f"{binary.name} terminated by signal {-result.returncode}")
    return result.returncode, result.stdout.decode("ascii"), result.stderr.decode("utf-8", "replace")


def compare(left, right):
    a, b = left.splitlines(), right.splitlines()
    if len(a) != len(b):
        return False, 0
    rounded = 0
    for x, y in zip(a, b):
        if x == y:
            continue
        if x.startswith("real\t") and y.startswith("real\t"):
            xv, yv = float(x[5:]), float(y[5:])
            if math.isfinite(xv) and math.isfinite(yv) and math.isclose(xv, yv, rel_tol=2e-6, abs_tol=1e-7):
                rounded += 1
                continue
        return False, rounded
    return True, rounded


def cases(seed, count):
    yield b""
    yield b"\x00\t\n\f\r % comment\r\n"
    yield b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>"
    yield b"true false null obj endobj stream endstream xref trailer startxref R"
    yield b"(a\\n\\r\\t\\b\\f\\(\\)\\\\z) (a\r\nb\rc\nd) (a\\\r\nb\\\rc\\\nd)"
    yield b"(\\0\\00\\000\\377\\400\\777) (nested (parentheses))"
    yield b"<00ff> <a> <a b c> <> / /A#20B /#ff#fe /#23 /#2f"
    yield b"0 -0 +0 9223372036854775807 -9223372036854775808 1. -.5 +.5 .0 0012"
    rng = random.Random(seed)
    whitespace = [b" ", b"\n", b"\r\n", b"\x00", b"\t", b"\f", b"%c\r\n"]
    keywords = [b"q", b"Q", b"cm", b"BT", b"ET", b"Tf", b"Td", b"Tj", b"TJ", b"true", b"false", b"null", b"R"]
    for _ in range(count):
        pieces = []
        for _ in range(rng.randint(1, 50)):
            choice = rng.randrange(7)
            if choice == 0:
                pieces.append(str(rng.randint(-2**40, 2**40)).encode())
            elif choice == 1:
                pieces.append(f"{rng.randint(-9999, 9999)}.{rng.randrange(100000):05d}".encode())
            elif choice == 2:
                raw = bytes(rng.randrange(1, 256) for _ in range(rng.randrange(24)))
                pieces.append(b"/" + b"".join(f"#{byte:02x}".encode() for byte in raw))
            elif choice == 3:
                raw = bytes(rng.randrange(256) for _ in range(rng.randrange(24)))
                pieces.append(b"<" + raw.hex().encode() + b">")
            elif choice == 4:
                raw = bytes(rng.randrange(256) for _ in range(rng.randrange(24)))
                pieces.append(b"(" + b"".join(f"\\{byte:03o}".encode() for byte in raw) + b")")
            elif choice == 5:
                pieces.append(rng.choice(keywords))
            else:
                pieces.append(rng.choice([b"[", b"]", b"<<", b">>"]))
            pieces.append(rng.choice(whitespace))
        yield b"".join(pieces)


STRICT_CASES = [
    (b"(unterminated", "unterminated literal string"),
    (b"<12", "unterminated hex string"),
    (b"<gg>", "invalid hex digit"),
    (b"/bad#xz", "invalid name escape"),
    (b"/bad#00", "null name escape"),
    (b"9223372036854775808", "integer overflow"),
    (b"--1", "multiple numeric signs"),
    (b"1.2.3", "multiple decimal points"),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust", type=Path, default=Path("target/debug/pdf-inspect"))
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--cases", type=int, default=1000)
    parser.add_argument("--seed", type=int, default=20261003)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    if not 0 <= args.cases <= 100000:
        parser.error("--cases must be between 0 and 100000")
    rust, reference = args.rust.resolve(), args.reference.resolve()
    failures, strict, total, rounded = [], [], 0, 0
    for i, data in enumerate(cases(args.seed, args.cases)):
        total += 1
        rr, ro, re = invoke(rust, data)
        cr, co, ce = invoke(reference, data)
        equal, rounding = compare(ro, co)
        rounded += rounding
        if rr or cr or not equal:
            failures.append({"case": i, "input_hex": data.hex(), "rust_code": rr,
                "reference_code": cr, "rust": ro, "reference": co, "rust_error": re, "reference_error": ce})
            if len(failures) >= 20:
                break
    for data, reason in STRICT_CASES:
        rr, _, re = invoke(rust, data)
        cr, _, ce = invoke(reference, data)
        strict.append({"reason": reason, "rust_rejected": rr == 2,
            "reference_code": cr, "reference_error": ce})
        if rr != 2:
            failures.append({"strict_case": reason, "input_hex": data.hex(), "rust_code": rr, "rust_error": re})
    report = {"seed": args.seed, "valid_cases": total, "real_rounding_differences": rounded,
        "real_comparison": "finite, rel_tol=2e-6, abs_tol=1e-7 (MuPDF f32 vs Rust f64)",
        "strict_refusals": strict, "failures": failures, "passed": not failures}
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({k: v for k, v in report.items() if k != "failures"}, indent=2))
    if failures:
        print(json.dumps(failures[:3], indent=2), file=sys.stderr)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
