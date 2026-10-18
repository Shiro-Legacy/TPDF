# AGPL Corresponding Source — what exists, what does not

> Licence note (October 18, 2026): the Rust engine and the app are now MIT, and no MuPDF code ships. Where this document says engine code is AGPL or MuPDF-derived, or that a build owes AGPL obligations, it records the state before the clean-room relicense (see the Licence section of [`PROVENANCE.md`](../../engine/reference/PROVENANCE.md)).

Owner: ds2. Written 2026-09-15 for the macOS package (`out/PDF4All-P0-darwin-arm64`); app-licence
row corrected the same day when the user chose MIT.
**Facts and verified URLs only, no legal conclusions, no licence choice made here.** Nothing in
this file is distribution sign-off; it is the inventory of the source material plus the
reproduction recipe.

Evidence tags: **[V-url]** URL fetched today and hashed, **[V-src]** read out of the official
source tarball, **[V-bin]** read out of the artifact we ship, **[V-tree]** read out of
`node_modules/`, **[V-api]** upstream git metadata via the GitHub API, **[?]** uncertain.

## 1. The copyleft components inside the macOS package

| Component | Licence | Where it is in the package | Source |
|---|---|---|---|
| **MuPDF 1.28.1** | AGPL-3.0-or-later [V-tree: `node_modules/mupdf/package.json`, `LICENSE`] | `app.asar:/build/renderer/vendor/mupdf-wasm.wasm` (10,409,826 B), `mupdf.js`, `mupdf-wasm.js`; notice at `app.asar:/licenses/MuPDF-AGPL-3.0.txt` | §2 below |
| **jbig2dec** | AGPL-3.0-or-later [V-bin: 62 distinct `jbig2_*` symbols in the wasm; pinned notices in `lib/`] | compiled **into** `mupdf-wasm.wasm`; no separate file | §3 below |
| the app's own source (`src/`, `scripts/`, `fixtures/` generators) | **MIT** — `LICENSE` at the repo root (`Copyright (c) 2026 Shiro-Legacy`), `package.json` `"license": "MIT"`, staged by `scripts/package.mjs` as `licenses/PDF4All-MIT.txt` [V-tree]. It is the AGPL component it ships with, not the app licence, that this document is about: the same bundle embeds AGPL MuPDF + jbig2dec, so the source material inventoried below is still required for the build we ship | repo root; not yet assembled as a packet |

**No other AGPL/GPL component is in the package.** `mupdf` is the only entry in
`package-lock.json` whose licence matches `AGPL|LGPL|GPL|MPL` (verified by scanning all 75
installed entries: the rest are MIT, Apache-2.0, BlueOak-1.0.0, ISC and BSD-2-Clause) [V-tree].
Inside the wasm the copyleft set is
MuPDF + jbig2dec; the other linked libraries (harfbuzz MIT, lcms2mt MIT, freetype FTL, libjpeg IJG,
openjpeg BSD-2, gumbo Apache-2.0, zlib, cmark-gfm MIT, brotli MIT) are permissive
(`docs/third-party/mupdf-wasm-assets.md` §4). FreeType is dual-licensed "FTL or GPLv2" and **we
take FTL** [V-tree: `lib/freetype-docs_FTL-TXT.txt`], so no GPL option is exercised. MuJS is
**not** compiled in. The LGPL parts of Electron/Chromium are a separate document:
`lgpl-relink.md`.

## 2. MuPDF source material, exact and verified

### 2.1 Official source release (the primary reference)

