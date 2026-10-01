#!/usr/bin/env python3
"""Redaction verifier. QA only, never shipped. Promotes scripts/p0b/recover.py.

Usage:
  .venv-qa/bin/python scripts/verify-redact.py <pdf> --token STR
      [--all-toggles | --expect-surfaces none|info,xmp,...] [--expect-alt]

Token hits split in two (§8.4). content = decompressed page /Contents plus
every XObject stream reachable from a page; any content hit fails the run.
other = everything else (Info, XMP, annots, fields, …); those fail only under
--all-toggles, or when they sit outside --expect-surfaces. --all-toggles also
requires every §2.2 surface absent, /XFA absent, /SigFlags absent. Leftover
/Subtype /Redact, /Prev, and a %%EOF count other than 1 fail every run.
P4-ALT-4242 is reported present-and-expected and is never a token hit (§8.1).

ponytail: hex scan is contiguous digits only; whitespace-separated hex is a
ceiling, add a strip-whitespace pass if a producer emits that form.
"""
import argparse
import json
import re
import sys
from pathlib import Path

from pypdf import PdfReader
from pypdf.generic import ArrayObject, DictionaryObject, IndirectObject, StreamObject

ALT = "P4-ALT-4242"
SURFACES = ("info", "xmp", "attachments", "outlines", "annots", "links", "fields", "layers", "struct")
KEEP_INFO = {"/Producer", "/CreationDate", "/ModDate"}
LAYER_OK = re.compile(r"Layer \d+")


def raw_hits(data: bytes, token: str) -> dict:
    ascii_n = token.encode("utf-8")
    u16 = token.encode("utf-16-be")
    bom = b"\xfe\xff" + u16
    n_bom = data.count(bom)
    low = data.lower()
    return {
        "ascii": data.count(ascii_n),
        "utf16be": n_bom + max(0, data.count(u16) - n_bom),
        "hex": sum(low.count(needle.hex().encode()) for needle in (ascii_n, u16, bom)),
    }


def _check_scanner():
    token = "P0B-SECRET-7731"
    blob = token.encode() + b"\xfe\xff" + token.encode("utf-16-be") + b"<" + token.encode().hex().upper().encode() + b">"
    assert raw_hits(blob, token) == {"ascii": 1, "utf16be": 1, "hex": 1}
    assert raw_hits(b"clean", token) == {"ascii": 0, "utf16be": 0, "hex": 0}
    assert sum(raw_hits(ALT.encode(), token).values()) == 0


def add(total, extra):
    for key in total:
        total[key] += extra[key]


def blob(obj) -> bytes:
    raw = getattr(obj, "original_bytes", None)
    if isinstance(raw, (bytes, bytearray)):
        return bytes(raw)
    if isinstance(obj, (bytes, bytearray)):
        return bytes(obj)
    return str(obj).encode("utf-8", "replace")


def empty_hits():
    return {"ascii": 0, "utf16be": 0, "hex": 0}


def content_hits(reader, token):
    """Decompressed page /Contents and XObject streams reachable from pages."""
    hits, ids, seen = empty_hits(), set(), set()

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
            if isinstance(obj, StreamObject):
                ids.add(key)
                try:
                    add(hits, raw_hits(obj.get_data(), token))
                except Exception:
                    add(hits, raw_hits(blob(obj), token))
                resources = get(obj, "/Resources") or {}
                xobjs = get(resources, "/XObject")
                if isinstance(xobjs, DictionaryObject):
                    for value in xobjs.values():
                        absorb(value)
                return
            if isinstance(obj, ArrayObject):
                for item in obj:
                    absorb(item)
            return
        if isinstance(obj, ArrayObject):
            for item in obj:
                absorb(item)

    for page in reader.pages:
        absorb(page.get("/Contents"))
        resources = get(page, "/Resources") or {}
        xobjs = get(resources, "/XObject")
        if isinstance(xobjs, DictionaryObject):
            for value in xobjs.values():
                absorb(value)
    return hits, ids


# Do not follow these back into the page tree, or a surface walk recounts content.
SKIP_KEYS = {"/Parent", "/P", "/Pg", "/Dest", "/D", "/Root", "/Pages", "/StructParents", "/Annots", "/Contents"}


def hit_n(obj, token, seen=None):
    if obj is None:
        return 0
    if seen is None:
        seen = set()
    if isinstance(obj, IndirectObject):
        key = (obj.idnum, obj.generation)
        if key in seen:
            return 0
        seen.add(key)
        try:
            obj = obj.get_object()
        except Exception:
            return 0
    if isinstance(obj, StreamObject):
        n = sum(raw_hits(blob(obj), token).values())
        try:
            n += sum(raw_hits(obj.get_data(), token).values())
        except Exception:
            pass
        for key, value in obj.items():
            if str(key) not in SKIP_KEYS:
                n += hit_n(value, token, seen)
        return n
    if isinstance(obj, DictionaryObject):
        n = 0
        for key, value in obj.items():
            if str(key) in SKIP_KEYS:
                continue
            n += sum(raw_hits(blob(value), token).values())
            n += hit_n(value, token, seen)
        return n
    if isinstance(obj, ArrayObject):
        return sum(hit_n(value, token, seen) for value in obj)
    found = sum(raw_hits(blob(obj), token).values())
    return found or (1 if token in str(obj) else 0)


