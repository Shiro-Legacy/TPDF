#!/usr/bin/env python3
"""DocMDP P=1 fixture. Recipe from fixtures/p0b/make-signed.py. Not byte-stable.

Run: .venv-qa/bin/python fixtures/p5/make-certified-p1.py
"""
import importlib.util
import pathlib

from pyhanko.pdf_utils.incremental_writer import IncrementalPdfFileWriter
from pyhanko.sign import signers
from pyhanko.sign.fields import MDPPerm

ROOT = pathlib.Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('p0b_make_signed', ROOT / 'fixtures/p0b/make-signed.py')
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)

ca = ROOT / 'fixtures/p0b/.ca'
src = ROOT / 'fixtures/p5/_unsigned.pdf'
dest = ROOT / 'fixtures/p5/certified-p1.pdf'
src.write_bytes(mod.base_pdf())
sig = mod.signer(ca / 'leaf.pem', ca / 'leaf.key', ca / 'ca.pem')
meta = signers.PdfSignatureMetadata(
    field_name='Sig1', reason='p5-certified-p1', certify=True, docmdp_permissions=MDPPerm.NO_CHANGES,
)
with open(src, 'rb') as inf, open(dest, 'wb') as out:
    signers.sign_pdf(IncrementalPdfFileWriter(inf), meta, signer=sig, output=out)
src.unlink()
print(f'wrote {dest} ({dest.stat().st_size} bytes)')
