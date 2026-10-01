#!/usr/bin/env python3
"""Deterministic fixtures/redact.pdf and fixtures/redact-shared-only.pdf.

Promotes fixtures/p0b/make-redact.py. Keeps every p0b surface and adds the P4
U1 extras: inline /Span ActualText, a shared Image XObject, an unsigned
/FT /Sig field (no /V, no /SV), and a page-2-only FreeText. python3 stdlib + fixtures/minipdf.py.
No dates, no /ID. Does not touch fixtures/p0b/.

Run: python3 fixtures/make-redact.py
"""
import hashlib
import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from minipdf import emit, stream  # noqa: E402

TOKEN = "P0B-SECRET-7731"
ALT = "P4-ALT-4242"  # inline /ActualText only; applyRedactions leaves this (contract §8.1)
ROOT = pathlib.Path(__file__).resolve().parent
OUT = ROOT / "redact.pdf"
SHARED = ROOT / "redact-shared-only.pdf"

# 5×7 bitmap, bit 4 = leftmost column.
GLYPHS = {
    "P": [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
    "0": [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
    "B": [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
    "-": [0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000],
    "S": [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
    "E": [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
    "C": [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
    "R": [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
    "T": [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
    "7": [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
    "3": [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
    "1": [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
}


def raster_text(text, scale=2, pad=2):
    gw, gh, gap = 5, 7, 1
    width = len(text) * (gw + gap) * scale
    height = gh * scale + pad * 2
    rgb = bytearray(b"\xff" * (width * height * 3))
    for i, ch in enumerate(text):
        glyph = GLYPHS[ch]
        ox = i * (gw + gap) * scale
        for row, bits in enumerate(glyph):
            for col in range(gw):
                if not bits & (1 << (gw - 1 - col)):
                    continue
                for dy in range(scale):
                    for dx in range(scale):
                        x = ox + col * scale + dx
                        y = pad + row * scale + dy
                        off = (y * width + x) * 3
                        rgb[off : off + 3] = b"\x00\x00\x00"
    return bytes(rgb), width, height


def emit_info(path, objects, info_num):
    """Classic xref with an /Info ref. minipdf.emit has no Info slot."""
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
    trailer = b"<< /Size %d /Root 1 0 R /Info %d 0 R >>" % (len(objects) + 1, info_num)
    out += b"trailer\n%s\nstartxref\n%d\n%%%%EOF\n" % (trailer, xref)
    data = bytes(out)
    path.write_bytes(data)
    return len(data), hashlib.sha256(data).hexdigest(), data


def y_down(rect, page_h=792):
    """PDF y-up rect → MuPDF page space (y-down), which drive rects use."""
    x0, y0, x1, y1 = rect
    return [x0, page_h - y1, x1, page_h - y0]


def provenance(path, size, digest, extra):
    script = pathlib.Path(__file__).read_bytes()
    rec = {
        "id": path.stem,
        "file": f"fixtures/{path.name}",
        "bytes": size,
        "sha256": digest,
        "pages": 2,
        "page_size_pt": [612, 792],
        "token": TOKEN,
        "generator": {
            "path": "fixtures/make-redact.py",
            "helper": "fixtures/minipdf.py",
            "sha256": hashlib.sha256(script).hexdigest(),
            "deterministic": True,
            "timestamps_in_output": False,
            "dependencies": ["python3 stdlib only"],
            "reproduce": "python3 fixtures/make-redact.py",
        },
        "provenance": {
            "authorship": "written by the project team (grok47); constructed byte-by-byte",
            "third_party_content": "none",
            "fonts": "Base-14 Helvetica, referenced not embedded",
            "network_access": "none",
            "source_documents": "none - synthetic token P0B-SECRET-7731",
        },
        "license": "no license selected here; follows the app's final license choice",
        "generated_on": "2026-09-22",
    }
    rec.update(extra)
    path.with_suffix(".provenance.json").write_text(json.dumps(rec, indent=2) + "\n")


def build_redact():
    raster, rw, rh = raster_text(TOKEN)
    cover = b"\x00" * (200 * 24 * 3)
    smask = b"\xff" * (rw * rh)
    # Solid black. No token bytes — sharing probe only, not a rasterised secret.
    shared_px = b"\x00" * (8 * 8 * 3)
    form = b"BT /F1 12 Tf 0 2 Td (formxo %s) Tj ET\n" % TOKEN.encode()
    page1 = (
        "BT /F1 12 Tf 72 740 Td (visible %s here) Tj ET\n"
        "BT /F1 12 Tf 3 Tr 72 700 Td (invisible %s) Tj 0 Tr ET\n"
        "1 1 1 rg\nBT /F1 12 Tf 72 660 Td (white %s) Tj ET\n0 g\n"
        "BT /F1 12 Tf 72 620 Td (underimg %s) Tj ET\n"
        "q 200 0 0 24 72 610 cm /ImCover Do Q\n"
        "q %d 0 0 %d 72 500 cm /ImRaster Do Q\n"
        "q 1 0 0 1 72 450 cm /Fm1 Do Q\n"
        "/OC /ocS BDC\nBT /F1 12 Tf 72 200 Td (layer %s) Tj ET\nEMC\n"
        "/Span << /MCID 0 >> BDC\nBT /F1 12 Tf 72 160 Td (tagged %s) Tj ET\nEMC\n"
        "/Span << /ActualText (%s) >> BDC\nBT /F1 12 Tf 72 120 Td (inline-span) Tj ET\nEMC\n"
        "q 40 0 0 40 520 700 cm /ImShared Do Q\n"
        % (TOKEN, TOKEN, TOKEN, TOKEN, rw, rh, TOKEN, TOKEN, ALT)
    ).encode("ascii")
    page2 = (
        b"q 1 0 0 1 72 700 cm /Fm1 Do Q\n"
        b"BT /F1 12 Tf 72 650 Td (page2-only ok) Tj ET\n"
        b"q 40 0 0 40 520 700 cm /ImShared Do Q\n"
    )
    ap_ft = b"BT /Helv 12 Tf 2 8 Td (%s) Tj ET\n" % TOKEN.encode()
    ap_field = b"BT /Helv 12 Tf 2 8 Td (%s) Tj ET\n" % TOKEN.encode()
    xmp = (
        '<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>\n'
        '<x:xmpmeta xmlns:x="adobe:ns:meta/">\n'
        '<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">\n'
        '<rdf:Description xmlns:dc="http://purl.org/dc/elements/1.1/" rdf:about="">\n'
        "<dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">%s</rdf:li></rdf:Alt></dc:title>\n"
        "</rdf:Description></rdf:RDF></x:xmpmeta>\n"
        '<?xpacket end="w"?>\n' % TOKEN
    ).encode("ascii")
    embed = (TOKEN + "\n").encode("ascii")
    form_res = b" /Type /XObject /Subtype /Form /BBox [0 0 400 16] /Resources << /Font << /F1 5 0 R >> >>"
    ap_res = b" /Type /XObject /Subtype /Form /BBox [0 0 328 30] /Resources << /Font << /Helv 5 0 R >> >>"
    xfa = (
        b'<?xml version="1.0" encoding="UTF-8"?>\n'
        b'<xdp:xdp xmlns:xdp="http://ns.adobe.com/xdp/">%s</xdp:xdp>\n' % TOKEN.encode()
    )

    #  1 Catalog  2 Pages  3 Page1  4 Page2  5 Font
    #  6 Contents1  7 Contents2  8 Fm1  9 ImCover  10 ImRaster  11 SMask
    # 12 FreeText p1  13 FT-AP  14 Widget  15 Field-AP  16 Link
    # 17 Outlines  18 Outline item  19 Info  20 XMP
    # 21 EmbeddedFiles  22 Filespec  23 EF stream  24 OCG
    # 25 StructTreeRoot  26 StructElem  27 ParentTree
    # 28 ImShared  29 FreeText p2  30 FT-AP p2  31 Sig widget (unsigned)  32 XFA
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R /Outlines 17 0 R "
        b"/AcroForm << /Fields [14 0 R 31 0 R] /DA (/Helv 12 Tf 0 g) /SigFlags 3 /XFA 32 0 R >> "
        b"/Names << /EmbeddedFiles 21 0 R >> "
        b"/OCProperties << /OCGs [24 0 R] /D << /Order [24 0 R] /ON [24 0 R] >> >> "
        b"/MarkInfo << /Marked true >> /StructTreeRoot 25 0 R /Metadata 20 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /StructParents 0 "
        b"/Resources << /Font << /F1 5 0 R >> "
        b"/XObject << /Fm1 8 0 R /ImCover 9 0 R /ImRaster 10 0 R /ImShared 28 0 R >> "
        b"/Properties << /ocS 24 0 R >> >> /Contents 6 0 R "
        b"/Annots [12 0 R 14 0 R 16 0 R 31 0 R] >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Resources << /Font << /F1 5 0 R >> /XObject << /Fm1 8 0 R /ImShared 28 0 R >> >> "
        b"/Contents 7 0 R /Annots [29 0 R] >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        stream(page1),
        stream(page2),
        stream(form, form_res),
        stream(cover, b" /Type /XObject /Subtype /Image /Width 200 /Height 24 "
               b"/ColorSpace /DeviceRGB /BitsPerComponent 8"),
        stream(raster, b" /Type /XObject /Subtype /Image /Width %d /Height %d "
               b"/ColorSpace /DeviceRGB /BitsPerComponent 8 /SMask 11 0 R" % (rw, rh)),
        stream(smask, b" /Type /XObject /Subtype /Image /Width %d /Height %d "
               b"/ColorSpace /DeviceGray /BitsPerComponent 8" % (rw, rh)),
        b"<< /Type /Annot /Subtype /FreeText /Rect [72 400 400 430] /F 4 /P 3 0 R "
        b"/Contents (%s) /DA (/Helv 12 Tf 0 g) /AP << /N 13 0 R >> >>" % TOKEN.encode(),
        stream(ap_ft, ap_res),
        b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (secret) /V (%s) /DV (%s) "
        b"/Rect [72 350 400 380] /F 4 /P 3 0 R /DA (/Helv 12 Tf 0 g) "
        b"/AP << /N 15 0 R >> >>" % (TOKEN.encode(), TOKEN.encode()),
        stream(ap_field, ap_res),
        b"<< /Type /Annot /Subtype /Link /Rect [72 300 400 330] /Border [0 0 0] /F 4 "
        b"/P 3 0 R /A << /S /URI /URI (https://example.com/%s) >> >>" % TOKEN.encode(),
        b"<< /Type /Outlines /First 18 0 R /Last 18 0 R /Count 1 >>",
        b"<< /Title (%s) /Parent 17 0 R /Dest [3 0 R /Fit] >>" % TOKEN.encode(),
        b"<< /Title (%s) /Producer (p0b-redact) >>" % TOKEN.encode(),
        stream(xmp, b" /Type /Metadata /Subtype /XML"),
        b"<< /Names [(%s.txt) 22 0 R] >>" % TOKEN.encode(),
        b"<< /Type /Filespec /F (%s.txt) /UF (%s.txt) /EF << /F 23 0 R >> >>"
        % (TOKEN.encode(), TOKEN.encode()),
        stream(embed, b" /Type /EmbeddedFile"),
        b"<< /Type /OCG /Name (%s) >>" % TOKEN.encode(),
        b"<< /Type /StructTreeRoot /K [26 0 R] /ParentTree 27 0 R >>",
        b"<< /Type /StructElem /S /Span /P 25 0 R /Pg 3 0 R /K 0 "
        b"/ActualText (%s) /Alt (%s) >>" % (TOKEN.encode(), TOKEN.encode()),
        b"<< /Nums [0 [26 0 R]] >>",
        stream(shared_px, b" /Type /XObject /Subtype /Image /Width 8 /Height 8 "
               b"/ColorSpace /DeviceRGB /BitsPerComponent 8"),
        b"<< /Type /Annot /Subtype /FreeText /Rect [72 400 400 430] /F 4 /P 4 0 R "
        b"/Contents (%s) /DA (/Helv 12 Tf 0 g) /AP << /N 30 0 R >> >>" % TOKEN.encode(),
        stream(ap_ft, ap_res),
        b"<< /Type /Annot /Subtype /Widget /FT /Sig /T (sig) "
        b"/Rect [72 250 400 280] /F 4 /P 3 0 R >>",
        stream(xfa),
    ]
    size, digest, data = emit_info(OUT, objects, info_num=19)
    needles = [
        b"visible " + TOKEN.encode(),
        b"invisible " + TOKEN.encode(),
        b"white " + TOKEN.encode(),
        b"underimg " + TOKEN.encode(),
        b"formxo " + TOKEN.encode(),
        b"layer " + TOKEN.encode(),
        b"tagged " + TOKEN.encode(),
        b"/Span << /ActualText (%s) >> BDC" % ALT.encode(),
        b"/Contents (%s)" % TOKEN.encode(),
        b"/V (%s)" % TOKEN.encode(),
        b"/FT /Sig",
        b"https://example.com/%s" % TOKEN.encode(),
        b"/Title (%s)" % TOKEN.encode(),
        b"/Name (%s)" % TOKEN.encode(),
        b"/ActualText (%s)" % TOKEN.encode(),
        b"/Alt (%s)" % TOKEN.encode(),
        TOKEN.encode() + b".txt",
        TOKEN.encode() + b"\n",
        b"page2-only ok",
        b"/ImShared Do",
    ]
    for needle in needles:
        assert needle in data, needle
    assert data.count(b"/ImShared Do") == 2
    assert data.count(b"%%EOF") == 1 and b"/Prev" not in data
    assert TOKEN.encode() not in raster and TOKEN.encode() not in shared_px
    assert data.count(ALT.encode()) == 1
    assert b"/XFA 32 0 R" in data and b"/SigFlags 3" in data
    assert b"/FT /Sig /T (sig) /Rect" in data and b"/SV" not in data
    assert b"/Subtype /Redact" not in data
    form_pdf = [72, 450, 472, 466]
    image_pdf = [520, 700, 560, 740]
    ft_pdf = [72, 400, 400, 430]
    sig_pdf = [72, 250, 400, 280]
    provenance(OUT, size, digest, {
        "raster_image": {"width": rw, "height": rh},
        "surfaces": [
            "visible text",
            "invisible text (Tr 3)",
            "white text",
            "text under image",
            "rasterised text image + SMask",
            "Form XObject (shared page 1+2)",
            "shared Image XObject (both pages, no token bytes)",
            "inline Span BDC /ActualText (P4-ALT-4242, not the token)",
            "FreeText /Contents + /AP (page 1 and page 2)",
            "form field /V + /DV + /AP",
            "unsigned signature field /FT /Sig (no /V, no /SV)",
            "AcroForm /XFA stub + /SigFlags 3",
            "outline /Title",
            "Info /Title",
            "XMP dc:title",
            "embedded file name + bytes",
            "OCG name + layer content",
            "struct ActualText + Alt",
            "Link /URI",
        ],
        "placements": {
            "note": "rect is MuPDF page space y-down (drive/engine). rect_pdf is the content-stream box, y-up.",
            "shared_form": {"pages": [1, 2], "name": "Fm1", "rect_pdf": form_pdf, "rect": y_down(form_pdf)},
            "shared_image": {"pages": [1, 2], "name": "ImShared", "rect_pdf": image_pdf, "rect": y_down(image_pdf)},
            "page2_freetext": {"page": 2, "rect_pdf": ft_pdf, "rect": y_down(ft_pdf)},
            "signature": {"page": 1, "rect_pdf": sig_pdf, "rect": y_down(sig_pdf)},
        },
        "inline_alt": ALT,
        "xfa": True,
        "sigFlags": 3,
        "sanitize": {
            "info": 1,
            "xmp": True,
            "attachments": 1,
            "outlines": 1,
            "annots": 2,
            "links": 1,
            "fields": 2,
            "layers": 1,
            "struct": 2,
            "counting": "info=Info keys other than Producer/CreationDate/ModDate; attachments=embedded files; outlines=outline items; annots=non-Widget non-Redact non-Link; links=Link annots; fields=widgets including Sig; layers=OCG count; struct=ActualText+Alt+E keys on StructElems. Inline BDC ActualText is P4-ALT-4242 and is not in struct. /XFA and /SigFlags are extra fields-toggle targets (contract §8.8), not part of these nine counts.",
        },
    })
    print(f"wrote {OUT} ({size} bytes) sha256={digest} token_count={data.count(TOKEN.encode())}")
    return digest


def build_shared_only():
    token = TOKEN.encode()
    form = b"BT /F1 12 Tf 0 2 Td (formxo %s) Tj ET\n" % token
    page1 = b"q 1 0 0 1 72 450 cm /Fm1 Do Q\nBT /F1 12 Tf 72 700 Td (page one) Tj ET\n"
    page2 = b"q 1 0 0 1 72 450 cm /Fm1 Do Q\nBT /F1 12 Tf 72 700 Td (page two) Tj ET\n"
    form_res = b" /Type /XObject /Subtype /Form /BBox [0 0 400 16] /Resources << /Font << /F1 5 0 R >> >>"
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Resources << /Font << /F1 5 0 R >> /XObject << /Fm1 8 0 R >> >> /Contents 6 0 R >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Resources << /Font << /F1 5 0 R >> /XObject << /Fm1 8 0 R >> >> /Contents 7 0 R >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        stream(page1),
        stream(page2),
        stream(form, form_res),
    ]
    size, digest = emit(SHARED, objects)
    data = SHARED.read_bytes()
    assert data.count(token) == 1, data.count(token)
    assert b"page one" in data and b"page two" in data
    assert data.count(b"%%EOF") == 1 and b"/Prev" not in data
    form_pdf = [72, 450, 472, 466]
    provenance(SHARED, size, digest, {
        "token_only_in": "Form XObject Fm1, placed on both pages",
        "placements": {
            "note": "Mark one page's rect and preview must report shared-content. rect is MuPDF y-down.",
            "shared_form": {"pages": [1, 2], "name": "Fm1", "rect_pdf": form_pdf, "rect": y_down(form_pdf)},
        },
        "sanitize": {
            "info": 0, "xmp": False, "attachments": 0, "outlines": 0,
            "annots": 0, "links": 0, "fields": 0, "layers": 0, "struct": 0,
        },
    })
    print(f"wrote {SHARED} ({size} bytes) sha256={digest} token_count={data.count(token)}")
    return digest


if __name__ == "__main__":
    build_redact()
    build_shared_only()
