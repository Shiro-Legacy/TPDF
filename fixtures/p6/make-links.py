#!/usr/bin/env python3
"""Deterministic fixtures/p6/links.pdf for P6 U4 (links on the page; contract-p6-unit4.md §5, §9.1).

Page 0: runs One/Two/Three and four /Link annots in /Annots order L1 (URI), L2 (/Dest page 2),
L5 (bare: no /A, no /Dest; mupdf skips it), L3 (ftp URI, blocked). y-up 100..500 is empty (draw space).
Page 1: /Rotate 90, run "Rotated" and one URI link L4. Page 2: run "Target".
python3 stdlib + fixtures/minipdf.py.

Run: python3 fixtures/p6/make-links.py
"""
import hashlib
import json
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from minipdf import emit, stream  # noqa: E402

OUT = HERE / "links.pdf"
PAGE_H = 792

# Object numbers (1-based, fixed by the list order in main()).
OBJ = {"L1": 10, "L2": 11, "L5": 12, "L3": 13, "L4": 14}
LINKS = {  # name -> (page, y-up /Rect, action bytes or None, kind, target)
    "L1": (0, [72, 700, 200, 720], b"/A << /S /URI /URI (https://example.org/one) >>", "uri", "https://example.org/one"),
    "L2": (0, [72, 650, 200, 670], b"/Dest [5 0 R /Fit]", "page", 2),
    "L5": (0, [72, 630, 200, 640], None, "skipped", None),
    "L3": (0, [72, 600, 200, 620], b"/A << /S /URI /URI (ftp://example.org/x) >>", "blocked", "ftp://example.org/x"),
    "L4": (1, [72, 700, 200, 720], b"/A << /S /URI /URI (https://example.org/rot) >>", "uri", "https://example.org/rot"),
}
RUNS = {0: [("One", 72, 704), ("Two", 72, 654), ("Three", 72, 604)], 1: [("Rotated", 72, 700)], 2: [("Target", 72, 700)]}
# mupdf page-space rect of L4 on the /Rotate 90 page, read back through the former build/renderer/engine.js
# (`links` page 1) by .quad/scratch/p6u4-fixture-prove.mjs; asserted there, recorded here.
L4_PAGE_SPACE = [700, 72, 720, 200]


def link(name):
    page, rect, action, _, _ = LINKS[name]
    tail = b" " + action if action else b""
    return (b"<< /Type /Annot /Subtype /Link /P %d 0 R /Rect [%s] /Border [0 0 0] /F 4%s >>"
            % (3 + page, " ".join(str(n) for n in rect).encode(), tail))


def content(page):
    return b"".join(b"BT /F1 12 Tf %d %d Td (%s) Tj ET\n" % (x, y, t.encode()) for t, x, y in RUNS[page])


def ydown(rect):
    x0, y0, x1, y1 = rect
    return [x0, PAGE_H - y1, x1, PAGE_H - y0]


def link_record(name):
    page, rect, action, kind, target = LINKS[name]
    rec = {"page": page, "obj": OBJ[name], "kind": kind, "rect_pdf": rect}
    if kind == "uri" or kind == "blocked":
        rec["uri"] = target
    elif kind == "page":
        rec["target_page"] = target
    if page == 0:
        d = ydown(rect)
        rec.update(rect_y_down=d, centre_y_down=[(d[0] + d[2]) / 2, (d[1] + d[3]) / 2])
    else:
        p = L4_PAGE_SPACE
        rec.update(rect_page_space=p, centre_page_space=[(p[0] + p[2]) / 2, (p[1] + p[3]) / 2],
                   note="/Rotate 90: page shown 792x612; drive/click space is mupdf page space (engine `links`), not the y-up mirror")
    if action is None:
        rec["note"] = "bare /Link: no /A, no /Dest; getLinks() skips it (index-drift guard)"
    return rec


def provenance(size, digest):
    rec = {
        "id": "p6-links",
        "file": "fixtures/p6/links.pdf",
        "bytes": size,
        "sha256": digest,
        "pages": 3,
        "page_size_pt": [612, 792],
        "blocks": {
            "note": "pdf = y-up page pt; y_down = 792 - y (drive click space on the unrotated pages 0 and 2).",
            "runs": {str(p): [{"text": t, "font": "Helvetica", "size": 12, "origin_pdf": [x, y], "origin_y_down": [x, PAGE_H - y]}
                              for t, x, y in r] for p, r in RUNS.items()},
            "links": {name: link_record(name) for name in ("L1", "L2", "L5", "L3", "L4")},
            "annots_order": {"0": ["L1", "L2", "L5", "L3"], "1": ["L4"], "2": []},
            "annots_obj": {"0": [OBJ["L1"], OBJ["L2"], OBJ["L5"], OBJ["L3"]], "1": [OBJ["L4"]], "2": []},
            "engine_links": {
                "0": [{"id": 0, "annotId": OBJ["L1"], "kind": "uri", "uri": "https://example.org/one"},
                      {"id": 1, "annotId": OBJ["L2"], "kind": "page", "page": 2},
                      {"id": 2, "annotId": OBJ["L3"], "kind": "blocked", "uri": "ftp://example.org/x"}],
                "1": [{"id": 0, "annotId": OBJ["L4"], "kind": "uri", "uri": "https://example.org/rot", "rect": L4_PAGE_SPACE}],
                "2": [],
            },
            "p0_empty": {"band_pdf": [0, 100, 612, 500], "band_y_down": [0, 292, 612, 692], "note": "nothing painted here (link draw space)"},
            "p1_rotate": 90,
        },
        "generator": {
            "path": "fixtures/p6/make-links.py",
            "helper": "fixtures/minipdf.py",
            "sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
            "deterministic": True,
            "timestamps_in_output": False,
            "dependencies": ["python3 stdlib only"],
            "reproduce": "python3 fixtures/p6/make-links.py",
        },
        "provenance": {
            "authorship": "written by the project team (opus55); constructed byte-by-byte",
            "third_party_content": "none",
            "fonts": "Base-14 Helvetica, referenced not embedded",
            "network_access": "none",
        },
        "license": "no license selected here; follows the app's final license choice",
        "generated_on": "2026-09-24",
    }
    OUT.with_suffix(".provenance.json").write_text(json.dumps(rec, indent=2) + "\n")


def main():
    font = b"/Font << /F1 6 0 R >>"

    def page(n, annots, extra=b""):
        return (b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792]%s /Resources << %s >> /Contents %d 0 R%s >>"
                % (extra, font, 7 + n, b" /Annots [%s]" % annots if annots else b""))

    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>",
        page(0, b"10 0 R 11 0 R 12 0 R 13 0 R"),
        page(1, b"14 0 R", b" /Rotate 90"),
        page(2, b""),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        stream(content(0)),
        stream(content(1)),
        stream(content(2)),
        link("L1"),
        link("L2"),
        link("L5"),
        link("L3"),
        link("L4"),
    ]
    size, digest = emit(OUT, objects)
    data = OUT.read_bytes()
    assert data.count(b"%%EOF") == 1 and b"/Prev" not in data
    assert b"/JavaScript" not in data and b"/OpenAction" not in data
    provenance(size, digest)
    print(f"wrote {OUT} ({size} bytes) sha256={digest}")


if __name__ == "__main__":
    main()
