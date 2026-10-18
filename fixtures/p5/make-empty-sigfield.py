#!/usr/bin/env python3
"""Deterministic fixtures/p5/empty-sigfield.pdf for P5 U3 (sign into an existing empty field;
contract-p5-unit3.md §3, §9.7). Three EMPTY /Sig fields (no /V). Page 0 unrotated: Signature1
(obj 7, no /AP) and Signature2 (obj 8, a grey /AP box, obj 9); page 1 /Rotate 90: Signature3
(obj 10, no /AP). /SigFlags 1. python3 stdlib + fixtures/minipdf.py.

Run: python3 fixtures/p5/make-empty-sigfield.py
"""
import hashlib
import json
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
FIX = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(FIX))
from minipdf import emit, stream  # noqa: E402

OUT = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else HERE / "empty-sigfield.pdf"
PAGE_H = 792
DA = b"/DA (/Helv 12 Tf 0 g)"
# name -> (page, obj, raw y-up /Rect, mupdf page-space rect as the engine `fields` reply lists it, hasAP)
FIELDS = {
    "Signature1": (0, 7, [72, 112, 340, 172], [72, 620, 340, 680], False),
    "Signature2": (0, 8, [360, 112, 540, 172], [360, 620, 540, 680], True),
    "Signature3": (1, 10, [72, 500, 300, 560], [500, 72, 560, 300], False),
}

objects = [
    b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [7 0 R 8 0 R 10 0 R] /SigFlags 1 /DR << /Font << /Helv 3 0 R >> >> " + DA + b" >> >>",
    b"<< /Type /Pages /Kids [4 0 R 6 0 R] /Count 2 >>",
    b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents 5 0 R /Annots [7 0 R 8 0 R] >>",
    stream(b"BT /F1 24 Tf 72 700 Td (PDF4All sign on the page) Tj ET\n"),
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Rotate 90 /Resources << /Font << /F1 3 0 R >> >> /Contents 5 0 R /Annots [10 0 R] >>",
    # 7: Signature1, no /AP, y-up [72 112 340 172] -> page space y-down [72 620 340 680]
    b"<< /Type /Annot /Subtype /Widget /FT /Sig /T (Signature1) /F 4 /P 4 0 R /Rect [72 112 340 172] " + DA + b" >>",
    # 8: Signature2 with an existing grey /AP (an empty-field look)
    b"<< /Type /Annot /Subtype /Widget /FT /Sig /T (Signature2) /F 4 /P 4 0 R /Rect [360 112 540 172] /AP << /N 9 0 R >> " + DA + b" >>",
    stream(b"q 0.85 g 0 0 180 60 re f Q\n", b" /Type /XObject /Subtype /Form /BBox [0 0 180 60]"),
    # 10: Signature3 on the rotated page, raw rect [72 500 300 560]
    b"<< /Type /Annot /Subtype /Widget /FT /Sig /T (Signature3) /F 4 /P 6 0 R /Rect [72 500 300 560] " + DA + b" >>",
]


def field_record(name):
    page, obj, rect, ps, has_ap = FIELDS[name]
    rec = {"page": page, "obj": obj, "ft": "Sig", "value": None, "hasAP": has_ap, "rect_pdf": rect, "rect_page_space": ps}
    if page == 0:
        rec["centre_y_down"] = [(ps[0] + ps[2]) / 2, (ps[1] + ps[3]) / 2]
    else:
        rec["centre_page_space"] = [(ps[0] + ps[2]) / 2, (ps[1] + ps[3]) / 2]
        rec["note"] = "/Rotate 90: page shown 792x612; drive/click space is mupdf page space (engine `fields`), not the y-up mirror"
    return rec


def provenance(out, size, digest):
    rec = {
        "id": "p5-empty-sigfield",
        "file": "fixtures/p5/empty-sigfield.pdf",
        "bytes": size,
        "sha256": digest,
        "pages": 2,
        "page_size_pt": [612, 792],
        "blocks": {
            "note": "rect_pdf = raw y-up /Rect; rect_page_space = mupdf page space (y-down on page 0, = 792 - y), as the engine `fields` reply lists it.",
            "runs": {"0": [{"text": "PDF4All sign on the page", "font": "Helvetica", "size": 24, "origin_pdf": [72, 700], "origin_y_down": [72, 92]}],
                     "1": [{"text": "PDF4All sign on the page", "font": "Helvetica", "size": 24, "origin_pdf": [72, 700], "note": "same content stream 5"}]},
            "fields": {name: field_record(name) for name in FIELDS},
            "annots_order": {"0": ["Signature1", "Signature2"], "1": ["Signature3"]},
            "annots_obj": {"0": [7, 8], "1": [10]},
            "acroform_fields_order": ["Signature1", "Signature2", "Signature3"],
            "acroform_fields_obj": [7, 8, 10],
            "sig_flags": 1,
            "with_ap": ["Signature2"],
            "ap_obj": {"Signature2": 9},
            "p1_rotate": 90,
        },
        "generator": {
            "path": "fixtures/p5/make-empty-sigfield.py",
            "helper": "fixtures/minipdf.py",
            "sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
            "deterministic": True,
            "timestamps_in_output": False,
            "dependencies": ["python3 stdlib only"],
            "reproduce": "python3 fixtures/p5/make-empty-sigfield.py",
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
    out.with_suffix(".provenance.json").write_text(json.dumps(rec, indent=2) + "\n")


def main():
    size, digest = emit(OUT, objects)
    data = OUT.read_bytes()
    assert data.count(b"%%EOF") == 1 and b"/Prev" not in data
    assert b"/JavaScript" not in data and b"/OpenAction" not in data and b"/V " not in data
    provenance(OUT, size, digest)
    print(f"wrote {OUT} ({size} bytes) sha256={digest}")


if __name__ == "__main__":
    main()
