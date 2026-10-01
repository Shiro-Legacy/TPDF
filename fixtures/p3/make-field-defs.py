#!/usr/bin/env python3
"""Deterministic fixture for verify-roundtrip.py --field-defs tu/ff/dv (from grok's P3 U7 make-scratch.py).

One page: a merged text field "nom" (/TU "Nom (é)", /Ff 2, /DV "défaut"), a checkbox "ok" (/DV /Yes),
a two-kid radio parent "color" (/Ff 32768, /DV /B) and a plain text field "plain".
Run: .venv-qa/bin/python fixtures/p3/make-field-defs.py  (writes field-defs.pdf + field-defs.provenance.json)
"""
import hashlib
import io
import json
import pathlib

import pypdf
from pypdf import PdfWriter
from pypdf.generic import (ArrayObject, DictionaryObject, FloatObject, NameObject,
                           NumberObject, TextStringObject)

OUT = pathlib.Path(__file__).with_name("field-defs.pdf")


def widget(page, rect):
    d = DictionaryObject()
    d.update({
        NameObject("/Type"): NameObject("/Annot"),
        NameObject("/Subtype"): NameObject("/Widget"),
        NameObject("/Rect"): ArrayObject([FloatObject(x) for x in rect]),
        NameObject("/F"): NumberObject(4),
        NameObject("/P"): page.indirect_reference,
    })
    return d


def build():
    writer = PdfWriter()
    page = writer.add_blank_page(width=612, height=792)
    annots = ArrayObject()
    add = writer._add_object

    text = widget(page, [72, 650, 252, 680])
    text[NameObject("/FT")] = NameObject("/Tx")
    text[NameObject("/T")] = TextStringObject("nom")
    text[NameObject("/TU")] = TextStringObject("Nom (é)")
    text[NameObject("/Ff")] = NumberObject(2)
    text[NameObject("/DV")] = TextStringObject("défaut")
    annots.append(add(text))

    check = widget(page, [72, 600, 92, 620])
    check[NameObject("/FT")] = NameObject("/Btn")
    check[NameObject("/T")] = TextStringObject("ok")
    check[NameObject("/DV")] = NameObject("/Yes")
    check[NameObject("/V")] = NameObject("/Yes")
    check[NameObject("/AS")] = NameObject("/Yes")
    annots.append(add(check))

    parent = DictionaryObject()
    parent.update({
        NameObject("/FT"): NameObject("/Btn"),
        NameObject("/T"): TextStringObject("color"),
        NameObject("/Ff"): NumberObject(32768),
        NameObject("/DV"): NameObject("/B"),
        NameObject("/V"): NameObject("/B"),
    })
    kids = ArrayObject()
    for rect, on in (([72, 560, 92, 580], False), ([100, 560, 120, 580], True)):
        kid = widget(page, rect)
        kid[NameObject("/AS")] = NameObject("/B" if on else "/Off")
        states = DictionaryObject()
        for state in ("A", "B", "Off"):
            states[NameObject("/" + state)] = DictionaryObject()
        kid[NameObject("/AP")] = DictionaryObject({NameObject("/N"): states})
        kids.append(add(kid))
        annots.append(kids[-1])
    parent[NameObject("/Kids")] = kids
    parent_ref = add(parent)
    for ref in kids:
        ref.get_object()[NameObject("/Parent")] = parent_ref

    plain = widget(page, [72, 520, 192, 545])
    plain[NameObject("/FT")] = NameObject("/Tx")
    plain[NameObject("/T")] = TextStringObject("plain")
    annots.append(add(plain))

    page[NameObject("/Annots")] = annots
    writer.root_object[NameObject("/AcroForm")] = DictionaryObject({
        NameObject("/Fields"): ArrayObject([annots[0], annots[1], parent_ref, annots[-1]]),
    })
    buf = io.BytesIO()
    writer.write(buf)
    return buf.getvalue()


data = build()
assert data == build(), "generator is not deterministic"
OUT.write_bytes(data)
digest = hashlib.sha256(data).hexdigest()
provenance = {
    "id": "p3-field-defs",
    "file": "fixtures/p3/field-defs.pdf",
    "bytes": len(data),
    "sha256": digest,
    "pages": 1,
    "page_size_pt": [612, 792],
    "notes": [
        "text field nom: /TU (Nom (é)), /Ff 2, /DV (défaut)",
        "checkbox ok: /DV /Yes, /V /Yes",
        "radio parent color: /Ff 32768, /DV /B, two kids with /AP /N states A, B, Off",
        "text field plain: no /TU, /Ff or /DV",
        "self-check input for scripts/verify-roundtrip.py --field-defs tu/ff/dv",
    ],
    "generator": {
        "path": "fixtures/p3/make-field-defs.py",
        "sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
        "reproduce": ".venv-qa/bin/python fixtures/p3/make-field-defs.py",
        "deterministic": True,
        "timestamps_in_output": False,
        "dependencies": [f"pypdf {pypdf.__version__} (.venv-qa, dev-only)"],
    },
    "provenance": {"authorship": "written by the project team for P3 U7 (grok), promoted in debt sweep 1", "third_party_content": "none", "network_access": "none"},
    "generated_on": "2026-09-24",
}
OUT.with_suffix(".provenance.json").write_text(json.dumps(provenance, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
print(f"wrote {OUT} ({len(data)} bytes) sha256={digest}")
