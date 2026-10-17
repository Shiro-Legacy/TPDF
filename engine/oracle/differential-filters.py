#!/usr/bin/env python3
"""Compare strict Rust PDF filters to pinned MuPDF and independent Python encoders.

Run from any directory. Uses Python's standard-library zlib/base64 encoders plus
an explicit PDF RLE encoder. Generated valid cases must match both the original
unencoded bytes and MuPDF exactly. Malformed inputs are separate strict-refusal
checks, never counted as valid parity. Every subprocess has a bounded timeout.
"""
import argparse
import base64
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
FILTERS = ("flate", "asciihex", "ascii85", "runlength")
WHITESPACE = (b" ", b"\t", b"\n", b"\r", b"\f", b"\0", b"\r\n")


def rle_encode(data):
    """PDF literal packets (1..128 bytes), repeated packets (2..128), then EOD."""
    output = bytearray()
    position = 0
    while position < len(data):
        count = 1
        while count < 128 and position + count < len(data) and data[position + count] == data[position]:
            count += 1
        if count > 1:
            output.extend((257 - count, data[position]))
            position += count
            continue
        start = position
        position += 1
        while position - start < 128 and position < len(data):
            if position + 1 < len(data) and data[position] == data[position + 1]:
                break
            position += 1
        output.append(position - start - 1)
        output.extend(data[start:position])
    output.append(128)
    return bytes(output)


def encode(name, data, rng):
    if name == "flate":
        return zlib.compress(data, level=rng.randrange(10))
    if name == "runlength":
        return rle_encode(data)
    if name == "asciihex":
        body = data.hex().encode("ascii")
        if rng.randrange(2):
            body = body.upper()
        if data and data[-1] & 15 == 0 and rng.randrange(2):
            body = body[:-1]  # Legal odd-nibble padding reproduces the zero nibble.
        marker = b">"
    elif name == "ascii85":
        body = base64.a85encode(data, adobe=False, foldspaces=False, wrapcol=0, pad=False)
        marker = b"~>"
    else:
        raise ValueError(name)
    # PDF whitespace may occur between any data digits, but not inside '~>'.
    if rng.randrange(2):
        size = rng.randrange(1, 33)
        body = rng.choice(WHITESPACE).join(body[i:i + size] for i in range(0, len(body), size))
    return rng.choice(WHITESPACE) + body + marker + rng.choice(WHITESPACE)


