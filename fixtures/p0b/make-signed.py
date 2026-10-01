#!/usr/bin/env python3
"""P0-b probe 3 fixtures via pyHanko 0.37.0 (MIT) + certs from make-signed.sh.

Run: bash fixtures/p0b/make-signed.sh && .venv-qa/bin/python fixtures/p0b/make-signed.py
Writes signed-intact, certified-p2, certified-p2-filled, certified-p2-annot,
signed-tampered, signed-untrusted under fixtures/p0b/. No network.
"""
import hashlib
import io
import json
import pathlib
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
from minipdf import stream  # noqa: E402

from pyhanko.pdf_utils.incremental_writer import IncrementalPdfFileWriter
from pyhanko.sign import signers
from pyhanko.sign.fields import MDPPerm
from pypdf import PdfWriter
from pypdf.annotations import Text

DIR = pathlib.Path(__file__).resolve().parent
CA = DIR / ".ca"


def base_pdf():
    content = b"BT /F1 12 Tf 72 700 Td (PDF4All signed) Tj ET\n"
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R /AcroForm 6 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R /Annots [7 0 R] >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        stream(content),
        b"<< /Fields [7 0 R] /DA (/Helv 12 Tf 0 g) /SigFlags 3 >>",
        b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /V () /DV () "
        b"/Rect [72 640 340 670] /F 4 /P 3 0 R /DA (/Helv 12 Tf 0 g) >>",
    ]
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
    out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objects) + 1, xref)
    return bytes(out)


def signer(leaf, key, ca):
    return signers.SimpleSigner.load(str(key), str(leaf), ca_chain_files=(str(ca),))


def sign(src, dest, sig, *, certify=False):
    meta = signers.PdfSignatureMetadata(
        field_name="Sig1",
        reason="p0b-probe3",
        certify=certify,
        docmdp_permissions=MDPPerm.FILL_FORMS,
    )
    with open(src, "rb") as inf, open(dest, "wb") as outf:
        w = IncrementalPdfFileWriter(inf)
        signers.sign_pdf(w, meta, signer=sig, output=outf)


def incremental_fill(src, dest, value):
    w = PdfWriter(fileobj=str(src), incremental=True)
    w.update_page_form_field_values(w.pages[0], {"name": value})
    with open(dest, "wb") as out:
        w.write(out)


def incremental_annot(src, dest):
    w = PdfWriter(fileobj=str(src), incremental=True)
    w.add_annotation(0, Text(rect=(72, 400, 200, 450), text="p0b-annot"))
    with open(dest, "wb") as out:
        w.write(out)


def tamper(src, dest):
    data = bytearray(pathlib.Path(src).read_bytes())
    m = re.search(rb"/ByteRange\s*\[\s*(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s*\]", data)
    if not m:
        raise SystemExit("no /ByteRange")
    a, b, c, d = map(int, m.groups())
    needle = b"PDF4All signed"
    i = data.find(needle)
    if i < 0:
        raise SystemExit("no tamper needle")
    in_range = (a <= i < a + b) or (c <= i < c + d)
    if not in_range:
        raise SystemExit(f"needle at {i} not in ByteRange {a,b,c,d}")
    data[i] ^= 0x01
    pathlib.Path(dest).write_bytes(data)


def provenance(name, path):
    data = path.read_bytes()
    rec = {
        "id": name,
        "file": f"fixtures/p0b/{path.name}",
        "bytes": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
        "generator": {
            "path": "fixtures/p0b/make-signed.py",
            "certs": "fixtures/p0b/make-signed.sh",
            "dependencies": ["pyHanko 0.37.0 MIT", "openssl", "pypdf 6.18.1"],
            "reproduce": "bash fixtures/p0b/make-signed.sh && .venv-qa/bin/python fixtures/p0b/make-signed.py",
            "timestamps_in_output": True,
            "deterministic": False,
            "note": "CMS signatures carry signing-time; bytes change every run",
        },
        "provenance": {
            "authorship": "self-signed P0B Test CA (fixtures/p0b/.ca, gitignored); PDF constructed here",
            "network_access": "none",
        },
        "generated_on": "2026-09-16",
    }
    path.with_suffix(".provenance.json").write_text(json.dumps(rec, indent=2) + "\n")


def main():
    for p in (CA / "leaf.pem", CA / "leaf.key", CA / "ca.pem"):
        if not p.exists():
            raise SystemExit(f"missing {p}; run bash fixtures/p0b/make-signed.sh")
    unsigned = DIR / "_unsigned.pdf"
    unsigned.write_bytes(base_pdf())
    trusted = signer(CA / "leaf.pem", CA / "leaf.key", CA / "ca.pem")
    other = signer(CA / "other-leaf.pem", CA / "other-leaf.key", CA / "other-ca.pem")

    intact = DIR / "signed-intact.pdf"
    sign(unsigned, intact, trusted)
    cert = DIR / "certified-p2.pdf"
    sign(unsigned, cert, trusted, certify=True)
    filled = DIR / "certified-p2-filled.pdf"
    incremental_fill(cert, filled, "filled")
    annot = DIR / "certified-p2-annot.pdf"
    incremental_annot(cert, annot)
    tamp = DIR / "signed-tampered.pdf"
    tamper(intact, tamp)
    untrusted = DIR / "signed-untrusted.pdf"
    sign(unsigned, untrusted, other)
    unsigned.unlink()
    for p in (intact, cert, filled, annot, tamp, untrusted):
        provenance(p.stem, p)
        print(f"wrote {p} ({p.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
