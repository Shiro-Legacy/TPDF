#!/usr/bin/env python3
"""Independent round-trip check (no MuPDF). QA-only; never bundled.

pypdf==6.18.1 in .venv-qa (BSD-3-Clause).
Usage: python3 scripts/verify-roundtrip.py <saved.pdf>
       python3 scripts/verify-roundtrip.py <saved.pdf> --text-notes 1 --contents "PDF4All P2 note"
       python3 scripts/verify-roundtrip.py <saved.pdf> --annots '{"Highlight":1}' --appearance Highlight
       python3 scripts/verify-roundtrip.py <saved.pdf> --annots '{"Stamp":2}' --appearance Stamp:2 --no-sig
       python3 scripts/verify-roundtrip.py <saved.pdf> --pages '{"count":2,"rotate":[0,90],"order":["three","one"]}'
       python3 scripts/verify-roundtrip.py <saved.pdf> --fields '{"name":"PDF4All P2 form","agree":"/Yes","size":"/M","choice":"two"}' --appearance Field:name
       python3 scripts/verify-roundtrip.py <saved.pdf> --fields-unique
       python3 scripts/verify-roundtrip.py <saved.pdf> --field-defs '{"n":{"ft":"/Tx","rect":[72,650,252,680],"page":0,"ap":true,"da":true}}'
       python3 scripts/verify-roundtrip.py <saved.pdf> --field-defs '{"n":{"ft":"/Tx","tu":"Nom (é)","ff":2,"dv":"hi"}}'
       python3 scripts/verify-roundtrip.py <saved.pdf> --no-scripts <source.pdf>
       python3 scripts/verify-roundtrip.py <saved.pdf> --outline '[["One",0],["Two",null],["Two-A",null],["Three",1],["PDF4All P2 nav",1]]'
       python3 scripts/verify-roundtrip.py <saved.pdf> --links '{"0":[{"uri":"https://example.com/"},{"page":1}],"1":[{"blocked":"file"}]}'

The plain form is the P0 smoke (form field + red Square + Quick Look pixels).
With --text-notes the check is the P2 sticky-note one instead: exactly N
/Subtype /Text annotations on page 1, each /Contents equal to --contents, and
no /T (nothing personal) anywhere in them.
With --annots the page-1 subtype counts must match the map for Text/Highlight/
Underline/StrikeOut/FreeText plus any extra keys (Square/Circle/Line/Ink/Stamp);
markups need /QuadPoints of length 8k; FreeText needs /DA and /AP; no annotation
has /T. --appearance <type> Quick Look-renders and checks a per-type colour
predicate inside that annot's /Rect (markup grid fraction > 0.3; FreeText every
pixel in the top 16 pt, dark fraction > 0.01; control points white).
--appearance Stamp:n (1-based) is the img-gray.png band (image rows 100-120)
inside that stamp's /Rect. --no-sig: no /Sig field, no /ByteRange, no /SigFlags.
With --pages the page count, per-page /Rotate (missing = 0), and first word of
extract_text() must match the JSON {count, rotate[], order[]}.
With --fields the named pypdf get_fields() /V values must match and each named
field's widget has /AP. --appearance Field:<name> reuses the FreeText dark-band
scan on that widget's /Rect.
With --fields-unique get_fields() key count must equal the top-level /Fields
length (prints both counts).
With --outline the flattened preorder of reader.outline (title, page index via
get_destination_page_number, null when no dest) must match the JSON list.
With --links Link annots per page in /Annots order: GoTo → {page}, http/https/
mailto URI → {uri}, other schemes → {blocked: scheme}.
"""
import argparse
import json
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

PYPDF = "6.18.1"
PAGE = (612.0, 792.0)
FIVE = ("Text", "Highlight", "Underline", "StrikeOut", "FreeText")
MARKUP = ("Highlight", "Underline", "StrikeOut")


def fields_and_annots(path: Path):
    from pypdf import PdfReader
    reader = PdfReader(str(path))
    values = reader.get_form_text_fields() or {}
    field = values.get("name")
    squares = []
    for annot in reader.pages[0].annotations or []:
        obj = annot.get_object()
        if str(obj.get("/Subtype")) == "/Square":
            squares.append({
                "contents": str(obj.get("/Contents", "")),
                "rect": [float(x) for x in obj.get("/Rect")],
            })
    return field, squares


def text_notes(path: Path, expected_count, expected_contents):
    """Page 1 sticky notes: count, contents and the absence of an /T author."""
    from pypdf import PdfReader
    reader = PdfReader(str(path))
    notes = []
    for annot in reader.pages[0].annotations or []:
        obj = annot.get_object()
        if str(obj.get("/Subtype")) != "/Text":
            continue
        notes.append({
            "contents": str(obj.get("/Contents", "")),
            "has_T": "/T" in obj,
            "rect": [float(x) for x in obj.get("/Rect")],
        })
    ok = all(not note["has_T"] for note in notes)
    if expected_count is not None:
        ok = ok and len(notes) == expected_count
    if expected_contents is not None:
        ok = ok and all(note["contents"] == expected_contents for note in notes)
    return {
        "ok": ok,
        "pypdf": PYPDF,
        "count": len(notes),
        "expected": expected_count,
        "contents": expected_contents,
        "no_author": all(not note["has_T"] for note in notes),
        "notes": notes,
    }


