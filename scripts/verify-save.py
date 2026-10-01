#!/usr/bin/env python3
"""QA only: compare a no-op Save As with its source using pypdf, not MuPDF.

Usage: .venv-qa/bin/python scripts/verify-save.py source.pdf saved.pdf [--render]
       .venv-qa/bin/python scripts/verify-save.py --encryption file.pdf [--password pw]
Compares the reachable catalog graph (including tags, outlines, attachments,
forms, resources and decoded streams); ignores object numbering/compression.
--render also compares the first-page pixels produced by macOS Quick Look.
"""
import argparse
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path

import pypdf
from pypdf import PdfReader
from pypdf.generic import (
    ArrayObject, BooleanObject, ByteStringObject, DictionaryObject,
    IndirectObject, NameObject, NullObject, StreamObject, TextStringObject,
)


def catalog_hash(reader):
    refs, objects, pending = {}, [], []

    def value(obj):
        if isinstance(obj, IndirectObject):
            key = (obj.idnum, obj.generation)
            if key not in refs:
                refs[key] = len(pending)
                pending.append(obj)
            return {"ref": refs[key]}
        if isinstance(obj, DictionaryObject):
            ignored = {"/Length", "/Filter", "/DecodeParms"} if isinstance(obj, StreamObject) else set()
            result = {str(k): value(v) for k, v in sorted(obj.items()) if k not in ignored}
            if isinstance(obj, StreamObject):
                result["decoded-stream-sha256"] = hashlib.sha256(obj.get_data()).hexdigest()
            return result
        if isinstance(obj, ArrayObject):
            return [value(item) for item in obj]
        if isinstance(obj, NameObject):
            return {"name": str(obj)}
        if isinstance(obj, (ByteStringObject, TextStringObject)):
            return {"string": bytes(obj.original_bytes).hex()}
        if isinstance(obj, BooleanObject):
            return obj.value
        if isinstance(obj, NullObject):
            return None
        if isinstance(obj, (int, float)):
            return float(obj)
        raise TypeError(f"Unrecognized PDF object: {type(obj).__name__}")

    root = {key: value(reader.trailer.raw_get(key)) for key in ("/Root", "/Info") if key in reader.trailer}
    index = 0
    while index < len(pending):
        objects.append(value(pending[index].get_object()))
        index += 1
    encoded = json.dumps([root, objects], sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def image_hash(path):
    # Reuse the existing independent P0 renderer helpers; never bundled in app.
    import importlib.util
    spec = importlib.util.spec_from_file_location("roundtrip", Path(__file__).with_name("verify-roundtrip.py"))
    helpers = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helpers)
    with tempfile.TemporaryDirectory() as directory:
        directory = Path(directory)
        png = helpers.ql_png(path.resolve(), directory)
        bmp = directory / "page.bmp"
        subprocess.run(["sips", "-s", "format", "bmp", str(png), "--out", str(bmp)], check=True, capture_output=True)
        width, height, pixel = helpers.bmp_pixels(bmp)
        digest = hashlib.sha256()
        for y in range(height):
            digest.update(bytes(channel for x in range(width) for channel in pixel(x, y)))
        return {"width": width, "height": height, "sha256": digest.hexdigest()}


def encryption_report(path: Path, password: str = ""):
    """QA-only: /V /R /Length /P, whether user and empty passwords open it.
    Needs cryptography in .venv-qa (Apache-2.0 OR BSD-3-Clause) for AES-256.
    """
    import cryptography  # noqa: F401 — AES-256 files need the extra
    reader = PdfReader(str(path), strict=False)
    encrypted = bool(reader.is_encrypted)
    user_ok = empty_ok = False
    if encrypted:
        try:
            user_ok = reader.decrypt(password) != 0 if password else False
        except Exception:
            user_ok = False
        try:
            empty = PdfReader(str(path), strict=False)
            empty_ok = empty.decrypt("") != 0
        except Exception:
            empty_ok = False
        reader = PdfReader(str(path), strict=False)
        if password:
            try:
                reader.decrypt(password)
            except Exception:
                pass
    encrypt = None
    try:
        enc = reader.trailer.get("/Encrypt")
        if enc is not None:
            obj = enc.get_object() if hasattr(enc, "get_object") else enc
            encrypt = {
                "V": int(obj.get("/V", 0) or 0),
                "R": int(obj.get("/R", 0) or 0),
                "Length": int(obj.get("/Length", 0) or 0),
                "P": int(obj.get("/P", 0) or 0),
            }
    except Exception as exc:
        encrypt = {"error": type(exc).__name__}
    flags = {}
    if encrypt and "P" in encrypt:
        p = encrypt["P"]
        flags = {
            "print": bool(p & 4),
            "edit": bool(p & 8),
            "copy": bool(p & 16),
            "annotate": bool(p & 32),
            "form": bool(p & 256),
        }
    return {
        "ok": True,
        "encrypted": encrypted,
        "userOpens": user_ok,
        "emptyOpens": empty_ok,
        "encrypt": encrypt,
        "permissions": flags,
        "cryptography": __import__("cryptography").__version__,
        "pypdf": pypdf.__version__,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path, nargs="?")
    parser.add_argument("saved", type=Path, nargs="?")
    parser.add_argument("--render", action="store_true")
    parser.add_argument("--encryption", type=Path, metavar="PDF")
    parser.add_argument("--password", default="")
    args = parser.parse_args()
    if args.encryption:
        print(json.dumps(encryption_report(args.encryption, args.password)))
        return 0
    if not args.source or not args.saved:
        parser.error("source and saved are required unless --encryption")
    source, saved = PdfReader(args.source, strict=True), PdfReader(args.saved, strict=True)
    before, after = catalog_hash(source), catalog_hash(saved)
    result = {
        "ok": len(source.pages) == len(saved.pages) and before == after,
        "pypdf": pypdf.__version__, "pages": [len(source.pages), len(saved.pages)],
        "catalogHashes": [before, after],
    }
    if args.render:
        original_image, saved_image = image_hash(args.source), image_hash(args.saved)
        result["images"] = [original_image, saved_image]
        result["ok"] = result["ok"] and original_image == saved_image
    print(json.dumps(result))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    try:
        code = main()
    except Exception as error:
        print(json.dumps({"ok": False, "error": type(error).__name__}))
        code = 1
    raise SystemExit(code)
