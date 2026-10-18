#!/usr/bin/env python3
"""Deterministic fixtures/edit-tj.pdf for P3b U1.

Page 1: kerned TJ, a growable short+tail with 200 pt free, a distinct
tight+tail-two line blocked 10 pt to the right, a ' operator, and Times-Roman at Tz 80 Tc 1.5 Tw 3.
Page 2: /Rotate 90. Page 3: unmatched q/cm element, then page text.
Page 4: MediaBox origin [20 30 632 822]. python3 stdlib + fixtures/minipdf.py.

Run: python3 fixtures/make-edit-tj.py
"""
import hashlib
import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from minipdf import emit, stream  # noqa: E402

OUT = pathlib.Path(__file__).resolve().parent / "edit-tj.pdf"

# PDF y-up. Neighbour and margin x are set so the measured gaps are 10 pt and 200 pt
# (MuPDF Helvetica advances; see provenance runs).
GROW_Y = 640
BLOCK_Y = 560
NEIGHBOUR_X = 142.024  # 72 + Helvetica-12 advances of "tight"+"tail two" + 10
MARGIN_X = 306.020
MARGIN_Y = 280


def provenance(size, digest):
    script = pathlib.Path(__file__).read_bytes()
    rec = {
        "id": "edit-tj",
        "file": "fixtures/edit-tj.pdf",
        "bytes": size,
        "sha256": digest,
        "pages": 4,
        "page_size_pt": [612, 792],
        "runs": {
            "note": "origin_pdf is the text baseline, y-up. rect is MuPDF page space, y-down, filled from measurement.",
            "a_tj": {
                "text": "KernedTEXThere",
                "op": "[(Kerned) -250 (TEXT) 120 (here)] TJ",
                "font": "Helvetica",
                "size": 12,
                "origin_pdf": [72, 720],
            },
            "b_grow": {
                "text": "short",
                "tail": "tail follows",
                "op": "(short) Tj (tail follows) Tj",
                "origin_pdf": [72, GROW_Y],
                "free_right_pt": 200,
                "note": "no same-baseline neighbour; page text right margin is the MARGIN run, 200 pt past the tail",
            },
            "c_blocked": {
                "text": "tight",
                "tail": "tail two",
                "op": "(tight) Tj (tail two) Tj",
                "origin_pdf": [72, BLOCK_Y],
                "neighbour": "block",
                "neighbour_origin_pdf": [NEIGHBOUR_X, BLOCK_Y],
                "gap_pt": 10,
                "note": "strings unique on the page (§8.16); absolutely positioned neighbour; growth must refuse",
            },
            "d_quote": {
                "text": "quoted",
                "op": "'",
                "origin_pdf": [72, 480],
                "note": "still refused unsupported-text-operator",
            },
            "e_tz": {
                "text": "scaled",
                "font": "Times-Roman",
                "size": 12,
                "tz": 80,
                "tc": 1.5,
                "tw": 3,
                "origin_pdf": [72, 400],
            },
            "margin": {
                "text": "MARGIN",
                "origin_pdf": [MARGIN_X, MARGIN_Y],
                "note": "sets the page text right margin; not on the grow or blocked baseline",
            },
            "page2": {"page_index": 1, "rotate": 90, "text": "rotated", "origin_pdf": [72, 700]},
            "page3": {
                "page_index": 2,
                "text": "unbalanced",
                "note": "first /Contents element is unmatched 'q 2 0 0 2 100 100 cm'; page text is the next element",
            },
            "page4": {
                "page_index": 3,
                "media_box": [20, 30, 632, 822],
                "text": "shifted",
                "origin_pdf": [72, 700],
            },
        },
        "generator": {
            "path": "fixtures/make-edit-tj.py",
            "helper": "fixtures/minipdf.py",
            "sha256": hashlib.sha256(script).hexdigest(),
            "deterministic": True,
            "timestamps_in_output": False,
            "dependencies": ["python3 stdlib only"],
            "reproduce": "python3 fixtures/make-edit-tj.py",
        },
        "provenance": {
            "authorship": "written by the project team (grok47); constructed byte-by-byte",
            "third_party_content": "none",
            "fonts": "Base-14 Helvetica and Times-Roman, referenced not embedded",
            "network_access": "none",
        },
        "license": "MIT (LICENSE at the repo root)",
        "generated_on": "2026-09-22",
    }
    OUT.with_suffix(".provenance.json").write_text(json.dumps(rec, indent=2) + "\n")


def main():
    helv = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
    times = b"<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding /WinAnsiEncoding >>"
    page1 = (
        b"BT /F1 12 Tf 72 720 Td [(Kerned) -250 (TEXT) 120 (here)] TJ ET\n"
        b"BT /F1 12 Tf 72 %d Td (short) Tj (tail follows) Tj ET\n"
        b"BT /F1 12 Tf 72 %d Td (tight) Tj (tail two) Tj ET\n"
        b"BT /F1 12 Tf %.3f %d Td (block) Tj ET\n"
        b"BT /F1 12 Tf %.3f %d Td (MARGIN) Tj ET\n"
        b"BT /F1 12 Tf 72 480 Td (quoted) ' ET\n"
        b"BT /F2 12 Tf 80 Tz 1.5 Tc 3 Tw 72 400 Td (scaled) Tj ET\n"
        % (GROW_Y, BLOCK_Y, NEIGHBOUR_X, BLOCK_Y, MARGIN_X, MARGIN_Y)
    )
    page2 = b"BT /F1 12 Tf 72 700 Td (rotated) Tj ET\n"
    #  9 Page3  10 unmatched q  11 page3 text  12 Page4  13 page4 text
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R 4 0 R 9 0 R 12 0 R] /Count 4 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Resources << /Font << /F1 5 0 R /F2 6 0 R >> >> /Contents 7 0 R >>",
        b"<< /Type /Page /Parent 2 0 R /Rotate 90 /MediaBox [0 0 612 792] "
        b"/Resources << /Font << /F1 5 0 R >> >> /Contents 8 0 R >>",
        helv,
        times,
        stream(page1),
        stream(page2),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Resources << /Font << /F1 5 0 R >> >> /Contents [10 0 R 11 0 R] >>",
        stream(b"q 2 0 0 2 100 100 cm\n"),
        stream(b"BT /F1 12 Tf 72 700 Td (unbalanced) Tj ET\n"),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [20 30 632 822] "
        b"/Resources << /Font << /F1 5 0 R >> >> /Contents 13 0 R >>",
        stream(b"BT /F1 12 Tf 72 700 Td (shifted) Tj ET\n"),
    ]
    size, digest = emit(OUT, objects)
    data = OUT.read_bytes()
    for needle in (
        b"[(Kerned) -250 (TEXT) 120 (here)] TJ",
        b"(short) Tj (tail follows) Tj",
        b"(tight) Tj (tail two) Tj",
        b"(quoted) '",
        b"80 Tz 1.5 Tc 3 Tw",
        b"q 2 0 0 2 100 100 cm",
        b"/MediaBox [20 30 632 822]",
        b"(unbalanced)",
        b"(shifted)",
        b"/BaseFont /Times-Roman",
        b"/Rotate 90",
        b"(MARGIN)",
        b"(block)",
    ):
        assert needle in data, needle
    assert data.count(b"(short) Tj (tail follows) Tj") == 1
    assert data.count(b"(tight) Tj (tail two) Tj") == 1
    assert data.count(b"%%EOF") == 1 and b"/Prev" not in data
    provenance(size, digest)
    print(f"wrote {OUT} ({size} bytes) sha256={digest}")


if __name__ == "__main__":
    main()
