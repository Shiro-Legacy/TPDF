#!/usr/bin/env python3
"""Deterministic generator for fixtures/large.pdf.

500 pages, Base-14 Helvetica only, no embedded font programs, no dates and no
/ID -> byte-identical on every run. Each page carries a "PDF4All" header with
its page number, a wrapped paragraph and three vector paths (line, bezier
stroke, filled rectangle). No compression, so parsing cost is in no way hidden
by FlateDecode.
Run: python3 fixtures/make-large.py
"""
import json
import pathlib
import textwrap

from minipdf import emit, sha256_file, stream

PAGES = 500
OUT = pathlib.Path(__file__).with_name("large.pdf")
PARAGRAPH = textwrap.wrap(
    "PDF4All is a feasibility spike for a local-first desktop PDF application. "
    "Every page of this fixture carries the same self-authored ASCII paragraph so "
    "that full-document text extraction, search and rendering can be measured on a "
    "document with a realistic page count and a realistic number of content-stream "
    "operators. No third-party document was used to build it, and the generator is "
    "checked in next to the file it produces.",
    width=92,
)

objects = [
    b"<< /Type /Catalog /Pages 2 0 R >>",
    None,  # 2: page tree, patched below
    b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
]

page_nums = []
for i in range(1, PAGES + 1):
    page_num = len(objects) + 1
    content_num = page_num + 1
    page_nums.append(page_num)
    objects.append(
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Resources << /Font << /F1 3 0 R >> >> /Contents %d 0 R >>" % content_num
    )
    body = [
        "BT /F1 14 Tf 1 0 0 1 72 740 Tm (PDF4All - page %d of %d) Tj ET" % (i, PAGES),
        "BT /F1 10 Tf 1 0 0 1 72 700 Tm",
    ]
    body += ["(%s) Tj 0 -14 Td" % line.replace("(", r"\(").replace(")", r"\)") for line in PARAGRAPH]
    body.append("ET")
    body.append("0.2 0.4 0.9 RG 1.5 w 72 120 m 540 120 l S")
    body.append("0.9 0.4 0.2 rg 200 160 160 36 re f")
    body.append("0.1 0.6 0.3 RG 1 w 72 60 m 200 200 400 200 540 60 c S")
    objects.append(stream(("\n".join(body) + "\n").encode("ascii")))

objects[1] = b"<< /Type /Pages /Kids [%s] /Count %d >>" % (
    b" ".join(b"%d 0 R" % n for n in page_nums),
    PAGES,
)

size, digest = emit(OUT, objects)
pathlib.Path(OUT.with_suffix(".provenance.json")).write_text(
    json.dumps(
        {
            "id": "large",
            "file": "fixtures/large.pdf",
            "bytes": size,
            "sha256": digest,
            "pages": PAGES,
            "page_size_pt": [612, 792],
            "content_per_page": {
                "text": ["PDF4All - page <n> of 500", *PARAGRAPH],
                "vector": ["stroked line", "filled rectangle", "cubic bezier stroke"],
                "compression": "none (content streams are stored raw)",
            },
            "generator": {
                "path": "fixtures/make-large.py",
                "helper": "fixtures/minipdf.py",
                "sha256": sha256_file(__file__),
                "dependencies": ["python3 stdlib only"],
                "reproduce": "python3 fixtures/make-large.py",
                "deterministic": True,
                "timestamps_in_output": False,
            },
            "provenance": {
                "authorship": "written by the project team for this spike (ds2)",
                "third_party_content": "none; the paragraph is self-authored ASCII",
                "fonts": "Base-14 Helvetica reference only, no font program embedded",
                "network_access": "none",
                "source_documents": "none",
            },
            "extraction_contract": {
                "mupdf_1.28.1": "countPages() === 500; search('PDF4All') returns exactly "
                                "two hits per page (header line plus the paragraph opener); "
                                "every page renders non-blank",
            },
            "license": "no license selected here; follows the app's final license choice",
        },
        indent=2,
    )
    + "\n"
)
print(f"wrote {OUT} ({size} bytes) sha256={digest}")