def page1_annots(path: Path):
    from pypdf import PdfReader
    reader = PdfReader(str(path))
    items = []
    for annot in reader.pages[0].annotations or []:
        obj = annot.get_object()
        subtype = str(obj.get("/Subtype", "")).lstrip("/")
        rec = {
            "type": subtype,
            "contents": str(obj.get("/Contents", "")),
            "has_T": "/T" in obj,
            "rect": [float(x) for x in obj.get("/Rect")] if obj.get("/Rect") is not None else None,
        }
        if subtype in MARKUP:
            qp = obj.get("/QuadPoints")
            rec["quad_len"] = len(qp) if qp is not None else 0
        if subtype == "FreeText":
            rec["has_DA"] = "/DA" in obj
            rec["has_AP"] = "/AP" in obj
        items.append(rec)
    return items


def annots_check(path: Path, expected, expected_contents):
    items = page1_annots(path)
    counts = {t: 0 for t in FIVE}
    extra = {}
    for rec in items:
        if rec["type"] in counts:
            counts[rec["type"]] += 1
        else:
            extra[rec["type"]] = extra.get(rec["type"], 0) + 1
    ok = all(not rec["has_T"] for rec in items)
    for t in FIVE:
        if counts[t] != int(expected.get(t, 0)):
            ok = False
    for t, n in expected.items():
        if t not in FIVE and extra.get(t, 0) != int(n):
            ok = False
    for rec in items:
        if rec["type"] in MARKUP:
            q = rec.get("quad_len", 0)
            if q < 8 or q % 8 != 0:
                ok = False
        if rec["type"] == "FreeText" and not (rec.get("has_DA") and rec.get("has_AP")):
            ok = False
        if expected_contents is not None and rec["type"] in ("Text", "FreeText"):
            if rec["contents"] != expected_contents:
                ok = False
    return {
        "ok": ok,
        "pypdf": PYPDF,
        "counts": counts,
        "expected": expected,
        "contents": expected_contents,
        "no_author": all(not rec["has_T"] for rec in items),
        "annots": items,
    }


def field_name(obj):
    t = obj.get("/T")
    if t is None and "/Parent" in obj:
        t = obj["/Parent"].get_object().get("/T")
    return "" if t is None else str(t)


def field_widgets(reader, name):
    found = []
    for annot in reader.pages[0].annotations or []:
        obj = annot.get_object()
        if field_name(obj) == name:
            found.append(obj)
    return found


def fields_check(path: Path, expected):
    from pypdf import PdfReader
    reader = PdfReader(str(path))
    fields = reader.get_fields() or {}
    values = {}
    ap = {}
    ok = True
    for name, want in expected.items():
        rec = fields.get(name)
        got = "" if rec is None or rec.get("/V") is None else str(rec.get("/V"))
        values[name] = None if rec is None else got
        if rec is None or got != want:
            ok = False
        widgets = field_widgets(reader, name)
        has_ap = bool(widgets) and all("/AP" in widget for widget in widgets)
        ap[name] = has_ap
        if not has_ap:
            ok = False
    return {
        "ok": ok,
        "pypdf": PYPDF,
        "values": values,
        "expected": expected,
        "ap": ap,
    }


def field_rect(path: Path, name):
    from pypdf import PdfReader
    widgets = field_widgets(PdfReader(str(path)), name)
    if not widgets:
        return None
    # Radio kids share one name; render the selected kid, not an arbitrary Off kid.
    widget = next((item for item in widgets if str(item.get("/AS", "/Off")) != "/Off"), widgets[0])
    if widget.get("/Rect") is None:
        return None
    return [float(x) for x in widget.get("/Rect")]


def pages_check(path: Path, expected):
    from pypdf import PdfReader
    reader = PdfReader(str(path))
    rotate = []
    order = []
    for page in reader.pages:
        rotate.append(int(page.rotation))
        words = (page.extract_text() or "").split()
        order.append(words[0] if words else "")
    count = len(reader.pages)
    ok = count == int(expected["count"])
    if "rotate" in expected:
        ok = ok and rotate == list(expected["rotate"])
    if "order" in expected:
        ok = ok and order == list(expected["order"])
    return {
        "ok": ok,
        "pypdf": PYPDF,
        "count": count,
        "rotate": rotate,
        "order": order,
        "expected": expected,
    }


def labels_check(path: Path, expected):
    from pypdf import PdfReader
    reader = PdfReader(str(path))
    labels = list(reader.page_labels)
    return {"ok": labels == expected, "pypdf": PYPDF, "labels": labels, "expected": expected}


def text_has_check(path: Path, expected):
    from pypdf import PdfReader
    reader = PdfReader(str(path))
    texts = {}
    ok = True
    for key, needles in expected.items():
        text = reader.pages[int(key)].extract_text() or ""
        texts[str(key)] = text
        for needle in needles:
            if needle not in text:
                ok = False
    return {"ok": ok, "pypdf": PYPDF, "texts": texts, "expected": expected}


FONT_FILES = ("/FontFile", "/FontFile2", "/FontFile3")