def surface_tokens(reader, token):
    """Token hits outside page/XObject content, by §2.2 surface."""
    root = get(reader.trailer, "/Root") or {}
    info = get(reader.trailer, "/Info") or {}
    counts = {name: 0 for name in SURFACES}
    if isinstance(info, DictionaryObject):
        for key, value in info.items():
            if key not in KEEP_INFO:
                counts["info"] += hit_n(value, token)
    counts["xmp"] = hit_n(get(root, "/Metadata"), token)
    names = get(root, "/Names") or {}
    counts["attachments"] = hit_n(get(names, "/EmbeddedFiles"), token)
    counts["outlines"] = hit_n(get(root, "/Outlines"), token)
    for page in reader.pages:
        for annot in annots_of(page):
            sub = str(annot.get("/Subtype", ""))
            if sub == "/Link":
                counts["links"] += hit_n(annot, token)
            elif sub == "/FileAttachment":
                counts["attachments"] += hit_n(annot, token)
            elif sub not in ("/Widget", "/Redact"):
                counts["annots"] += hit_n(annot, token)
            if sub == "/Widget" or annot.get("/FT") is not None:
                counts["fields"] += hit_n(annot, token)
    acro = get(root, "/AcroForm") or {}
    counts["fields"] += hit_n(get(acro, "/XFA"), token)
    props = get(root, "/OCProperties") or {}
    for ocg in get(props, "/OCGs") or []:
        counts["layers"] += hit_n(ocg, token)
    counts["struct"] = hit_n(get(root, "/StructTreeRoot"), token)
    return counts


def walk_other(obj, token, content_ids, hits, seen):
    if isinstance(obj, IndirectObject):
        key = (obj.idnum, obj.generation)
        if key in seen:
            return
        seen.add(key)
        try:
            obj = obj.get_object()
        except Exception:
            return
        if key in content_ids and isinstance(obj, StreamObject):
            for value in obj.values():
                walk_other(value, token, content_ids, hits, seen)
            return
    if isinstance(obj, StreamObject):
        add(hits, raw_hits(blob(obj), token))
        try:
            add(hits, raw_hits(obj.get_data(), token))
        except Exception:
            pass
        for value in obj.values():
            walk_other(value, token, content_ids, hits, seen)
        return
    if isinstance(obj, DictionaryObject):
        for key, value in obj.items():
            add(hits, raw_hits(blob(key), token))
            add(hits, raw_hits(blob(value), token))
            walk_other(value, token, content_ids, hits, seen)
        return
    if isinstance(obj, ArrayObject):
        for value in obj:
            walk_other(value, token, content_ids, hits, seen)
        return
    found = raw_hits(blob(obj), token)
    if sum(found.values()) == 0 and token in str(obj):
        found["ascii"] += 1
    add(hits, found)


def get(obj, key):
    if obj is None:
        return None
    try:
        value = obj[key] if key in obj else None
    except Exception:
        return None
    if isinstance(value, IndirectObject):
        try:
            return value.get_object()
        except Exception:
            return None
    return value


def annots_of(page):
    arr = page.get("/Annots") or []
    out = []
    for item in arr:
        try:
            out.append(item.get_object() if isinstance(item, IndirectObject) else item)
        except Exception:
            continue
    return out


def inventory(reader):
    root = get(reader.trailer, "/Root") or {}
    info = get(reader.trailer, "/Info") or {}
    acro = get(root, "/AcroForm") or {}
    present = {name: False for name in SURFACES}
    present["info"] = any(key not in KEEP_INFO for key in getattr(info, "keys", lambda: [])())
    present["xmp"] = get(root, "/Metadata") is not None
    names = get(root, "/Names") or {}
    present["attachments"] = get(names, "/EmbeddedFiles") is not None
    present["outlines"] = get(root, "/Outlines") is not None
    redact = 0
    for page in reader.pages:
        for annot in annots_of(page):
            sub = str(annot.get("/Subtype", ""))
            if sub == "/Redact":
                redact += 1
            elif sub == "/Link":
                action = get(annot, "/A") or {}
                if get(annot, "/URI") is not None or get(action, "/URI") is not None or get(action, "/F") is not None:
                    present["links"] = True
            elif sub == "/FileAttachment":
                present["attachments"] = True
            elif sub != "/Widget":
                present["annots"] = True
            if sub == "/Widget" or annot.get("/FT") is not None:
                if str(annot.get("/FT", "")) == "/Sig" or any(key in annot for key in ("/V", "/DV", "/AP")):
                    present["fields"] = True
    fields = get(acro, "/Fields") or []
    for field in fields:
        try:
            obj = field.get_object() if isinstance(field, IndirectObject) else field
        except Exception:
            continue
        if str(obj.get("/FT", "")) == "/Sig" or any(key in obj for key in ("/V", "/DV", "/AP")):
            present["fields"] = True
    props = get(root, "/OCProperties") or {}
    for ocg in get(props, "/OCGs") or []:
        try:
            name = str((ocg.get_object() if isinstance(ocg, IndirectObject) else ocg).get("/Name", ""))
        except Exception:
            name = ""
        if not LAYER_OK.fullmatch(name):
            present["layers"] = True
    seen = set()

    def struct(node):
        if isinstance(node, IndirectObject):
            key = (node.idnum, node.generation)
            if key in seen:
                return
            seen.add(key)
            try:
                node = node.get_object()
            except Exception:
                return
        if not isinstance(node, DictionaryObject):
            return
        if any(key in node for key in ("/ActualText", "/Alt", "/E")):
            present["struct"] = True
        kids = node.get("/K")
        if isinstance(kids, ArrayObject):
            for kid in kids:
                struct(kid)
        elif kids is not None:
            struct(kids)

    struct(get(root, "/StructTreeRoot"))
    return present, {
        "redact": redact,
        "xfa": get(acro, "/XFA") is not None,
        "sigFlags": get(acro, "/SigFlags") is not None,
    }


