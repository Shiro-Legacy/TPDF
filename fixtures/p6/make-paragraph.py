#!/usr/bin/env python3
"""Deterministic fixtures/p6/paragraph.pdf for P6 U2 §3 (paragraph reflow) and §2 (run move).

Page 0: a 4-line Helvetica 11 paragraph (Td lines, leading 14), a run 34 pt below its last
baseline (the growth obstacle), a single moveable run, a tail pair, and a 1x1 image placement.
Page 1: an MCID-tagged 3-line paragraph, a T*/TL paragraph, a centred 2-line block (not a
paragraph), a TJ paragraph, and a TD paragraph followed by `0 g T*` (TL restore check).
python3 stdlib + fixtures/minipdf.py.

Run: python3 fixtures/p6/make-paragraph.py
"""
import hashlib
import json
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from minipdf import emit, stream  # noqa: E402

OUT = HERE / "paragraph.pdf"

PARA = ["The quick brown fox jumps over", "the lazy dog while the cat", "watches from a sunny window", "and the birds sing."]
PAGE0 = (
    b"BT /F1 11 Tf 72 700 Td (" + PARA[0].encode() + b") Tj 0 -14 Td (" + PARA[1].encode() + b") Tj 0 -14 Td ("
    + PARA[2].encode() + b") Tj 0 -14 Td (" + PARA[3].encode() + b") Tj ET\n"
    b"BT /F1 11 Tf 72 624 Td (Below the paragraph) Tj ET\n"
    b"BT /F1 11 Tf 300 500 Td (Move me) Tj ET\n"
    b"BT /F1 11 Tf 300 450 Td (Head run) Tj (tail run) Tj ET\n"
    b"q 100 0 0 50 72 300 cm /Im1 Do Q\n"
)
PAGE1 = (
    b"/P << /MCID 0 >> BDC BT /F1 11 Tf 72 740 Td (Tagged line one) Tj 0 -14 Td (Tagged line two) Tj "
    b"0 -14 Td (Tagged line three) Tj ET EMC\n"
    b"BT /F1 11 Tf 13 TL 72 640 Td (Leading one) Tj T* (Leading two) Tj T* (Leading three) Tj ET\n"
    b"BT /F1 11 Tf 250 540 Td (Middle block) Tj -10 -14 Td (Centred text) Tj ET\n"
    b"BT /F1 11 Tf 72 440 Td [(Kerned) -200 ( line one)] TJ 0 -14 Td [(Kerned) -100 ( line two)] TJ ET\n"
    b"BT /F1 11 Tf 72 340 Td (TD one) Tj 0 -14 TD (TD two) Tj 0 g T* (After TD) Tj ET\n"
)


def provenance(size, digest):
    rec = {
        "id": "p6-paragraph",
        "file": "fixtures/p6/paragraph.pdf",
        "bytes": size,
        "sha256": digest,
        "pages": 2,
        "page_size_pt": [612, 792],
        "blocks": {
            "note": "origin_pdf is the first baseline, y-up; leading in pt.",
            "p0_paragraph": {"lines": PARA, "font": "Helvetica", "size": 11, "leading": 14, "origin_pdf": [72, 700]},
            "p0_below": {"text": "Below the paragraph", "origin_pdf": [72, 624], "gap_below_last_baseline_pt": 34, "note": "room for exactly one more line at leading 14"},
            "p0_move": {"text": "Move me", "origin_pdf": [300, 500], "moveable": True},
            "p0_tail": {"text": "Head run", "tail": "tail run", "note": "head: tail-would-move; tail: reason tail"},
            "p0_image": {"cm": [100, 0, 0, 50, 72, 300]},
            "p1_tagged": {"lines": ["Tagged line one", "Tagged line two", "Tagged line three"], "mcid": 0, "note": "MCID only, no ActualText"},
            "p1_tstar": {"lines": ["Leading one", "Leading two", "Leading three"], "tl": 13},
            "p1_centred": {"lines": ["Middle block", "Centred text"], "note": "second line moved -10: not a paragraph"},
            "p1_tj": {"lines": ["Kerned line one", "Kerned line two"], "note": "TJ lines, kerned"},
            "p1_td": {"lines": ["TD one", "TD two"], "after": "After TD", "note": "TD sets TL 14; `0 g` ends the paragraph; After TD uses T*"},
        },
        "generator": {
            "path": "fixtures/p6/make-paragraph.py",
            "helper": "fixtures/minipdf.py",
            "sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
            "deterministic": True,
            "timestamps_in_output": False,
            "dependencies": ["python3 stdlib only"],
            "reproduce": "python3 fixtures/p6/make-paragraph.py",
        },
        "provenance": {
            "authorship": "written by the project team (opus55); constructed byte-by-byte",
            "third_party_content": "none",
            "fonts": "Base-14 Helvetica, referenced not embedded",
            "network_access": "none",
        },
        "license": "no license selected here; follows the app's final license choice",
        "generated_on": "2026-09-23",
    }
    OUT.with_suffix(".provenance.json").write_text(json.dumps(rec, indent=2) + "\n")


def main():
    helv = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
    image = stream(b"\x80", b" /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8")
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Resources << /Font << /F1 5 0 R >> /XObject << /Im1 8 0 R >> >> /Contents 6 0 R >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Resources << /Font << /F1 5 0 R >> >> /Contents 7 0 R >>",
        helv,
        stream(PAGE0),
        stream(PAGE1),
        image,
    ]
    size, digest = emit(OUT, objects)
    data = OUT.read_bytes()
    assert data.count(b"%%EOF") == 1 and b"/Prev" not in data
    provenance(size, digest)
    print(f"wrote {OUT} ({size} bytes) sha256={digest}")


if __name__ == "__main__":
    main()
