#!/usr/bin/env python3
"""Deterministic generator for fixtures/encrypted-aes256-empty.pdf.

fixtures/hello.pdf encrypted AES-256 (Standard V5 R6) with an EMPTY user
password and owner password "pdf4all-owner", all permissions granted: a reader
opens it without a password prompt, yet the file is encrypted. The engine's
security-set refuses an empty AES-256 user password, so the suite cannot make
this file itself.

AES-256 needs random bytes (file key, salts, IVs). pypdf draws them from
secrets.token_bytes; this script replaces that with a fixed SHA-256 counter
stream so the output is byte-identical on every run. Fixture only: never do this
for a real document. No dates. Requires pypdf and cryptography (in .venv-qa).
Run: .venv-qa/bin/python fixtures/make-encrypted-aes256-empty.py
"""
import hashlib
import io
import pathlib
import secrets

from pypdf import PdfReader, PdfWriter

SRC = pathlib.Path(__file__).with_name("hello.pdf")
OUT = pathlib.Path(__file__).with_name("encrypted-aes256-empty.pdf")


def fixed_token_bytes():
    counter = 0

    def token_bytes(n: int = 32) -> bytes:
        nonlocal counter
        out = b""
        while len(out) < n:
            out += hashlib.sha256(b"tpdf-fixture-encrypted-aes256-empty:%d" % counter).digest()
            counter += 1
        return out[:n]

    return token_bytes


secrets.token_bytes = fixed_token_bytes()

writer = PdfWriter(clone_from=PdfReader(SRC))
writer.encrypt(user_password="", owner_password="pdf4all-owner", algorithm="AES-256")
buf = io.BytesIO()
writer.write(buf)
data = buf.getvalue()
OUT.write_bytes(data)
print(f"wrote {OUT} ({len(data)} bytes) sha256={hashlib.sha256(data).hexdigest()}")
