"""Self-check fixture for verify-roundtrip.py --field-defs tu/ff/dv (grok U7).

Authors one page with a merged text field (/TU non-ASCII, /Ff bit 2, /DV),
a checkbox (/DV /Yes) and a two-kid radio parent (/Ff 32768, /DV /B).
Usage: python make-scratch.py <out.pdf>
"""
import sys
from pathlib import Path

from pypdf import PdfWriter
from pypdf.generic import (ArrayObject, DictionaryObject, FloatObject, NameObject,
                           NumberObject, TextStringObject)


def widget(page, rect, extra=None):
    d = DictionaryObject()
    d.update({
        NameObject("/Type"): NameObject("/Annot"),
        NameObject("/Subtype"): NameObject("/Widget"),
        NameObject("/Rect"): ArrayObject([FloatObject(x) for x in rect]),
        NameObject("/F"): NumberObject(4),
        NameObject("/P"): page.indirect_reference,
    })
    for k, v in (extra or {}).items():
        d[NameObject(k)] = v
    return d


def main():
    out = Path(sys.argv[1])
    writer = PdfWriter()
    page = writer.add_blank_page(width=612, height=792)
    annots = ArrayObject()

    def add(d):
        return writer._add_object(d)

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
    for name, rect, on in (("A", [72, 560, 92, 580], False), ("B", [100, 560, 120, 580], True)):
        kid = widget(page, rect)
        kid[NameObject("/AS")] = NameObject("/B" if on else "/Off")
        states = DictionaryObject()
        for state in ("A", "B", "Off"):
            states[NameObject("/" + state)] = DictionaryObject()
        kid[NameObject("/AP")] = DictionaryObject({NameObject("/N"): states})
        kids.append(add(kid))
        annots.append(kids[-1])
    parent[NameObject("/Kids")] = kids

    plain = widget(page, [72, 520, 192, 545])
    plain[NameObject("/FT")] = NameObject("/Tx")
    plain[NameObject("/T")] = TextStringObject("plain")
    annots.append(add(plain))

    page[NameObject("/Annots")] = annots
    writer.root_object[NameObject("/AcroForm")] = DictionaryObject({
        NameObject("/Fields"): ArrayObject([annots[0], annots[1], add(parent), annots[-1]]),
    })
    with out.open("wb") as fh:
        writer.write(fh)
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