| Item | Value |
|---|---|
| URL (Artifex archive; 307-redirects to `casper.mupdf.com`) | `https://mupdf.com/downloads/archive/mupdf-1.28.1-source.tar.gz` |
| Final URL after redirect | `https://casper.mupdf.com/downloads/archive/mupdf-1.28.1-source.tar.gz` |
| Size | 68,898,834 bytes |
| **sha256** | `dc94c60b2537e2ac9a2d379dd3801545f84a3a302d15c9da358362a1270707c3` |
| sha1 | `411049ab29a66f39db53684c35d42153523ea46d` |
| Contents | `mupdf-1.28.1-source/`, `include/mupdf/fitz/version.h` says `FZ_VERSION "1.28.1"` [V-src] |
| Third party | **self-contained**: `thirdparty/` is populated (7,171 entries), not empty submodules [V-src] |

All of the above was fetched and hashed today [V-url]. The earlier `mupdf-1.28.1-source.tar.gz`
request without following redirects returns a 15-byte `Redirecting...` body — use `curl -L`.

### 2.2 Git identity (ties the npm build to the tag)

| Item | Value | Evidence |
|---|---|---|
| Upstream repo (declared by the package) | `https://cgit.ghostscript.com/mupdf.git/` | [V-tree: `node_modules/mupdf/package.json`] |
| Mirror | `https://github.com/ArtifexSoftware/mupdf` | [V-url] |
| Tag `1.28.1` (lightweight, points at a commit) | commit `20061bd45183f5a2bff8f43675e4da91e5ac2901` | [V-api: `git/ref/tags/1.28.1`; cgit `tag/?h=1.28.1` shows the same id] |
| `gitHead` declared by the npm tarball | `20061bd45183f5a2bff8f43675e4da91e5ac2901` — identical to the tag | [V-api: registry metadata] |
| Tag tarball (not used as the reference: GitHub archives are re-gzipped and not byte-stable) | `https://github.com/ArtifexSoftware/mupdf/archive/refs/tags/1.28.1.tar.gz` | [V-url: 200] |

So the released npm artifact *declares* it was built from the 1.28.1 commit. That declaration is
metadata, not proof [V-api] — see §5.

### 2.3 npm package we install

| Item | Value |
|---|---|
| Package | `mupdf@1.28.1`, `license: AGPL-3.0-or-later` |
| Tarball | `https://registry.npmjs.org/mupdf/-/mupdf-1.28.1.tgz` |
| Registry `dist.shasum` | `c66d2714bafa1f5c14e076504ed43d5e07bbb3e7` — matches the tarball we downloaded [V-url] |
| sha256 of that tarball | `e6bd485e9bfa4c618f02beea042fdcee0e6397e1a70cc8fb6baa55095b6d7601` |
| Registry `dist.integrity` | `sha512-Gi11Ow2G1SlrXKJNZBL1eAIGFVih5+4ZKqjptamTVaj/5hnlrcVrVbyb7lHE2lfFKdxZPyv9ZtfOOq7XgjzEig==` (identical to `package-lock.json` [V-tree]) |
| Contents (11 files) | `package/{LICENSE,README.md,package.json,dist/*}` — **no `tools/`, no build script** [V-url: `tar tzf`] |
| `prepack` script in the published `package.json` | `bash tools/build.sh && bash tools/compress.sh` — scripts the tarball does not contain [V-tree] |

### 2.4 The shipped artifacts, hashed

Read out of `node_modules/mupdf/dist/` — byte-identical to the copies inside both packages
(`.quad/shared/p0/p0-package-license-check.md` §2) [V-bin]:

| File | sha256 |
|---|---|
| `mupdf-wasm.wasm` | `5a30ef7b027f541ea8fc54e7c73f16414b0b59940741a12efe5e55f1fd0a99d7` |
| `mupdf.js` | `5d237eadcb9640584e0050f6f958e547887b7a54d80a04043e12be9420931076` |
| `mupdf-wasm.js` | `25382abbfbf5c52191e5ac59e1412a0da88647fecfab0c1d797bb5948443eea4` |

## 3. jbig2dec

