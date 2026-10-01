#!/usr/bin/env python3
"""Fetch the licence texts for the third-party libraries compiled into mupdf-wasm.wasm.

Pinned to MuPDF tag 1.28.1's submodule gitlinks (Artifex forks), so the texts correspond to
the code that is actually linked into the artefact we ship - not to whatever upstream HEAD is.

Re-run on every engine bump: `python3 docs/third-party/fetch-notices.py`.
Writes docs/third-party/lib/*.txt and docs/third-party/lib/PROVENANCE.json.
"""
import hashlib
import json
import pathlib
import urllib.request

# MuPDF 1.28.1 -> thirdparty submodule commits (mode 160000), read from the tag's git tree.
MUPDF_COMMIT = "20061bd45183"  # tag 1.28.1 peels to this commit

LIBS = [
    # name, repo, pinned commit, file in repo, SPDX, what it is
    ("freetype", "ArtifexSoftware/thirdparty-freetype2",
     "0a0221a1347e2f1e07c395263540026e9a0aa7c7", "docs/FTL.TXT", "FTL",
     "FreeType License; LICENSE.TXT in the same repo lists FTL or GPLv2 as the two options"),
    ("harfbuzz", "ArtifexSoftware/thirdparty-harfbuzz",
     "c28ba35a1e6dc4b6b32e2b1d11fd2a408f6c86bb", "COPYING", "MIT",
     "Old-style MIT text"),
    ("lcms2", "ArtifexSoftware/thirdparty-lcms2",
     "d69c64417c4a33a4629957fc0e08f4f4c5abcc3d", "LICENSE", "MIT",
     "Artifex fork of lcms2, built as lcms2mt"),
    ("libjpeg", "ArtifexSoftware/thirdparty-libjpeg",
     "627faae398468d5d11e3d694dc325432f734b5c4", "README", "IJG",
     "IJG libjpeg; the README itself is the licence and the repo has no LICENSE/COPYING file"),
    ("openjpeg", "ArtifexSoftware/thirdparty-openjpeg",
     "957029eb875eee1118743f200cb86da9d8289de2", "LICENSE", "BSD-2-Clause",
     "Two-clause BSD"),
    ("gumbo", "ArtifexSoftware/thirdparty-gumbo-parser",
     "61bd4e736c3b342e592f65be0d65ec6a0acd8dcb", "COPYING", "Apache-2.0",
     "Apache-2.0 text; repo ships no separate NOTICE file"),
    ("zlib", "ArtifexSoftware/thirdparty-zlib",
     "da607da739fa6047df13e66a2af6b8bec7c2a498", "LICENSE", "Zlib",
     "Artifex fork of zlib"),
    ("jbig2dec", "ArtifexSoftware/jbig2dec",
     "dc15c39bbbddc90f79c14563d2eb5a794106be8f", "COPYING", "AGPL-3.0-or-later",
     "Same licence family as MuPDF itself"),
    ("jbig2dec", "ArtifexSoftware/jbig2dec",
     "dc15c39bbbddc90f79c14563d2eb5a794106be8f", "LICENSE", "AGPL-3.0-or-later",
     "Per-file statement naming AGPL-3.0-or-later explicitly"),
    # Below: no symbol traces in the shipped wasm, but MuPDF's docs list them among the required
    # libraries and the brotli error string is present - ship the notices rather than argue.
    ("brotli", "ArtifexSoftware/thirdparty-brotli",
     "2523f3314501aa5c90e561922b2e668b3ccee26e", "LICENSE", "MIT",
     "Only the message 'brotli compression not enabled' is present (compiled-out path); "
     "notice shipped defensively"),
    ("cmark-gfm", "ArtifexSoftware/thirdparty-cmark-gfm",
     "74e0f5b6b62bbdd1e262c92f4bb2c6c871c6307e", "COPYING", "MIT",
     "61 distinct cmark_* symbols in the wasm (cmark_arena_push, cmark_iter_new, "
     "cmark_inline_parser_*) - compiled in, not optional"),
]

OUT = pathlib.Path(__file__).parent / "lib"


def fetch(repo, commit, path):
    url = f"https://raw.githubusercontent.com/{repo}/{commit}/{path}"
    with urllib.request.urlopen(url) as r:
        return r.read()


def main():
    OUT.mkdir(exist_ok=True)
    records = []
    for name, repo, commit, path, spdx, note in LIBS:
        blob = fetch(repo, commit, path)
        dest = OUT / f"{name}-{path.replace('/', '_').replace('.', '-')}.txt"
        dest.write_bytes(blob)
        records.append({
            "library": name,
            "spdx": spdx,
            "source_repo": f"https://github.com/{repo}",
            "pinned_commit": commit,
            "path_in_repo": path,
            "file": dest.name,
            "sha256": hashlib.sha256(blob).hexdigest(),
            "bytes": len(blob),
            "note": note,
        })
        print(f"{dest.name:38} {spdx:18} {len(blob):>7} B")
    (OUT / "PROVENANCE.json").write_text(json.dumps({
        "muPDF_tag": "1.28.1",
        "muPDF_submodule_source": "git tree 160000 entries of tag 1.28.1",
        "note": "Pinned to the Artifex thirdparty forks/builds used by MuPDF, not upstream HEAD.",
        "fetched_on": "2026-09-15",
        "files": records,
    }, indent=2) + "\n")


if __name__ == "__main__":
    main()
