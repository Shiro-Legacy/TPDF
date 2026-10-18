#!/usr/bin/env python3
"""Deterministic generator for fixtures/nav.pdf.

Three text pages, a four-entry outline (one nested child), four links.
No /OpenAction, no JavaScript. python3 stdlib only.
Run: python3 fixtures/make-nav.py
"""
import json
import pathlib

from minipdf import emit, sha256_file, stream

OUT = pathlib.Path(__file__).with_name("nav.pdf")


def page_content(text):
    return stream(b"BT /F1 24 Tf 72 700 Td (%s) Tj ET\n" % text)


def page(contents, annots=b""):
    extra = b" /Annots %s" % annots if annots else b""
    return (
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Resources << /Font << /F1 6 0 R >> >> /Contents %d 0 R%s >>" % (contents, extra)
    )


def link(parent, rect, action):
    return (
        b"<< /Type /Annot /Subtype /Link /P %d 0 R /Rect %s /Border [0 0 0] /F 4 "
        b"/A %s >>" % (parent, rect, action)
    )


GOTO_P2 = b"<< /S /GoTo /D [4 0 R /Fit] >>"
URI_WEB = b"<< /S /URI /URI (https://example.com/) >>"
URI_FILE = b"<< /S /URI /URI (file:///etc/hosts) >>"
R_GOTO = b"[72 680 200 730]"
R_URI = b"[72 600 300 630]"

objects = [
    b"<< /Type /Catalog /Pages 2 0 R /Outlines 10 0 R >>",
    b"<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>",
    page(7, b"[15 0 R 16 0 R]"),
    page(8),
    page(9, b"[17 0 R 18 0 R]"),
    b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    page_content(b"one"),
    page_content(b"two"),
    page_content(b"three"),
    b"<< /Type /Outlines /First 11 0 R /Last 14 0 R /Count 4 >>",
    b"<< /Title (One) /Parent 10 0 R /Next 12 0 R /Dest [3 0 R /Fit] >>",
    b"<< /Title (Two) /Parent 10 0 R /Prev 11 0 R /Next 14 0 R "
    b"/First 13 0 R /Last 13 0 R /Count 1 /Dest [4 0 R /Fit] >>",
    b"<< /Title (Two-A) /Parent 12 0 R /Dest [4 0 R /Fit] >>",
    b"<< /Title (Three) /Parent 10 0 R /Prev 12 0 R /Dest [5 0 R /Fit] >>",
    link(3, R_GOTO, GOTO_P2),
    link(3, R_URI, URI_WEB),
    link(5, R_GOTO, GOTO_P2),
    link(5, R_URI, URI_FILE),
]

size, digest = emit(OUT, objects)
data = OUT.read_bytes()
for needle in (
    b"(one)", b"(two)", b"(three)",
    b"/Title (One)", b"/Title (Two)", b"/Title (Two-A)", b"/Title (Three)",
    b"/Count 4", b"/Count 1", b"/Fit",
    b"https://example.com/", b"file:///etc/hosts",
    b"/S /GoTo", b"/S /URI", R_GOTO, R_URI,
):
    assert needle in data, needle
assert b"/OpenAction" not in data
assert b"/JavaScript" not in data
assert b"/JS " not in data and b"/JS(" not in data

pathlib.Path(OUT.with_suffix(".provenance.json")).write_text(
    json.dumps(
        {
            "id": "nav",
            "file": "fixtures/nav.pdf",
            "bytes": size,
            "sha256": digest,
            "pages": 3,
            "page_size_pt": [612, 792],
            "extractable_text": "one",
            "outline": [
                {"title": "One", "depth": 0, "page": 0},
                {"title": "Two", "depth": 0, "page": 1},
                {"title": "Two-A", "depth": 1, "page": 1},
                {"title": "Three", "depth": 0, "page": 2},
            ],
            "links": {
                "0": [
                    {"kind": "page", "page": 1, "rect": [72, 680, 200, 730]},
                    {"kind": "uri", "uri": "https://example.com/", "rect": [72, 600, 300, 630]},
                ],
                "2": [
                    {"kind": "page", "page": 1, "rect": [72, 680, 200, 730]},
                    {"kind": "blocked", "uri": "file:///etc/hosts", "rect": [72, 600, 300, 630]},
                ],
            },
            "generator": {
                "path": "fixtures/make-nav.py",
                "helper": "fixtures/minipdf.py",
                "sha256": sha256_file(__file__),
                "deterministic": True,
                "timestamps_in_output": False,
                "dependencies": ["python3 stdlib only"],
                "reproduce": "python3 fixtures/make-nav.py",
            },
            "provenance": {
                "authorship": "written by the project team for this spike (grok); constructed byte-by-byte, not exported from any third-party tool",
                "third_party_content": "none",
                "fonts": "Base-14 Helvetica, referenced not embedded (/BaseFont /Helvetica, /Encoding /WinAnsiEncoding)",
                "network_access": "none",
                "source_documents": "none - content is the literal ASCII strings one/two/three plus the outline and links named in contract-unit5.md",
            },
            "license": "MIT (LICENSE at the repo root)",
            "generated_on": "2026-09-16",
        },
        indent=2,
    )
    + "\n"
)
print(f"wrote {OUT} ({size} bytes) sha256={digest}")
