#!/usr/bin/env python3
"""Deterministic P0-b probe-1 fixtures (Helvetica / minipdf).

Writes edit-transformed.pdf, edit-shared.pdf, edit-tagged.pdf.
Font fixtures: node fixtures/p0b/make-edit-fonts.mjs
Run: python3 fixtures/p0b/make-edit.py
"""
import hashlib
import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
from minipdf import stream  # noqa: E402

DIR = pathlib.Path(__file__).resolve().parent
OLD, NEW = "ALPHA", "BRAVO"
NEI = "NEIGHBOR"


def emit(path, objects, extra_trailer=b""):
    out = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    offsets = []
    for num, body in enumerate(objects, 1):
        offsets.append(len(out))
        out += b"%d 0 obj\n%s\nendobj\n" % (num, body)
    xref = len(out)
    out += b"xref\n0 %d\n" % (len(objects) + 1)
    out += b"0000000000 65535 f \n"
    for off in offsets:
        out += b"%010d 00000 n \n" % off
    out += b"trailer\n<< /Size %d /Root 1 0 R%s >>\nstartxref\n%d\n%%%%EOF\n" % (
        len(objects) + 1, extra_trailer, xref,
    )
    data = bytes(out)
    path.write_bytes(data)
    return len(data), hashlib.sha256(data).hexdigest()


def provenance(ident, path, size, digest, extra):
    rec = {
        "id": ident,
        "file": f"fixtures/p0b/{path.name}",
        "bytes": size,
        "sha256": digest,
        "old": OLD,
        "new": NEW,
        "pages": extra.get("pages", 1),
        "page_size_pt": [612, 792],
        **{k: v for k, v in extra.items() if k != "pages"},
        "generator": {
            "path": "fixtures/p0b/make-edit.py",
            "helper": "fixtures/minipdf.py",
            "sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
            "deterministic": True,
            "timestamps_in_output": False,
            "dependencies": ["python3 stdlib only"],
            "reproduce": "python3 fixtures/p0b/make-edit.py",
        },
        "provenance": {
            "authorship": "written by the project team for this spike (grok); constructed byte-by-byte",
            "third_party_content": "none",
            "fonts": "Base-14 Helvetica, referenced not embedded",
            "network_access": "none",
        },
        "license": "MIT (LICENSE at the repo root)",
        "generated_on": "2026-09-16",
    }
    path.with_suffix(".provenance.json").write_text(json.dumps(rec, indent=2) + "\n")


def write(name, objects, extra, needles):
    path = DIR / name
    size, digest = emit(path, objects)
    data = path.read_bytes()
    for n in needles:
        assert n in data, n
    provenance(name[:-4], path, size, digest, extra)
    print(f"wrote {path} ({size} bytes) sha256={digest}")


# Tiny 8×8 RGB: left half red, right half blue. Unique so we can see it move.
img = bytes(ch for y in range(8) for x in range(8) for ch in ((255, 0, 0) if x < 4 else (0, 0, 255)))
img_extra = b" /Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceRGB /BitsPerComponent 8"
font = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"

# --- transformed: image under rotate+scale cm; text under a scale cm; neighbour ---
t_content = (
    b"q 0.707 0.707 -0.707 0.707 220 480 cm 64 0 0 64 0 0 cm /Im1 Do Q\n"
    b"q 2 0 0 2 72 300 cm\nBT /F1 12 Tf 0 0 Td (%s) Tj ET\nQ\n"
    b"BT /F1 12 Tf 72 500 Td (%s) Tj ET\n" % (OLD.encode(), NEI.encode())
)
write(
    "edit-transformed.pdf",
    [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Resources << /Font << /F1 4 0 R >> /XObject << /Im1 6 0 R >> >> "
        b"/Contents 5 0 R >>",
        font,
        stream(t_content),
        stream(img, img_extra),
    ],
    {"pages": 1, "extractable_text": f"{OLD} {NEI}", "note": "image cm rotate+scale; text cm scale 2"},
    [OLD.encode(), NEI.encode(), b"/Im1"],
)

# --- shared: Form XObject with ALPHA drawn twice on p1; image on p1 and p2 ---
form = b"BT /F1 12 Tf 0 0 Td (%s) Tj ET\n" % OLD.encode()
form_res = b" /Type /XObject /Subtype /Form /BBox [0 0 200 16] /Resources << /Font << /F1 5 0 R >> >>"
p1 = (
    b"q 1 0 0 1 72 700 cm /Fm1 Do Q\n"
    b"q 1 0 0 1 72 600 cm /Fm1 Do Q\n"
    b"q 40 0 0 40 400 700 cm /Im1 Do Q\n"
    b"BT /F1 12 Tf 72 500 Td (%s) Tj ET\n" % NEI.encode()
)
p2 = (
    b"q 40 0 0 40 72 700 cm /Im1 Do Q\n"
    b"BT /F1 12 Tf 72 500 Td (PAGE2) Tj ET\n"
)
write(
    "edit-shared.pdf",
    [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Resources << /Font << /F1 5 0 R >> /XObject << /Fm1 8 0 R /Im1 9 0 R >> >> "
        b"/Contents 6 0 R >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Resources << /Font << /F1 5 0 R >> /XObject << /Im1 9 0 R >> >> "
        b"/Contents 7 0 R >>",
        font,
        stream(p1),
        stream(p2),
        stream(form, form_res),
        stream(img, img_extra),
    ],
    {
        "pages": 2,
        "extractable_text": f"{OLD} {OLD} {NEI} / PAGE2",
        "note": "Fm1 (ALPHA) twice on page 1; Im1 on pages 1 and 2",
    },
    [OLD.encode(), NEI.encode(), b"PAGE2", b"/Fm1", b"/Im1"],
)

# --- tagged: MCID + ActualText + Link next to ALPHA ---
tag_content = (
    b"/Span << /MCID 0 >> BDC\nBT /F1 12 Tf 72 700 Td (%s) Tj ET\nEMC\n"
    b"BT /F1 12 Tf 72 650 Td (%s) Tj ET\n" % (OLD.encode(), NEI.encode())
)
write(
    "edit-tagged.pdf",
    [
        b"<< /Type /Catalog /Pages 2 0 R /MarkInfo << /Marked true >> /StructTreeRoot 6 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /StructParents 0 "
        b"/Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R /Annots [9 0 R] >>",
        font,
        stream(tag_content),
        b"<< /Type /StructTreeRoot /K [7 0 R] /ParentTree 8 0 R >>",
        b"<< /Type /StructElem /S /Span /P 6 0 R /Pg 3 0 R /K 0 /ActualText (%s) >>" % OLD.encode(),
        b"<< /Nums [0 [7 0 R]] >>",
        b"<< /Type /Annot /Subtype /Link /Rect [200 695 280 720] /Border [0 0 0] /F 4 "
        b"/P 3 0 R /A << /S /URI /URI (https://example.com/alpha) >> >>",
    ],
    {
        "pages": 1,
        "extractable_text": f"{OLD} {NEI}",
        "note": "MCID 0 + ActualText ALPHA; Link rect [200 695 280 720]",
        "link_rect": [200, 695, 280, 720],
    },
    [OLD.encode(), NEI.encode(), b"/ActualText (%s)" % OLD.encode(), b"/MCID 0", b"https://example.com/alpha"],
)
