#!/usr/bin/env python3
"""P5 U2 verification fixtures. pyHanko + openssl. No network. Not byte-stable.

Run: bash fixtures/p5/make-verify-fixtures.sh
"""
import hashlib
import json
import pathlib
import subprocess
import sys

from pyhanko.pdf_utils.incremental_writer import IncrementalPdfFileWriter
from pyhanko.sign import signers

ROOT = pathlib.Path(__file__).resolve().parents[2]
P0B = ROOT / "fixtures/p0b"
P5 = ROOT / "fixtures/p5"
CA = P0B / ".ca"
FALSE = P5 / "false-ca"
sys.path.insert(0, str(P0B.parent))
from minipdf import stream  # noqa: E402


def base_pdf():
    # Same one-page form as fixtures/p0b/make-signed.py, copied so this script stands alone.
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
    out += (
        b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n"
        % (len(objects) + 1, xref)
    )
    return bytes(out)


def openssl(*args):
    run = subprocess.run(["openssl", *args], capture_output=True, text=True)
    if run.returncode != 0:
        raise SystemExit(run.stderr.strip() or f"openssl {' '.join(args)} failed")


def mint_false_ca():
    FALSE.mkdir(exist_ok=True)
    ext = FALSE / "ext.cnf"
    ext.write_text("basicConstraints=CA:FALSE\n")
    if not (FALSE / "alice.pem").exists():
        openssl("genrsa", "-out", str(FALSE / "alice.key"), "2048")
        openssl("req", "-new", "-key", str(FALSE / "alice.key"), "-subj", "/CN=Honest Alice", "-out", str(FALSE / "alice.csr"))
        openssl(
            "x509", "-req", "-in", str(FALSE / "alice.csr"), "-CA", str(CA / "ca.pem"),
            "-CAkey", str(CA / "ca.key"), "-CAcreateserial", "-sha256", "-days", "30",
            "-extfile", str(ext), "-out", str(FALSE / "alice.pem"),
        )
    if not (FALSE / "bob2.pem").exists():
        openssl("genrsa", "-out", str(FALSE / "bob2.key"), "2048")
        openssl("req", "-new", "-key", str(FALSE / "bob2.key"), "-subj", "/CN=Forged Bob", "-out", str(FALSE / "bob2.csr"))
        openssl(
            "x509", "-req", "-in", str(FALSE / "bob2.csr"), "-CA", str(FALSE / "alice.pem"),
            "-CAkey", str(FALSE / "alice.key"), "-CAcreateserial", "-sha256", "-days", "30",
            "-extfile", str(ext), "-out", str(FALSE / "bob2.pem"),
        )


def mint_expired():
    FALSE.mkdir(exist_ok=True)
    ext = FALSE / "leaf.cnf"
    ext.write_text("basicConstraints=CA:FALSE\nkeyUsage=critical,digitalSignature\n")
    key = FALSE / "expired.key"
    if not key.exists():
        openssl("genrsa", "-out", str(key), "2048")
    openssl("req", "-new", "-key", str(key), "-subj", "/CN=P0B Expired", "-out", str(FALSE / "expired.csr"))
    openssl(
        "x509", "-req", "-in", str(FALSE / "expired.csr"), "-CA", str(CA / "ca.pem"),
        "-CAkey", str(CA / "ca.key"), "-CAcreateserial", "-sha256",
        "-not_before", "20200101000000Z", "-not_after", "20200102000000Z",
        "-extfile", str(ext), "-out", str(FALSE / "expired.pem"),
    )


def sign(dest, signer, *, md=None, reason="p5-verify"):
    unsigned = P5 / "_unsigned.pdf"
    unsigned.write_bytes(base_pdf())
    meta = signers.PdfSignatureMetadata(field_name="Sig1", reason=reason, md_algorithm=md)
    try:
        with open(unsigned, "rb") as inf, open(dest, "wb") as out:
            signers.sign_pdf(IncrementalPdfFileWriter(inf), meta, signer=signer, output=out)
    finally:
        unsigned.unlink(missing_ok=True)


def provenance(name, note):
    path = P5 / name
    data = path.read_bytes()
    rec = {
        "id": path.stem,
        "file": f"fixtures/p5/{name}",
        "bytes": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
        "generator": {
            "path": "fixtures/p5/make-verify-fixtures.py",
            "certs": "fixtures/p0b/.ca and fixtures/p5/false-ca",
            "dependencies": ["pyHanko", "openssl 3"],
            "reproduce": "bash fixtures/p5/make-verify-fixtures.sh",
            "timestamps_in_output": True,
            "deterministic": False,
            "note": note,
        },
        "provenance": {
            "authorship": "self-signed P0B Test CA; PDF constructed here",
            "network_access": "none",
        },
    }
    path.with_suffix(".provenance.json").write_text(json.dumps(rec, indent=2) + "\n")
    print(f"wrote {path.name} ({len(data)} bytes)")


def main():
    if not (CA / "ca.pem").exists() or not (CA / "ec.pem").exists():
        raise SystemExit("run bash fixtures/p0b/make-signed.sh and fixtures/p5/make-p12.sh first")
    mint_false_ca()
    mint_expired()
    sign(P5 / "signed-ec.pdf", signers.SimpleSigner.load(str(CA / "ec.key"), str(CA / "ec.pem"), ca_chain_files=(str(CA / "ca.pem"),)), reason="p5-ec")
    provenance("signed-ec.pdf", "ECDSA P-256 leaf fixtures/p0b/.ca/ec.pem, signed by P0B Test CA")
    sign(P5 / "signed-sha1.pdf", signers.SimpleSigner.load(str(CA / "leaf.key"), str(CA / "leaf.pem"), ca_chain_files=(str(CA / "ca.pem"),)), md="sha1", reason="p5-sha1")
    provenance("signed-sha1.pdf", "RSA leaf, md_algorithm=sha1; expect valid + weak-digest")
    try:
        sign(
            P5 / "signed-expired.pdf",
            signers.SimpleSigner.load(str(FALSE / "expired.key"), str(FALSE / "expired.pem"), ca_chain_files=(str(CA / "ca.pem"),)),
            reason="p5-expired",
        )
        provenance("signed-expired.pdf", "leaf notBefore/notAfter in 2020; expect trust expired")
    except Exception as exc:
        print(f"signed-expired.pdf skipped: {exc}", file=sys.stderr)
        (P5 / "signed-expired.SKIPPED").write_text(f"{type(exc).__name__}: {exc}\n")
    try:
        sign(
            P5 / "signed-false-ca.pdf",
            signers.SimpleSigner.load(str(FALSE / "bob2.key"), str(FALSE / "bob2.pem"), ca_chain_files=(str(FALSE / "alice.pem"), str(CA / "ca.pem"))),
            reason="p5-false-ca",
        )
        provenance("signed-false-ca.pdf", "Forged Bob issued by Honest Alice (CA:FALSE) issued by P0B Test CA; expect untrusted")
    except Exception as exc:
        print(f"signed-false-ca.pdf skipped: {exc}", file=sys.stderr)
        (P5 / "signed-false-ca.SKIPPED").write_text(f"{type(exc).__name__}: {exc}\n")


if __name__ == "__main__":
    main()
