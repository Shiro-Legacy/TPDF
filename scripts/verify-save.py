#!/usr/bin/env python3
"""QA only: compare a no-op Save As with its source using pypdf, not MuPDF.

Usage: .venv-qa/bin/python scripts/verify-save.py source.pdf saved.pdf [--render]
       .venv-qa/bin/python scripts/verify-save.py --encryption file.pdf [--password pw] [--owner-password pw]
Compares the reachable catalog graph (including tags, outlines, attachments,
forms, resources and decoded streams); ignores object numbering/compression.
--render also compares the first-page pixels produced by macOS Quick Look (Windows.Data.Pdf on Windows).
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
        bmp = helpers.page_bmp(path.resolve(), directory)
        width, height, pixel = helpers.bmp_pixels(bmp)
        digest = hashlib.sha256()
        for y in range(height):
            digest.update(bytes(channel for x in range(width) for channel in pixel(x, y)))
        return {"width": width, "height": height, "sha256": digest.hexdigest()}


# Table 22 (ISO 32000-2) user-access bits, by their 1-based bit position's value.
PERMISSION_BITS = {
    "print": 4, "modify": 8, "copy": 16, "annotate": 32,
    "fillForms": 256, "extract": 512, "assemble": 1024, "printHighRes": 2048,
}


def password_level(path: Path, password):
    """Decrypt a fresh reader with one password: pypdf's PasswordType as a word.
    pypdf tries the owner password first, so a password that is both says "owner".
    Returns (report, reader-or-None); the reader holds the file key on success.
    """
    if password is None:
        return {"given": False}, None
    reader = PdfReader(str(path), strict=False)
    try:
        level = reader.decrypt(password)
    except Exception as exc:
        return {"given": True, "decrypted": False, "level": "error", "error": type(exc).__name__}, None
    word = {0: "none", 1: "user", 2: "owner"}.get(int(level), str(int(level)))
    ok = int(level) != 0
    return {"given": True, "decrypted": ok, "level": word, "passwordType": level.name}, (reader if ok else None)


def perms_check(reader, encrypt_obj, p: int):
    """R6 only: decrypt /Perms with AES-256-ECB under the file key pypdf derived
    (reader._encryption._key, set by a successful decrypt) using cryptography,
    independently of pypdf's own AlgV5.verify_perms. Consistent means: bytes 0-3
    are /P as a 32-bit little-endian integer (compared modulo 2**32, since
    writers store /P signed or unsigned), bytes 4-7 are 0xff, byte 8 is
    'T' or 'F' matching /EncryptMetadata (default true), bytes 9-11 are "adb".
    pypdf's own verdict (_are_permissions_valid) is reported beside it.
    """
    from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
    key = getattr(getattr(reader, "_encryption", None), "_key", None)
    perms = encrypt_obj.get("/Perms")
    if not key or perms is None:
        return {"checked": False, "reason": "no file key" if not key else "no /Perms"}
    raw = bytes(perms.original_bytes) if hasattr(perms, "original_bytes") else bytes(perms)
    if len(raw) < 16 or len(key) != 32:
        return {"checked": False, "reason": "bad /Perms or key length"}
    decryptor = Cipher(algorithms.AES(key), modes.ECB()).decryptor()
    block = decryptor.update(raw[:16]) + decryptor.finalize()
    encrypt_metadata = bool(encrypt_obj.get("/EncryptMetadata", True))
    perms_p = int.from_bytes(block[0:4], "little", signed=True)
    result = {
        "checked": True,
        "permsP": perms_p,
        "pMatches": (perms_p & 0xFFFFFFFF) == (p & 0xFFFFFFFF),
        "highBytesFF": block[4:8] == b"\xff\xff\xff\xff",
        "metadataFlag": chr(block[8]) if 32 <= block[8] < 127 else block[8],
        "metadataMatches": block[8:9] == (b"T" if encrypt_metadata else b"F"),
        "adb": block[9:12] == b"adb",
        "pypdfPermsValid": getattr(reader._encryption, "_are_permissions_valid", None),
    }
    result["consistent"] = all(result[k] for k in ("pMatches", "highBytesFF", "metadataMatches", "adb"))
    return result


def encryption_report(path: Path, password: str = "", owner_password=None):
    """QA-only: /V /R /Length /P, whether user and empty passwords open it.
    Needs cryptography in .venv-qa (Apache-2.0 OR BSD-3-Clause) for AES-256.
    Added fields: algorithm, per-password decrypt level (user/owner), /P decoded
    bit by bit, and for R6 a /Perms-vs-/P consistency check (see perms_check).
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
    algorithm = None
    if encrypt and "V" in encrypt:
        v, length = encrypt["V"], encrypt["Length"]
        cfm = None
        if v >= 4:
            try:
                std = obj.get("/CF", {}).get(obj.get("/StmF", "/Identity"), {})
                cfm = str(std.get("/CFM", "")) or None
            except Exception:
                cfm = None
        if v == 5:
            algorithm = "AES-256"
        elif v == 4:
            algorithm = {"/AESV2": "AES-128", "/V2": "RC4-128", "/None": "none"}.get(cfm, f"V4 {cfm}")
        elif v in (1, 2):
            algorithm = f"RC4-{length or 40}"
        else:
            algorithm = f"V{v}"
    passwords = {}
    file_reader = None
    if encrypted:
        user_level, user_reader = password_level(path, password if password else None)
        owner_level, owner_reader = password_level(path, owner_password)
        passwords = {"user": user_level, "owner": owner_level}
        file_reader = user_reader or owner_reader
    bits = {}
    p_unsigned = None
    if encrypt and "P" in encrypt:
        p_unsigned = encrypt["P"] & 0xFFFFFFFF
        bits = {name: bool(encrypt["P"] & mask) for name, mask in PERMISSION_BITS.items()}
    perms = None
    if encrypt and encrypt.get("R") == 6:
        perms = perms_check(file_reader, obj, encrypt["P"]) if file_reader else {"checked": False, "reason": "no password decrypted"}
    return {
        "ok": True,
        "encrypted": encrypted,
        "algorithm": algorithm,
        "ownerOpens": bool(passwords.get("owner", {}).get("decrypted")),
        "passwords": passwords,
        "P": encrypt.get("P") if encrypt else None,
        "PUnsigned": p_unsigned,
        "PSigned": (p_unsigned - (1 << 32) if p_unsigned >= 1 << 31 else p_unsigned) if p_unsigned is not None else None,
        "permissionBits": bits,
        "perms": perms,
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
    parser.add_argument("--password", default="", help="the user password")
    parser.add_argument("--owner-password", default=None)
    args = parser.parse_args()
    if args.encryption:
        print(json.dumps(encryption_report(args.encryption, args.password, args.owner_password)))
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
