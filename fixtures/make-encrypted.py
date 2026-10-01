#!/usr/bin/env python3
"""Deterministic generator for the encrypted fixtures.

- fixtures/encrypted.pdf — one page, text "PDF4All encrypted", RC4-128, user
  password "pdf4all-user", all permissions granted.
- fixtures/encrypted-noprint.pdf — the same document with the print permission
  cleared (P1 unit 5: the print button must be refused), everything else granted.
- fixtures/encrypted-aes256.pdf — AES-256, owner ≠ user (pdf4all-owner).
- fixtures/encrypted-noedit.pdf — RC4-128, /P forbids modify+annotate.

No dates. Requires pypdf (in .venv-qa).
Run: .venv-qa/bin/python fixtures/make-encrypted.py
"""
import hashlib
import io
import pathlib

from pypdf import PdfWriter
from pypdf.constants import UserAccessPermissions
from pypdf.generic import (
    DecodedStreamObject,
    DictionaryObject,
    NameObject,
)

OUT = pathlib.Path(__file__).with_name("encrypted.pdf")
NO_PRINT_OUT = pathlib.Path(__file__).with_name("encrypted-noprint.pdf")
AES_OUT = pathlib.Path(__file__).with_name("encrypted-aes256.pdf")
NO_EDIT_OUT = pathlib.Path(__file__).with_name("encrypted-noedit.pdf")
TEXT = b"PDF4All encrypted"
NO_PRINT_TEXT = b"PDF4All encrypted no-print"
AES_TEXT = b"PDF4All encrypted AES-256"
NO_EDIT_TEXT = b"PDF4All encrypted no-edit"


def build(text: bytes, title: str, permissions_flag: int | None = None, algorithm: str = "RC4-128") -> bytes:
    writer = PdfWriter()
    writer.add_blank_page(width=612, height=792)
    page = writer.pages[0]
    font = writer._add_object(DictionaryObject({
        NameObject("/Type"): NameObject("/Font"),
        NameObject("/Subtype"): NameObject("/Type1"),
        NameObject("/BaseFont"): NameObject("/Helvetica"),
        NameObject("/Encoding"): NameObject("/WinAnsiEncoding"),
    }))
    page[NameObject("/Resources")] = DictionaryObject({
        NameObject("/Font"): DictionaryObject({NameObject("/F1"): font}),
    })
    content = DecodedStreamObject()
    content.set_data(b"BT /F1 24 Tf 72 700 Td (" + text + b") Tj ET\n")
    page[NameObject("/Contents")] = writer._add_object(content)
    writer.add_metadata({"/Title": title})
    # No permissions_flag for the plain fixture: the argument's default keeps its
    # bytes exactly as the first release of this generator wrote them.
    extra = {} if permissions_flag is None else {"permissions_flag": permissions_flag}
    writer.encrypt(user_password="pdf4all-user", owner_password="pdf4all-owner", algorithm=algorithm, **extra)
    buf = io.BytesIO()
    writer.write(buf)
    return buf.getvalue()


def write(path: pathlib.Path, data: bytes) -> None:
    path.write_bytes(data)
    print(f"wrote {path} ({len(data)} bytes) sha256={hashlib.sha256(data).hexdigest()}")


no_print_flags = UserAccessPermissions.all() & ~UserAccessPermissions.PRINT & ~UserAccessPermissions.PRINT_TO_REPRESENTATION
no_edit_flags = (
    UserAccessPermissions.all()
    & ~UserAccessPermissions.MODIFY
    & ~UserAccessPermissions.ADD_OR_MODIFY
)
write(OUT, build(TEXT, "PDF4All encrypted"))
write(NO_PRINT_OUT, build(NO_PRINT_TEXT, "PDF4All encrypted no print", no_print_flags))
write(AES_OUT, build(AES_TEXT, "PDF4All encrypted AES-256", algorithm="AES-256"))
write(NO_EDIT_OUT, build(NO_EDIT_TEXT, "PDF4All encrypted no edit", no_edit_flags))
