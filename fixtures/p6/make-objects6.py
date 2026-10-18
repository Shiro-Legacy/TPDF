#!/usr/bin/env python3
"""Deterministic fixtures/p6/objects6.pdf for P6 U6 (rotate / flip / crop, Form pass-through, fresh-name paste).

Page 0: a plain Im1, a rotated-cw Im2, a flipped Im3, a clipped (cropped) Im4, a Form Fm1 that wraps
Im5 and the "Over" run. Page 1: its own /Im1 naming a different (sixth) image.
Every placement is a top-level `q ... Q` block. python3 stdlib + fixtures/minipdf.py.

Run: python3 fixtures/p6/make-objects6.py
"""
import hashlib
import json
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from minipdf import emit, stream  # noqa: E402

OUT = HERE / "objects6.pdf"
PAGE_H = 792

PLACEMENTS = {  # name -> (cm, box_pdf of the full placement)
    "Im1": ([100, 0, 0, 50, 72, 600], [72, 600, 172, 650]),
    "Im2": ([0, -100, 50, 0, 400, 750], [400, 650, 450, 750]),
    "Im3": ([-100, 0, 0, 50, 300, 600], [200, 600, 300, 650]),
    "Im4": ([100, 0, 0, 50, 72, 400], [72, 400, 172, 450]),
    "Fm1": ([1, 0, 0, 1, 72, 100], [72, 100, 372, 300]),
    "Im5": ([50, 0, 0, 50, 300, 120], [300, 120, 350, 170]),
}
IMAGES = {"Im1": (4, 2), "Im2": (2, 4), "Im3": (3, 2), "Im4": (5, 2), "Im5": (2, 2), "p1_Im1": (6, 3)}
IM4_CLIP = [72, 400, 60, 40]  # x y w h -> visible [72,400,132,440]


def cm(name):
    return " ".join(str(n) for n in PLACEMENTS[name][0]).encode()


def place(name):
    return b"q %s cm /%s Do Q\n" % (cm(name), name.encode())


PAGE0 = (
    place("Im1") + place("Im2") + place("Im3")
    + b"q %s re W n %s cm /Im4 Do Q\n" % (" ".join(str(n) for n in IM4_CLIP).encode(), cm("Im4"))
    + place("Fm1") + place("Im5")
    + b"BT /F1 14 Tf 100 250 Td (Over) Tj ET\n"
)
PAGE1 = b"q 100 0 0 50 72 600 cm /Im1 Do Q\n"
FORM = b"0.9 g 0 0 300 200 re f\n"


def y_down(r):
    return [r[0], PAGE_H - r[3], r[2], PAGE_H - r[1]]


def box(name, rect=None):
    m, r = PLACEMENTS[name]
    r = rect or r
    return {"cm": m, "box_pdf": r, "rect_y_down": y_down(r),
            "centre_y_down": [(r[0] + r[2]) / 2, PAGE_H - (r[1] + r[3]) / 2]}


def provenance(size, digest):
    x, y, w, h = IM4_CLIP
    rec = {
        "id": "p6-objects6",
        "file": "fixtures/p6/objects6.pdf",
        "bytes": size,
        "sha256": digest,
        "pages": 2,
        "page_size_pt": [612, 792],
        "blocks": {
            "note": "pdf = y-up page pt; y_down = 792 - y (drive click/handle space). Images are DeviceGray 8-bit, every sample 0x80.",
            "p0_im1": {**box("Im1"), "image": "4x2", "orient": "plain"},
            "p0_im2": {**box("Im2"), "image": "2x4", "orient": "rotated cw"},
            "p0_im3": {**box("Im3"), "image": "3x2", "orient": "flipped horizontally"},
            "p0_im4": {**box("Im4"), "image": "5x2", "clip_re": IM4_CLIP,
                       "visible_pdf": [x, y, x + w, y + h], "visible_y_down": y_down([x, y, x + w, y + h]),
                       "centre_y_down": [x + w / 2, PAGE_H - (y + h / 2)]},
            "p0_fm1": {**box("Fm1"), "bbox": [0, 0, 300, 200], "draws": "0.9 g filled rect",
                       "band_click_y_down": [74, 600], "interior_empty_click_y_down": [200, 600]},
            "p0_im5": {**box("Im5"), "image": "2x2", "note": "inside Fm1's box"},
            "p0_over": {"text": "Over", "font": "Helvetica", "size": 14, "origin_pdf": [100, 250],
                        "origin_y_down": [100, 542], "click_y_down": [115, 537], "note": "inside Fm1's box"},
            "p0_empty": {"click_y_down": [500, 450], "note": "nothing painted here"},
            "p1_im1": {"cm": [100, 0, 0, 50, 72, 600], "box_pdf": [72, 600, 172, 650], "rect_y_down": [72, 142, 172, 192],
                       "image": "6x3", "note": "page 1's own /Im1 names a different image than page 0's /Im1"},
        },
        "generator": {
            "path": "fixtures/p6/make-objects6.py",
            "helper": "fixtures/minipdf.py",
            "sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
            "deterministic": True,
            "timestamps_in_output": False,
            "dependencies": ["python3 stdlib only"],
            "reproduce": "python3 fixtures/p6/make-objects6.py",
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


def image(name):
    w, h = IMAGES[name]
    return stream(b"\x80" * (w * h),
                  b" /Type /XObject /Subtype /Image /Width %d /Height %d /BitsPerComponent 8 /ColorSpace /DeviceGray" % (w, h))


def main():
    helv = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >>"
        b" /XObject << /Im1 8 0 R /Im2 9 0 R /Im3 10 0 R /Im4 11 0 R /Im5 12 0 R /Fm1 13 0 R >> >> /Contents 6 0 R >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Im1 14 0 R >> >> /Contents 7 0 R >>",
        helv,
        stream(PAGE0),
        stream(PAGE1),
        image("Im1"), image("Im2"), image("Im3"), image("Im4"), image("Im5"),
        stream(FORM, b" /Type /XObject /Subtype /Form /BBox [0 0 300 200] /Resources << >>"),
        image("p1_Im1"),
    ]
    size, digest = emit(OUT, objects)
    data = OUT.read_bytes()
    assert data.count(b"%%EOF") == 1 and b"/Prev" not in data
    provenance(size, digest)
    print(f"wrote {OUT} ({size} bytes) sha256={digest}")


if __name__ == "__main__":
    main()
