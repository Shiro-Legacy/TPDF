#!/usr/bin/env python3
"""Deterministic generator for fixtures/hello.pdf.

Self-authored ASCII-only PDF, one page, no third-party content, no dates
(byte-identical on every run). Run: python3 fixtures/make-hello.py
"""
import hashlib
import pathlib

TEXT = "PDF4All"
OUT = pathlib.Path(__file__).with_name("hello.pdf")

content = f"BT /F1 36 Tf 72 700 Td ({TEXT}) Tj ET\n".encode("ascii")
objects = [
    b"<< /Type /Catalog /Pages 2 0 R >>",
    b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
    b"/Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
    b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    b"<< /Length %d >>\nstream\n%sendstream" % (len(content), content),
]

out = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
offsets = []
for num, body in enumerate(objects, start=1):
    offsets.append(len(out))
    out += b"%d 0 obj\n%s\nendobj\n" % (num, body)

xref = len(out)
out += b"xref\n0 %d\n" % (len(objects) + 1)
out += b"0000000000 65535 f \n"
for off in offsets:
    out += b"%010d 00000 n \n" % off
out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (
    len(objects) + 1,
    xref,
)

OUT.write_bytes(bytes(out))
print(f"wrote {OUT} ({len(out)} bytes) sha256={hashlib.sha256(out).hexdigest()}")
