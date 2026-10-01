#!/usr/bin/env python3
"""Generate the deterministic four-page form-authoring geometry fixture."""
import json
import pathlib
import sys

FIXTURES = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(FIXTURES))
from minipdf import emit, sha256_file, stream

OUT = pathlib.Path(__file__).with_name("form-design.pdf")


def caption(page):
    return stream(f"BT /F1 24 Tf 72 700 Td (PDF4All form design page {page}) Tj ET\n".encode())


objects = [
    b"<< /Type /Catalog /Pages 2 0 R >>",
    b"<< /Type /Pages /Kids [4 0 R 6 0 R 8 0 R 10 0 R] /Count 4 >>",
    b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /CropBox [36 36 576 756] /Resources << /Font << /F1 3 0 R >> >> /Contents 5 0 R >>",
    caption(1),
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Rotate 90 /Resources << /Font << /F1 3 0 R >> >> /Contents 7 0 R >>",
    caption(2),
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents 9 0 R /Annots 12 0 R >>",
    caption(3),
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents 11 0 R /Annots 12 0 R >>",
    caption(4),
    b"[]",
]
size, digest = emit(OUT, objects)
data = OUT.read_bytes()
assert data.count(b"/Type /Page ") == 4
assert b"/CropBox [36 36 576 756]" in data and b"/Rotate 90" in data
assert data.count(b"/Annots 12 0 R") == 2 and b"/AcroForm" not in data
provenance = {
    "id": "form-design",
    "file": "fixtures/p3/form-design.pdf",
    "bytes": size,
    "sha256": digest,
    "pages": 4,
    "page_size_pt": [612, 792],
    "notes": [
        "page 0 has /CropBox [36 36 576 756]",
        "page 1 has /Rotate 90",
        "pages 2 and 3 share indirect empty /Annots object 12 0 R",
        "no /AcroForm and no fields",
    ],
    "generator": {
        "path": "fixtures/p3/make-form-design.py",
        "helper": "fixtures/minipdf.py",
        "sha256": sha256_file(__file__),
        "reproduce": "python3 fixtures/p3/make-form-design.py",
        "deterministic": True,
        "timestamps_in_output": False,
        "dependencies": ["python3 stdlib only"],
    },
    "provenance": {"authorship": "written by the project team for P3 U6", "third_party_content": "none", "network_access": "none"},
    "generated_on": "2026-09-17",
}
OUT.with_suffix(".provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
print(f"wrote {OUT} ({size} bytes) sha256={digest}")
