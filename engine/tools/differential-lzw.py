#!/usr/bin/env python3
"""Compare PDF LZW/EarlyChange/predictors against independent encoding and MuPDF.

The Python encoder builds a dictionary of byte strings. The Rust decoder uses
fixed prefix/suffix tables. Exact original-byte and native-oracle comparisons
cover width changes, dictionary clears, KwKwK, full tables, and predictors.
"""
import argparse
import collections
import datetime
import hashlib
import importlib.util
import json
import random
import subprocess
import sys
from pathlib import Path

ENGINE = Path(__file__).resolve().parent.parent


def pack_codes(codes):
    output = bytearray()
    accumulator = occupied = 0
    for code, width in codes:
        assert 0 <= code < (1 << width)
        accumulator = (accumulator << width) | code
        occupied += width
        while occupied >= 8:
            occupied -= 8
            output.append((accumulator >> occupied) & 255)
        accumulator &= (1 << occupied) - 1
    if occupied:
        output.append(accumulator << (8 - occupied))
    return bytes(output)


def encode(data, early_change, reset_interval=0):
    table = {bytes([value]): value for value in range(256)}
    next_code, width = 258, 9
    codes = [(256, width)]
    clears = 1
    prefix = b""
    entries_since_clear = 0
    for byte in data:
        current = bytes([byte])
        candidate = prefix + current
        if candidate in table:
            prefix = candidate
            continue
        codes.append((table[prefix], width))
        table[candidate] = next_code
        next_code += 1
        entries_since_clear += 1
        # The encoder is one dictionary insertion ahead of the decoder.
        if width < 12 and next_code == (1 << width) + 1 - early_change:
            width += 1
        if next_code == 4096 or reset_interval and entries_since_clear == reset_interval:
            codes.append((256, width))
            clears += 1
            table = {bytes([value]): value for value in range(256)}
            next_code, width, entries_since_clear = 258, 9, 0
        prefix = current
    if prefix:
        codes.append((table[prefix], width))
        # Final data code lets the decoder catch the encoder's last insertion.
        if width < 12 and next_code == (1 << width) - early_change:
            width += 1
    codes.append((257, width))
    return pack_codes(codes), {"code_widths": sorted({width for _, width in codes}), "clear_codes": clears}