| Item | Value |
|---|---|
| Repo | `https://github.com/ArtifexSoftware/jbig2dec` |
| Commit used by MuPDF tag 1.28.1 | `dc15c39bbbddc90f79c14563d2eb5a794106be8f` [V-api: gitlink 160000 entry at `thirdparty/jbig2dec`, tag 1.28.1] |
| Commit page / archive | `…/commit/dc15c39bbbddc90f79c14563d2eb5a794106be8f` (200), `…/archive/dc15c39….tar.gz` (151,391 B, 200) [V-url] |
| Vendored copy inside the MuPDF tarball | `thirdparty/jbig2dec/` (62 entries) — its `COPYING` and `LICENSE` are **byte-identical** (sha256 `57c8ff33…`, `1bf5258a…`) to the notices pinned in `lib/PROVENANCE.json` [V-src] |
| Notices already assembled | `lib/jbig2dec-COPYING.txt`, `lib/jbig2dec-LICENSE.txt` |

**Cross-check of the whole pinning table [V-api vs V-tree]:** all 10 repository pins recorded in
`docs/third-party/lib/PROVENANCE.json` (freetype, harfbuzz, lcms2, libjpeg, openjpeg, gumbo, zlib,
jbig2dec, brotli, cmark-gfm) match the live `thirdparty/*` gitlinks at tag 1.28.1, and all 11 of
their notice files match the vendored files in the tarball by sha256. The tag pins 18 submodules
in total; the other eight (`mujs`, `curl`, `extract`, `freeglut`, `leptonica`, `tesseract`,
`zint`, `zxing-cpp`) are not compiled into this build (`mupdf-wasm-assets.md` §4–5).

## 4. What a recipient needs to rebuild `mupdf-wasm.wasm`

The authority is `platform/wasm/tools/build.sh` at tag 1.28.1 —

| Item | Value |
|---|---|
| Path in the tarball | `mupdf-1.28.1-source/platform/wasm/tools/build.sh` |
| sha256 | `31f7c6a93f75938cb0f0a4dd781c1c87f33e71302bb69f5cdfaba1242c24d1a6` |
| Also needed | `platform/wasm/Makefile`, `BUILDING.md`, `package.json`, `lib/mupdf.c`, `lib/mupdf.ts`, `tools/make-wasm-type.js`, `tools/compress.sh` — all present in the tarball [V-src] |
| Pin | emsdk `4.0.8` (`emsdk install 4.0.8 && emsdk activate 4.0.8`) |
| Defaults the script uses | `BUILD=small`, `DEFINES="-DTOFU -DTOFU_CJK_EXT -DFZ_ENABLE_HYPHEN=0"`, `FEATURES="brotli=no mujs=no extract=no xps=no svg=no"` |
| Build steps | `make … OS=wasm XCFLAGS="$DEFINES" $FEATURES libs` → `emcc … -Os -g2 --no-entry -mno-nontrapping-fptoint -fwasm-exceptions -sSUPPORT_LONGJMP=wasm -sMODULARIZE=1 -sEXPORT_ES6=1 -sEXPORT_NAME='"libmupdf_wasm"' -sALLOW_MEMORY_GROWTH=1 -sTEXTDECODER=2 -sFILESYSTEM=0 -sEXPORTED_RUNTIME_METHODS=… lib/mupdf.c build/wasm/$BUILD/libmupdf.a build/wasm/$BUILD/libmupdf-third.a` → `make-wasm-type.js` for the `.d.ts` → `npx tsc -p .` → `sed` spaces→tabs → `npx terser --module -c -m` |

`docs/third-party/mupdf-wasm-assets.md` §1 lists the same pins but only a condensed subset of the
link flags (it omits `-mno-nontrapping-fptoint`, `-sSUPPORT_LONGJMP=wasm`, `-sEXPORT_NAME`,
`-sTEXTDECODER=2`, `-sEXPORTED_RUNTIME_METHODS`); the script hash above, not that table, is the
authority. The tarball's `thirdparty/README` states the release tarball's third-party sources are
"stripped down for size" relative to the git submodules [V-src] — a rebuild from the tarball uses
the stripped trees.

