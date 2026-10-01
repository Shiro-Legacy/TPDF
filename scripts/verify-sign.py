#!/usr/bin/env python3
"""P5 verifier. pyHanko, QA only, never shipped.

Usage: .venv-qa/bin/python scripts/verify-sign.py <pdf>
       [--ca fixtures/p0b/.ca/ca.pem] [--expect-signatures n] [--expect-certified P]
One line per signature: field INTACT|INVALID coverage trusted|untrusted docmdp-ok|docmdp-VIOLATED
Then DocMDP P or none. Exit 1 if an expectation fails or a signature is not intact.
"""
import argparse
import asyncio
import sys
from pathlib import Path

from pyhanko.pdf_utils.reader import PdfFileReader
from pyhanko.sign.signers import load_certs_from_pemder
from pyhanko.sign.validation import async_validate_pdf_signature
from pyhanko.sign.validation.pdf_embedded import collect_embedded_signatures, read_certification_data
from pyhanko_certvalidator import ValidationContext


def coverage_name(value) -> str:
    text = str(value)
    return text.rsplit('.', 1)[-1]


async def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument('pdf')
    parser.add_argument('--ca', default='fixtures/p0b/.ca/ca.pem')
    parser.add_argument('--expect-signatures', type=int)
    parser.add_argument('--expect-certified', type=int)
    args = parser.parse_args()
    ca = Path(args.ca)
    trust = list(load_certs_from_pemder([str(ca)])) if ca.exists() else []
    vc = ValidationContext(trust_roots=trust, allow_fetching=False, revocation_mode='soft-fail')
    failed = False
    with open(args.pdf, 'rb') as handle:
        reader = PdfFileReader(handle)
        sigs = collect_embedded_signatures(reader)
        cert = read_certification_data(reader)
        perm = None if cert is None else int(cert.permission.value)
        for sig in sigs:
            status = await async_validate_pdf_signature(sig, signer_validation_context=vc)
            intact = 'INTACT' if str(status.summary()).startswith('INTACT') else 'INVALID'
            trusted = 'trusted' if status.trusted else 'untrusted'
            docmdp = 'docmdp-ok' if status.docmdp_ok is not False else 'docmdp-VIOLATED'
            print(f'{sig.field_name} {intact} {coverage_name(status.coverage)} {trusted} {docmdp}')
            if intact != 'INTACT' or (trust and not status.trusted) or status.docmdp_ok is False:
                failed = True
        print(f'DocMDP {perm if perm is not None else "none"}')
    if args.expect_signatures is not None and len(sigs) != args.expect_signatures:
        print(f'expected {args.expect_signatures} signatures, got {len(sigs)}', file=sys.stderr)
        failed = True
    if args.expect_certified is not None and perm != args.expect_certified:
        print(f'expected DocMDP P={args.expect_certified}, got {perm}', file=sys.stderr)
        failed = True
    return 1 if failed else 0


if __name__ == '__main__':
    try:
        raise SystemExit(asyncio.run(main()))
    except Exception as exc:
        print(f'{type(exc).__name__}: {exc}', file=sys.stderr)
        raise SystemExit(1)
