# Embedded ICC profiles in `mupdf-wasm.wasm` — provenance

Resolves inventory gate G2 (rev 2 said "no stated licence"). Evidence is read from the profile
files themselves, not from any summary page.

## Method

Downloaded `resources/icc/*.icc` from the upstream repo at the tag matching our pin
(`mupdf@1.28.1`), parsed each file's ICC tag table, and printed the `desc` and `cprt`
(copyright) tags. Then confirmed the same strings are present inside the shipped
`node_modules/mupdf/dist/mupdf-wasm.wasm`.

## Result

| Profile | `desc` tag | `cprt` tag | Present in shipped wasm |
|---|---|---|---|
| `rgb.icc` | "Artifex Software sRGB ICC Profile | `Copyright Artifex Software 2011` | yes (both strings found) |
| `gray.icc` | `#Artifex Software sGray ICC Profile` | `Copyright Artifex Software 2018` | yes |
| `cmyk.icc` | `Artifex CMYK SWOP Profile` (187 KB) | `Copyright Artifex Software 2011` | yes |
| `lab.icc` | `Lab2Lab` | `Copyright 2009 Artifex Software Inc` | yes |

String hits in the wasm: `Copyright Artifex Software` ×4, `Artifex CMYK SWOP Profile` ×1,
`Artifex Software sGray ICC Profile` ×1, `Lab2Lab` ×1, `Artifex Software Inc` ×1. The wasm
contains 7 `acsp` (ICC magic) blobs against 4 upstream files, consistent with profiles being
embedded more than once (e.g. stored and regenerated copies); the descriptors above are the
authoritative match.

## Conclusion

**Label: audit inference, not a verified grant.** Nothing in this document is a licence
grant we found in writing; it is an inference drawn from (a) Artifex copyright tags inside the
files and (b) the absence of any separate grant upstream.

- Authorship: **Artifex Software** (their own profiles, including a 187 KB SWOP CMYK profile).
- `resources/icc/` carries **no separate licence file**, and the profiles are not listed on
  MuPDF's third-party page.
- Inference: these profiles are part of the MuPDF source distribution and are **covered by
  MuPDF's AGPL-3.0-or-later / commercial dual licence**, the same terms as the wasm that embeds
  them — no separate obligation beyond the AGPL obligations already recorded for MuPDF.
- Confidence: reasonable but not primary-sourced from Artifex. If Artifex ever publishes a
  different statement, or if a distribution needs a written grant, this must be raised explicitly
  rather than assumed. It is not on the P0-a critical path (the profiles are embedded in the
  engine, not something we add).

## Commands

```bash
# parse tags (python3 stdlib only)
python3 - <<'EOF'
import pathlib, re
for p in sorted(pathlib.Path("resources/icc").glob("*.icc")):
    b = p.read_bytes(); n = int.from_bytes(b[128:132], "big")
    for i in range(n):
        o = 132 + 12*i
        sig = b[o:o+4].decode("latin1"); to = int.from_bytes(b[o+4:o+8], "big")
        sz = int.from_bytes(b[o+8:o+12], "big")
        if sig in ("cprt", "desc"):
            t = re.sub(rb"[^\x20-\x7e]", b" ", b[to+8:to+sz]).decode("latin1")
            print(p.name, sig, re.sub(r"\s{2,}", " ", t).strip()[:120])
EOF
```