def payload(index, rng):
    lengths = (0, 1, 2, 3, 4, 5, 31, 32, 127, 128, 129, 255, 256, 257, 4096, 65536)
    length = lengths[index] if index < len(lengths) else rng.randrange(4097)
    pattern = index % 5
    if pattern == 0:
        return bytes((i % 256 for i in range(length)))
    if pattern == 1:
        return bytes([rng.randrange(256)]) * length
    if pattern == 2:
        return bytes(rng.randrange(256) for _ in range(length))
    if pattern == 3:
        text = b"Hello, PDF streams!\0\xff\r\n"
        return (text * ((length // len(text)) + 1))[:length]
    return bytes((i // 129) % 256 for i in range(length))


def generated_cases(seed, count):
    rng = random.Random(seed)
    for index in range(count):
        name = FILTERS[index % len(FILTERS)]
        data = payload(index // len(FILTERS), rng)
        yield f"generated-{index}", (name,), encode(name, data, rng), data


def generated_chains(seed, count):
    rng = random.Random(seed ^ 0xA85F1A7E)
    chains = (("ascii85", "flate"), ("asciihex", "runlength"),
              ("runlength", "flate"), ("flate", "ascii85", "runlength"))
    for index in range(count):
        chain = chains[index % len(chains)]
        plain = payload(index, rng)
        encoded = plain
        for name in reversed(chain):
            encoded = encode(name, encoded, rng)
        yield f"chain-{index}", chain, encoded, plain


def strict_cases():
    compressed = zlib.compress(b"strict malformed stream test")
    corrupt = compressed[:-1] + bytes([compressed[-1] ^ 1])
    return [
        ("flate", "checksum-mismatch", corrupt),
        ("flate", "trailing-payload", compressed + b"trailing"),
        ("flate", "trailing-newline", compressed + b"\n"),
        ("flate", "concatenated-zlib", compressed + compressed),
        ("asciihex", "trailing-payload", b"00> payload"),
        ("asciihex", "non-pdf-whitespace", b"0\x080>"),
        ("ascii85", "overflow-full-group", b"uuuuu~>"),
        ("ascii85", "overflow-short-group", b"uu~>"),
        ("ascii85", "single-final-digit", b"!~>"),
        ("ascii85", "bad-end-marker", b"z~x"),
        ("ascii85", "trailing-payload", b"z~> payload"),
        ("ascii85", "non-pdf-whitespace", b"z\x7f~>"),
        ("runlength", "trailing-payload", b"\x80payload"),
    ]


def invoke(command, data, timeout):
    try:
        process = subprocess.run(command, input=data, capture_output=True, timeout=timeout, check=False)
    except subprocess.TimeoutExpired:
        return {"exit": None, "stdout": b"", "diagnostic": "subprocess timeout", "timeout": True}
    except OSError as error:
        return {"exit": None, "stdout": b"", "diagnostic": str(error), "timeout": False}
    return {"exit": process.returncode, "stdout": process.stdout,
            "diagnostic": process.stderr.decode("utf-8", "replace")[:4096], "timeout": False}


def result_summary(result):
    return {key: value for key, value in result.items() if key != "stdout"} | {
        "output_bytes": len(result["stdout"]),
        "output_sha256": hashlib.sha256(result["stdout"]).hexdigest(),
        "output_prefix_hex": result["stdout"][:128].hex(),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust", type=Path, default=ENGINE / "target/debug/pdf-inspect")
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--vectors", type=Path, default=ENGINE / "reference/filter-vectors.json")
    parser.add_argument("--cases", type=int, default=1000, help="Generated single-filter cases, distributed evenly")
    parser.add_argument("--chains", type=int, default=32)
    parser.add_argument("--seed", type=int, default=20261003)
    parser.add_argument("--timeout", type=float, default=5.0)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    if not 0 <= args.cases <= 10000 or not 0 <= args.chains <= 1000:
        parser.error("--cases must be 0..10000 and --chains 0..1000")
    if not 0.1 <= args.timeout <= 60:
        parser.error("--timeout must be 0.1..60 seconds")
    rust, reference = args.rust.resolve(), args.reference.resolve()
    for binary in (rust, reference):
        if not binary.is_file():
            parser.error(f"missing executable: {binary}")
    snapshots = json.loads(args.vectors.read_text())["vectors"]
    known_valid, strict = [], []
    for vector in snapshots:
        data = bytes.fromhex(vector["input_hex"])
        if vector["mupdf_exit"] == 0 and not vector["intentional_strict_refusal"]:
            known_valid.append((f"snapshot-{vector['name']}-{vector['filter']}", (vector["filter"],),
                                data, bytes.fromhex(vector["mupdf_output_hex"])))
        else:
            strict.append((vector["filter"], f"snapshot-{vector['name']}", data))
    failures, refusals = [], []
    counts = collections.Counter()
    valid_total = chains_total = 0
    corpus_hash = hashlib.sha256()
    all_valid = [*known_valid, *generated_cases(args.seed, args.cases),
                 *generated_chains(args.seed, args.chains)]
    for name, chain, data, expected in all_valid:
        valid_total += 1
        chains_total += len(chain) > 1
        counts[",".join(chain)] += 1
        corpus_hash.update(len(data).to_bytes(8, "big"))
        corpus_hash.update(data)
        rust_result = invoke([str(rust), "--decode=" + ",".join(chain)], data, args.timeout)
        current = data
        reference_results = []
        for stage in chain:
            result = invoke([str(reference), stage], current, args.timeout)
            reference_results.append(result)
            if result["exit"] != 0:
                break
            current = result["stdout"]
        reference_ok = len(reference_results) == len(chain) and all(r["exit"] == 0 for r in reference_results)
        if rust_result["exit"] != 0 or not reference_ok or rust_result["stdout"] != expected or current != expected:
            failures.append({"case": name, "filters": chain, "input_bytes": len(data),
                "input_sha256": hashlib.sha256(data).hexdigest(), "input_prefix_hex": data[:128].hex(),
                "expected_bytes": len(expected), "expected_sha256": hashlib.sha256(expected).hexdigest(),
                "rust": result_summary(rust_result), "reference_stages": [result_summary(r) for r in reference_results]})
            if len(failures) == 20:
                break
    for filter_name, name, data in [*strict, *strict_cases()]:
        rust_result = invoke([str(rust), "--decode=" + filter_name], data, args.timeout)
        reference_result = invoke([str(reference), filter_name], data, args.timeout)
        rejected = rust_result["exit"] == 2 and not rust_result["stdout"]
        row = {"filter": filter_name, "case": name, "rust_rejected": rejected,
               "input_hex": data.hex(), "rust": result_summary(rust_result),
               "reference": result_summary(reference_result)}
        refusals.append(row)
        if not rejected or reference_result["exit"] not in (0, 2):
            failures.append({"strict_refusal_failure": row})
    planned = len(all_valid)
    report = {
        "reference": "MuPDF 1.28.1", "created_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "seed": args.seed, "generated_single_filter_cases": args.cases,
        "python_version": sys.version.split()[0], "zlib_runtime_version": zlib.ZLIB_RUNTIME_VERSION,
        "rust_binary_sha256": hashlib.sha256(rust.read_bytes()).hexdigest(),
        "reference_binary_sha256": hashlib.sha256(reference.read_bytes()).hexdigest(),
        "known_vectors_sha256": hashlib.sha256(args.vectors.read_bytes()).hexdigest(),
        "known_valid_vectors": len(known_valid), "planned_valid_cases": planned,
        "valid_cases": valid_total, "valid_chain_cases": chains_total,
        "valid_cases_by_chain": dict(sorted(counts.items())), "corpus_sha256": corpus_hash.hexdigest(),
        "comparison": "Exact bytes against independently encoded original payload and sequential native MuPDF oracle",
        "subprocess_timeout_seconds": args.timeout, "strict_refusals": refusals,
        "failures": failures, "passed": not failures and valid_total == planned,
    }
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({k: v for k, v in report.items() if k not in ("strict_refusals", "failures")} |
                     {"strict_refusal_cases": len(refusals), "failure_count": len(failures)}, indent=2))
    if failures:
        print(json.dumps(failures[:3], indent=2), file=sys.stderr)
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