def font_names_check(path: Path, expected):
    """Every font is Type1 + WinAnsi + unembedded. expected is {page: [BaseFont, ...]}."""
    from pypdf import PdfReader
    reader = PdfReader(str(path))
    pages = {}
    ok = True
    for index, page in enumerate(reader.pages):
        resources = page.get("/Resources") or {}
        fonts = resources.get("/Font")
        rows = []
        if fonts is not None:
            fonts = fonts.get_object()
            for key, ref in fonts.items():
                obj = ref.get_object()
                base = str(obj.get("/BaseFont", "")).lstrip("/")
                subtype = str(obj.get("/Subtype", ""))
                encoding = str(obj.get("/Encoding", ""))
                embedded = any(name in obj for name in FONT_FILES)
                rows.append({
                    "resource": str(key).lstrip("/"),
                    "base": base,
                    "subtype": subtype,
                    "encoding": encoding,
                    "embedded": embedded,
                })
                if subtype != "/Type1" or encoding != "/WinAnsiEncoding" or embedded:
                    ok = False
        pages[str(index)] = rows
        if expected and str(index) in expected:
            bases = [row["base"] for row in rows]
            for name in expected[str(index)]:
                if name not in bases:
                    ok = False
    return {"ok": ok, "pypdf": PYPDF, "fonts": pages, "expected": expected}


def _skip_literal(data: bytes, i: int) -> int:
    i += 1
    while i < len(data):
        if data[i] == 0x5C:
            i += 2
            continue
        if data[i] == 0x29:
            return i + 1
        i += 1
    return i


def _next_show_op(data: bytes, i: int):
    show = {b"Tj", b"TJ", b"'", b'"'}
    while i < len(data):
        c = data[i]
        if c in b" \t\r\n[]":
            i += 1
            continue
        if c == 0x28:
            i = _skip_literal(data, i)
            continue
        if c == 0x3C:
            end = data.find(b">", i)
            i = len(data) if end < 0 else end + 1
            continue
        if c == 0x2F:
            i += 1
            while i < len(data) and data[i] not in b" \t\r\n[]()<>/":
                i += 1
            continue
        j = i
        while j < len(data) and data[j] not in b" \t\r\n[]()<>/":
            j += 1
        token = data[i:j]
        if token in show:
            return token.decode("ascii")
        i = j if j > i else i + 1
    return None


def _content_bytes(reader):
    from pypdf.generic import ArrayObject, IndirectObject, StreamObject
    out, seen = [], set()

    def absorb(obj):
        if isinstance(obj, IndirectObject):
            key = (obj.idnum, obj.generation)
            if key in seen:
                return
            seen.add(key)
            try:
                obj = obj.get_object()
            except Exception:
                return
        if isinstance(obj, ArrayObject):
            for item in obj:
                absorb(item)
            return
        if isinstance(obj, StreamObject):
            try:
                out.append(obj.get_data())
            except Exception:
                pass
            resources = obj.get("/Resources") or {}
            xobjs = resources.get("/XObject")
            if xobjs is not None:
                for value in xobjs.get_object().values():
                    absorb(value)

    for page in reader.pages:
        absorb(page.get("/Contents"))
        resources = page.get("/Resources") or {}
        xobjs = resources.get("/XObject")
        if xobjs is not None:
            for value in xobjs.get_object().values():
                absorb(value)
    return out


def no_tj_on_check(path: Path, text: str):
    """The run showing `text` must be a Tj, from decompressed content streams."""
    from pypdf import PdfReader
    reader = PdfReader(str(path))
    needle = b"(" + text.encode("latin1") + b")"
    ops = []
    for data in _content_bytes(reader):
        start = 0
        while True:
            at = data.find(needle, start)
            if at < 0:
                break
            ops.append(_next_show_op(data, at + len(needle)))
            start = at + len(needle)
    ok = bool(ops) and all(op == "Tj" for op in ops)
    return {"ok": ok, "pypdf": PYPDF, "text": text, "ops": ops}


def ocg_name(ref):
    try:
        obj = ref.get_object()
    except Exception:
        obj = ref
    name = obj.get("/Name") if hasattr(obj, "get") else None
    return "" if name is None else str(name)


def layers_check(path: Path, expected):
    from pypdf import PdfReader
    reader = PdfReader(str(path))
    oc = reader.root_object.get("/OCProperties")
    default = oc.get("/D") if oc else None
    ocgs = list(oc.get("/OCGs") or []) if oc else []
    on = {ocg_name(item) for item in (default.get("/ON") or [])} if default else set()
    off = {ocg_name(item) for item in (default.get("/OFF") or [])} if default else set()
    items = []
    for ref in ocgs:
        name = ocg_name(ref)
        visible = False if name in off else True if name in on or not off else True
        items.append({"name": name, "on": visible})
    got = {(item["name"], item["on"]) for item in items}
    want = {(item["name"], item["on"]) for item in expected}
    return {"ok": got == want, "pypdf": PYPDF, "layers": items, "expected": expected, "on": sorted(on), "off": sorted(off)}


def image_page_check(path: Path, expected):
    from pypdf import PdfReader
    reader = PdfReader(str(path))
    page = reader.pages[int(expected["page"])]
    box = page.mediabox
    width, height = float(box.width), float(box.height)
    resources = page.get("/Resources") or {}
    xobj = resources.get("/XObject") or {}
    try:
        xobj = xobj.get_object()
    except Exception:
        pass
    images = []
    for key in xobj:
        obj = xobj[key]
        try:
            obj = obj.get_object()
        except Exception:
            pass
        if str(obj.get("/Subtype")) == "/Image":
            images.append({"width": int(obj.get("/Width")), "height": int(obj.get("/Height"))})
    want_w, want_h = int(expected["width"]), int(expected["height"])
    ok = (
        abs(width - want_w) < 0.51
        and abs(height - want_h) < 0.51
        and len(images) == 1
        and images[0]["width"] == want_w
        and images[0]["height"] == want_h
    )
    return {
        "ok": ok,
        "pypdf": PYPDF,
        "mediabox": [width, height],
        "images": images,
        "expected": expected,
    }


