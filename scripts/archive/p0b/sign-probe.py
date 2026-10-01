#!/usr/bin/env python3
"""P0-b probe 3: pyHanko validate (reference). QA only.

Usage: .venv-qa/bin/python scripts/p0b/sign-probe.py <pdf> [ca.pem]
Tools: pyHanko 0.37.0 (MIT), cryptography 50.0.1. No network.
Prints JSON: integrity, allowed-changes, trust, revocation, pretty details.
"""
import asyncio
import json
import sys
from pathlib import Path

from pyhanko.pdf_utils.reader import PdfFileReader
from pyhanko.sign.signers import load_certs_from_pemder
from pyhanko.sign.validation import async_validate_pdf_signature
from pyhanko.sign.validation.pdf_embedded import collect_embedded_signatures
from pyhanko_certvalidator import ValidationContext


async def one(path: Path, ca: Path | None):
    trust = list(load_certs_from_pemder([str(ca)])) if ca and ca.exists() else []
    # explicit VC: no OS trust store, no CRL/OCSP fetch
    vc = ValidationContext(trust_roots=trust, allow_fetching=False, revocation_mode="soft-fail")
    with open(path, "rb") as f:
        reader = PdfFileReader(f)
        sigs = collect_embedded_signatures(reader)
        if not sigs:
            return {"pdf": str(path), "signatures": 0, "error": "no embedded signatures"}
        out = []
        for sig in sigs:
            st = await async_validate_pdf_signature(sig, signer_validation_context=vc)
            out.append({
                "field": getattr(sig, "field_name", None),
                "integrity": {
                    "coverage": str(getattr(st, "coverage", None)),
                    "bottom_line": bool(st.bottom_line),
                    "summary": st.summary(),
                },
                "allowed_changes": {
                    "docmdp_ok": getattr(st, "docmdp_ok", None),
                    "modification_level": str(getattr(st, "modification_level", None)),
                    "diff_result": str(getattr(st, "diff_result", None)),
                },
                "trust": bool(st.trusted),
                "revocation": "not checked (no CRL/OCSP offline)" if st.revoked is None or st.revoked is False
                else f"revoked={st.revoked}",
                "revoked_flag": st.revoked,
                "pretty": st.pretty_print_details(),
            })
        return {"pdf": str(path), "signatures": len(sigs), "results": out}


def main():
    pdf = Path(sys.argv[1])
    ca = Path(sys.argv[2]) if len(sys.argv) > 2 else None
    print(json.dumps(asyncio.run(one(pdf, ca)), indent=2, default=str))


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(json.dumps({"ok": False, "error": type(exc).__name__, "msg": str(exc)}))
        raise SystemExit(1)
