#!/usr/bin/env python3
"""Deterministic fixtures/p6/tabs.pdf for P6 U4 (/Tabs field order; contract-p6-unit4.md §5, §9.17).

AcroForm text fields, 120 x 20 pt each (widget dict shape of fixtures/make-form.py).
Page 0 /Tabs /C: a1..a3 at x 72, b1..b3 at x 300, rows y-up 700/650/600; /Annots b3 a1 b1 a2 b2 a3.
Page 1 /Tabs /A: t1/t2/t3 at x 72/220/368, y-up 700; /Annots t3 t1 t2.
Page 2 /Tabs /S: s1/s2/s3 (same x's), /StructParent 0/1/2; StructTreeRoot /K [E2 E3 E1] -> OBJR s2, s3, s1;
  /Annots s1 s2 s3. No /StructParents on the page (no marked content).
Page 3 /Tabs /S: u1/u2 at x 72/220, no /StructParent(s) (fallback page); /Annots u2 u1.
python3 stdlib + fixtures/minipdf.py.

Run: python3 fixtures/p6/make-tabs.py
"""
import hashlib
import json
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from minipdf import emit, stream  # noqa: E402

OUT = HERE / "tabs.pdf"
PAGE_H = 792
W, H = 120, 20

# name -> (page, x, y-up bottom); object numbers follow this order from 13.
FIELDS = {
    "a1": (0, 72, 700), "a2": (0, 72, 650), "a3": (0, 72, 600),
    "b1": (0, 300, 700), "b2": (0, 300, 650), "b3": (0, 300, 600),
    "t1": (1, 72, 700), "t2": (1, 220, 700), "t3": (1, 368, 700),
    "s1": (2, 72, 700), "s2": (2, 220, 700), "s3": (2, 368, 700),
    "u1": (3, 72, 700), "u2": (3, 220, 700),
}
OBJ = {name: 13 + i for i, name in enumerate(FIELDS)}
STRUCT_PARENT = {"s1": 0, "s2": 1, "s3": 2}
TABS = ["C", "A", "S", "S"]
ANNOTS = [["b3", "a1", "b1", "a2", "b2", "a3"], ["t3", "t1", "t2"], ["s1", "s2", "s3"], ["u2", "u1"]]
ORDER = [["a1", "a2", "a3", "b1", "b2", "b3"], ["t3", "t1", "t2"], ["s2", "s3", "s1"], ["u1", "u2"]]
TABS_ORDER = ["C", "A", "S", "fallback"]
LABEL = ["Tabs C", "Tabs A", "Tabs S", "Tabs S fallback"]
# Struct objects: R = StructTreeRoot, E1/E2/E3 = StructElems holding the OBJR of s1/s2/s3.
R, E = 27, {"s1": 28, "s2": 29, "s3": 30}
FONT = 8


def rect(name):
    _, x, y = FIELDS[name]
    return [x, y, x + W, y + H]


def ref(name):
    return b"%d 0 R" % OBJ[name]


def widget(name):
    page = FIELDS[name][0]
    sp = b" /StructParent %d" % STRUCT_PARENT[name] if name in STRUCT_PARENT else b""
    return (b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (%s) /V () /DV () /Ff 0 /Rect [%s] /F 4 /P %d 0 R"
            b" /DA (/Helv 12 Tf 0 g)%s >>" % (name.encode(), " ".join(str(n) for n in rect(name)).encode(), 3 + page, sp))


def field_record(name):
    x0, y0, x1, y1 = rect(name)
    d = [x0, PAGE_H - y1, x1, PAGE_H - y0]
    return {"page": FIELDS[name][0], "obj": OBJ[name], "rect_pdf": [x0, y0, x1, y1], "rect_y_down": d,
            "centre_y_down": [(d[0] + d[2]) / 2, (d[1] + d[3]) / 2]}


def provenance(size, digest):
    rec = {
        "id": "p6-tabs",
        "file": "fixtures/p6/tabs.pdf",
        "bytes": size,
        "sha256": digest,
        "pages": 4,
        "page_size_pt": [612, 792],
        "blocks": {
            "note": "pdf = y-up page pt; y_down = 792 - y (drive click space; all pages unrotated). /Tx fields 120x20, /Ff 0.",
            "fields": {name: field_record(name) for name in FIELDS},
            "pages": {str(p): {"tabs": TABS[p], "annots_order": ANNOTS[p], "expected_fields_order": ORDER[p],
                               "expected_tabsOrder": TABS_ORDER[p]} for p in range(4)},
            "struct": {"root_obj": R, "K": ["E2", "E3", "E1"], "elem_obj": {"E1": E["s1"], "E2": E["s2"], "E3": E["s3"]},
                       "objr_dfs_order": ["s2", "s3", "s1"], "struct_parent": STRUCT_PARENT,
                       "note": "page 2 has no /StructParents; page 3 widgets carry no /StructParent (no structIndex)"},
        },
        "generator": {
            "path": "fixtures/p6/make-tabs.py",
            "helper": "fixtures/minipdf.py",
            "sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
            "deterministic": True,
            "timestamps_in_output": False,
            "dependencies": ["python3 stdlib only"],
            "reproduce": "python3 fixtures/p6/make-tabs.py",
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
    def page(n):
        annots = b" ".join(ref(name) for name in ANNOTS[n])
        return (b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Tabs /%s /Resources << /Font << /F1 %d 0 R >> >>"
                b" /Contents %d 0 R /Annots [%s] >>" % (TABS[n].encode(), FONT, 9 + n, annots))

    def elem(name):
        return (b"<< /Type /StructElem /S /Form /P %d 0 R /Pg 5 0 R /K << /Type /OBJR /Obj %s /Pg 5 0 R >> >>"
                % (R, ref(name)))

    objects = [
        b"<< /Type /Catalog /Pages 2 0 R /AcroForm 7 0 R /MarkInfo << /Marked true >> /StructTreeRoot %d 0 R >>" % R,
        b"<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R 6 0 R] /Count 4 >>",
        page(0), page(1), page(2), page(3),
        b"<< /Fields [%s] /DR << /Font << /Helv %d 0 R >> >> /DA (/Helv 12 Tf 0 g) >>"
        % (b" ".join(ref(name) for name in FIELDS), FONT),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        *[stream(b"BT /F1 12 Tf 72 750 Td (%s) Tj ET\n" % LABEL[n].encode()) for n in range(4)],
        *[widget(name) for name in FIELDS],
        b"<< /Type /StructTreeRoot /K [%d 0 R %d 0 R %d 0 R] /ParentTree << /Nums [0 %d 0 R 1 %d 0 R 2 %d 0 R] >>"
        b" /ParentTreeNextKey 3 >>" % (E["s2"], E["s3"], E["s1"], E["s1"], E["s2"], E["s3"]),
        elem("s1"), elem("s2"), elem("s3"),
    ]
    assert len(objects) == 30 and OBJ["u2"] == 26
    size, digest = emit(OUT, objects)
    data = OUT.read_bytes()
    assert data.count(b"%%EOF") == 1 and b"/Prev" not in data
    assert b"/TI" not in data and b"/StructParents" not in data
    provenance(size, digest)
    print(f"wrote {OUT} ({size} bytes) sha256={digest}")


if __name__ == "__main__":
    main()