def fields_unique_check(path: Path):
    from pypdf import PdfReader
    reader = PdfReader(str(path))
    fields = reader.get_fields() or {}
    keys = len(fields)
    acro = reader.root_object.get("/AcroForm") if hasattr(reader, "root_object") else None
    top = len(acro.get("/Fields") or []) if acro else 0
    widgets = missing_ap = 0
    for page in reader.pages:
        for annot in page.annotations or []:
            obj = annot.get_object()
            if str(obj.get("/Subtype")) != "/Widget":
                continue
            widgets += 1
            if "/AP" not in obj:
                missing_ap += 1
    return {
        "ok": keys == top,
        "pypdf": PYPDF,
        "keys": keys,
        "top": top,
        "widgets": widgets,
        "missing_ap": missing_ap,
    }


def deref(obj):
    try:
        return obj.get_object()
    except Exception:
        return obj


def ref_key(obj):
    from pypdf.generic import IndirectObject
    if isinstance(obj, IndirectObject):
        return (obj.idnum, obj.generation)
    ref = getattr(obj, "indirect_reference", None)
    return (ref.idnum, ref.generation) if ref is not None else ("direct", id(obj))


def field_defs_check(path: Path, expected):
    from pypdf import PdfReader
    reader = PdfReader(str(path))
    acro = deref(reader.root_object.get("/AcroForm"))
    roots = list(acro.get("/Fields") or []) if acro else []
    fields = {}

    def walk(ref, prefix="", depth=0):
        if depth > 32:
            return
        obj = deref(ref)
        local = obj.get("/T") if hasattr(obj, "get") else None
        name = prefix
        if local is not None:
            name = f"{prefix}.{local}" if prefix else str(local)
            fields[name] = obj
        for kid in obj.get("/Kids") or []:
            walk(kid, name, depth + 1)

    for root in roots:
        walk(root)

    page_keys = [ref_key(page) for page in reader.pages]
    page_annots = [{ref_key(ref) for ref in (page.annotations or [])} for page in reader.pages]
    errors = []
    facts = {}

    def widgets(node):
        found, seen = [], set()
        def visit(obj):
            obj = deref(obj)
            key = ref_key(obj)
            if key in seen:
                return
            seen.add(key)
            if str(obj.get("/Subtype", "")) == "/Widget":
                found.append(obj)
            for kid in obj.get("/Kids") or []:
                kid_obj = deref(kid)
                if kid_obj.get("/T") is None:
                    visit(kid_obj)
        visit(node)
        return found

    for name, spec in expected.items():
        node = fields.get(name)
        if node is None:
            errors.append(f"{name}: not reachable from /AcroForm /Fields")
            continue
        items = widgets(node)
        actual_ft = str(node.get("/FT", ""))
        if actual_ft != spec.get("ft"):
            errors.append(f"{name}: /FT {actual_ft!r} != {spec.get('ft')!r}")
        want_rect = spec.get("rect")
        rects = [[float(x) for x in item.get("/Rect")] for item in items if item.get("/Rect") is not None]
        if want_rect is not None and not any(len(rect) == 4 and all(abs(a - float(b)) <= .5 for a, b in zip(rect, want_rect)) for rect in rects):
            errors.append(f"{name}: no /Rect within 0.5 of {want_rect}")
        pages = []
        for item in items:
            p = item.raw_get("/P") if "/P" in item else None
            pkey = ref_key(p) if p is not None else None
            index = page_keys.index(pkey) if pkey in page_keys else None
            pages.append(index)
            if "page" in spec and index != int(spec["page"]):
                errors.append(f"{name}: widget /P page {index} != {spec['page']}")
            if index is None or ref_key(item) not in page_annots[index]:
                errors.append(f"{name}: widget is not in its /P page /Annots")
        if not items:
            errors.append(f"{name}: no reachable widget")
        ap = spec.get("ap")
        ap_states = []
        if ap is not None:
            for item in items:
                normal = deref(deref(item.get("/AP")).get("/N")) if item.get("/AP") is not None else None
                if ap is True:
                    if normal is None or not hasattr(normal, "get_data"):
                        errors.append(f"{name}: /AP /N is not a stream")
                else:
                    if normal is None or not hasattr(normal, "keys") or hasattr(normal, "get_data"):
                        errors.append(f"{name}: /AP /N is not a state dictionary")
                    else:
                        ap_states.extend(str(key).lstrip("/") for key in normal.keys())
            if ap is not True and sorted(set(ap_states)) != sorted(ap):
                errors.append(f"{name}: /AP /N states {sorted(set(ap_states))} != {sorted(ap)}")
        if spec.get("da") and (not items or any("/DA" not in item for item in items)):
            errors.append(f"{name}: widget /DA missing")
        if "opt" in spec:
            actual = [str(deref(value)) for value in (node.get("/Opt") or [])]
            if actual != list(spec["opt"]):
                errors.append(f"{name}: /Opt {actual} != {spec['opt']}")
        if "kids" in spec and len(node.get("/Kids") or []) != int(spec["kids"]):
            errors.append(f"{name}: /Kids count {len(node.get('/Kids') or [])} != {spec['kids']}")
        ff = int(node.get("/Ff", 0))
        if spec.get("radio") and not ff & 32768:
            errors.append(f"{name}: /Ff radio bit 16 is clear")
        if "tu" in spec:
            actual_tu = node.get("/TU")
            if spec["tu"] is None:
                if "/TU" in node:
                    errors.append(f"{name}: /TU present ({str(actual_tu)!r}), expected absent")
            elif actual_tu is None or str(actual_tu) != spec["tu"]:
                errors.append(f"{name}: /TU {None if actual_tu is None else str(actual_tu)!r} != {spec['tu']!r}")
        if "ff" in spec:
            if spec["ff"] is None:
                if "/Ff" in node:
                    errors.append(f"{name}: /Ff present ({ff}), expected absent")
            elif ff != int(spec["ff"]):
                errors.append(f"{name}: /Ff {ff} != {int(spec['ff'])}")
        dv = node.get("/DV")
        if "dv" in spec:
            want_dv = spec["dv"]
            if want_dv is None:
                if "/DV" in node:
                    errors.append(f"{name}: /DV present ({str(dv)!r}), expected absent")
            elif dv is None:
                errors.append(f"{name}: /DV absent, expected {want_dv!r}")
            else:
                got_dv = str(dv).lstrip("/") if str(dv).startswith("/") else str(dv)
                if got_dv != want_dv:
                    errors.append(f"{name}: /DV {got_dv!r} != {want_dv!r}")
        facts[name] = {"ft": actual_ft, "rects": rects, "pages": pages, "widgets": len(items), "ap_states": sorted(set(ap_states)), "ff": ff,
                       "tu": None if node.get("/TU") is None else str(node.get("/TU")),
                       "dv": None if dv is None else str(dv)}
    return {"ok": not errors, "pypdf": PYPDF, "fields": facts, "errors": errors}


