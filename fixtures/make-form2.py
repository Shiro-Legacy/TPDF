#!/usr/bin/env python3
"""Deterministic generator for fixtures/form2.pdf.

One-page AcroForm with the nine P2 unit-4 fields, visible captions, no dates,
no /NeedAppearances. python3 stdlib only.
Run: python3 fixtures/make-form2.py
"""
import json
import pathlib

from minipdf import emit, sha256_file, stream

OUT = pathlib.Path(__file__).with_name("form2.pdf")
DA = b"/DA (/Helv 12 Tf 0 g)"
JS = b"AFNumber_Format(2,0,0,0,\"\",true)"


def caption(size, x, y, text):
    return "BT /F1 %s Tf %s %s Td (%s) Tj ET\n" % (size, x, y, text)


content = (
    caption(24, 72, 740, "PDF4All form2")
    + caption(12, 72, 706, "name")
    + caption(12, 72, 656, "notes")
    + caption(12, 72, 592, "agree")
    + caption(12, 72, 552, "size")
    + caption(10, 182, 552, "S")
    + caption(10, 222, 552, "M")
    + caption(10, 262, 552, "L")
    + caption(12, 72, 516, "choice")
    + caption(12, 72, 456, "pick")
    + caption(12, 72, 396, "fixed")
    + caption(12, 72, 356, "amount")
    + caption(12, 72, 300, "sig")
).encode("ascii")

ap_yes = b"q 1 w 0.5 0.5 17 17 re S 3 3 m 15 15 l S 3 15 m 15 3 l S Q\n"
ap_off = b"q 1 w 0.5 0.5 17 17 re S Q\n"
ap_dot = b"q 1 w 0.5 0.5 17 17 re S 6 6 6 6 re f Q\n"
form_x = b" /Type /XObject /Subtype /Form /BBox [0 0 18 18]"


def widget(body):
    return b"<< /Type /Annot /Subtype /Widget /F 4 /P 3 0 R %s >>" % body


objects = [
    b"<< /Type /Catalog /Pages 2 0 R /AcroForm 6 0 R >>",
    b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
    b"/Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R "
    b"/Annots [7 0 R 8 0 R 9 0 R 11 0 R 12 0 R 13 0 R 14 0 R 15 0 R 16 0 R 17 0 R 18 0 R] >>",
    b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    stream(content),
    b"<< /Fields [7 0 R 8 0 R 9 0 R 10 0 R 14 0 R 15 0 R 16 0 R 17 0 R 18 0 R] "
    b"/DR << /Font << /Helv 4 0 R >> >> /DA (/Helv 12 Tf 0 g) /CO [17 0 R] >>",
    widget(b"/FT /Tx /T (name) /V () /DV () /MaxLen 20 /Rect [160 700 460 724] " + DA),
    widget(b"/FT /Tx /T (notes) /V () /Ff 4096 /Rect [160 620 460 690] " + DA),
    widget(
        b"/FT /Btn /T (agree) /V /Off /AS /Off /Rect [160 590 178 608] "
        b"/AP << /N << /Yes 19 0 R /Off 20 0 R >> >>"
    ),
    b"<< /FT /Btn /T (size) /Ff 49152 /Kids [11 0 R 12 0 R 13 0 R] /V /Off >>",
    widget(
        b"/Parent 10 0 R /Rect [160 550 178 568] /AS /Off "
        b"/AP << /N << /S 21 0 R /Off 20 0 R >> >>"
    ),
    widget(
        b"/Parent 10 0 R /Rect [200 550 218 568] /AS /Off "
        b"/AP << /N << /M 21 0 R /Off 20 0 R >> >>"
    ),
    widget(
        b"/Parent 10 0 R /Rect [240 550 258 568] /AS /Off "
        b"/AP << /N << /L 21 0 R /Off 20 0 R >> >>"
    ),
    widget(b"/FT /Ch /T (choice) /V () /Opt [(one) (two) (three)] /Ff 131072 /Rect [160 510 340 534] " + DA),
    widget(b"/FT /Ch /T (pick) /V () /Opt [(alpha) (beta) (gamma)] /Rect [160 430 340 500] " + DA),
    widget(b"/FT /Tx /T (fixed) /V (locked) /Ff 1 /Rect [160 390 460 414] " + DA),
    widget(
        b"/FT /Tx /T (amount) /V () /Rect [160 350 340 374] " + DA
        + b" /AA << /F << /S /JavaScript /JS (" + JS + b") >> >>"
    ),
    widget(b"/FT /Sig /T (sig) /Rect [160 280 400 340]"),
    stream(ap_yes, form_x),
    stream(ap_off, form_x),
    stream(ap_dot, form_x),
]

size, digest = emit(OUT, objects)
data = OUT.read_bytes()
for needle in (
    b"/T (name)", b"/T (notes)", b"/T (agree)", b"/T (size)", b"/T (choice)",
    b"/T (pick)", b"/T (fixed)", b"/T (amount)", b"/T (sig)", b"/MaxLen 20",
    b"/CO [17 0 R]", JS,
):
    assert needle in data, needle
assert b"/NeedAppearances" not in data

pathlib.Path(OUT.with_suffix(".provenance.json")).write_text(
    json.dumps(
        {
            "id": "form2",
            "file": "fixtures/form2.pdf",
            "bytes": size,
            "sha256": digest,
            "pages": 1,
            "page_size_pt": [612, 792],
            "extractable_text": "PDF4All form2",
            "acroform": {
                "fields": [
                    {"name": "name", "type": "Tx", "value": "", "max_len": 20},
                    {"name": "notes", "type": "Tx", "multiline": True, "value": ""},
                    {"name": "agree", "type": "Btn", "value": "Off", "on": "Yes"},
                    {"name": "size", "type": "Btn", "radio": ["S", "M", "L"], "value": "Off"},
                    {"name": "choice", "type": "Ch", "combo": True, "options": ["one", "two", "three"], "value": ""},
                    {"name": "pick", "type": "Ch", "options": ["alpha", "beta", "gamma"], "value": ""},
                    {"name": "fixed", "type": "Tx", "value": "locked", "read_only": True},
                    {"name": "amount", "type": "Tx", "scripted": True, "value": ""},
                    {"name": "sig", "type": "Sig"},
                ],
                "da": "/Helv 12 Tf 0 g",
                "need_appearances": False,
                "co": ["amount"],
                "scripts": "no /Names /JavaScript; only amount is scripted (/AA + /CO), so other fields stay authorable",
            },
            "generator": {
                "path": "fixtures/make-form2.py",
                "helper": "fixtures/minipdf.py",
                "sha256": sha256_file(__file__),
                "deterministic": True,
                "timestamps_in_output": False,
                "dependencies": ["python3 stdlib only"],
                "reproduce": "python3 fixtures/make-form2.py",
            },
            "provenance": {
                "authorship": "written by the project team for this spike (grok); constructed byte-by-byte, not exported from any third-party tool",
                "third_party_content": "none",
                "fonts": "Base-14 Helvetica, referenced not embedded (/BaseFont /Helvetica, /Encoding /WinAnsiEncoding); AcroForm /Helv aliases the same object",
                "network_access": "none",
                "source_documents": "none - content is the literal ASCII string PDF4All form2 plus nine AcroForm fields named in contract-unit4.md",
            },
            "license": "no license selected here; follows the app's final license choice (progress.md decision pending)",
            "generated_on": "2026-09-16",
        },
        indent=2,
    )
    + "\n"
)
print(f"wrote {OUT} ({size} bytes) sha256={digest}")
