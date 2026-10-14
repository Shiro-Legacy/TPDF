#!/usr/bin/env python3
"""Validate bounded Rust PDF predictors against independent encoders and MuPDF.

TIFF rows are generated as sample arrays, differenced per channel, and packed
MSB-first. PNG rows are independently filtered in Python. Both decoders must
reproduce the original bytes exactly. Malformed/dimension cases are separate.
"""
import argparse
import collections
import datetime
import hashlib
import json
import random
import subprocess
import sys
import zlib
from pathlib import Path

ENGINE = Path(__file__).resolve().parent.parent


def pack(samples, bits):
    output = bytearray()
    accumulator = occupied = 0
    for value in samples:
        accumulator = (accumulator << bits) | value
        occupied += bits
        while occupied >= 8:
            occupied -= 8
            output.append((accumulator >> occupied) & 255)
        accumulator &= (1 << occupied) - 1
    if occupied:
        output.append(accumulator << (8 - occupied))
    return bytes(output)


def paeth(left, above, upper_left):
    estimate = left + above - upper_left
    return min((left, above, upper_left), key=lambda value: abs(estimate - value))


def generate(seed, count):
    rng = random.Random(seed)
    algorithms = (2, 10, 11, 12, 13, 14, 15)
    depths = (1, 2, 4, 8, 16)
    for index in range(count):
        predictor = algorithms[index % len(algorithms)]
        bits = depths[(index // len(algorithms)) % len(depths)]
        colors = rng.choice((1, 2, 3, 4, 8, 32))
        columns = rng.choice((1, 2, 3, 7, 8, 17, 31, 64))
        rows = index % 6
        original, encoded = bytearray(), bytearray()
        samples = colors * columns
        row_bytes = (samples * bits + 7) // 8
        bpp = (colors * bits + 7) // 8
        previous = bytes(row_bytes)
        for row in range(rows):
            if predictor == 2:
                modulus = 1 << bits
                raw = [rng.randrange(modulus) for _ in range(samples)]
                differences = [(value - (raw[i - colors] if i >= colors else 0)) % modulus
                               for i, value in enumerate(raw)]
                original.extend(pack(raw, bits))
                encoded.extend(pack(differences, bits))
            else:
                raw = bytes(rng.randrange(256) for _ in range(row_bytes))
                # Fixed predictors and mixed actual row prefixes both occur.
                mode = predictor - 10 if predictor < 15 and index % 2 == 0 else (row + index) % 5
                encoded.append(mode)
                for column, value in enumerate(raw):
                    left = raw[column - bpp] if column >= bpp else 0
                    above = previous[column]
                    upper_left = previous[column - bpp] if column >= bpp else 0
                    prediction = (0, left, above, (left + above) // 2, paeth(left, above, upper_left))[mode]
                    encoded.append((value - prediction) % 256)
                original.extend(raw)
                previous = raw
        yield {"case": index, "predictor": predictor, "colors": colors, "bits": bits,
               "columns": columns, "rows": rows}, bytes(encoded), bytes(original)


def document(compressed, params):
    metadata = (f"/Filter /FlateDecode /DecodeParms << /Predictor {params['predictor']} "
                f"/Colors {params['colors']} /BitsPerComponent {params['bits']} /Columns {params['columns']} >>")
    stream = f"<< /Length {len(compressed)} {metadata} >>\nstream\n".encode() + compressed + b"\nendstream"
    objects = (b"<< /Type /Catalog >>", stream)
    data = bytearray(b"%PDF-1.7\n")
    offsets = []
    for number, body in enumerate(objects, 1):
        offsets.append(len(data))
        data.extend(f"{number} 0 obj\n".encode() + body + b"\nendobj\n")
    xref = len(data)
    data.extend(b"xref\n0 3\n0000000000 65535 f \n")
    for offset in offsets:
        data.extend(f"{offset:010} 00000 n \n".encode())
    data.extend(f"trailer\n<< /Size 3 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode())
    return bytes(data)


def invoke(command, data, timeout):
    try:
        result = subprocess.run(command, input=data, capture_output=True, timeout=timeout, check=False)
        return result.returncode, result.stdout, result.stderr.decode("utf-8", "replace")[:4096]
    except (OSError, subprocess.TimeoutExpired) as error:
        return None, b"", str(error)


def summary(result):
    status, output, error = result
    return {"exit": status, "output_bytes": len(output), "sha256": hashlib.sha256(output).hexdigest(), "diagnostic": error}


def strict_cases():
    default = {"predictor": 15, "colors": 1, "bits": 8, "columns": 2}
    yield "PNG-truncated-row", default, bytes([0, 1])
    yield "PNG-unknown-row-filter", default, bytes([5, 1, 2])
    yield "TIFF-truncated-row", default | {"predictor": 2}, bytes([1])
    for name, change in (
        ("unsupported-predictor", {"predictor": 3}),
        ("zero-colors", {"colors": 0}),
        ("too-many-colors", {"colors": 33}),
        ("zero-columns", {"columns": 0}),
        ("unsupported-bit-depth", {"bits": 3}),
        ("row-arithmetic-overflow", {"columns": 9223372036854775807, "bits": 16, "colors": 32}),
        ("row-exceeds-decoded-limit", {"columns": 64 * 1024 * 1024 + 1}),
    ):
        yield name, default | change, b""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust", type=Path, default=ENGINE / "target/debug/pdf-inspect")
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--cases", type=int, default=700)
    parser.add_argument("--seed", type=int, default=20261003)
    parser.add_argument("--timeout", type=float, default=5.0)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    if not 1 <= args.cases <= 10000 or not 0.1 <= args.timeout <= 60:
        parser.error("require --cases 1..10000 and --timeout 0.1..60")
    rust, oracle = args.rust.resolve(), args.reference.resolve()
    for binary in (rust, oracle):
        if not binary.is_file():
            parser.error(f"missing executable: {binary}")
    failures, strict = [], []
    counts = collections.Counter()
    digest = hashlib.sha256()
    total = decoded_bytes = 0
    for parameters, encoded, expected in generate(args.seed, args.cases):
        compressed = zlib.compress(encoded)
        pdf = document(compressed, parameters)
        rust_result = invoke([str(rust), "--stream=2:0"], pdf, args.timeout)
        reference_result = invoke([str(oracle), "flate", *(str(parameters[key]) for key in ("predictor", "colors", "bits", "columns"))], compressed, args.timeout)
        total += 1
        decoded_bytes += len(expected)
        counts[f"predictor{parameters['predictor']}-bits{parameters['bits']}"] += 1
        digest.update(len(pdf).to_bytes(8, "big"))
        digest.update(pdf)
        if rust_result[0] != 0 or reference_result[0] != 0 or rust_result[1] != expected or reference_result[1] != expected:
            failures.append({"parameters": parameters, "encoded_hex": encoded[:128].hex(),
                             "expected_sha256": hashlib.sha256(expected).hexdigest(),
                             "rust": summary(rust_result), "reference": summary(reference_result)})
            if len(failures) == 20:
                break
    for name, parameters, encoded in strict_cases():
        result = invoke([str(rust), "--stream=2:0"], document(zlib.compress(encoded), parameters), args.timeout)
        rejected = result[0] == 2 and not result[1]
        strict.append({"case": name, "parameters": parameters, "rust_rejected": rejected, "rust": summary(result)})
        if not rejected:
            failures.append({"strict_refusal_failure": strict[-1]})
    report = {
        "reference": "MuPDF 1.28.1 native Flate+predictor oracle",
        "created_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "seed": args.seed, "python_version": sys.version.split()[0], "zlib_runtime_version": zlib.ZLIB_RUNTIME_VERSION,
        "rust_binary_sha256": hashlib.sha256(rust.read_bytes()).hexdigest(),
        "reference_binary_sha256": hashlib.sha256(oracle.read_bytes()).hexdigest(),
        "planned_valid_cases": args.cases, "valid_cases": total, "decoded_bytes": decoded_bytes,
        "valid_cases_by_predictor_depth": dict(sorted(counts.items())), "corpus_sha256": digest.hexdigest(),
        "comparison": "Exact original independently encoded bytes equal Rust PDF dictionary decoder and native MuPDF predictor output",
        "subprocess_timeout_seconds": args.timeout, "strict_refusals": strict,
        "failures": failures, "passed": not failures and total == args.cases,
    }
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: value for key, value in report.items() if key not in ("strict_refusals", "failures", "valid_cases_by_predictor_depth")} |
                     {"strict_refusal_cases": len(strict), "failure_count": len(failures)}, indent=2))
    if failures:
        print(json.dumps(failures[:3], indent=2), file=sys.stderr)
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