def script_counts(path: Path):
    from pypdf import PdfReader
    from pypdf.generic import ArrayObject, DictionaryObject, IndirectObject
    reader = PdfReader(str(path))
    counts = {"AA": 0, "JS": 0, "Names.JavaScript": 0, "CO_entries": 0, "NeedAppearances": 0}
    seen_refs, seen_direct = set(), set()

    def visit(value):
        if isinstance(value, IndirectObject):
            key = (value.idnum, value.generation)
            if key in seen_refs:
                return
            seen_refs.add(key)
            value = value.get_object()
        elif isinstance(value, (DictionaryObject, ArrayObject)):
            key = id(value)
            if key in seen_direct:
                return
            seen_direct.add(key)
        if isinstance(value, DictionaryObject):
            counts["AA"] += int("/AA" in value)
            counts["JS"] += int("/JS" in value)
            counts["NeedAppearances"] += int("/NeedAppearances" in value)
            if "/CO" in value:
                counts["CO_entries"] += len(deref(value.raw_get("/CO")) or [])
            if "/Names" in value:
                names = deref(value.raw_get("/Names"))
                counts["Names.JavaScript"] += int(isinstance(names, DictionaryObject) and "/JavaScript" in names)
            for key in value:
                visit(value.raw_get(key))
        elif isinstance(value, ArrayObject):
            for item in value:
                visit(item)

    visit(reader.trailer.raw_get("/Root"))
    return counts


def no_scripts_check(path: Path, source: Path):
    saved_counts, source_counts = script_counts(path), script_counts(source)
    return {"ok": saved_counts == source_counts, "pypdf": PYPDF, "saved": saved_counts, "source": source_counts}


def flatten_outline(reader, items=None):
    if items is None:
        items = reader.outline or []
    out = []
    for item in items:
        if isinstance(item, list):
            out.extend(flatten_outline(reader, item))
            continue
        title = item.title if hasattr(item, "title") else str(item)
        try:
            page = reader.get_destination_page_number(item)
        except Exception:
            page = None
        out.append([title, page])
    return out


def outline_check(path: Path, expected):
    from pypdf import PdfReader
    reader = PdfReader(str(path))
    items = flatten_outline(reader)
    return {
        "ok": items == expected,
        "pypdf": PYPDF,
        "outline": items,
        "expected": expected,
    }


def dest_page_index(reader, dest):
    if dest is None:
        return None
    try:
        dest = dest.get_object()
    except Exception:
        pass
    try:
        ref = dest[0]
    except Exception:
        return None
    for i, page in enumerate(reader.pages):
        if page.indirect_reference == ref:
            return i
    return None


def uri_kind(uri):
    uri = str(uri)
    scheme = uri.split(":", 1)[0].lower() if ":" in uri else ""
    if scheme in ("http", "https", "mailto"):
        return {"uri": uri}
    return {"blocked": scheme or "unknown"}


def classify_link(obj, reader):
    action = obj.get("/A")
    dest = obj.get("/Dest")
    if action is not None:
        try:
            action = action.get_object()
        except Exception:
            pass
        kind = str(action.get("/S", "")).lstrip("/")
        if kind == "URI":
            return uri_kind(action.get("/URI", ""))
        if kind == "GoTo":
            dest = action.get("/D")
        else:
            return {"blocked": kind.lower()}
    page = dest_page_index(reader, dest)
    if page is not None:
        return {"page": page}
    return {"blocked": "unknown"}


def links_check(path: Path, expected):
    from pypdf import PdfReader
    reader = PdfReader(str(path))
    got = {}
    for i, page in enumerate(reader.pages):
        items = []
        for annot in page.annotations or []:
            obj = annot.get_object()
            if str(obj.get("/Subtype")) != "/Link":
                continue
            items.append(classify_link(obj, reader))
        got[str(i)] = items
    return {
        "ok": got == expected,
        "pypdf": PYPDF,
        "links": got,
        "expected": expected,
    }


