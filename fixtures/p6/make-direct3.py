#!/usr/bin/env python3
"""Deterministic fixtures/p6/direct3.pdf for P6 U3 (objects on the page, add text, redaction marks).

Page 0: a 4-line Helvetica 11 paragraph (Td lines, leading 13.2), image Im1 directly below it,
image Im2 top right, Form Fm1 (filled rect), a single "Move me" run, and y-up 200..350 left empty.
Page 1: /Rotate 90 with one run (add-text refusal). Page 2: a page-covering image Bg under one run.
Every placement is a top-level `q <cm> /Name Do Q` block (the engine's simple-placement rule).
python3 stdlib + fixtures/minipdf.py.

Run: python3 fixtures/p6/make-direct3.py
"""
import hashlib
import json
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from minipdf import emit, stream  # noqa: E402

OUT = HERE / "direct3.pdf"
PAGE_H = 792

PARA = ["Alpha bravo charlie delta echo", "foxtrot golf hotel india", "juliet kilo lima mike november", "oscar papa quebec"]
PLACEMENTS = {  # name -> cm; Fm1's BBox is [0 0 100 30] under an identity cm translation
    "Im1": [100, 0, 0, 50, 72, 600],
    "Im2": [60, 0, 0, 40, 400, 700],
    "Fm1": [1, 0, 0, 1, 400, 500],
    "Bg": [612, 0, 0, 792, 0, 0],
}


def place(name):
    return b"q %s cm /%s Do Q\n" % (" ".join(str(n) for n in PLACEMENTS[name]).encode(), name.encode())


PAGE0 = (
    b"BT /F1 11 Tf 72 700 Td (" + PARA[0].encode() + b") Tj 0 -13.2 Td (" + PARA[1].encode() + b") Tj 0 -13.2 Td ("
    + PARA[2].encode() + b") Tj 0 -13.2 Td (" + PARA[3].encode() + b") Tj ET\n"
    + place("Im1") + place("Im2") + place("Fm1")
    + b"BT /F1 14 Tf 72 400 Td (Move me) Tj ET\n"
)
PAGE1 = b"BT /F1 12 Tf 72 700 Td (Rotated) Tj ET\n"
PAGE2 = place("Bg") + b"BT /F1 12 Tf 72 700 Td (On top) Tj ET\n"
FORM = b"0.2 0.5 0.8 rg 0 0 100 30 re f\n"


def box(name, rect=(0, 0, 1, 1)):
    a, _, _, d, e, f = PLACEMENTS[name]
    x0, y0, x1, y1 = a * rect[0] + e, d * rect[1] + f, a * rect[2] + e, d * rect[3] + f
    return {
        "cm": PLACEMENTS[name],
        "box_pdf": [x0, y0, x1, y1],
        "rect_y_down": [x0, PAGE_H - y1, x1, PAGE_H - y0],
        "centre_y_down": [(x0 + x1) / 2, PAGE_H - (y0 + y1) / 2],
    }


def run(text, x, y, size):
    # click_y_down: a point inside the first glyphs (baseline minus a third of the size).
    return {"text": text, "font": "Helvetica", "size": size, "origin_pdf": [x, y], "origin_y_down": [x, PAGE_H - y],
            "click_y_down": [x + 10, round(PAGE_H - y - size / 3, 2)]}


def provenance(size, digest):
    rec = {
        "id": "p6-direct3",
        "file": "fixtures/p6/direct3.pdf",
        "bytes": size,
        "sha256": digest,
        "pages": 3,
        "page_size_pt": [612, 792],
        "blocks": {
            "note": "pdf = y-up page pt; y_down = 792 - y (drive click/handle space, unrotated pages). origin = first baseline.",
            "p0_paragraph": {"lines": PARA, "font": "Helvetica", "size": 11, "leading": 13.2, "origin_pdf": [72, 700],
                             "origin_y_down": [72, 92], "baselines_pdf": [700, 686.8, 673.6, 660.4],
                             "click_y_down": [82, round(92 - 11 / 3, 2)]},
            "p0_im1": {**box("Im1"), "image": "1x1 DeviceGray 0x80", "note": "directly below the paragraph (§1.3 obstacle)"},
            "p0_im2": {**box("Im2"), "image": "1x1 DeviceRGB 20 60 c0"},
            "p0_fm1": {**box("Fm1", (0, 0, 100, 30)), "bbox": [0, 0, 100, 30], "draws": "filled rect 0.2 0.5 0.8 rg"},
            "p0_move": {**run("Move me", 72, 400, 14), "moveable": True},
            "p0_empty": {"band_pdf": [0, 200, 612, 350], "band_y_down": [0, 442, 612, 592], "click_y_down": [300, 500],
                         "note": "nothing painted here (click-to-add)"},
            "p1_rotated": {**run("Rotated", 72, 700, 12), "rotate": 90,
                           "note": "page shown 792x612; y_down mirror is for the unrotated page; add-text refuses unsupported-transform"},
            "p2_bg": {**box("Bg"), "image": "1x1 DeviceGray 0xe8 (own object)", "note": "covers the page: no on-page target (page-cover rule)"},
            "p2_on_top": {**run("On top", 72, 700, 12), "note": "drawn after Bg"},
        },
        "generator": {
            "path": "fixtures/p6/make-direct3.py",
            "helper": "fixtures/minipdf.py",
            "sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
            "deterministic": True,
            "timestamps_in_output": False,
            "dependencies": ["python3 stdlib only"],
            "reproduce": "python3 fixtures/p6/make-direct3.py",
        },
        "provenance": {
            "authorship": "written by the project team (opus55); constructed byte-by-byte",
            "third_party_content": "none",
            "fonts": "Base-14 Helvetica, referenced not embedded",
            "network_access": "none",
        },
        "license": "MIT (LICENSE at the repo root)",
        "generated_on": "2026-09-24",
    }
    OUT.with_suffix(".provenance.json").write_text(json.dumps(rec, indent=2) + "\n")


def main():
    helv = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
    img = b" /Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace "
    font = b"/Font << /F1 6 0 R >>"
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << " + font
        + b" /XObject << /Im1 10 0 R /Im2 11 0 R /Fm1 12 0 R >> >> /Contents 7 0 R >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Rotate 90 /Resources << " + font + b" >> /Contents 8 0 R >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << " + font
        + b" /XObject << /Bg 13 0 R >> >> /Contents 9 0 R >>",
        helv,
        stream(PAGE0),
        stream(PAGE1),
        stream(PAGE2),
        stream(b"\x80", img + b"/DeviceGray"),
        stream(b"\x20\x60\xc0", img + b"/DeviceRGB"),
        stream(FORM, b" /Type /XObject /Subtype /Form /BBox [0 0 100 30] /Resources << >>"),
        stream(b"\xe8", img + b"/DeviceGray"),
    ]
    size, digest = emit(OUT, objects)
    data = OUT.read_bytes()
    assert data.count(b"%%EOF") == 1 and b"/Prev" not in data
    provenance(size, digest)
    print(f"wrote {OUT} ({size} bytes) sha256={digest}")


if __name__ == "__main__":
    main()