A recipient who rebuilds gets `dist/{mupdf.js,mupdf-wasm.js,mupdf-wasm.wasm}` from that recipe.
They should expect the bytes to differ from ours: nothing in the tarball records the compiler
environment, timestamps or path prefixes of Artifex's release build, so there is no hash to
compare against.

## 5. What we cannot currently provide

- **No source packet is shipped with the package.** `out/PDF4All-P0-darwin-arm64/` contains the
  `.app`, Electron's `LICENSE` and `LICENSES.chromium.html`; it does not contain MuPDF source and
  carries `PRIVATE-SPIKE.txt` ("do not redistribute this spike build"). Providing the
  corresponding source — by shipping it or by a written offer — is what makes a distribution
  compliant; that step has not happened and is what this file exists to make possible.
- **We did not build the wasm ourselves and have not reproduced the build.** Everything in §4 is
  the upstream recipe; no one on this project has run it, so we cannot state that running it
  yields a byte-identical or functionally identical `mupdf-wasm.wasm`.
- **The only link between the npm artifact and the tag is the `gitHead` field** Artifex publishes
  [V-api]. There is no signature, SBOM or build log in the tarball that independently proves the
  release was produced from commit `20061bd4…`.
- **The app's own source is not assembled as a packet.** The project code is MIT (`LICENSE` at
  the repo root, staged into the package as `licenses/PDF4All-MIT.txt`) — but the bundle also
  embeds AGPL MuPDF + jbig2dec, and this repository is not yet prepared, versioned or offered as
  the Corresponding Source for that combination. `PRIVATE-SPIKE.txt` inside the package now reads:
  "PDF4All code is MIT (licenses/PDF4All-MIT.txt); the bundled MuPDF is AGPL-3.0-or-later, so any
  distribution of this build must meet AGPL obligations including corresponding source; do not
  redistribute this spike build."
- **`package-lock.json` still records `"license": "UNLICENSED"`** in its root entry while
  `package.json` says `MIT` — the lockfile needs an `npm install` to catch up [V-tree].
- **No decision on how to fulfil the source obligation** (bundle the ~66 MB tarball, or offer it),
  and no mirror of the tarball we control. If the upstream URL disappears, we have no copy.
- **Windows output untested at runtime.** The same source reasoning applies to
  `out/PDF4All-P0-win32-x64`, but that package was cross-built and never executed
  (`p0-package-license-check.md` F6).

## 6. Re-check commands

```bash
# 1. the official source tarball, size and hash
curl -sSL -o mupdf-src.tar.gz https://mupdf.com/downloads/archive/mupdf-1.28.1-source.tar.gz
shasum -a 256 mupdf-src.tar.gz    # dc94c60b2537e2ac9a2d379dd3801545f84a3a302d15c9da358362a1270707c3
tar xzOf mupdf-src.tar.gz mupdf-1.28.1-source/platform/wasm/tools/build.sh | shasum -a 256
                                  # 31f7c6a93f75938cb0f0a4dd781c1c87f33e71302bb69f5cdfaba1242c24d1a6
# 2. tag -> commit, and the npm tarball's declared gitHead
curl -sS https://api.github.com/repos/ArtifexSoftware/mupdf/git/ref/tags/1.28.1
curl -sS https://registry.npmjs.org/mupdf/1.28.1 | python3 -c 'import json,sys;d=json.load(sys.stdin);print(d["gitHead"],d["dist"]["integrity"])'
# 3. the notices against the vendored third-party trees
python3 - <<'EOF'
import json,subprocess
p=json.load(open('docs/third-party/lib/PROVENANCE.json'))['files']
for f in p:
    print(f['library'], f['sha256'][:12], f['pinned_commit'][:12])
EOF
# 4. hashes of the artifacts we actually ship
shasum -a 256 node_modules/mupdf/dist/mupdf-wasm.wasm node_modules/mupdf/dist/mupdf.js
```