def ql_png(pdf: Path, outdir: Path) -> Path:
    subprocess.run(["qlmanage", "-t", "-s", "800", "-o", str(outdir), str(pdf)], check=True, capture_output=True)
    pngs = list(outdir.glob("*.png"))
    if not pngs:
        raise SystemExit("qlmanage produced no png")
    return pngs[0]


def bmp_pixels(bmp: Path):
    data = bmp.read_bytes()
    off = struct.unpack_from("<I", data, 10)[0]
    width = struct.unpack_from("<i", data, 18)[0]
    height = struct.unpack_from("<i", data, 22)[0]
    bpp = struct.unpack_from("<H", data, 28)[0]
    top_down = height < 0
    height = abs(height)
    row = ((width * bpp + 31) // 32) * 4
    def at(x, y):
        if top_down:
            raw_y = y
        else:
            raw_y = height - 1 - y
        i = off + raw_y * row + x * (bpp // 8)
        b, g, r = data[i], data[i + 1], data[i + 2]
        return r, g, b
    return width, height, at


def redish(rgb):
    r, g, b = rgb
    return r > 140 and r > g + 20 and r > b + 20


def yellowish(rgb):
    r, g, b = rgb
    return r > 140 and g > 140 and r > b + 20 and g > b + 20


def whiteish(rgb):
    return all(c > 230 for c in rgb)


def darkish(rgb):
    return not whiteish(rgb)


PREDICATE = {
    "Highlight": yellowish,
    "Underline": redish,
    "StrikeOut": redish,
    "FreeText": darkish,
    "Square": redish,
}


def appearance(pdf: Path, rect, pred=redish):
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        png = ql_png(pdf, tmp)
        bmp = tmp / "page.bmp"
        subprocess.run(["sips", "-s", "format", "bmp", str(png), "--out", str(bmp)], check=True, capture_output=True)
        w, h, at = bmp_pixels(bmp)
        sx, sy = w / PAGE[0], h / PAGE[1]
        x0, y0, x1, y1 = rect
        # image y-down: PDF y-up
        def img(x, y):
            return min(w - 1, max(0, int(x * sx))), min(h - 1, max(0, int((PAGE[1] - y) * sy)))
        if pred is darkish:
            # Helv 12 sits in the top ~16 pt; scan every thumbnail pixel in that band
            band_lo = y1 - min(16.0, max(y1 - y0, 1))
            ix0, iy0 = img(x0, y1)
            ix1, iy1 = img(x1, band_lo)
            xa, xb = min(ix0, ix1), max(ix0, ix1)
            ya, yb = min(iy0, iy1), max(iy0, iy1)
            hits = total = 0
            for iy in range(ya, yb + 1):
                for ix in range(xa, xb + 1):
                    total += 1
                    if pred(at(ix, iy)):
                        hits += 1
            frac = hits / total if total else 0
        else:
            dx, dy = max(x1 - x0, 1), max(y1 - y0, 1)
            # ponytail: 10/20 grid matches P0 squares; shrink pad/step so a word-quad still samples
            pad_x, pad_y = min(10.0, dx / 4), min(10.0, dy / 4)
            step_x = max(1, min(20, int(dx / 4) or 1))
            step_y = max(1, min(20, int(dy / 4) or 1))
            xs = list(range(int(x0 + pad_x), int(x1 - pad_x) + 1, step_x)) or [int((x0 + x1) / 2)]
            ys = list(range(int(y0 + pad_y), int(y1 - pad_y) + 1, step_y)) or [int((y0 + y1) / 2)]
            inside = [pred(at(*img(px, py))) for px in xs for py in ys]
            frac = (sum(inside) / len(inside)) if inside else 0
        outside = [
            whiteish(at(*img(40, 40))),
            whiteish(at(*img(500, 400))),
            whiteish(at(*img(200, 500))),
        ]
        return {
            "png": png.name,
            "bmp": [w, h],
            "inside_frac": frac,
            "inside_red_frac": frac,
            "outside_white": all(outside),
        }


# img-gray.png is 200×300 with black rows 100–120 (fixtures/make-images.py).
IMG_H = 300
BAND = (100, 121)  # exclusive end; 100 <= y <= 120


def pick_annot(items, spec):
    """'Stamp:2' → 2nd Stamp (1-based); 'Highlight' → first Highlight."""
    kind, sep, rest = spec.partition(":")
    if not sep:
        n = 1
    else:
        try:
            n = int(rest)
        except ValueError:
            return None, spec
    found = [rec for rec in items if rec["type"] == kind and rec.get("rect")]
    if n < 1 or n > len(found):
        return None, kind
    return found[n - 1], kind


def stamp_band(pdf: Path, rect):
    """Dark fraction of the img-gray band mapped into the stamp /Rect."""
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        png = ql_png(pdf, tmp)
        bmp = tmp / "page.bmp"
        subprocess.run(["sips", "-s", "format", "bmp", str(png), "--out", str(bmp)], check=True, capture_output=True)
        w, h, at = bmp_pixels(bmp)
        sx, sy = w / PAGE[0], h / PAGE[1]
        x0, y0, x1, y1 = rect
        height = max(y1 - y0, 1)

        def img(x, y):
            return min(w - 1, max(0, int(x * sx))), min(h - 1, max(0, int((PAGE[1] - y) * sy)))

        # ponytail: 2pt pad — a 60pt signature maps the 21-row band to ~4pt at QL 800px
        pad = 2.0
        band_hi = min(y1, y1 - (BAND[0] / IMG_H) * height + pad)
        band_lo = max(y0, y1 - (BAND[1] / IMG_H) * height - pad)
        ix0, iy0 = img(x0, band_hi)
        ix1, iy1 = img(x1, band_lo)
        xa, xb = min(ix0, ix1), max(ix0, ix1)
        ya, yb = min(iy0, iy1), max(iy0, iy1)
        hits = total = 0
        for iy in range(ya, yb + 1):
            for ix in range(xa, xb + 1):
                total += 1
                if darkish(at(ix, iy)):
                    hits += 1
        frac = hits / total if total else 0
        return {
            "png": png.name,
            "bmp": [w, h],
            "inside_frac": frac,
            "inside_red_frac": frac,
            "outside_white": True,
            "band": [xa, ya, xb, yb, total],
        }


def no_sig_check(path: Path):
    raw = path.read_bytes()
    hits = {
        "ByteRange": b"/ByteRange" in raw,
        "SigFlags": b"/SigFlags" in raw,
        "FT_Sig": b"/FT /Sig" in raw or b"/FT/Sig" in raw,
    }
    from pypdf import PdfReader
    reader = PdfReader(str(path))
    acro = None
    try:
        acro = reader.trailer["/Root"].get("/AcroForm")
        if acro is not None:
            acro = acro.get_object()
    except Exception:
        acro = None
    sig_flags = bool(acro) and "/SigFlags" in acro
    sig_fields = [
        name for name, rec in (reader.get_fields() or {}).items()
        if str(rec.get("/FT", "")).lstrip("/") == "Sig"
    ]
    ok = not any(hits.values()) and not sig_flags and not sig_fields
    return {"ok": ok, "pypdf": PYPDF, "raw": hits, "sigFlags": sig_flags, "sigFields": sig_fields}


def main():
    parser = argparse.ArgumentParser(description="independent PDF round-trip check")
    parser.add_argument("pdf")
    parser.add_argument("--text-notes", type=int, default=None,
                        help="require exactly N sticky notes on page 1 (P2 check)")
    parser.add_argument("--contents", default=None,
                        help="require Text/FreeText /Contents to match")
    parser.add_argument("--annots", default=None,
                        help="JSON map of page-1 subtype counts, e.g. {\"Highlight\":1}")
    parser.add_argument("--appearance", default=None,
                        help="Quick Look colour proof for this subtype (Stamp:n = nth stamp)")
    parser.add_argument("--no-sig", action="store_true",
                        help="require no /Sig field, /ByteRange, or /SigFlags")
    parser.add_argument("--pages", default=None,
                        help="JSON {count, rotate[], order[]} page-organisation check")
    parser.add_argument("--fields", default=None,
                        help="JSON map of AcroForm name -> expected /V")
    parser.add_argument("--fields-unique", action="store_true",
                        help="require unique top-level AcroForm /T names")
    parser.add_argument("--field-defs", default=None,
                        help="JSON field structure definitions (FT, rect, page, AP, DA, Opt, Kids/radio, tu/ff/dv)")
    parser.add_argument("--no-scripts", default=None, metavar="SOURCE_PDF",
                        help="require script-related structure counts to equal SOURCE_PDF")
    parser.add_argument("--image-page", default=None,
                        help="JSON {page,width,height} image XObject + MediaBox check")
    parser.add_argument("--labels", default=None,
                        help="JSON list of page labels")
    parser.add_argument("--text-has", default=None,
                        help="JSON {page:[needle,...]} extract_text contains")
    parser.add_argument("--font-names", default=None,
                        help="JSON {page:[BaseFont,...]}; every font must be Type1 WinAnsi, no /FontFile*")
    parser.add_argument("--no-tj-on", default=None,
                        help="target text whose showing operator must be Tj, not TJ")
    parser.add_argument("--layers", default=None,
                        help="JSON [{name,on}, ...] OCG /ON /OFF membership")
    parser.add_argument("--outline", default=None,
                        help="JSON [[title, page|null], ...] flattened preorder outline")
    parser.add_argument("--links", default=None,
                        help="JSON {page: [{uri}|{page}|{blocked}, ...]} Link annots")
    args = parser.parse_args()
    pdf = Path(args.pdf)

    if args.field_defs is not None or args.no_scripts is not None:
        verdict = {"ok": True, "pypdf": PYPDF}
        if args.field_defs is not None:
            expected = json.loads(args.field_defs)
            if not isinstance(expected, dict):
                raise SystemExit("bad --field-defs")
            verdict["field_defs"] = field_defs_check(pdf, expected)
            verdict["ok"] = verdict["ok"] and verdict["field_defs"]["ok"]
        if args.no_scripts is not None:
            verdict["no_scripts"] = no_scripts_check(pdf, Path(args.no_scripts))
            verdict["ok"] = verdict["ok"] and verdict["no_scripts"]["ok"]
        print(json.dumps(verdict, separators=(",", ":")))
        sys.exit(0 if verdict["ok"] else 1)

    if args.fields is not None:
        expected = json.loads(args.fields)
        if not isinstance(expected, dict):
            raise SystemExit("bad --fields")
        verdict = fields_check(pdf, expected)
        if args.appearance:
            name = args.appearance[6:] if args.appearance.startswith("Field:") else args.appearance
            rect = field_rect(pdf, name)
            vis = appearance(pdf, rect, darkish) if rect else {
                "inside_frac": 0, "inside_red_frac": 0, "outside_white": False, "png": None, "bmp": None,
            }
            vis_ok = vis["inside_frac"] > 0.01 and vis["outside_white"]
            verdict["appearance"] = vis
            verdict["ok"] = bool(verdict.get("ok", True)) and vis_ok
        print(json.dumps(verdict, separators=(",", ":")))
        sys.exit(0 if verdict["ok"] else 1)

    if args.fields_unique:
        verdict = fields_unique_check(pdf)
        print(json.dumps(verdict, separators=(",", ":")))
        sys.exit(0 if verdict["ok"] else 1)

    if args.image_page is not None:
        expected = json.loads(args.image_page)
        if not isinstance(expected, dict):
            raise SystemExit("bad --image-page")
        verdict = image_page_check(pdf, expected)
        print(json.dumps(verdict, separators=(",", ":")))
        sys.exit(0 if verdict["ok"] else 1)

    if args.labels is not None:
        expected = json.loads(args.labels)
        if not isinstance(expected, list):
            raise SystemExit("bad --labels")
        verdict = labels_check(pdf, expected)
        print(json.dumps(verdict, separators=(",", ":")))
        sys.exit(0 if verdict["ok"] else 1)

    if args.text_has is not None:
        expected = json.loads(args.text_has)
        if not isinstance(expected, dict):
            raise SystemExit("bad --text-has")
        verdict = text_has_check(pdf, expected)
        print(json.dumps(verdict, separators=(",", ":")))
        sys.exit(0 if verdict["ok"] else 1)

    if args.font_names is not None:
        expected = json.loads(args.font_names)
        if not isinstance(expected, dict):
            raise SystemExit("bad --font-names")
        verdict = font_names_check(pdf, expected)
        print(json.dumps(verdict, separators=(",", ":")))
        sys.exit(0 if verdict["ok"] else 1)

    if args.no_tj_on is not None:
        verdict = no_tj_on_check(pdf, args.no_tj_on)
        print(json.dumps(verdict, separators=(",", ":")))
        sys.exit(0 if verdict["ok"] else 1)

    if args.layers is not None:
        expected = json.loads(args.layers)
        if not isinstance(expected, list):
            raise SystemExit("bad --layers")
        verdict = layers_check(pdf, expected)
        print(json.dumps(verdict, separators=(",", ":")))
        sys.exit(0 if verdict["ok"] else 1)

    if args.pages is not None:
        expected = json.loads(args.pages)
        if not isinstance(expected, dict):
            raise SystemExit("bad --pages")
        verdict = pages_check(pdf, expected)
        print(json.dumps(verdict, separators=(",", ":")))
        sys.exit(0 if verdict["ok"] else 1)

    if args.outline is not None:
        expected = json.loads(args.outline)
        if not isinstance(expected, list):
            raise SystemExit("bad --outline")
        verdict = outline_check(pdf, expected)
        print(json.dumps(verdict, separators=(",", ":")))
        sys.exit(0 if verdict["ok"] else 1)

    if args.links is not None:
        expected = json.loads(args.links)
        if not isinstance(expected, dict):
            raise SystemExit("bad --links")
        verdict = links_check(pdf, expected)
        print(json.dumps(verdict, separators=(",", ":")))
        sys.exit(0 if verdict["ok"] else 1)

    if args.annots is not None or args.appearance is not None or args.no_sig:
        expected = json.loads(args.annots) if args.annots is not None else {}
        if args.annots is not None and not isinstance(expected, dict):
            raise SystemExit("bad --annots")
        verdict = annots_check(pdf, expected, args.contents) if args.annots is not None else {
            "ok": True, "pypdf": PYPDF, "annots": page1_annots(pdf) if args.appearance else [],
        }
        if args.appearance:
            items = verdict.get("annots") or page1_annots(pdf)
            match, kind = pick_annot(items, args.appearance)
            empty = {"inside_frac": 0, "inside_red_frac": 0, "outside_white": False, "png": None, "bmp": None}
            if kind == "Stamp":
                vis = stamp_band(pdf, match["rect"]) if match else empty
                # setStampImage letterboxes img-gray (2:3) into the 200×60 sign rect
                vis_ok = vis["inside_frac"] > 0.15
            else:
                pred = PREDICATE.get(kind)
                vis = appearance(pdf, match["rect"], pred) if match and pred else empty
                need = 0.01 if kind == "FreeText" else 0.3
                vis_ok = vis["inside_frac"] > need and vis["outside_white"]
            verdict["appearance"] = vis
            verdict["ok"] = bool(verdict.get("ok", True)) and vis_ok
        if args.no_sig:
            sig = no_sig_check(pdf)
            verdict["no_sig"] = sig
            verdict["ok"] = bool(verdict.get("ok", True)) and sig["ok"]
        print(json.dumps(verdict, separators=(",", ":")))
        sys.exit(0 if verdict["ok"] else 1)

    if args.text_notes is not None or args.contents is not None:
        verdict = text_notes(pdf, args.text_notes, args.contents)
        print(json.dumps(verdict, separators=(",", ":")))
        sys.exit(0 if verdict["ok"] else 1)

    field, squares = fields_and_annots(pdf)
    square = next((s for s in squares if "PDF4All P0 note" in s["contents"]), None)
    vis = appearance(pdf, square["rect"]) if square else {"inside_red_frac": 0, "outside_white": False, "png": None, "bmp": None}
    ok = (
        field == "PDF4All P0"
        and square is not None
        and vis["inside_red_frac"] > 0.5
        and vis["outside_white"]
    )
    verdict = {
        "ok": ok,
        "pypdf": PYPDF,
        "field": field,
        "squares": squares,
        "appearance": vis,
    }
    print(json.dumps(verdict, separators=(",", ":")))
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
