"""Minimal deterministic PDF writer shared by the python fixture generators.

No creation date, no /ID, no producer string, so the same input always yields
the same bytes. Mirrors the object/xref layout already used by make-hello.py.
"""
import hashlib
import pathlib


def stream(data, extra=b""):
    """A stream object body. `extra` is appended inside the dict."""
    return b"<< /Length %d%s >>\nstream\n%sendstream" % (len(data), extra, data)


def pdf_bytes(objects):
    """Serialise `objects` (1-indexed by position) into a classic xref PDF."""
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
    out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (
        len(objects) + 1,
        xref,
    )
    return bytes(out)


def emit(path, objects):
    """Write `path` and return (bytes_len, sha256) for the provenance record."""
    data = pdf_bytes(objects)
    pathlib.Path(path).write_bytes(data)
    return len(data), hashlib.sha256(data).hexdigest()


def sha256_file(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()
