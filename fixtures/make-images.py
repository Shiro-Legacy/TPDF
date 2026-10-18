#!/usr/bin/env python3
"""Deterministic PNG fixtures for P2 unit 7. python3 stdlib only.

img-rgb.png  300×200 RGB gradient
img-gray.png 200×300 grayscale with a black band at rows 100–120
Run: python3 fixtures/make-images.py
"""
import hashlib
import json
import pathlib
import struct
import zlib

DIR = pathlib.Path(__file__).resolve().parent
SIG = b"\x89PNG\r\n\x1a\n"


def chunk(tag, data):
    return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)


def png(width, height, color, rows):
    ihdr = struct.pack(">IIBBBBB", width, height, 8, color, 0, 0, 0)
    raw = b"".join(b"\x00" + row for row in rows)
    return SIG + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def provenance(ident, path, data, pages_size, extra):
    rec = {
        "id": ident,
        "file": f"fixtures/{path.name}",
        "bytes": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
        "kind": "png",
        **pages_size,
        **extra,
        "generator": {
            "path": "fixtures/make-images.py",
            "sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
            "deterministic": True,
            "timestamps_in_output": False,
            "dependencies": ["python3 stdlib only"],
            "reproduce": "python3 fixtures/make-images.py",
        },
        "provenance": {
            "authorship": "written by the project team for this spike (grok); constructed byte-by-byte, not exported from any third-party tool",
            "third_party_content": "none",
            "network_access": "none",
            "source_documents": "none - synthetic pixels named in contract-unit7.md",
        },
        "license": "MIT (LICENSE at the repo root)",
        "generated_on": "2026-09-16",
    }
    path.with_name(path.name.replace(".png", ".provenance.json")).write_text(json.dumps(rec, indent=2) + "\n")


def emit(name, width, height, color, rows, extra):
    path = DIR / name
    data = png(width, height, color, rows)
    path.write_bytes(data)
    provenance(name[:-4], path, data, {"width": width, "height": height}, extra)
    print(f"wrote {path} ({len(data)} bytes)")


emit(
    "img-rgb.png",
    300,
    200,
    2,
    [bytes(ch for x in range(300) for ch in (x * 255 // 299, y * 255 // 199, 128)) for y in range(200)],
    {"extractable_text": "", "note": "RGB gradient r=x g=y b=128"},
)
emit(
    "img-gray.png",
    200,
    300,
    0,
    [bytes(0 if 100 <= y <= 120 else 255 for _ in range(200)) for y in range(300)],
    {"extractable_text": "", "note": "white with black band rows 100-120"},
)
