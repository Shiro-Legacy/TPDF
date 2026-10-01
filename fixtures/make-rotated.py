#!/usr/bin/env python3
"""Deterministic generator for fixtures/rotated.pdf.

One page, Base-14 Helvetica, the same ASCII string drawn three times: upright,
rotated 90 deg and rotated 45 deg, each placed with its own text matrix (Tm).
No dates, no /ID -> byte-identical on every run.
Run: python3 fixtures/make-rotated.py
"""
import json
import pathlib

from minipdf import emit, sha256_file, stream

TEXT = "PDF4All rotated"
OUT = pathlib.Path(__file__).with_name("rotated.pdf")

# (rotation deg, Tm) - Tm maps the text-space x axis onto the page; the last two
# numbers are the origin of the text line.
RUNS = [
    (0, "1 0 0 1 72 700"),
    (90, "0 1 -1 0 420 200"),
    (45, "0.70710678 0.70710678 -0.70710678 0.70710678 120 360"),
]

content = "".join(
    "BT /F1 30 Tf %s Tm (%s) Tj ET\n" % (tm, TEXT) for _, tm in RUNS
).encode("ascii")

objects = [
    b"<< /Type /Catalog /Pages 2 0 R >>",
    b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
    b"/Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
    b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    stream(content),
]

size, digest = emit(OUT, objects)
pathlib.Path(OUT.with_suffix(".provenance.json")).write_text(
    json.dumps(
        {
            "id": "rotated",
            "file": "fixtures/rotated.pdf",
            "bytes": size,
            "sha256": digest,
            "pages": 1,
            "page_size_pt": [612, 792],
            "text_runs": [
                {"string": TEXT, "rotation_deg": deg, "tm": tm} for deg, tm in RUNS
            ],
            "generator": {
                "path": "fixtures/make-rotated.py",
                "helper": "fixtures/minipdf.py",
                "sha256": sha256_file(__file__),
                "dependencies": ["python3 stdlib only"],
                "reproduce": "python3 fixtures/make-rotated.py",
                "deterministic": True,
                "timestamps_in_output": False,
            },
            "provenance": {
                "authorship": "written by the project team for this spike (ds2)",
                "third_party_content": "none",
                "fonts": "Base-14 Helvetica reference only, no font program embedded "
                         "(MuPDF substitutes Nimbus Sans from the wasm build)",
                "network_access": "none",
            },
            "extraction_contract": {
                "mupdf_1.28.1": "search('PDF4All') returns 3 hits, one per run, with "
                                "quads whose baseline direction is 0deg / 90deg / 45deg",
                "note": "text order is the drawing order (upright, 90, 45)",
            },
            "license": "no license selected here; follows the app's final license choice",
        },
        indent=2,
    )
    + "\n"
)
print(f"wrote {OUT} ({size} bytes) sha256={digest}")