def document(encoded, parameters):
    parameters = {"predictor": 1, "colors": 1, "bits": 8, "columns": 1, "early": 1} | parameters
    metadata = (f"/Filter /LZWDecode /DecodeParms << /EarlyChange {parameters['early']} "
                f"/Predictor {parameters['predictor']} /Colors {parameters['colors']} "
                f"/BitsPerComponent {parameters['bits']} /Columns {parameters['columns']} >>")
    stream = f"<< /Length {len(encoded)} {metadata} >>\nstream\n".encode() + encoded + b"\nendstream"
    data = bytearray(b"%PDF-1.7\n")
    offsets = []
    for number, body in enumerate((b"<< /Type /Catalog >>", stream), 1):
        offsets.append(len(data))
        data.extend(f"{number} 0 obj\n".encode() + body + b"\nendobj\n")
    xref = len(data)
    data.extend(b"xref\n0 3\n0000000000 65535 f \n")
    for offset in offsets:
        data.extend(f"{offset:010} 00000 n \n".encode())
    data.extend(f"trailer\n<< /Size 3 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode())
    return bytes(data)


def generate(seed, count):
    rng = random.Random(seed)
    boundaries = (0, 1, 2, 3, 253, 254, 255, 256, 765, 766, 767, 768, 1789, 1790, 1791, 1792, 3838, 3839, 4096, 16384, 65536)
    for index in range(count):
        pair = index // 2
        size = boundaries[pair] if pair < len(boundaries) else rng.randrange(8193)
        if pair % 4 == 0:
            original = bytes(rng.randrange(256) for _ in range(size))
        elif pair % 4 == 1:
            original = b"A" * size
        elif pair % 4 == 2:
            original = (b"ABABABA" * (size // 7 + 1))[:size]
        else:
            original = bytes((value % 256 for value in range(size)))
        early = index % 2
        reset = rng.choice((0, 0, 0, 127, 1024))
        encoded, stats = encode(original, early, reset)
        yield f"generated-{index}", {"early": early}, encoded, original, stats
    # Independently packed explicit literal codes hit every width and the full
    # decoder table, including exactly-full EOD and clear followed by KwKwK.
    for early in (0, 1):
        def width(processed):
            return 9 if processed < 255 - early else 10 if processed < 767 - early else 11 if processed < 1791 - early else 12
        for clear in (False, True):
            codes = [(256, 9)] + [(value % 256, width(value)) for value in range(3839)]
            original = bytes(value % 256 for value in range(3839))
            if clear:
                codes.extend(((256, 12), (65, 9), (258, 9), (257, 9)))
                original += b"AAA"
            else:
                codes.append((257, 12))
            yield f"full-table-{early}-clear{clear}", {"early": early}, pack_codes(codes), original, {"code_widths": [9, 10, 11, 12], "clear_codes": 1 + clear}


def generate_predictors(seed, count):
    specification = importlib.util.spec_from_file_location("predictor_vectors", ENGINE / "tools/differential-predictors.py")
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    for parameters, filtered, original in module.generate(seed, count):
        early = parameters["case"] % 2
        encoded, stats = encode(filtered, early)
        yield f"predictor-{parameters['case']}", parameters | {"early": early}, encoded, original, stats


def invoke(command, data, timeout):
    try:
        result = subprocess.run(command, input=data, capture_output=True, timeout=timeout, check=False)
        return result.returncode, result.stdout, result.stderr.decode("utf-8", "replace")[:4096]
    except (OSError, subprocess.TimeoutExpired) as error:
        return None, b"", str(error)


def summary(result):
    status, output, error = result
    return {"exit": status, "output_bytes": len(output), "sha256": hashlib.sha256(output).hexdigest(), "diagnostic": error}


def malformed():
    valid, _ = encode(b"ABABABA", 1)
    yield "missing-initial-clear", pack_codes([(65, 9), (257, 9)])
    yield "undefined-first-code", pack_codes([(256, 9), (258, 9), (257, 9)])
    yield "invalid-forward-reference", pack_codes([(256, 9), (65, 9), (259, 9), (257, 9)])
    yield "truncated-EOD", valid[:-1]
    yield "missing-EOD", pack_codes([(256, 9), (65, 9)])
    yield "nonzero-padding", valid[:-1] + bytes([valid[-1] | 1])
    yield "trailing-zero-byte", valid + b"\0"
    yield "trailing-payload", valid + b"more"
    codes = [(256, 9)]
    for value in range(3839):
        width = 9 if value < 254 else 10 if value < 766 else 11 if value < 1790 else 12
        codes.append((value % 256, width))
    codes.extend(((65, 12), (257, 12)))
    yield "full-dictionary-without-clear", pack_codes(codes)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust", type=Path, default=ENGINE / "target/debug/pdf-inspect")
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--cases", type=int, default=400)
    parser.add_argument("--predictors", type=int, default=350)
    parser.add_argument("--seed", type=int, default=20261003)
    parser.add_argument("--timeout", type=float, default=5.0)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    if not 1 <= args.cases <= 10000 or not 0 <= args.predictors <= 10000 or not 0.1 <= args.timeout <= 60:
        parser.error("require --cases 1..10000, --predictors 0..10000, --timeout 0.1..60")
    rust, oracle = args.rust.resolve(), args.reference.resolve()
    for binary in (rust, oracle):
        if not binary.is_file(): parser.error(f"missing executable: {binary}")
    failures, strict = [], []
    counts = collections.Counter()
    widths = {0: set(), 1: set()}
    clears = collections.Counter()
    digest = hashlib.sha256()
    valid = decoded_bytes = 0
    cases = [*generate(args.seed, args.cases), *generate_predictors(args.seed, args.predictors)]
    for name, parameters, encoded, expected, stats in cases:
        early = parameters["early"]
        pdf = document(encoded, parameters)
        command = [str(oracle), "lzw", str(early)]
        if "predictor" in parameters:
            command.extend(str(parameters[key]) for key in ("predictor", "colors", "bits", "columns"))
        rust_result = invoke([str(rust), "--stream=2:0"], pdf, args.timeout)
        reference_result = invoke(command, encoded, args.timeout)
        valid += 1
        decoded_bytes += len(expected)
        counts[f"early{early}-predictor{parameters.get('predictor', 1)}"] += 1
        widths[early].update(stats["code_widths"])
        clears[early] += stats["clear_codes"]
        digest.update(len(pdf).to_bytes(8, "big")); digest.update(pdf)
        if rust_result[0] != 0 or reference_result[0] != 0 or rust_result[1] != expected or reference_result[1] != expected:
            failures.append({"case": name, "parameters": parameters, "encoded_bytes": len(encoded), "encoded_prefix_hex": encoded[:128].hex(),
                             "expected_sha256": hashlib.sha256(expected).hexdigest(), "rust": summary(rust_result), "reference": summary(reference_result)})
            if len(failures) == 20: break
    for name, encoded in malformed():
        rust_result = invoke([str(rust), "--stream=2:0"], document(encoded, {"early": 1}), args.timeout)
        reference_result = invoke([str(oracle), "lzw", "1"], encoded, args.timeout)
        rejected = rust_result[0] == 2 and not rust_result[1]
        strict.append({"case": name, "rust_rejected": rejected, "rust": summary(rust_result), "reference": summary(reference_result)})
        if not rejected or reference_result[0] not in (0, 2): failures.append({"strict_refusal_failure": strict[-1]})
    report = {
        "reference": "MuPDF 1.28.1 native LZW/predictor oracle", "created_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "seed": args.seed, "python_version": sys.version.split()[0],
        "rust_binary_sha256": hashlib.sha256(rust.read_bytes()).hexdigest(), "reference_binary_sha256": hashlib.sha256(oracle.read_bytes()).hexdigest(),
        "planned_valid_cases": len(cases), "valid_cases": valid, "decoded_bytes": decoded_bytes,
        "cases_by_earlychange_predictor": dict(sorted(counts.items())), "code_widths_by_earlychange": {str(k): sorted(v) for k, v in widths.items()},
        "clear_codes_by_earlychange": dict(clears), "corpus_sha256": digest.hexdigest(), "subprocess_timeout_seconds": args.timeout,
        "comparison": "Original independent Python encoder payload equals Rust dictionary stream decoder and native MuPDF",
        "strict_refusals": strict, "failures": failures, "passed": not failures and valid == len(cases),
    }
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: value for key, value in report.items() if key not in ("strict_refusals", "failures", "cases_by_earlychange_predictor")} |
                     {"strict_refusal_cases": len(strict), "failure_count": len(failures)}, indent=2))
    if failures: print(json.dumps(failures[:3], indent=2), file=sys.stderr)
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
