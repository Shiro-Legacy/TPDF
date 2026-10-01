#!/usr/bin/env python3
"""P0-b probe 2 recoverability check (pypdf + raw scan). QA only, never shipped.

Usage: .venv-qa/bin/python scripts/p0b/recover.py <pdf> <token>
Tools: pypdf 6.18.1 (BSD-3-Clause) in .venv-qa.
Prints JSON: raw hits, object/stream hits, extract_text, image uniqueness.
"""
import json
import sys
from pathlib import Path

from pypdf import PdfReader
from pypdf.generic import (
    ArrayObject,
    ByteStringObject,
    DictionaryObject,
    IndirectObject,
    NameObject,
    StreamObject,
    TextStringObject,
)

TOKEN_DEFAULT = "P0B-SECRET-7731"


def b(obj):
    if isinstance(obj, (bytes, bytearray)):
        return bytes(obj)
    if isinstance(obj, (ByteStringObject, TextStringObject)):
        try:
            return bytes(obj.original_bytes)
        except Exception:
            return str(obj).encode("latin1", "replace")
    if isinstance(obj, NameObject):
        return str(obj).encode("latin1", "replace")
    if isinstance(obj, str):
        return obj.encode("latin1", "replace")
    return str(obj).encode("latin1", "replace")


def walk(obj, needle, seen, hits, path):
    if isinstance(obj, IndirectObject):
        key = (obj.idnum, obj.generation)
        if key in seen:
            return
        seen.add(key)
        path = f"{path}->{obj.idnum}0R"
        try:
            obj = obj.get_object()
        except Exception as exc:
            hits.append({"path": path, "error": type(exc).__name__})
            return
    if isinstance(obj, StreamObject):
        if needle in b(obj):
            hits.append({"path": path, "where": "stream-dict"})
        try:
            data = obj.get_data()
        except Exception:
            data = b""
        if needle in data:
            hits.append({"path": path, "where": "decompressed-stream", "n": data.count(needle)})
        for k, v in obj.items():
            walk(v, needle, seen, hits, f"{path}/{k}")
        return
    if isinstance(obj, DictionaryObject):
        for k, v in obj.items():
            walk(k, needle, seen, hits, f"{path}/k")
            walk(v, needle, seen, hits, f"{path}/{k}")
        return
    if isinstance(obj, ArrayObject):
        for i, v in enumerate(obj):
            walk(v, needle, seen, hits, f"{path}[{i}]")
        return
    if needle in b(obj):
        hits.append({"path": path, "where": "string", "sample": b(obj)[:80].decode("latin1", "replace")})


def classify(hits):
    def any_path(*parts):
        return any(any(p in h.get("path", "") or p in h.get("sample", "") for p in parts) for h in hits)
    return {
        "info_title": any_path("Info", "/Title"),
        "xmp": any_path("Metadata"),
        "embedded": any_path("Embedded", ".txt"),
        "outline": any_path("Outlines"),
        "ocg": any_path("OCProperties", "/OCG", "/Name"),
        "struct": any_path("Struct", "ActualText", "/Alt"),
        "fields": any_path("AcroForm", "/V"),
        "annots": any_path("Annot", "FreeText", "/Contents"),
        "links": any_path("/URI", "URI"),
    }


def images(reader):
    report, seen = [], set()

    def visit(obj):
        if isinstance(obj, IndirectObject):
            key = (obj.idnum, obj.generation)
            if key in seen:
                return
            seen.add(key)
            try:
                obj = obj.get_object()
            except Exception:
                return
        if isinstance(obj, StreamObject) and str(obj.get("/Subtype", "")) == "/Image":
            try:
                data = obj.get_data()
            except Exception as exc:
                report.append({"error": type(exc).__name__})
                return
            report.append({
                "width": int(obj.get("/Width", 0) or 0),
                "height": int(obj.get("/Height", 0) or 0),
                "len": len(data),
                "unique_byte_values": len(set(data)),
                "one_colour": len(set(data)) <= 1,
            })
        if isinstance(obj, DictionaryObject):
            for v in obj.values():
                visit(v)
        elif isinstance(obj, ArrayObject):
            for v in obj:
                visit(v)

    visit(reader.root_object)
    return report


def main():
    pdf = Path(sys.argv[1])
    token = sys.argv[2] if len(sys.argv) > 2 else TOKEN_DEFAULT
    needle = token.encode("ascii")
    raw = pdf.read_bytes()
    reader = PdfReader(str(pdf), strict=False)
    hits = []
    walk(reader.trailer, needle, set(), hits, "trailer")
    texts = [(p.extract_text() or "") for p in reader.pages]
    result = {
        "ok": raw.count(needle) == 0 and not hits and not any(token in t for t in texts),
        "pdf": str(pdf),
        "token": token,
        "raw_count": raw.count(needle),
        "object_hit_count": len(hits),
        "object_hits": hits[:100],
        "surfaces": classify(hits),
        "extract_text": texts,
        "extract_has_token": [token in t for t in texts],
        "images": images(reader),
        "pages": len(reader.pages),
    }
    print(json.dumps(result, indent=2, default=str))
    return 0 if result["ok"] else 2


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(json.dumps({"ok": False, "error": type(exc).__name__, "msg": str(exc)}))
        raise SystemExit(1)
