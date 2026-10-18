#!/usr/bin/env python3
"""Deterministic generator for fixtures/layers.pdf.

One page, two OCGs (Base, Notes), both on. python3 stdlib only.
Run: python3 fixtures/make-layers.py
"""
import json
import pathlib

from minipdf import emit, sha256_file, stream

OUT = pathlib.Path(__file__).with_name("layers.pdf")

content = (
    b"/OC /ocA BDC\nBT /F1 24 Tf 72 700 Td (base) Tj ET\nEMC\n"
    b"/OC /ocB BDC\nBT /F1 24 Tf 72 600 Td (notes) Tj ET\nEMC\n"
)

objects = [
    b"<< /Type /Catalog /Pages 2 0 R "
    b"/OCProperties << /OCGs [7 0 R 6 0 R] /D << /Order [7 0 R 6 0 R] /ON [7 0 R 6 0 R] >> >> >>",
    b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
    b"/Resources << /Font << /F1 4 0 R >> /Properties << /ocA 7 0 R /ocB 6 0 R >> >> "
    b"/Contents 5 0 R >>",
    b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    stream(content),
    b"<< /Type /OCG /Name (Notes) >>",
    b"<< /Type /OCG /Name (Base) >>",
]

size, digest = emit(OUT, objects)
data = OUT.read_bytes()
for needle in (
    b"(base)", b"(notes)", b"/Name (Base)", b"/Name (Notes)",
    b"/OC /ocA", b"/OC /ocB", b"/OCGs [7 0 R 6 0 R]", b"/Name (Base)", b"/Name (Notes)",
):
    assert needle in data, needle
assert b"/OpenAction" not in data
assert b"/JavaScript" not in data

pathlib.Path(OUT.with_suffix(".provenance.json")).write_text(
    json.dumps(
        {
            "id": "layers",
            "file": "fixtures/layers.pdf",
            "bytes": size,
            "sha256": digest,
            "pages": 1,
            "page_size_pt": [612, 792],
            "extractable_text": "base notes",
            "layers": [
                {"name": "Base", "on": True},
                {"name": "Notes", "on": True},
            ],
            "generator": {
                "path": "fixtures/make-layers.py",
                "helper": "fixtures/minipdf.py",
                "sha256": sha256_file(__file__),
                "deterministic": True,
                "timestamps_in_output": False,
                "dependencies": ["python3 stdlib only"],
                "reproduce": "python3 fixtures/make-layers.py",
            },
            "provenance": {
                "authorship": "written by the project team for this spike (grok); constructed byte-by-byte, not exported from any third-party tool",
                "third_party_content": "none",
                "fonts": "Base-14 Helvetica, referenced not embedded (/BaseFont /Helvetica, /Encoding /WinAnsiEncoding)",
                "network_access": "none",
                "source_documents": "none - content is the literal ASCII strings base/notes plus OCGs named in contract-unit8.md",
            },
            "license": "MIT (LICENSE at the repo root)",
            "generated_on": "2026-09-16",
        },
        indent=2,
    )
    + "\n"
)
print(f"wrote {OUT} ({size} bytes) sha256={digest}")