def main():
    _check_scanner()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pdf", type=Path)
    parser.add_argument("--token", required=True)
    parser.add_argument("--all-toggles", action="store_true")
    parser.add_argument("--expect-surfaces", help="none, or comma list allowed to remain")
    parser.add_argument("--expect-alt", action="store_true")
    args = parser.parse_args()
    if args.expect_surfaces == "none":
        args.all_toggles = True
    allowed = None
    if args.expect_surfaces and args.expect_surfaces != "none":
        allowed = set(args.expect_surfaces.split(","))
        bad = allowed - set(SURFACES)
        if bad:
            print(json.dumps({"ok": False, "error": "unknown surface", "names": sorted(bad)}))
            return 1

    raw = args.pdf.read_bytes()
    content, other = empty_hits(), empty_hits()
    by_surface = {name: 0 for name in SURFACES}
    surfaces = {name: False for name in SURFACES}
    extra = {"redact": 0, "xfa": False, "sigFlags": False}
    encrypted = b"/Encrypt" in raw
    error = None
    reader = None
    try:
        reader = PdfReader(str(args.pdf), strict=False)
        if reader.is_encrypted:
            encrypted = True
        else:
            content, content_ids = content_hits(reader, args.token)
            walk_other(reader.trailer, args.token, content_ids, other, set())
            surfaces, extra = inventory(reader)
            by_surface = surface_tokens(reader, args.token)
    except Exception as exc:
        error = type(exc).__name__
        # Can't split streams. Treat a raw hit as content so a parse failure cannot pass.
        add(content, raw_hits(raw, args.token))

    alt = "present-and-expected" if ALT.encode() in raw else "absent"
    # The xref chain, not a byte scan: outline siblings legitimately carry /Prev.
    prev = reader is not None and "/Prev" in reader.trailer
    eof = raw.count(b"%%EOF")
    content_n, other_n = sum(content.values()), sum(other.values())
    failures = []
    if prev:
        failures.append("prev")
    if eof != 1:
        failures.append("eof")
    if extra["redact"] or b"/Subtype /Redact" in raw or b"/Subtype/Redact" in raw:
        failures.append("redact-annot")
    if content_n:
        failures.append("content")
    if args.all_toggles:
        if other_n:
            failures.append("other")
        if extra["xfa"] or b"/XFA" in raw:
            failures.append("xfa")
        if extra["sigFlags"]:
            failures.append("sigflags")
        failures.extend(name for name, on in surfaces.items() if on)
    elif allowed is not None:
        failures.extend(name for name, on in surfaces.items() if on and name not in allowed)
        outside = [name for name, n in by_surface.items() if n and name not in allowed]
        failures.extend(f"other:{name}" for name in outside)
        if other_n and not any(by_surface.values()):
            failures.append("other")
    if args.expect_alt and alt != "present-and-expected":
        failures.append("alt-missing")
    if error and (args.all_toggles or encrypted or content_n):
        failures.append(error)

    result = {
        "ok": not failures,
        "pdf": str(args.pdf),
        "token": args.token,
        "content_hits": content_n,
        "other_hits": other_n,
        "content": content,
        "other": other,
        "other_by_surface": by_surface,
        "inline_alt": alt,
        "surfaces": surfaces,
        "redact_annots": extra["redact"],
        "xfa": extra["xfa"],
        "sigFlags": extra["sigFlags"],
        "prev": prev,
        "eof": eof,
        "encrypted": encrypted,
        "failures": failures,
        "error": error,
    }
    print(json.dumps(result, indent=2))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except SystemExit:
        raise
    except Exception as exc:
        print(json.dumps({"ok": False, "error": type(exc).__name__, "msg": str(exc)}))
        raise SystemExit(1)
